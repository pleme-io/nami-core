//! `(defrouting)` — declarative per-host network routing.
//!
//! Absorbs per-site VPN selection (Firefox Multi-Account Containers +
//! Mozilla VPN), Tor Browser's circuit-per-origin, and proxy
//! per-tab switching into the substrate. Each rule names a routing
//! strategy (direct, VPN tunnel, Tor circuit, SOCKS5 proxy, or
//! pluggable transport) plus a host scope.
//!
//! Ties into pleme-io's network stack:
//! - `tunnel:<name>` — a mamorigami WireGuard tunnel by name
//! - `tor:<isolation>` — a kakuremino IsolationToken stream
//! - `socks5:<url>` — generic SOCKS5 proxy
//! - `pt:<transport>` — maboroshi pluggable transport
//!
//! ```lisp
//! (defrouting :name     "default"
//!             :host     "*"
//!             :via      :direct)
//!
//! (defrouting :name     "banking"
//!             :host     "*://*.bank.com/*"
//!             :via      "tunnel:home"
//!             :fallback :block)
//!
//! (defrouting :name     "onion"
//!             :host     "*.onion"
//!             :via      "tor:per-origin")
//!
//! (defrouting :name     "media"
//!             :host     "*://netflix.com/*"
//!             :via      "tunnel:us-east"
//!             :kill-switch #t)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// What to do when the primary route fails.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RouteFallback {
    /// Fall through to the next matching rule (typically the wildcard).
    NextRule,
    /// Fall through to direct internet.
    Direct,
    /// Refuse the request entirely — connection-state privacy.
    Block,
}

impl Default for RouteFallback {
    fn default() -> Self {
        Self::NextRule
    }
}

/// Routing rule.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defrouting"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RoutingSpec {
    pub name: String,
    /// Host glob. `"*"` = everywhere.
    #[serde(default = "default_host")]
    pub host: String,
    /// Route strategy. Tokens:
    ///   `direct` | `tunnel:<name>` | `tor:<isolation>` |
    ///   `socks5:<url>` | `pt:<transport>`
    pub via: String,
    /// Refuse direct when the route is unavailable.
    #[serde(default)]
    pub kill_switch: bool,
    /// What to do when the `via` route fails (honored only when
    /// `kill_switch` is false).
    #[serde(default)]
    pub fallback: RouteFallback,
    /// Required capabilities the route must satisfy. Reserved for
    /// future IsolationToken integration; V1 stores them.
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}

/// Route strategy parsed from the `via` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteVia {
    Direct,
    /// `tunnel:<name>` — mamorigami WireGuard.
    Tunnel(String),
    /// `tor:<isolation-name>` — kakuremino IsolationToken stream.
    Tor(String),
    /// `socks5://host:port` — generic proxy.
    Socks5(String),
    /// `pt:<transport-name>` — maboroshi pluggable transport.
    PluggableTransport(String),
    /// Unrecognized strategy — caller decides what to do
    /// (reject at validate time, log + passthrough, etc.).
    Unknown(String),
}

impl RouteVia {
    #[must_use]
    pub fn parse(s: &str) -> Self {
        let s = s.trim();
        if s.eq_ignore_ascii_case("direct") || s.is_empty() {
            return Self::Direct;
        }
        if let Some(rest) = s.strip_prefix("tunnel:") {
            return Self::Tunnel(rest.to_owned());
        }
        if let Some(rest) = s.strip_prefix("tor:") {
            return Self::Tor(rest.to_owned());
        }
        if s.starts_with("socks5:") || s.starts_with("socks5://") {
            return Self::Socks5(s.to_owned());
        }
        if let Some(rest) = s.strip_prefix("pt:") {
            return Self::PluggableTransport(rest.to_owned());
        }
        Self::Unknown(s.to_owned())
    }

    #[must_use]
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }

    #[must_use]
    pub fn is_direct(&self) -> bool {
        matches!(self, Self::Direct)
    }
}

impl RoutingSpec {
    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        if self.host.is_empty() || self.host == "*" {
            return true;
        }
        // Bare "*.onion" style — leading `*` directly followed by `.`
        // with no other glob chars in the rest. Simple suffix match.
        if self.host.starts_with('*')
            && self.host.len() > 1
            && self.host.as_bytes()[1] == b'.'
            && !self.host[1..].contains(|c: char| c == '*' || c == '/' || c == ':')
        {
            return host.ends_with(&self.host[1..]);
        }
        crate::extension::glob_match_host(&self.host, host)
    }

    #[must_use]
    pub fn parsed_via(&self) -> RouteVia {
        RouteVia::parse(&self.via)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("routing rule name is empty".into());
        }
        if self.via.trim().is_empty() {
            return Err(format!("routing '{}' has empty :via", self.name));
        }
        if !self.parsed_via().is_known() {
            return Err(format!(
                "routing '{}' has unrecognized :via = {:?}",
                self.name, self.via
            ));
        }
        Ok(())
    }
}

/// Registry. Most-specific host match wins at resolve time.
#[derive(Debug, Clone, Default)]
pub struct RoutingRegistry {
    specs: Vec<RoutingSpec>,
}

