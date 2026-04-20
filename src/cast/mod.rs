//! `(defcast)` — media casting to remote receivers.
//!
//! Absorbs Chromecast (Cast SDK), Apple AirPlay, Microsoft Miracast,
//! and generic DLNA/UPnP into a substrate DSL. Each profile names a
//! discovery strategy, supported protocols, and per-host gating.
//! The actual cast implementation lives outside this crate — nami-
//! core only describes the policy and records discovered receivers.
//!
//! ```lisp
//! (defcast :name           "default"
//!          :protocols      (chromecast airplay miracast dlna)
//!          :discovery      :mdns-and-ssdp
//!          :allowed-hosts  ("*")
//!          :session-timeout-seconds 3600)
//!
//! (defcast :name           "apple-only"
//!          :protocols      (airplay)
//!          :discovery      :mdns
//!          :default-receiver "living-room")
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Cast protocol.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum CastProtocol {
    /// Google Cast (Chromecast receivers).
    Chromecast,
    /// Apple AirPlay 2.
    AirPlay,
    /// Microsoft Miracast (WiFi Direct, no LAN requirement).
    Miracast,
    /// DLNA / UPnP AV — generic smart-TV fallback.
    Dlna,
    /// Web `PresentationRequest` API (receiver is a browser page).
    WebPresentation,
}

/// Discovery transport.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Discovery {
    /// mDNS/Bonjour — Chromecast + AirPlay default.
    Mdns,
    /// SSDP — DLNA + Miracast default.
    Ssdp,
    /// Both mDNS and SSDP in parallel.
    MdnsAndSsdp,
    /// No auto-discovery — the user adds receivers by hostname.
    Manual,
}

impl Default for Discovery {
    fn default() -> Self {
        Self::MdnsAndSsdp
    }
}

/// Cast profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defcast"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CastSpec {
    pub name: String,
    /// Accepted protocols. Empty defaults to all five.
    #[serde(default = "default_protocols")]
    pub protocols: Vec<CastProtocol>,
    #[serde(default)]
    pub discovery: Discovery,
    /// Host globs the profile applies to. Empty = all hosts.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    /// Receivers the user prefers — when multiple are reachable,
    /// offer these first. Matched by name.
    #[serde(default)]
    pub preferred_receivers: Vec<String>,
    /// Default receiver to auto-select when the user clicks "cast"
    /// without picking one. Empty = always prompt.
    #[serde(default)]
    pub default_receiver: Option<String>,
    /// Seconds before an idle session auto-disconnects. `0` = never.
    #[serde(default = "default_session_timeout")]
    pub session_timeout_seconds: u64,
    /// Require user confirmation before starting a cast (anti-auto-play).
    #[serde(default = "default_require_confirm")]
    pub require_confirm: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_protocols() -> Vec<CastProtocol> {
    vec![
        CastProtocol::Chromecast,
        CastProtocol::AirPlay,
        CastProtocol::Miracast,
        CastProtocol::Dlna,
        CastProtocol::WebPresentation,
    ]
}
fn default_session_timeout() -> u64 {
    3600
}
fn default_require_confirm() -> bool {
    true
}

impl CastSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            protocols: default_protocols(),
            discovery: Discovery::MdnsAndSsdp,
            allowed_hosts: vec![],
            preferred_receivers: vec![],
            default_receiver: None,
            session_timeout_seconds: default_session_timeout(),
            require_confirm: true,
            description: Some(
                "Default cast — all protocols, mDNS+SSDP discovery.".into(),
            ),
        }
    }

    #[must_use]
    pub fn supports(&self, p: CastProtocol) -> bool {
        self.protocols.is_empty() || self.protocols.contains(&p)
    }

    /// Is this profile active for `host`? Empty allow-list = every host.
    #[must_use]
    pub fn applies_to(&self, host: &str) -> bool {
        if self.allowed_hosts.is_empty() {
            return true;
        }
        self.allowed_hosts
            .iter()
            .any(|g| crate::extension::glob_match_host(g, host))
    }
}

/// One discovered receiver.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CastReceiver {
    /// Stable id — BLAKE3 over address+protocol, 26-char base32.
    pub id: String,
    /// Friendly name from the receiver's own advertisement.
    pub name: String,
    pub protocol: CastProtocol,
    /// Address — host:port for IP receivers, bluetooth addr for Miracast.
    pub address: String,
    /// Capability strings the receiver advertises (video, audio,
    /// 4k, hdr, atmos, etc.). Vendor-defined.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Seen-at unix seconds; for pruning stale entries.
    #[serde(default)]
    pub seen_at: i64,
    /// Whether the receiver is currently handling our session.
    #[serde(default)]
    pub active: bool,
}

