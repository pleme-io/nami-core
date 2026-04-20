//! `(defreferrer)` — per-host Referer header policy.
//!
//! Absorbs the W3C Referrer-Policy spec (8 policies), Brave Shields
//! referer trimming, Firefox `network.http.referer.XOriginPolicy`,
//! Safari ITP cross-site referer down-grading, and uBlock Origin's
//! per-host spoof rules. Nobody ships a declarative, host-glob-driven
//! authoring surface.
//!
//! ```lisp
//! (defreferrer :name      "strict"
//!              :host      "*"
//!              :policy    :strict-origin-when-cross-origin
//!              :strip-to-origin-on-cross-site #t)
//!
//! (defreferrer :name   "no-referrer-search"
//!              :host   "*://*.google.com/*"
//!              :policy :no-referrer)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// W3C Referrer-Policy (HTML Living Standard §8.10.2). Eight values
/// specify what `Referer:` the browser sends on a given request.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ReferrerPolicy {
    /// Send no `Referer:` header, ever.
    NoReferrer,
    /// Send the full URL, but strip on HTTPS→HTTP downgrade.
    /// Matches historical browser behavior (pre-2020).
    NoReferrerWhenDowngrade,
    /// Always send only the origin (scheme + host + port).
    Origin,
    /// Same-origin → full URL; cross-origin → origin only.
    OriginWhenCrossOrigin,
    /// Same-origin only; cross-origin sends nothing.
    SameOrigin,
    /// Like Origin, but strip on HTTPS→HTTP downgrade.
    StrictOrigin,
    /// The W3C / HTML Living Standard default as of 2020.
    /// Same-origin → full; cross-origin → origin; downgrade → nothing.
    #[default]
    StrictOriginWhenCrossOrigin,
    /// Always send the full URL (path + query). Unsafe; explicit opt-in.
    UnsafeUrl,
}

/// Referrer policy spec.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defreferrer"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReferrerSpec {
    pub name: String,
    /// Host glob the policy applies to. `"*"` = everywhere.
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    #[serde(default)]
    pub policy: ReferrerPolicy,
    /// Strip query string from the sent Referer even when the policy
    /// would send the full URL. Privacy win for intra-site nav that
    /// still wants to show a Referer for analytics.
    #[serde(default)]
    pub strip_query: bool,
    /// Strip fragment (`#section`) on send. The W3C spec already does
    /// this for ALL policies — kept as an explicit flag so authors can
    /// assert it in tests.
    #[serde(default = "default_strip_fragment")]
    pub strip_fragment: bool,
    /// Refuse to send Referer on HTTPS → HTTP transitions regardless
    /// of policy. Even `UnsafeUrl` respects this when `true`.
    #[serde(default = "default_strip_on_downgrade")]
    pub strip_on_downgrade: bool,
    /// Exempt hosts (glob) — always send full URL regardless. Reverse
    /// allow-list for analytics partners.
    #[serde(default)]
    pub exempt_hosts: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_strip_fragment() -> bool {
    true
}
fn default_strip_on_downgrade() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

/// Minimal URL decomposition used for policy decisions. Caller
/// parses once; this module remains dependency-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UrlParts<'a> {
    pub scheme: &'a str,
    pub host: &'a str,
    pub port: Option<u16>,
    pub path: &'a str,
    pub query: Option<&'a str>,
}

impl<'a> UrlParts<'a> {
    #[must_use]
    pub fn is_https(&self) -> bool {
        self.scheme.eq_ignore_ascii_case("https")
    }

    #[must_use]
    pub fn origin(&self) -> String {
        match self.port {
            Some(p) => format!("{}://{}:{p}", self.scheme, self.host),
            None => format!("{}://{}", self.scheme, self.host),
        }
    }

    #[must_use]
    pub fn full(&self) -> String {
        let mut s = self.origin();
        s.push_str(self.path);
        if let Some(q) = self.query {
            s.push('?');
            s.push_str(q);
        }
        s
    }

    #[must_use]
    pub fn same_origin(&self, other: &UrlParts) -> bool {
        self.scheme.eq_ignore_ascii_case(other.scheme)
            && self.host.eq_ignore_ascii_case(other.host)
            && self.port == other.port
    }
}

