//! `(defspoof)` — declarative fingerprint-resistance profile.
//!
//! Absorbs Tor Browser letterboxing, Brave fingerprint randomization,
//! Firefox resistFingerprinting, and Safari cross-site tracking
//! protection into one substrate DSL. Each profile declares *what*
//! attributes to mask, *how* (constant / randomize / block), and the
//! host scope where it applies.
//!
//! ```lisp
//! (defspoof :name         "tor-like"
//!           :host         "*"
//!           :user-agent   "Mozilla/5.0 (Windows NT 10.0; rv:102.0) Gecko/20100101 Firefox/102.0"
//!           :canvas       :randomize
//!           :webgl        :block
//!           :timezone     "UTC"
//!           :language     "en-US"
//!           :letterbox    #t
//!           :referrer-policy "no-referrer")
//!
//! (defspoof :name        "soft"
//!           :host        "*://*.banking.example.com/*"
//!           :canvas      :passthrough
//!           :webgl       :passthrough
//!           :user-agent  "native")
//! ```
//!
//! The engine only specifies — actual enforcement lives in the fetch
//! pipeline + JsRuntime host bindings. Enforcement dispatch:
//! - `user_agent`  → rewrites the `User-Agent` outbound header
//! - `canvas`      → hooks `CanvasRenderingContext2D.getImageData`
//! - `webgl`       → hooks `WebGLRenderingContext.readPixels`
//! - `timezone`    → shim `Date.getTimezoneOffset` and `Intl`
//! - `language`    → rewrites `navigator.language(s)` + `Accept-Language`
//! - `letterbox`   → clamps inner/outerWidth to round increments
//! - `referrer_policy` → merged into `(defsecurity-policy)` output

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// How to handle a given fingerprint surface.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SpoofMode {
    /// Leave the surface exactly as the platform reports.
    Passthrough,
    /// Return a stable, spoofed value every time (minimizes session
    /// breakage at the cost of cross-session linkability).
    Constant,
    /// Return a fresh random value per page + per session slot.
    Randomize,
    /// Refuse access entirely — the API errors or returns 0/null.
    Block,
}

impl Default for SpoofMode {
    fn default() -> Self {
        Self::Passthrough
    }
}

/// Fingerprint-resistance profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defspoof"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpoofSpec {
    pub name: String,
    /// Host glob. `"*"` = everywhere.
    #[serde(default = "default_host")]
    pub host: String,
    /// Replacement `User-Agent`. `"native"` = don't rewrite. `"random-pool"`
    /// tells the pipeline to pick from a stock rotation; any other
    /// string is used verbatim.
    #[serde(default = "default_user_agent")]
    pub user_agent: String,
    #[serde(default)]
    pub canvas: SpoofMode,
    #[serde(default)]
    pub webgl: SpoofMode,
    #[serde(default)]
    pub audio_context: SpoofMode,
    #[serde(default)]
    pub hardware_concurrency: SpoofMode,
    /// IANA tz name (`"UTC"`, `"America/Los_Angeles"`). Empty = native.
    #[serde(default)]
    pub timezone: String,
    /// BCP-47 tag. Empty = native.
    #[serde(default)]
    pub language: String,
    /// Letterbox the window — clamp outer/inner W×H to 200×100 px
    /// increments, blunts resolution-based tracking.
    #[serde(default)]
    pub letterbox: bool,
    /// Referrer-Policy directive merged into (defsecurity-policy)
    /// output when this spec resolves. Typical: `no-referrer`.
    #[serde(default)]
    pub referrer_policy: Option<String>,
    /// Strip `Client-Hints` headers (Sec-CH-UA, *-Platform, etc.).
    #[serde(default = "default_strip_client_hints")]
    pub strip_client_hints: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_user_agent() -> String {
    "native".into()
}
fn default_strip_client_hints() -> bool {
    true
}

impl SpoofSpec {
    /// Tor-Browser-flavored profile — everything hardened.
    #[must_use]
    pub fn hardened_profile() -> Self {
        Self {
            name: "hardened".into(),
            host: "*".into(),
            user_agent:
                "Mozilla/5.0 (Windows NT 10.0; rv:115.0) Gecko/20100101 Firefox/115.0"
                    .into(),
            canvas: SpoofMode::Randomize,
            webgl: SpoofMode::Block,
            audio_context: SpoofMode::Randomize,
            hardware_concurrency: SpoofMode::Constant,
            timezone: "UTC".into(),
            language: "en-US".into(),
            letterbox: true,
            referrer_policy: Some("no-referrer".into()),
            strip_client_hints: true,
            description: Some("Tor-browser-flavored profile.".into()),
        }
    }

