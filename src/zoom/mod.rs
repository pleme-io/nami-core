//! `(defzoom)` — per-host zoom preferences.
//!
//! Absorbs Chrome's per-site zoom, Firefox's text-only zoom, and
//! Safari's Reader zoom into a substrate DSL. The browser chrome
//! asks the registry for a zoom level before rendering; the fetched
//! level is either a document-wide scale (default) or a text-only
//! scale (leaves images + video at 1.0).
//!
//! ```lisp
//! (defzoom :host "*"                 :level 1.0)    ; global default
//! (defzoom :host "news.ycombinator.com" :level 1.15)
//! (defzoom :host "*://*.github.com/*"   :level 1.10)
//! (defzoom :host "reading.example.com"  :level 1.25 :text-only #t)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// One zoom rule scoped by host glob.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defzoom"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ZoomSpec {
    /// WebExtensions-style glob — reuses the extension module's matcher.
    /// `"*"` matches everything (global default).
    #[serde(default = "default_host")]
    pub host: String,
    /// Zoom multiplier. Clamped to `[0.25, 5.0]` at apply time.
    pub level: f32,
    /// When true, apply zoom only to text (leave media unscaled).
    #[serde(default)]
    pub text_only: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}

const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 5.0;

impl ZoomSpec {
    /// Clamped zoom level — never outside `[MIN_ZOOM, MAX_ZOOM]` so
    /// callers don't have to defensively clamp at every apply site.
    #[must_use]
    pub fn clamped(&self) -> f32 {
        self.level.clamp(MIN_ZOOM, MAX_ZOOM)
    }

    /// Does this rule apply to `host`?
    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }
}

/// Registry. Most-specific host match wins at resolve time.
#[derive(Debug, Clone, Default)]
pub struct ZoomRegistry {
    specs: Vec<ZoomSpec>,
}

impl ZoomRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: ZoomSpec) {
        self.specs.retain(|s| s.host != spec.host);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ZoomSpec>) {
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
    pub fn specs(&self) -> &[ZoomSpec] {
        &self.specs
    }

    /// Most-specific host-glob match. Returns `None` when no rule
    /// applies (caller should use its own default — typically `1.0`).
    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&ZoomSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.matches_host(host)))
    }

    /// Convenience: resolved zoom level or 1.0 when no rule matches.
    #[must_use]
    pub fn level_for(&self, host: &str) -> f32 {
        self.resolve(host).map(ZoomSpec::clamped).unwrap_or(1.0)
    }

    /// Convenience: text-only flag or false when no rule matches.
    #[must_use]
    pub fn text_only_for(&self, host: &str) -> bool {
        self.resolve(host).map(|s| s.text_only).unwrap_or(false)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ZoomSpec>, String> {
    tatara_lisp::compile_typed::<ZoomSpec>(src)
        .map_err(|e| format!("failed to compile defzoom forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ZoomSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(host: &str, level: f32) -> ZoomSpec {
        ZoomSpec {
            host: host.into(),
            level,
            text_only: false,
            description: None,
        }
    }

    #[test]
    fn clamped_is_a_no_op_in_range() {
        assert_eq!(z("*", 1.25).clamped(), 1.25);
        assert_eq!(z("*", 1.0).clamped(), 1.0);
    }

    #[test]
    fn clamped_respects_bounds() {
        assert_eq!(z("*", 0.01).clamped(), MIN_ZOOM);
        assert_eq!(z("*", 99.0).clamped(), MAX_ZOOM);
    }

    #[test]
    fn wildcard_applies_to_every_host() {
        let s = z("*", 1.0);
        assert!(s.matches_host("example.com"));
        assert!(s.matches_host(""));
    }

    #[test]
    fn glob_host_matches_subdomains() {
        let s = z("*://*.example.com/*", 1.15);
        assert!(s.matches_host("blog.example.com"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn registry_dedupes_by_host() {
        let mut reg = ZoomRegistry::new();
        reg.insert(z("example.com", 1.10));
        reg.insert(z("example.com", 1.25));
        assert_eq!(reg.len(), 1);
        assert!((reg.specs()[0].level - 1.25).abs() < f32::EPSILON);
    }

    #[test]
    fn resolve_prefers_specific_over_wildcard() {
        let mut reg = ZoomRegistry::new();
        reg.insert(z("*", 1.0));
        reg.insert(z("*://*.news.com/*", 1.30));
        let site = reg.resolve("daily.news.com").unwrap();
        assert!((site.level - 1.30).abs() < f32::EPSILON);
        let other = reg.resolve("other.org").unwrap();
        assert!((other.level - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn level_for_defaults_to_one_when_empty() {
        let reg = ZoomRegistry::new();
        assert!((reg.level_for("anywhere.com") - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn text_only_roundtrips_through_registry() {
        let mut reg = ZoomRegistry::new();
        reg.insert(ZoomSpec {
            host: "reading.example.com".into(),
            level: 1.25,
            text_only: true,
            description: None,
        });
        assert!(reg.text_only_for("reading.example.com"));
        assert!(!reg.text_only_for("other.com"));
    }

    #[test]
    fn level_for_clamps_out_of_range_spec() {
        let mut reg = ZoomRegistry::new();
        reg.insert(z("a.com", 99.0));
        reg.insert(z("b.com", 0.01));
        assert!((reg.level_for("a.com") - MAX_ZOOM).abs() < f32::EPSILON);
        assert!((reg.level_for("b.com") - MIN_ZOOM).abs() < f32::EPSILON);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_zoom_form() {
        let src = r#"
            (defzoom :host "*://*.github.com/*"
                     :level 1.10
                     :text-only #f)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].host, "*://*.github.com/*");
        assert!((specs[0].level - 1.10).abs() < 0.001);
        assert!(!specs[0].text_only);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_text_only_zoom() {
        let src = r#"
            (defzoom :host "reading.example.com"
                     :level 1.25
                     :text-only #t)
        "#;
        let specs = compile(src).unwrap();
        assert!(specs[0].text_only);
    }
}
