//! `(defbridge)` — Tor bridge + pluggable-transport endpoints.
//!
//! Absorbs Tor Browser's bridge configuration UI, Snowflake, Meek,
//! Obfs4/4proxy, and WebTunnel into a substrate DSL. Each spec names
//! one bridge line; the routing engine ([`crate::routing`]) references
//! bridge names via `via: "pt:<name>"`.
//!
//! ```lisp
//! (defbridge :name        "snowflake-1"
//!            :transport   :snowflake
//!            :address     "192.0.2.3:80"
//!            :fingerprint "2B280B23E1107BB62ABFC40DDCC8824814F80A72"
//!            :extra       "ice=stun:stun.l.google.com:19302 url=https://snowflake-broker.torproject.net/")
//!
//! (defbridge :name        "obfs4-eu-1"
//!            :transport   :obfs4
//!            :address     "198.51.100.7:443"
//!            :fingerprint "A09D536DD1752D542E1FBB3C9CE4449D51298239"
//!            :extra       "cert=…,iat-mode=0")
//!
//! (defbridge :name        "meek-azure"
//!            :transport   :meek
//!            :address     "0.0.2.20:80"
//!            :fingerprint "97772A3B46631F1F…"
//!            :extra       "url=https://ajax.aspnetcdn.com/ front=ajax.aspnetcdn.com")
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Pluggable-transport protocol.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BridgeTransport {
    /// Direct Tor (no PT) — vanilla relay.
    Direct,
    /// obfs4proxy.
    Obfs4,
    /// Meek (HTTPS over fronted CDN).
    Meek,
    /// Snowflake (WebRTC).
    Snowflake,
    /// WebTunnel (HTTP/2 HTTPS disguise).
    Webtunnel,
    /// Experimental / future transport — opaque string.
    Other,
}

impl Default for BridgeTransport {
    fn default() -> Self {
        Self::Direct
    }
}

/// One bridge entry.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defbridge"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BridgeSpec {
    pub name: String,
    #[serde(default)]
    pub transport: BridgeTransport,
    /// `host:port` for Direct/Obfs4/Meek/Webtunnel; freeform for
    /// Snowflake (broker URL typically goes in `extra`).
    pub address: String,
    /// 40-char hex Tor relay fingerprint. Technically optional for
    /// Snowflake but enforced for fixed-address PTs.
    #[serde(default)]
    pub fingerprint: Option<String>,
    /// Transport-specific bridge-line tail — obfs4 `cert=…,iat-mode=…`,
    /// meek `url=… front=…`, snowflake broker params, etc.
    #[serde(default)]
    pub extra: Option<String>,
    /// Runtime toggle — disabled bridges stay registered but aren't
    /// selectable. Supports A/B-testing bridge pools.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_enabled() -> bool {
    true
}

impl BridgeSpec {
    /// Structural validation: address required, fingerprint shape
    /// enforced when present (40 hex chars), transport-specific
    /// fingerprint requirement for Direct/Obfs4/Meek/Webtunnel.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("bridge name is empty".into());
        }
        if self.address.trim().is_empty() {
            return Err(format!("bridge '{}' has empty :address", self.name));
        }
        match self.transport {
            BridgeTransport::Direct
            | BridgeTransport::Obfs4
            | BridgeTransport::Meek
            | BridgeTransport::Webtunnel => {
                if self.fingerprint.is_none() {
                    return Err(format!(
                        "bridge '{}' ({:?}) requires :fingerprint",
                        self.name, self.transport
                    ));
                }
            }
            BridgeTransport::Snowflake | BridgeTransport::Other => {
                // Fingerprint optional.
            }
        }
        if let Some(fp) = &self.fingerprint {
            if !is_valid_fingerprint(fp) {
                return Err(format!(
                    "bridge '{}' has malformed fingerprint: {fp:?}",
                    self.name
                ));
            }
        }
        Ok(())
    }

    /// Format this bridge as a `torrc Bridge` line — the format Tor
    /// itself consumes. Used by callers that shell into a Tor daemon.
    #[must_use]
    pub fn to_torrc_line(&self) -> String {
        let transport_token = match self.transport {
            BridgeTransport::Direct => String::new(),
            BridgeTransport::Obfs4 => "obfs4 ".into(),
            BridgeTransport::Meek => "meek_lite ".into(),
            BridgeTransport::Snowflake => "snowflake ".into(),
            BridgeTransport::Webtunnel => "webtunnel ".into(),
            BridgeTransport::Other => String::new(),
        };
        let fp = self.fingerprint.as_deref().unwrap_or("").trim().to_owned();
        let extra = self.extra.as_deref().unwrap_or("").trim().to_owned();
        let mut out = format!("{transport_token}{} {fp}", self.address);
        if !extra.is_empty() {
            out.push(' ');
            out.push_str(&extra);
        }
        out.trim().to_owned()
    }
}