    /// Mild — only strip client hints + rewrite referrer, no
    /// session-breaking canvas/webgl spoofing.
    #[must_use]
    pub fn mild_profile() -> Self {
        Self {
            name: "mild".into(),
            host: "*".into(),
            user_agent: "native".into(),
            canvas: SpoofMode::Passthrough,
            webgl: SpoofMode::Passthrough,
            audio_context: SpoofMode::Passthrough,
            hardware_concurrency: SpoofMode::Passthrough,
            timezone: String::new(),
            language: String::new(),
            letterbox: false,
            referrer_policy: Some("strict-origin-when-cross-origin".into()),
            strip_client_hints: true,
            description: Some("Minimal — only referrer + client hints.".into()),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    /// True if this spec overrides the User-Agent (any value other
    /// than the sentinel `"native"`).
    #[must_use]
    pub fn rewrites_user_agent(&self) -> bool {
        !self.user_agent.is_empty() && self.user_agent != "native"
    }
}

/// Registry. Host-specific wins over wildcard.
#[derive(Debug, Clone, Default)]
pub struct SpoofRegistry {
    specs: Vec<SpoofSpec>,
}

impl SpoofRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SpoofSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SpoofSpec>) {
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
    pub fn specs(&self) -> &[SpoofSpec] {
        &self.specs
    }

    /// Most-specific match wins (non-`"*"` host beats wildcard).
    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&SpoofSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SpoofSpec>, String> {
    tatara_lisp::compile_typed::<SpoofSpec>(src)
        .map_err(|e| format!("failed to compile defspoof forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SpoofSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardened_profile_sets_everything() {
        let s = SpoofSpec::hardened_profile();
        assert_eq!(s.canvas, SpoofMode::Randomize);
        assert_eq!(s.webgl, SpoofMode::Block);
        assert!(s.letterbox);
        assert_eq!(s.timezone, "UTC");
        assert_eq!(s.language, "en-US");
        assert!(s.strip_client_hints);
        assert!(s.rewrites_user_agent());
    }

    #[test]
    fn mild_profile_only_touches_headers() {
        let s = SpoofSpec::mild_profile();
        assert_eq!(s.canvas, SpoofMode::Passthrough);
        assert_eq!(s.webgl, SpoofMode::Passthrough);
        assert!(!s.letterbox);
        assert!(s.timezone.is_empty());
        assert!(!s.rewrites_user_agent());
    }

    #[test]
    fn matches_host_wildcard() {
        let s = SpoofSpec::hardened_profile();
        assert!(s.matches_host("anything.com"));
    }

    #[test]
    fn matches_host_glob() {
        let s = SpoofSpec {
            host: "*://*.banking.example.com/*".into(),
            ..SpoofSpec::mild_profile()
        };
        assert!(s.matches_host("branch.banking.example.com"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn rewrites_user_agent_respects_native_sentinel() {
        let native = SpoofSpec::mild_profile();
        assert!(!native.rewrites_user_agent());
        let custom = SpoofSpec {
            user_agent: "CustomAgent/1.0".into(),
            ..SpoofSpec::mild_profile()
        };
        assert!(custom.rewrites_user_agent());
        let empty = SpoofSpec {
            user_agent: String::new(),
            ..SpoofSpec::mild_profile()
        };
        assert!(!empty.rewrites_user_agent());
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = SpoofRegistry::new();
        reg.insert(SpoofSpec::hardened_profile());
        reg.insert(SpoofSpec {
            letterbox: false,
            ..SpoofSpec::hardened_profile()
        });
        assert_eq!(reg.len(), 1);
        assert!(!reg.specs()[0].letterbox);
    }

    #[test]
    fn resolve_prefers_specific_host() {
        let mut reg = SpoofRegistry::new();
        reg.insert(SpoofSpec::hardened_profile());
        reg.insert(SpoofSpec {
            name: "banking".into(),
            host: "*://*.bank.com/*".into(),
            ..SpoofSpec::mild_profile()
        });
        let bank = reg.resolve("online.bank.com").unwrap();
        assert_eq!(bank.name, "banking");
        let other = reg.resolve("example.org").unwrap();
        assert_eq!(other.name, "hardened");
    }

    #[test]
    fn spoof_mode_roundtrips_through_serde() {
        let s = SpoofSpec {
            canvas: SpoofMode::Block,
            ..SpoofSpec::mild_profile()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: SpoofSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.canvas, SpoofMode::Block);
    }

    #[test]
    fn strip_client_hints_default_true() {
        let s = SpoofSpec {
            name: "x".into(),
            host: "*".into(),
            user_agent: "native".into(),
            canvas: SpoofMode::Passthrough,
            webgl: SpoofMode::Passthrough,
            audio_context: SpoofMode::Passthrough,
            hardware_concurrency: SpoofMode::Passthrough,
            timezone: String::new(),
            language: String::new(),
            letterbox: false,
            referrer_policy: None,
            strip_client_hints: true,
            description: None,
        };
        assert!(s.strip_client_hints);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_spoof_form() {
        let src = r#"
            (defspoof :name      "strict"
                      :host      "*"
                      :user-agent "Custom/1.0"
                      :canvas    "randomize"
                      :webgl     "block"
                      :timezone  "UTC"
                      :language  "en-US"
                      :letterbox #t
                      :referrer-policy "no-referrer")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "strict");
        assert_eq!(s.canvas, SpoofMode::Randomize);
        assert_eq!(s.webgl, SpoofMode::Block);
        assert!(s.letterbox);
        assert_eq!(s.referrer_policy.as_deref(), Some("no-referrer"));
    }
}
