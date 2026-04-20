//! `(defdns)` — declarative DNS-resolver preference.
//!
//! Absorbs Firefox DNS-over-HTTPS (DoH), Chrome DoH, iOS Private
//! Relay-style DNS settings, and pleme-io's [kurayami](
//! https://github.com/pleme-io/kurayami) privacy-DNS resolver into
//! one substrate DSL. Each spec names a resolver profile — the
//! upstream endpoint, protocol, caching policy, and bootstrap
//! server (how to resolve the resolver's own hostname without a
//! chicken-and-egg loop).
//!
//! ```lisp
//! (defdns :name      "cloudflare-doh"
//!         :protocol  :doh
//!         :endpoint  "https://1.1.1.1/dns-query"
//!         :bootstrap "1.1.1.1"
//!         :cache-ttl-seconds 300
//!         :privacy-level :standard)
//!
//! (defdns :name      "mullvad-doq"
//!         :protocol  :doq
//!         :endpoint  "quic://dns.mullvad.net"
//!         :bootstrap "194.242.2.2"
//!         :privacy-level :strict)
//!
//! (defdns :name      "local-unbound"
//!         :protocol  :system
//!         :privacy-level :legacy)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// DNS transport protocol. Matches the kurayami vocabulary so the
/// substrate spec can be fed straight into kurayami's resolver loop.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DnsProtocol {
    /// Defer to the OS resolver (stock `/etc/resolv.conf` flow).
    System,
    /// Plain UDP — legacy, plaintext, avoid.
    Udp,
    /// DNS-over-TLS (RFC 7858).
    Dot,
    /// DNS-over-HTTPS (RFC 8484).
    Doh,
    /// DNS-over-QUIC (RFC 9250).
    Doq,
    /// Anonymized DNS via dnscrypt-proxy relays.
    AnonymizedDnscrypt,
    /// ODoH (Oblivious DoH, RFC 9230).
    Odoh,
}

impl Default for DnsProtocol {
    fn default() -> Self {
        Self::System
    }
}

/// Privacy tier — informs what optional behaviors to enable (0-RTT
/// replay, cached-key reuse, happy-eyeballs fallback to v4, etc.).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PrivacyLevel {
    /// Happily uses plaintext fallback, IPv4 first, classic behavior.
    Legacy,
    /// DoH/DoT preferred but fallback to system resolver on error.
    Standard,
    /// Refuse plaintext; error-out when secure channel unavailable.
    Strict,
    /// Strict + circuit-isolated per query (pairs with kakuremino).
    Isolated,
}

impl Default for PrivacyLevel {
    fn default() -> Self {
        Self::Standard
    }
}

/// DNS resolver profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defdns"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DnsSpec {
    pub name: String,
    #[serde(default)]
    pub protocol: DnsProtocol,
    /// Resolver endpoint URL (protocol-dependent form). Empty when
    /// protocol = System.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Bootstrap IP (literal) — avoids a circular DNS lookup for
    /// the resolver's own hostname. Required for DoH/DoQ/DoT unless
    /// `endpoint` is already an IP literal.
    #[serde(default)]
    pub bootstrap: Option<String>,
    /// Cache TTL in seconds (clamp on answers with shorter TTLs).
    /// `0` = honor upstream TTL verbatim.
    #[serde(default)]
    pub cache_ttl_seconds: u64,
    #[serde(default)]
    pub privacy_level: PrivacyLevel,
    /// Refuse lookups for these domain suffixes — block-list style.
    /// Pairs with (defblocker) at the URL layer.
    #[serde(default)]
    pub blocked_suffixes: Vec<String>,
    /// Suffixes that bypass the secure resolver (e.g. ".local").
    #[serde(default)]
    pub bypass_suffixes: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl DnsSpec {
    /// System resolver — the "do nothing clever" default.
    #[must_use]
    pub fn system_default() -> Self {
        Self {
            name: "system".into(),
            protocol: DnsProtocol::System,
            endpoint: None,
            bootstrap: None,
            cache_ttl_seconds: 0,
            privacy_level: PrivacyLevel::Legacy,
            blocked_suffixes: vec![],
            bypass_suffixes: vec![".local".into(), ".localhost".into()],
            description: Some("System resolver — OS default.".into()),
        }
    }

    /// Cloudflare 1.1.1.1 over DoH. Sensible out-of-the-box privacy.
    #[must_use]
    pub fn cloudflare_doh() -> Self {
        Self {
            name: "cloudflare-doh".into(),
            protocol: DnsProtocol::Doh,
            endpoint: Some("https://1.1.1.1/dns-query".into()),
            bootstrap: Some("1.1.1.1".into()),
            cache_ttl_seconds: 300,
            privacy_level: PrivacyLevel::Standard,
            blocked_suffixes: vec![],
            bypass_suffixes: vec![".local".into(), ".localhost".into()],
            description: Some("Cloudflare 1.1.1.1 over DoH.".into()),
        }
    }

    /// Basic structural checks — no DoH without an endpoint, no
    /// secure protocols without bootstrap when endpoint is a hostname.
    pub fn validate(&self) -> Result<(), String> {
        match self.protocol {
            DnsProtocol::System => {
                // System mode ignores endpoint/bootstrap. No validation.
                Ok(())
            }
            _ if self.endpoint.is_none() => {
                Err(format!("{:?} requires :endpoint", self.protocol))
            }
            _ => {
                // Secure protocols: if endpoint isn't an IP literal,
                // we need a bootstrap to break the DNS loop.
                if let Some(ep) = &self.endpoint {
                    if !is_ip_literal_host(ep) && self.bootstrap.is_none() {
                        return Err(format!(
                            "{:?} with hostname endpoint {ep:?} requires :bootstrap",
                            self.protocol
                        ));
                    }
                }
                Ok(())
            }
        }
    }

    /// Should this resolver handle `suffix`? False when the suffix
    /// matches `bypass_suffixes` (e.g. `.local` goes to system).
    #[must_use]
    pub fn handles_suffix(&self, name: &str) -> bool {
        let n = name.to_ascii_lowercase();
        for s in &self.bypass_suffixes {
            if n.ends_with(&s.to_ascii_lowercase()) {
                return false;
            }
        }
        true
    }

    /// Is `name` on the blocklist?
    #[must_use]
    pub fn is_blocked(&self, name: &str) -> bool {
        let n = name.to_ascii_lowercase();
        self.blocked_suffixes
            .iter()
            .any(|s| n.ends_with(&s.to_ascii_lowercase()))
    }
}