fn is_valid_fingerprint(s: &str) -> bool {
    let cleaned = s.replace(' ', "");
    cleaned.len() == 40 && cleaned.chars().all(|c| c.is_ascii_hexdigit())
}

/// Registry. Rejects invalid specs on insert.
#[derive(Debug, Clone, Default)]
pub struct BridgeRegistry {
    specs: Vec<BridgeSpec>,
}

impl BridgeRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: BridgeSpec) -> Result<(), String> {
        spec.validate()?;
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
        Ok(())
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = BridgeSpec>) {
        for s in specs {
            if let Err(e) = self.insert(s.clone()) {
                tracing::warn!("defbridge '{}' rejected: {}", s.name, e);
            }
        }
    }

    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> bool {
        for s in &mut self.specs {
            if s.name == name {
                s.enabled = enabled;
                return true;
            }
        }
        false
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    #[must_use]
    pub fn specs(&self) -> &[BridgeSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&BridgeSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// Every enabled bridge with transport == `t`.
    #[must_use]
    pub fn by_transport(&self, t: BridgeTransport) -> Vec<&BridgeSpec> {
        self.specs
            .iter()
            .filter(|s| s.enabled && s.transport == t)
            .collect()
    }

    /// Full torrc `Bridge …` configuration block — one line per
    /// enabled bridge. Caller pipes this into a Tor instance.
    #[must_use]
    pub fn to_torrc_block(&self) -> String {
        self.specs
            .iter()
            .filter(|s| s.enabled)
            .map(|s| format!("Bridge {}", s.to_torrc_line()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<BridgeSpec>, String> {
    tatara_lisp::compile_typed::<BridgeSpec>(src)
        .map_err(|e| format!("failed to compile defbridge forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<BridgeSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, transport: BridgeTransport, addr: &str, fp: Option<&str>) -> BridgeSpec {
        BridgeSpec {
            name: name.into(),
            transport,
            address: addr.into(),
            fingerprint: fp.map(str::to_owned),
            extra: None,
            enabled: true,
            description: None,
        }
    }

    const VALID_FP: &str = "A09D536DD1752D542E1FBB3C9CE4449D51298239";

    #[test]
    fn validates_full_obfs4_form() {
        let s = BridgeSpec {
            extra: Some("cert=abc,iat-mode=0".into()),
            ..sample("b", BridgeTransport::Obfs4, "198.51.100.7:443", Some(VALID_FP))
        };
        assert!(s.validate().is_ok());
    }

    #[test]
    fn rejects_empty_address() {
        let s = sample("b", BridgeTransport::Direct, "", Some(VALID_FP));
        assert!(s.validate().is_err());
    }

    #[test]
    fn rejects_empty_name() {
        let s = sample("", BridgeTransport::Direct, "1.2.3.4:9001", Some(VALID_FP));
        assert!(s.validate().is_err());
    }

    #[test]
    fn obfs4_without_fingerprint_rejected() {
        let s = sample("b", BridgeTransport::Obfs4, "1.2.3.4:443", None);
        assert!(s.validate().is_err());
    }

    #[test]
    fn snowflake_without_fingerprint_ok() {
        let s = sample("b", BridgeTransport::Snowflake, "192.0.2.3:80", None);
        assert!(s.validate().is_ok());
    }

    #[test]
    fn malformed_fingerprint_rejected() {
        let s = sample("b", BridgeTransport::Direct, "1.2.3.4:9001", Some("NOTAHEX"));
        assert!(s.validate().is_err());
    }

    #[test]
    fn fingerprint_with_spaces_normalizes() {
        // Real torrc files often space-separate the fingerprint.
        let mut fp_spaced = String::new();
        for (i, c) in VALID_FP.chars().enumerate() {
            if i > 0 && i % 4 == 0 {
                fp_spaced.push(' ');
            }
            fp_spaced.push(c);
        }
        let s = sample("b", BridgeTransport::Direct, "1.2.3.4:9001", Some(&fp_spaced));
        assert!(s.validate().is_ok());
    }

    #[test]
    fn to_torrc_line_obfs4_roundtrip() {
        let s = BridgeSpec {
            extra: Some("cert=xyz,iat-mode=0".into()),
            ..sample("b", BridgeTransport::Obfs4, "198.51.100.7:443", Some(VALID_FP))
        };
        let line = s.to_torrc_line();
        assert!(line.starts_with("obfs4 "));
        assert!(line.contains("198.51.100.7:443"));
        assert!(line.contains(VALID_FP));
        assert!(line.contains("cert=xyz"));
    }

    #[test]
    fn to_torrc_line_direct_has_no_transport_prefix() {
        let s = sample("b", BridgeTransport::Direct, "1.2.3.4:9001", Some(VALID_FP));
        let line = s.to_torrc_line();
        assert!(!line.starts_with("obfs4"));
        assert!(line.starts_with("1.2.3.4"));
    }

    #[test]
    fn registry_insert_validates() {
        let mut reg = BridgeRegistry::new();
        assert!(reg.insert(sample("a", BridgeTransport::Obfs4, "", None)).is_err());
        assert!(reg
            .insert(sample("a", BridgeTransport::Obfs4, "1.2.3.4:443", Some(VALID_FP)))
            .is_ok());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn registry_extend_drops_invalid() {
        let mut reg = BridgeRegistry::new();
        reg.extend(vec![
            sample("good", BridgeTransport::Snowflake, "192.0.2.3:80", None),
            sample("bad", BridgeTransport::Obfs4, "1.2.3.4:443", None),
        ]);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].name, "good");
    }

    #[test]
    fn by_transport_filter() {
        let mut reg = BridgeRegistry::new();
        reg.insert(sample("sf1", BridgeTransport::Snowflake, "192.0.2.3:80", None))
            .unwrap();
        reg.insert(sample(
            "o1",
            BridgeTransport::Obfs4,
            "1.2.3.4:443",
            Some(VALID_FP),
        ))
        .unwrap();
        assert_eq!(reg.by_transport(BridgeTransport::Snowflake).len(), 1);
        assert_eq!(reg.by_transport(BridgeTransport::Obfs4).len(), 1);
    }

    #[test]
    fn torrc_block_combines_enabled_bridges() {
        let mut reg = BridgeRegistry::new();
        reg.insert(sample(
            "a",
            BridgeTransport::Obfs4,
            "1.2.3.4:443",
            Some(VALID_FP),
        ))
        .unwrap();
        reg.insert(sample(
            "b",
            BridgeTransport::Snowflake,
            "192.0.2.3:80",
            None,
        ))
        .unwrap();
        reg.set_enabled("a", false);
        let block = reg.to_torrc_block();
        assert!(!block.contains("1.2.3.4:443"));
        assert!(block.contains("192.0.2.3:80"));
        assert!(block.lines().all(|l| l.starts_with("Bridge ")));
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_bridge_form() {
        let src = format!(
            r#"
            (defbridge :name        "b"
                       :transport   "obfs4"
                       :address     "198.51.100.7:443"
                       :fingerprint "{VALID_FP}"
                       :extra       "cert=xyz,iat-mode=0")
        "#
        );
        let specs = compile(&src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.transport, BridgeTransport::Obfs4);
        assert_eq!(s.address, "198.51.100.7:443");
        assert_eq!(s.fingerprint.as_deref(), Some(VALID_FP));
    }
}
