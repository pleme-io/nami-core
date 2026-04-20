//! `(defboost)` — per-site CSS / Lisp / JS injection.
//!
//! Absorbs Arc Boosts, Stylus/Stylish, Tampermonkey/Greasemonkey,
//! and the Brave "per-site shields" per-origin override pattern
//! into one substrate DSL. A boost is a named overlay that fires
//! when the host matches its glob, and injects:
//!
//!   :css          — raw stylesheet text (appended to `<head>`)
//!   :lisp         — tatara-lisp expression run via (defdom-transform)
//!   :js           — raw JavaScript (runs only when the J1 runtime
//!                   is loaded — see `crates/nami-core/docs/j1.md`)
//!   :normalize    — inline (defnormalize) rule(s)
//!   :blockers     — extra CSS selectors fed to the blocker pipeline
//!
//! Boosts compose with the rest of the substrate — they're not a
//! parallel surface. A boost's `:css` and `:js` go through the same
//! enforcement path as anything from `(defextension)`; its
//! `:normalize` rules participate in the same fold that reader uses.
//!
//! ```lisp
//! (defboost :name "hn-dark"
//!           :host "news.ycombinator.com"
//!           :css  "body { background:#111 !important; color:#eee !important; }"
//!           :description "Dark mode for Hacker News")
//!
//! (defboost :name "github-zen"
//!           :host "*://*.github.com/*"
//!           :css  ".Header, .footer { display: none !important; }"
//!           :blockers (".js-notification-shelf" ".pagehead-actions"))
//!
//! (defboost :name "everywhere-readable"
//!           :host "*"
//!           :css  "* { font-family: system-ui; line-height: 1.6; }")
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// One boost — a scoped overlay bundle.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defboost"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoostSpec {
    pub name: String,
    /// Host glob. `"*"` = everywhere.
    #[serde(default = "default_host")]
    pub host: String,
    /// Raw CSS to append to the page `<head>` after matched navigate.
    #[serde(default)]
    pub css: Option<String>,
    /// Tatara-lisp expression evaluated under the DOM-transform arena
    /// (same environment as `(defdom-transform :body …)`).
    #[serde(default)]
    pub lisp: Option<String>,
    /// Raw JavaScript. Executed only when the J1 runtime is loaded.
    /// Present in the spec so boosts round-trip through the store
    /// even when the runtime isn't loaded.
    #[serde(default)]
    pub js: Option<String>,
    /// Extra block-selectors — merged into the blocker pipeline
    /// for matched hosts.
    #[serde(default)]
    pub blockers: Vec<String>,
    /// Runtime toggle — disabled boosts stay declared but skipped.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_enabled() -> bool {
    true
}

