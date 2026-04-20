//! `(defprerender-rule)` — Chrome Speculation Rules API.
//!
//! Absorbs `<script type="speculationrules">` — declarative
//! prefetching + prerendering of likely next pages. Also models
//! Firefox's `<link rel="prerender">` (deprecated but still honored
//! in some forks) and Safari's historical prerender hints.
//!
//! ```lisp
//! (defprerender-rule :name          "likely-next"
//!                    :host          "*"
//!                    :mode          :prerender
//!                    :eagerness     :moderate
//!                    :urls          ("/next" "/common")
//!                    :where-hrefs   (starts-with "/chapter-")
//!                    :relative-to   :document
//!                    :max-concurrent 2)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// What to do with the URL — prefetch fetches bytes; prerender runs
/// the full document.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SpeculationMode {
    /// Fetch bytes only (no JS run). `prefetch` in the spec.
    #[default]
    Prefetch,
    /// Full navigation preparation — fetch + run JS + first paint.
    Prerender,
}

/// Eagerness levels — matches Chrome Speculation Rules verbatim.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Eagerness {
    /// Triggered only on strong hover/pointerdown signals.
    #[default]
    Conservative,
    /// Hover ≥ 200 ms.
    Moderate,
    /// Hover ≥ ~0 ms — starts speculating quickly.
    Eager,
    /// Triggered immediately on rule parse — use for must-have
    /// prefetches.
    Immediate,
}

/// Resolve relative URLs against.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RelativeTo {
    #[default]
    Document,
    Ruleset,
}

/// How the spec selects URLs to speculate on.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SelectorKind {
    /// Explicit list in `urls` — no predicate.
    #[default]
    Explicit,
    /// Every same-origin link on the page.
    SameOrigin,
    /// Every anchor whose href starts with any entry in
    /// `where_prefix`.
    HrefPrefix,
    /// Every anchor whose href matches a regex in `where_regex`.
    HrefRegex,
    /// Every anchor within a DOM node matching `where_selector`
    /// (CSS selector syntax).
    DomSelector,
}

/// Network isolation strategy — matches Chrome's `requires:`
/// clause.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkIsolation {
    /// No requirement — use the document's default.
    #[default]
    None,
    /// Require `"anonymous-client-ip-when-cross-origin"` — strips
    /// the Client-IP when prefetching cross-origin pages (Chrome
    /// privacy mode).
    AnonymousClientIpCrossOrigin,
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defprerender-rule"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrerenderRuleSpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub mode: SpeculationMode,
    #[serde(default)]
    pub eagerness: Eagerness,
    #[serde(default)]
    pub selector: SelectorKind,
    /// Explicit URL list (when selector = Explicit).
    #[serde(default)]
    pub urls: Vec<String>,
    /// Prefix list (when selector = HrefPrefix).
    #[serde(default)]
    pub where_prefix: Vec<String>,
    /// Regex list (when selector = HrefRegex).
    #[serde(default)]
    pub where_regex: Vec<String>,
    /// CSS selector (when selector = DomSelector).
    #[serde(default)]
    pub where_selector: Option<String>,
    #[serde(default)]
    pub relative_to: RelativeTo,
    /// Max number of concurrent speculations allowed.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
    /// Hosts exempt from the rule (e.g. ad servers).
    #[serde(default)]
    pub exclude_hosts: Vec<String>,
    #[serde(default)]
    pub network_isolation: NetworkIsolation,
    /// Cap prerenders to Save-Data=off clients (honor data-saver
    /// headers).
    #[serde(default = "default_honor_save_data")]
    pub honor_save_data: bool,
    /// Max memory per prerender (MB). 0 = no cap.
    #[serde(default)]
    pub max_prerender_memory_mb: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_max_concurrent() -> u32 {
    2
}
fn default_honor_save_data() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