impl RoutingRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: RoutingSpec) -> Result<(), String> {
        spec.validate()?;
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
        Ok(())
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = RoutingSpec>) {
        for s in specs {
            if let Err(e) = self.insert(s.clone()) {
                tracing::warn!("defrouting '{}' rejected: {}", s.name, e);
            }
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
    pub fn specs(&self) -> &[RoutingSpec] {
        &self.specs
    }

    /// Most-specific host match wins (non-wildcard beats `*`).
    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&RoutingSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.matches_host(host)))
    }

    /// Convenience: resolved route for a host (Direct when empty).
    #[must_use]
    pub fn via_for(&self, host: &str) -> RouteVia {
        self.resolve(host)
            .map(|s| s.parsed_via())
            .unwrap_or(RouteVia::Direct)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<RoutingSpec>, String> {
    tatara_lisp::compile_typed::<RoutingSpec>(src)
        .map_err(|e| format!("failed to compile defrouting forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<RoutingSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(name: &str, host: &str, via: &str) -> RoutingSpec {
        RoutingSpec {
            name: name.into(),
            host: host.into(),
            via: via.into(),
            kill_switch: false,
            fallback: RouteFallback::NextRule,
            required_capabilities: vec![],
            description: None,
        }
    }

    #[test]
    fn parse_direct() {
        assert_eq!(RouteVia::parse("direct"), RouteVia::Direct);
        assert_eq!(RouteVia::parse(""), RouteVia::Direct);
    }

    #[test]
    fn parse_tunnel() {
        assert_eq!(
            RouteVia::parse("tunnel:home"),
            RouteVia::Tunnel("home".into())
        );
    }

    #[test]
    fn parse_tor() {
        assert_eq!(
            RouteVia::parse("tor:per-origin"),
            RouteVia::Tor("per-origin".into())
        );
    }

    #[test]
    fn parse_socks5_forms() {
        assert!(matches!(
            RouteVia::parse("socks5://127.0.0.1:9050"),
            RouteVia::Socks5(_)
        ));
        assert!(matches!(RouteVia::parse("socks5:local"), RouteVia::Socks5(_)));
    }

    #[test]
    fn parse_pluggable_transport() {
        assert_eq!(
            RouteVia::parse("pt:obfs4"),
            RouteVia::PluggableTransport("obfs4".into())
        );
    }

    #[test]
    fn parse_unknown() {
        let v = RouteVia::parse("magic");
        assert!(matches!(v, RouteVia::Unknown(_)));
        assert!(!v.is_known());
    }

    #[test]
    fn wildcard_host_matches_everything() {
        let s = r("x", "*", "direct");
        assert!(s.matches_host("anywhere.com"));
    }

    #[test]
    fn onion_star_suffix_match() {
        let s = r("onion", "*.onion", "tor:per-origin");
        assert!(s.matches_host("abc.onion"));
        assert!(!s.matches_host("example.com"));
    }

    #[test]
    fn glob_host_matches_subdomain() {
        let s = r("b", "*://*.bank.com/*", "tunnel:home");
        assert!(s.matches_host("online.bank.com"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn validate_rejects_empty_via() {
        let s = r("x", "*", "");
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_unknown_via() {
        let s = r("x", "*", "teleport:fast");
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_accepts_all_known_forms() {
        assert!(r("a", "*", "direct").validate().is_ok());
        assert!(r("a", "*", "tunnel:home").validate().is_ok());
        assert!(r("a", "*", "tor:default").validate().is_ok());
        assert!(r("a", "*", "socks5://127.0.0.1:9050").validate().is_ok());
        assert!(r("a", "*", "pt:obfs4").validate().is_ok());
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = RoutingRegistry::new();
        reg.insert(r("default", "*", "direct")).unwrap();
        reg.insert(r("default", "*", "tunnel:home")).unwrap();
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].via, "tunnel:home");
    }

    #[test]
    fn registry_extend_drops_invalid_silently() {
        let mut reg = RoutingRegistry::new();
        reg.extend(vec![
            r("ok", "*", "direct"),
            r("bad", "*", "teleport"),
        ]);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].name, "ok");
    }

    #[test]
    fn resolve_prefers_specific_host() {
        let mut reg = RoutingRegistry::new();
        reg.insert(r("default", "*", "direct")).unwrap();
        reg.insert(r("bank", "*://*.bank.com/*", "tunnel:home")).unwrap();
        reg.insert(r("onion", "*.onion", "tor:per-origin")).unwrap();

        let bank = reg.resolve("online.bank.com").unwrap();
        assert_eq!(bank.name, "bank");
        let onion = reg.resolve("abc.onion").unwrap();
        assert_eq!(onion.name, "onion");
        let other = reg.resolve("example.com").unwrap();
        assert_eq!(other.name, "default");
    }

    #[test]
    fn via_for_defaults_to_direct() {
        let reg = RoutingRegistry::new();
        assert_eq!(reg.via_for("anywhere.com"), RouteVia::Direct);
    }

    #[test]
    fn kill_switch_round_trips_through_serde() {
        let s = RoutingSpec {
            kill_switch: true,
            ..r("x", "*", "tunnel:home")
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: RoutingSpec = serde_json::from_str(&json).unwrap();
        assert!(back.kill_switch);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_routing_form() {
        let src = r#"
            (defrouting :name "bank"
                        :host "*://*.bank.com/*"
                        :via  "tunnel:home"
                        :kill-switch #t
                        :fallback "block")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "bank");
        assert!(s.kill_switch);
        assert_eq!(s.fallback, RouteFallback::Block);
        assert_eq!(s.parsed_via(), RouteVia::Tunnel("home".into()));
    }
}