impl BoostSpec {
    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    /// True if the boost has any non-empty injection payload.
    #[must_use]
    pub fn has_payload(&self) -> bool {
        self.css.as_deref().is_some_and(|c| !c.is_empty())
            || self.lisp.as_deref().is_some_and(|s| !s.is_empty())
            || self.js.as_deref().is_some_and(|s| !s.is_empty())
            || !self.blockers.is_empty()
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct BoostRegistry {
    specs: Vec<BoostSpec>,
}

impl BoostRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: BoostSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = BoostSpec>) {
        for s in specs {
            self.insert(s);
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
    pub fn specs(&self) -> &[BoostSpec] {
        &self.specs
    }

    /// Every enabled boost whose host matches, in insertion order.
    /// Multiple boosts can apply to the same host (e.g., a wildcard
    /// typography boost plus a host-specific dark-mode one).
    #[must_use]
    pub fn applicable(&self, host: &str) -> Vec<&BoostSpec> {
        self.specs
            .iter()
            .filter(|s| s.enabled && s.matches_host(host))
            .collect()
    }

    /// Merged CSS block — concatenate every applicable boost's CSS
    /// with blank-line separators. Caller injects as one `<style>`
    /// element for minimal DOM churn.
    #[must_use]
    pub fn merged_css(&self, host: &str) -> String {
        self.applicable(host)
            .iter()
            .filter_map(|b| b.css.as_ref())
            .filter(|s| !s.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Merged extra-blocker selectors across every applicable boost.
    #[must_use]
    pub fn merged_blocker_selectors(&self, host: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for b in self.applicable(host) {
            for s in &b.blockers {
                if !out.contains(s) {
                    out.push(s.clone());
                }
            }
        }
        out
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<BoostSpec>, String> {
    tatara_lisp::compile_typed::<BoostSpec>(src)
        .map_err(|e| format!("failed to compile defboost forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<BoostSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, host: &str) -> BoostSpec {
        BoostSpec {
            name: name.into(),
            host: host.into(),
            css: Some(format!("/* {name} */")),
            lisp: None,
            js: None,
            blockers: vec![],
            enabled: true,
            description: None,
        }
    }

    #[test]
    fn matches_host_wildcard() {
        let s = sample("x", "*");
        assert!(s.matches_host("example.com"));
    }

    #[test]
    fn matches_host_glob() {
        let s = sample("x", "*://*.github.com/*");
        assert!(s.matches_host("blog.github.com"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn has_payload_detects_any_field_set() {
        assert!(sample("x", "*").has_payload());
        let empty = BoostSpec {
            name: "x".into(),
            host: "*".into(),
            css: None,
            lisp: None,
            js: None,
            blockers: vec![],
            enabled: true,
            description: None,
        };
        assert!(!empty.has_payload());
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = BoostRegistry::new();
        reg.insert(sample("x", "*"));
        reg.insert(sample("x", "example.com"));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].host, "example.com");
    }

    #[test]
    fn set_enabled_toggles() {
        let mut reg = BoostRegistry::new();
        reg.insert(sample("x", "*"));
        assert!(reg.set_enabled("x", false));
        assert!(!reg.specs()[0].enabled);
        assert!(!reg.set_enabled("nonexistent", false));
    }

    #[test]
    fn applicable_excludes_disabled() {
        let mut reg = BoostRegistry::new();
        reg.insert(sample("a", "*"));
        reg.insert(sample("b", "*"));
        reg.set_enabled("b", false);
        let hits: Vec<&str> = reg
            .applicable("example.com")
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(hits, vec!["a"]);
    }

    #[test]
    fn applicable_excludes_non_matching_host() {
        let mut reg = BoostRegistry::new();
        reg.insert(sample("only-github", "*://*.github.com/*"));
        reg.insert(sample("everywhere", "*"));
        let on_example: Vec<&str> = reg
            .applicable("example.com")
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(on_example, vec!["everywhere"]);
    }

    #[test]
    fn merged_css_concatenates_applicable_boosts() {
        let mut reg = BoostRegistry::new();
        reg.insert(BoostSpec {
            name: "base".into(),
            host: "*".into(),
            css: Some("body { font: system-ui; }".into()),
            ..sample("base", "*")
        });
        reg.insert(BoostSpec {
            name: "dark".into(),
            host: "example.com".into(),
            css: Some("body { background: #000; }".into()),
            ..sample("dark", "example.com")
        });
        let merged = reg.merged_css("example.com");
        assert!(merged.contains("font: system-ui"));
        assert!(merged.contains("background: #000"));
    }

    #[test]
    fn merged_blocker_selectors_dedupe() {
        let mut reg = BoostRegistry::new();
        reg.insert(BoostSpec {
            name: "a".into(),
            blockers: vec![".ad".into(), ".promo".into()],
            ..sample("a", "*")
        });
        reg.insert(BoostSpec {
            name: "b".into(),
            blockers: vec![".promo".into(), ".tracker".into()],
            ..sample("b", "*")
        });
        let merged = reg.merged_blocker_selectors("example.com");
        assert_eq!(merged.len(), 3);
        assert!(merged.iter().any(|s| s == ".ad"));
        assert!(merged.iter().any(|s| s == ".promo"));
        assert!(merged.iter().any(|s| s == ".tracker"));
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_boost_form() {
        let src = r#"
            (defboost :name "hn-dark"
                      :host "news.ycombinator.com"
                      :css  "body { background:#111; }"
                      :blockers (".ad" ".promo"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "hn-dark");
        assert_eq!(s.host, "news.ycombinator.com");
        assert!(s.css.as_deref().unwrap().contains("#111"));
        assert_eq!(s.blockers.len(), 2);
    }
}