impl PrerenderRuleSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            mode: SpeculationMode::Prefetch,
            eagerness: Eagerness::Conservative,
            selector: SelectorKind::SameOrigin,
            urls: vec![],
            where_prefix: vec![],
            where_regex: vec![],
            where_selector: None,
            relative_to: RelativeTo::Document,
            max_concurrent: 2,
            exclude_hosts: vec![],
            network_isolation: NetworkIsolation::None,
            honor_save_data: true,
            max_prerender_memory_mb: 0,
            enabled: true,
            description: Some(
                "Default rule — conservative prefetch of same-origin links, respects Save-Data.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    #[must_use]
    pub fn is_excluded(&self, host: &str) -> bool {
        self.exclude_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    /// Would this rule speculate on `href` from a document at
    /// `document_host`?
    #[must_use]
    pub fn would_speculate(&self, document_host: &str, href: &str) -> bool {
        if !self.enabled {
            return false;
        }
        if self.is_excluded(document_host) {
            return false;
        }
        match self.selector {
            SelectorKind::Explicit => self.urls.iter().any(|u| u == href),
            SelectorKind::SameOrigin => is_same_origin(document_host, href),
            SelectorKind::HrefPrefix => {
                self.where_prefix.iter().any(|p| href.starts_with(p))
            }
            SelectorKind::HrefRegex => {
                // Regex matching deferred to caller — we only report
                // whether there's a candidate pattern. Authors should
                // pre-compile through the host's regex engine.
                !self.where_regex.is_empty()
            }
            SelectorKind::DomSelector => self.where_selector.is_some(),
        }
    }
}

fn is_same_origin(document_host: &str, href: &str) -> bool {
    if href.starts_with('/') || href.starts_with('#') || href.starts_with('?') {
        return true;
    }
    // crude absolute-URL check: scheme://host/…
    if let Some(rest) = href.split("://").nth(1) {
        let host = rest.split('/').next().unwrap_or("");
        // strip port
        let host = host.split(':').next().unwrap_or("");
        return host == document_host;
    }
    // Relative path without scheme
    true
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct PrerenderRuleRegistry {
    specs: Vec<PrerenderRuleSpec>,
}

impl PrerenderRuleRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: PrerenderRuleSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = PrerenderRuleSpec>) {
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
    pub fn specs(&self) -> &[PrerenderRuleSpec] {
        &self.specs
    }

    /// All enabled rules applicable to `host`.
    #[must_use]
    pub fn rules_for(&self, host: &str) -> Vec<&PrerenderRuleSpec> {
        self.specs
            .iter()
            .filter(|s| s.enabled && s.matches_host(host))
            .collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<PrerenderRuleSpec>, String> {
    tatara_lisp::compile_typed::<PrerenderRuleSpec>(src)
        .map_err(|e| format!("failed to compile defprerender-rule forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<PrerenderRuleSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_conservative_same_origin_prefetch() {
        let s = PrerenderRuleSpec::default_profile();
        assert_eq!(s.mode, SpeculationMode::Prefetch);
        assert_eq!(s.eagerness, Eagerness::Conservative);
        assert_eq!(s.selector, SelectorKind::SameOrigin);
    }

    #[test]
    fn would_speculate_explicit_matches_exact_url() {
        let s = PrerenderRuleSpec {
            selector: SelectorKind::Explicit,
            urls: vec!["/next".into(), "/common".into()],
            ..PrerenderRuleSpec::default_profile()
        };
        assert!(s.would_speculate("example.com", "/next"));
        assert!(!s.would_speculate("example.com", "/other"));
    }

    #[test]
    fn would_speculate_same_origin_accepts_relative_paths() {
        let s = PrerenderRuleSpec::default_profile();
        assert!(s.would_speculate("example.com", "/page"));
        assert!(s.would_speculate("example.com", "/page?q=1"));
        assert!(s.would_speculate("example.com", "#frag"));
    }

    #[test]
    fn would_speculate_same_origin_rejects_cross_origin_abs_url() {
        let s = PrerenderRuleSpec::default_profile();
        assert!(!s.would_speculate("example.com", "https://evil.com/page"));
    }

    #[test]
    fn would_speculate_same_origin_accepts_abs_self() {
        let s = PrerenderRuleSpec::default_profile();
        assert!(s.would_speculate("example.com", "https://example.com/page"));
        // Port-agnostic.
        assert!(s.would_speculate("example.com", "https://example.com:8080/page"));
    }

    #[test]
    fn would_speculate_prefix_selector() {
        let s = PrerenderRuleSpec {
            selector: SelectorKind::HrefPrefix,
            where_prefix: vec!["/chapter-".into(), "/appendix-".into()],
            ..PrerenderRuleSpec::default_profile()
        };
        assert!(s.would_speculate("ex.com", "/chapter-2"));
        assert!(s.would_speculate("ex.com", "/appendix-a"));
        assert!(!s.would_speculate("ex.com", "/random"));
    }

    #[test]
    fn would_speculate_skips_excluded_host() {
        let s = PrerenderRuleSpec {
            exclude_hosts: vec!["*://*.ads.com/*".into()],
            ..PrerenderRuleSpec::default_profile()
        };
        assert!(!s.would_speculate("tracker.ads.com", "/foo"));
    }

    #[test]
    fn would_speculate_disabled_rejects_all() {
        let s = PrerenderRuleSpec {
            enabled: false,
            selector: SelectorKind::SameOrigin,
            ..PrerenderRuleSpec::default_profile()
        };
        assert!(!s.would_speculate("example.com", "/page"));
    }

    #[test]
    fn would_speculate_regex_selector_requires_at_least_one_pattern() {
        let s = PrerenderRuleSpec {
            selector: SelectorKind::HrefRegex,
            where_regex: vec![],
            ..PrerenderRuleSpec::default_profile()
        };
        assert!(!s.would_speculate("ex.com", "/anything"));
    }

    #[test]
    fn mode_eagerness_roundtrip_through_serde() {
        for m in [SpeculationMode::Prefetch, SpeculationMode::Prerender] {
            for e in [
                Eagerness::Conservative,
                Eagerness::Moderate,
                Eagerness::Eager,
                Eagerness::Immediate,
            ] {
                let s = PrerenderRuleSpec {
                    mode: m,
                    eagerness: e,
                    ..PrerenderRuleSpec::default_profile()
                };
                let json = serde_json::to_string(&s).unwrap();
                let back: PrerenderRuleSpec = serde_json::from_str(&json).unwrap();
                assert_eq!(back.mode, m);
                assert_eq!(back.eagerness, e);
            }
        }
    }

    #[test]
    fn selector_roundtrips_through_serde() {
        for k in [
            SelectorKind::Explicit,
            SelectorKind::SameOrigin,
            SelectorKind::HrefPrefix,
            SelectorKind::HrefRegex,
            SelectorKind::DomSelector,
        ] {
            let s = PrerenderRuleSpec {
                selector: k,
                ..PrerenderRuleSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: PrerenderRuleSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.selector, k);
        }
    }

    #[test]
    fn network_isolation_roundtrips_through_serde() {
        for n in [
            NetworkIsolation::None,
            NetworkIsolation::AnonymousClientIpCrossOrigin,
        ] {
            let s = PrerenderRuleSpec {
                network_isolation: n,
                ..PrerenderRuleSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: PrerenderRuleSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.network_isolation, n);
        }
    }

    #[test]
    fn registry_rules_for_filters_host_and_enabled() {
        let mut reg = PrerenderRuleRegistry::new();
        reg.insert(PrerenderRuleSpec::default_profile());
        reg.insert(PrerenderRuleSpec {
            name: "gh".into(),
            host: "*://*.github.com/*".into(),
            ..PrerenderRuleSpec::default_profile()
        });
        reg.insert(PrerenderRuleSpec {
            name: "off".into(),
            enabled: false,
            ..PrerenderRuleSpec::default_profile()
        });
        assert_eq!(reg.rules_for("www.github.com").len(), 2); // default + gh
        assert_eq!(reg.rules_for("example.org").len(), 1); // default
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_prerender_rule_form() {
        let src = r#"
            (defprerender-rule :name "likely-next"
                               :host "*"
                               :mode "prerender"
                               :eagerness "moderate"
                               :selector "href-prefix"
                               :where-prefix ("/chapter-")
                               :relative-to "document"
                               :max-concurrent 2
                               :honor-save-data #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.mode, SpeculationMode::Prerender);
        assert_eq!(s.eagerness, Eagerness::Moderate);
        assert_eq!(s.selector, SelectorKind::HrefPrefix);
        assert_eq!(s.where_prefix.len(), 1);
        assert_eq!(s.max_concurrent, 2);
    }
}