impl ReferrerSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            policy: ReferrerPolicy::StrictOriginWhenCrossOrigin,
            strip_query: false,
            strip_fragment: true,
            strip_on_downgrade: true,
            exempt_hosts: vec![],
            enabled: true,
            description: Some(
                "W3C default — strict-origin-when-cross-origin with downgrade strip.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    #[must_use]
    pub fn is_exempt(&self, host: &str) -> bool {
        self.exempt_hosts
            .iter()
            .any(|g| crate::extension::host_pattern_matches(g, host))
    }

    /// Compute the `Referer:` header the browser should send when
    /// navigating from `from` to `to`. Returns `None` when no
    /// Referer is to be sent.
    #[must_use]
    pub fn header_for(&self, from: &UrlParts, to: &UrlParts) -> Option<String> {
        if !self.enabled {
            return Some(from.full());
        }
        if self.is_exempt(to.host) {
            return Some(from.full());
        }
        let downgrade = from.is_https() && !to.is_https();
        if self.strip_on_downgrade && downgrade {
            // Policies that carry strict downgrade behavior implicitly
            // drop the header; we also apply the flag to UnsafeUrl.
            match self.policy {
                ReferrerPolicy::UnsafeUrl
                | ReferrerPolicy::NoReferrerWhenDowngrade
                | ReferrerPolicy::StrictOrigin
                | ReferrerPolicy::StrictOriginWhenCrossOrigin => return None,
                _ => {}
            }
        }
        let same = from.same_origin(to);
        let raw = match self.policy {
            ReferrerPolicy::NoReferrer => return None,
            ReferrerPolicy::NoReferrerWhenDowngrade => {
                if downgrade {
                    return None;
                }
                self.render_url(from)
            }
            ReferrerPolicy::Origin | ReferrerPolicy::StrictOrigin => from.origin(),
            ReferrerPolicy::SameOrigin => {
                if !same {
                    return None;
                }
                self.render_url(from)
            }
            ReferrerPolicy::OriginWhenCrossOrigin
            | ReferrerPolicy::StrictOriginWhenCrossOrigin => {
                if same {
                    self.render_url(from)
                } else {
                    from.origin()
                }
            }
            ReferrerPolicy::UnsafeUrl => self.render_url(from),
        };
        Some(raw)
    }

    fn render_url(&self, u: &UrlParts) -> String {
        let mut s = u.origin();
        s.push_str(u.path);
        if !self.strip_query {
            if let Some(q) = u.query {
                s.push('?');
                s.push_str(q);
            }
        }
        s
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct ReferrerRegistry {
    specs: Vec<ReferrerSpec>,
}

impl ReferrerRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: ReferrerSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ReferrerSpec>) {
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
    pub fn specs(&self) -> &[ReferrerSpec] {
        &self.specs
    }

    /// Resolve by destination host (the policy is keyed to where the
    /// request is going, not where it came from).
    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&ReferrerSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ReferrerSpec>, String> {
    tatara_lisp::compile_typed::<ReferrerSpec>(src)
        .map_err(|e| format!("failed to compile defreferrer forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ReferrerSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p<'a>(scheme: &'a str, host: &'a str, path: &'a str, query: Option<&'a str>) -> UrlParts<'a> {
        UrlParts {
            scheme,
            host,
            port: None,
            path,
            query,
        }
    }

    #[test]
    fn default_is_strict_origin_when_cross_origin() {
        let s = ReferrerSpec::default_profile();
        assert_eq!(s.policy, ReferrerPolicy::StrictOriginWhenCrossOrigin);
    }

    #[test]
    fn no_referrer_sends_nothing() {
        let s = ReferrerSpec {
            policy: ReferrerPolicy::NoReferrer,
            ..ReferrerSpec::default_profile()
        };
        let from = p("https", "a.com", "/x", Some("q=1"));
        let to = p("https", "b.com", "/", None);
        assert_eq!(s.header_for(&from, &to), None);
    }

    #[test]
    fn origin_sends_origin_only_across_origins() {
        let s = ReferrerSpec {
            policy: ReferrerPolicy::Origin,
            ..ReferrerSpec::default_profile()
        };
        let from = p("https", "a.com", "/secret", Some("q=1"));
        let to = p("https", "b.com", "/", None);
        assert_eq!(s.header_for(&from, &to).as_deref(), Some("https://a.com"));
    }

    #[test]
    fn same_origin_sends_nothing_cross_site() {
        let s = ReferrerSpec {
            policy: ReferrerPolicy::SameOrigin,
            ..ReferrerSpec::default_profile()
        };
        let from = p("https", "a.com", "/x", None);
        let to = p("https", "b.com", "/y", None);
        assert_eq!(s.header_for(&from, &to), None);
    }

    #[test]
    fn same_origin_sends_full_url_intra_site() {
        let s = ReferrerSpec {
            policy: ReferrerPolicy::SameOrigin,
            ..ReferrerSpec::default_profile()
        };
        let from = p("https", "a.com", "/x", Some("k=v"));
        let to = p("https", "a.com", "/y", None);
        assert_eq!(
            s.header_for(&from, &to).as_deref(),
            Some("https://a.com/x?k=v")
        );
    }

    #[test]
    fn strict_origin_when_cross_origin_matches_w3c_default() {
        let s = ReferrerSpec::default_profile();
        // Same-origin → full URL.
        assert_eq!(
            s.header_for(
                &p("https", "a.com", "/x", Some("k=v")),
                &p("https", "a.com", "/y", None)
            )
            .as_deref(),
            Some("https://a.com/x?k=v")
        );
        // Cross-origin HTTPS→HTTPS → origin only.
        assert_eq!(
            s.header_for(
                &p("https", "a.com", "/x", Some("k=v")),
                &p("https", "b.com", "/", None)
            )
            .as_deref(),
            Some("https://a.com")
        );
        // HTTPS → HTTP downgrade → nothing.
        assert_eq!(
            s.header_for(
                &p("https", "a.com", "/x", None),
                &p("http", "b.com", "/", None)
            ),
            None
        );
    }

    #[test]
    fn unsafe_url_sends_full_unless_downgrade_strip() {
        let s = ReferrerSpec {
            policy: ReferrerPolicy::UnsafeUrl,
            ..ReferrerSpec::default_profile()
        };
        // Safe: HTTPS → HTTPS sends full.
        assert_eq!(
            s.header_for(
                &p("https", "a.com", "/x", Some("k=v")),
                &p("https", "b.com", "/", None),
            )
            .as_deref(),
            Some("https://a.com/x?k=v")
        );
        // Downgrade: default strip_on_downgrade=true kills even unsafe-url.
        assert_eq!(
            s.header_for(
                &p("https", "a.com", "/x", None),
                &p("http", "b.com", "/", None),
            ),
            None
        );
    }

    #[test]
    fn strip_query_trims_queries_on_send() {
        let s = ReferrerSpec {
            policy: ReferrerPolicy::SameOrigin,
            strip_query: true,
            ..ReferrerSpec::default_profile()
        };
        let from = p("https", "a.com", "/x", Some("token=secret"));
        let to = p("https", "a.com", "/y", None);
        assert_eq!(
            s.header_for(&from, &to).as_deref(),
            Some("https://a.com/x")
        );
    }

    #[test]
    fn downgrade_strip_flag_preserved_when_off() {
        let s = ReferrerSpec {
            policy: ReferrerPolicy::UnsafeUrl,
            strip_on_downgrade: false,
            ..ReferrerSpec::default_profile()
        };
        assert_eq!(
            s.header_for(
                &p("https", "a.com", "/x", None),
                &p("http", "b.com", "/", None)
            )
            .as_deref(),
            Some("https://a.com/x")
        );
    }

    #[test]
    fn disabled_profile_bypasses_everything() {
        let s = ReferrerSpec {
            enabled: false,
            policy: ReferrerPolicy::NoReferrer,
            ..ReferrerSpec::default_profile()
        };
        // Disabled → full URL passes through.
        assert_eq!(
            s.header_for(
                &p("https", "a.com", "/x", None),
                &p("https", "b.com", "/", None)
            )
            .as_deref(),
            Some("https://a.com/x")
        );
    }

    #[test]
    fn exempt_hosts_always_full_url() {
        let s = ReferrerSpec {
            policy: ReferrerPolicy::NoReferrer,
            exempt_hosts: vec!["*://*.analytics.com/*".into()],
            ..ReferrerSpec::default_profile()
        };
        // Exempt destination → full URL even under no-referrer.
        assert_eq!(
            s.header_for(
                &p("https", "a.com", "/x", None),
                &p("https", "mixpanel.analytics.com", "/", None)
            )
            .as_deref(),
            Some("https://a.com/x")
        );
        // Non-exempt destination → dropped.
        assert_eq!(
            s.header_for(
                &p("https", "a.com", "/x", None),
                &p("https", "b.com", "/", None)
            ),
            None
        );
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = ReferrerRegistry::new();
        reg.insert(ReferrerSpec::default_profile());
        reg.insert(ReferrerSpec {
            name: "google-no-ref".into(),
            host: "*://*.google.com/*".into(),
            policy: ReferrerPolicy::NoReferrer,
            ..ReferrerSpec::default_profile()
        });
        assert_eq!(
            reg.resolve("www.google.com").unwrap().policy,
            ReferrerPolicy::NoReferrer
        );
        assert_eq!(
            reg.resolve("example.org").unwrap().policy,
            ReferrerPolicy::StrictOriginWhenCrossOrigin
        );
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = ReferrerRegistry::new();
        reg.insert(ReferrerSpec {
            enabled: false,
            ..ReferrerSpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[test]
    fn policy_roundtrips_through_serde() {
        for p in [
            ReferrerPolicy::NoReferrer,
            ReferrerPolicy::NoReferrerWhenDowngrade,
            ReferrerPolicy::Origin,
            ReferrerPolicy::OriginWhenCrossOrigin,
            ReferrerPolicy::SameOrigin,
            ReferrerPolicy::StrictOrigin,
            ReferrerPolicy::StrictOriginWhenCrossOrigin,
            ReferrerPolicy::UnsafeUrl,
        ] {
            let s = ReferrerSpec {
                policy: p,
                ..ReferrerSpec::default_profile()
            };
            let j = serde_json::to_string(&s).unwrap();
            let b: ReferrerSpec = serde_json::from_str(&j).unwrap();
            assert_eq!(b.policy, p);
        }
    }

    #[test]
    fn origin_includes_explicit_port() {
        let u = UrlParts {
            scheme: "https",
            host: "a.com",
            port: Some(8443),
            path: "/x",
            query: None,
        };
        assert_eq!(u.origin(), "https://a.com:8443");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_referrer_form() {
        let src = r#"
            (defreferrer :name "no-ref-google"
                         :host "*://*.google.com/*"
                         :policy "no-referrer"
                         :strip-query #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].policy, ReferrerPolicy::NoReferrer);
        assert!(specs[0].strip_query);
    }
}