fn is_ip_literal_host(url: &str) -> bool {
    // Quick structural heuristic — doesn't need to be URL-perfect.
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let host = after_scheme
        .split(|c| c == '/' || c == ':')
        .next()
        .unwrap_or("");
    host.parse::<std::net::IpAddr>().is_ok()
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct DnsRegistry {
    specs: Vec<DnsSpec>,
}

impl DnsRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: DnsSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = DnsSpec>) {
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
    pub fn specs(&self) -> &[DnsSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&DnsSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<DnsSpec>, String> {
    tatara_lisp::compile_typed::<DnsSpec>(src)
        .map_err(|e| format!("failed to compile defdns forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<DnsSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_default_validates() {
        assert!(DnsSpec::system_default().validate().is_ok());
    }

    #[test]
    fn cloudflare_doh_validates() {
        assert!(DnsSpec::cloudflare_doh().validate().is_ok());
    }

    #[test]
    fn doh_requires_endpoint() {
        let s = DnsSpec {
            protocol: DnsProtocol::Doh,
            endpoint: None,
            ..DnsSpec::system_default()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn doh_with_hostname_requires_bootstrap() {
        let s = DnsSpec {
            protocol: DnsProtocol::Doh,
            endpoint: Some("https://dns.example.com/dns-query".into()),
            bootstrap: None,
            ..DnsSpec::system_default()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn doh_with_ip_literal_needs_no_bootstrap() {
        let s = DnsSpec {
            protocol: DnsProtocol::Doh,
            endpoint: Some("https://1.1.1.1/dns-query".into()),
            bootstrap: None,
            ..DnsSpec::system_default()
        };
        assert!(s.validate().is_ok());
    }

    #[test]
    fn bypass_suffix_excludes_local_names() {
        let s = DnsSpec::cloudflare_doh();
        assert!(!s.handles_suffix("printer.local"));
        assert!(!s.handles_suffix("workstation.localhost"));
        assert!(s.handles_suffix("example.com"));
    }

    #[test]
    fn bypass_is_case_insensitive() {
        let s = DnsSpec::cloudflare_doh();
        assert!(!s.handles_suffix("Foo.LOCAL"));
    }

    #[test]
    fn blocked_suffix_check() {
        let s = DnsSpec {
            blocked_suffixes: vec![".tracker.net".into(), ".ad.com".into()],
            ..DnsSpec::cloudflare_doh()
        };
        assert!(s.is_blocked("pixel.tracker.net"));
        assert!(s.is_blocked("banner.ad.com"));
        assert!(!s.is_blocked("example.com"));
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = DnsRegistry::new();
        reg.insert(DnsSpec::cloudflare_doh());
        reg.insert(DnsSpec {
            cache_ttl_seconds: 60,
            ..DnsSpec::cloudflare_doh()
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("cloudflare-doh").unwrap().cache_ttl_seconds, 60);
    }

    #[test]
    fn protocol_roundtrips_through_serde() {
        for p in [
            DnsProtocol::System,
            DnsProtocol::Doh,
            DnsProtocol::Doq,
            DnsProtocol::Dot,
            DnsProtocol::Odoh,
        ] {
            let s = DnsSpec {
                protocol: p,
                ..DnsSpec::system_default()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: DnsSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.protocol, p);
        }
    }

    #[test]
    fn privacy_level_default_is_standard() {
        assert_eq!(PrivacyLevel::default(), PrivacyLevel::Standard);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_dns_form() {
        let src = r#"
            (defdns :name      "cf"
                    :protocol  "doh"
                    :endpoint  "https://1.1.1.1/dns-query"
                    :bootstrap "1.1.1.1"
                    :cache-ttl-seconds 300
                    :privacy-level "strict"
                    :bypass-suffixes (".local" ".localhost"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "cf");
        assert_eq!(s.protocol, DnsProtocol::Doh);
        assert_eq!(s.privacy_level, PrivacyLevel::Strict);
        assert_eq!(s.cache_ttl_seconds, 300);
        assert_eq!(s.bypass_suffixes.len(), 2);
    }
}