impl CastReceiver {
    /// Does this receiver advertise every capability in `required`?
    #[must_use]
    pub fn supports_all(&self, required: &[&str]) -> bool {
        required
            .iter()
            .all(|r| self.capabilities.iter().any(|c| c == r))
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct CastRegistry {
    specs: Vec<CastSpec>,
}

impl CastRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: CastSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = CastSpec>) {
        for s in specs {
            self.insert(s);
        }
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
    pub fn specs(&self) -> &[CastSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&CastSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// Every profile that applies to `host`.
    #[must_use]
    pub fn applicable(&self, host: &str) -> Vec<&CastSpec> {
        self.specs.iter().filter(|s| s.applies_to(host)).collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<CastSpec>, String> {
    tatara_lisp::compile_typed::<CastSpec>(src)
        .map_err(|e| format!("failed to compile defcast forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<CastSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_supports_all_protocols() {
        let s = CastSpec::default_profile();
        for p in [
            CastProtocol::Chromecast,
            CastProtocol::AirPlay,
            CastProtocol::Miracast,
            CastProtocol::Dlna,
            CastProtocol::WebPresentation,
        ] {
            assert!(s.supports(p));
        }
    }

    #[test]
    fn empty_protocols_list_supports_everything() {
        let s = CastSpec {
            protocols: vec![],
            ..CastSpec::default_profile()
        };
        assert!(s.supports(CastProtocol::Dlna));
    }

    #[test]
    fn applies_to_empty_hosts_is_every_host() {
        assert!(CastSpec::default_profile().applies_to("anywhere.com"));
    }

    #[test]
    fn applies_to_filters_by_host_glob() {
        let s = CastSpec {
            allowed_hosts: vec!["*://*.youtube.com/*".into()],
            ..CastSpec::default_profile()
        };
        assert!(s.applies_to("www.youtube.com"));
        assert!(!s.applies_to("evil.com"));
    }

    #[test]
    fn receiver_supports_all_honors_cap_list() {
        let r = CastReceiver {
            id: "abc".into(),
            name: "Living Room".into(),
            protocol: CastProtocol::Chromecast,
            address: "10.0.0.42:8009".into(),
            capabilities: vec!["video".into(), "audio".into(), "4k".into()],
            seen_at: 0,
            active: false,
        };
        assert!(r.supports_all(&["video", "audio"]));
        assert!(r.supports_all(&["4k"]));
        assert!(!r.supports_all(&["atmos"]));
    }

    #[test]
    fn registry_dedupes_by_name_and_filters_by_host() {
        let mut reg = CastRegistry::new();
        reg.insert(CastSpec::default_profile());
        reg.insert(CastSpec {
            name: "yt-only".into(),
            allowed_hosts: vec!["*://*.youtube.com/*".into()],
            ..CastSpec::default_profile()
        });
        assert_eq!(reg.len(), 2);
        let on_yt: Vec<&str> = reg
            .applicable("www.youtube.com")
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(on_yt.len(), 2);
        let on_other: Vec<&str> = reg
            .applicable("example.com")
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(on_other, vec!["default"]);
    }

    #[test]
    fn discovery_default_is_parallel_mdns_ssdp() {
        assert_eq!(Discovery::default(), Discovery::MdnsAndSsdp);
    }

    #[test]
    fn protocol_and_discovery_roundtrip_through_serde() {
        for p in [
            CastProtocol::Chromecast,
            CastProtocol::AirPlay,
            CastProtocol::Miracast,
            CastProtocol::Dlna,
            CastProtocol::WebPresentation,
        ] {
            let s = CastSpec {
                protocols: vec![p],
                ..CastSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: CastSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.protocols, vec![p]);
        }
    }

    #[test]
    fn default_session_timeout_and_confirm_flag() {
        let s = CastSpec::default_profile();
        assert_eq!(s.session_timeout_seconds, 3600);
        assert!(s.require_confirm);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_cast_form() {
        let src = r#"
            (defcast :name "apple-only"
                     :protocols ("air-play")
                     :discovery "mdns"
                     :default-receiver "living-room"
                     :require-confirm #f)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "apple-only");
        assert!(s.supports(CastProtocol::AirPlay));
        assert!(!s.supports(CastProtocol::Chromecast));
        assert_eq!(s.discovery, Discovery::Mdns);
        assert_eq!(s.default_receiver.as_deref(), Some("living-room"));
        assert!(!s.require_confirm);
    }
}
