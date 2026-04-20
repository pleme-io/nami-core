//! `(defsuggestion-source)` — pluggable omnibox suggestion source.
//!
//! Absorbs Chrome Omnibox providers (HistoryURL, HistoryQuickProvider,
//! Bookmark, Search, Shortcuts), Firefox Awesome Bar sources, Arc
//! Command Bar, Raycast-style extensions. Each source is a weighted
//! contributor — the ranker (see [`crate::suggestion_ranker`]) merges
//! results across sources and orders them.
//!
//! ```lisp
//! (defsuggestion-source :name       "history"
//!                       :kind       :history
//!                       :weight     1.0
//!                       :max-results 8
//!                       :min-input-len 2
//!                       :host       "*"
//!                       :inline     #t)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Where suggestions come from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    /// Browsing history.
    #[default]
    History,
    /// Saved bookmarks.
    Bookmarks,
    /// Currently open tabs.
    OpenTabs,
    /// Matches from registered search engines ((defsearch-engine)).
    SearchEngines,
    /// Autocomplete from the currently-selected search engine's
    /// `suggest` endpoint.
    SearchSuggest,
    /// DuckDuckGo/Kagi !bang triggers ((defsearch-bang)).
    SearchBangs,
    /// Clipboard contents (one-off paste suggestion).
    Clipboard,
    /// URL deduced from partial input ("git" → "github.com").
    TopLevelDomain,
    /// Local files/apps (blackmatter-style launcher mode).
    LocalFiles,
    /// LLM-generated suggestions via (defllm-provider).
    Llm,
    /// Internal/extension-registered custom provider.
    Custom,
}

/// How the source surfaces a result.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SuggestionShape {
    /// Plain text row.
    #[default]
    Text,
    /// URL row (with favicon + url + title).
    Url,
    /// Rich row (screenshot + title + snippet).
    Rich,
    /// Action row (invokes a command, not a navigation).
    Action,
}

/// Suggestion-source profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsuggestion-source"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionSourceSpec {
    pub name: String,
    #[serde(default)]
    pub kind: SourceKind,
    /// Relative importance — the ranker multiplies raw scores by
    /// weight before merging. 0.0 disables, 1.0 is neutral.
    #[serde(default = "default_weight")]
    pub weight: f32,
    /// Max results this source contributes per query.
    #[serde(default = "default_max_results")]
    pub max_results: u32,
    /// Don't query this source until input length is >= this.
    #[serde(default)]
    pub min_input_len: u32,
    /// Host glob — omnibox behavior when the user is already on a
    /// page. "*" = always.
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    /// Allow this source to propose inline completions (typed URL
    /// gets auto-completed in place).
    #[serde(default)]
    pub inline: bool,
    /// Shape used when the ranker can't pick a shape on its own.
    #[serde(default)]
    pub shape: SuggestionShape,
    /// Max age in days for entries the source considers (0 = no
    /// age filter; applies to history/tabs).
    #[serde(default)]
    pub max_age_days: u32,
    /// Priority tiebreak — higher wins when two sources report the
    /// same final score.
    #[serde(default)]
    pub priority: i32,
    /// Extra opaque config (e.g. LLM provider name, custom endpoint).
    #[serde(default)]
    pub config: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_weight() -> f32 {
    1.0
}
fn default_max_results() -> u32 {
    6
}
fn default_enabled() -> bool {
    true
}

impl SuggestionSourceSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "history".into(),
            kind: SourceKind::History,
            weight: 1.0,
            max_results: 8,
            min_input_len: 1,
            host: "*".into(),
            inline: true,
            shape: SuggestionShape::Url,
            max_age_days: 365,
            priority: 0,
            config: None,
            enabled: true,
            description: Some("Default omnibox history source — 8 results, inline-completion on.".into()),
        }
    }

    /// Clamped weight in `[0.0, ∞)` — negative weights turn into 0.
    #[must_use]
    pub fn clamped_weight(&self) -> f32 {
        self.weight.max(0.0)
    }

    /// Is this source active for `input` on `host`?
    #[must_use]
    pub fn is_active(&self, input: &str, host: &str) -> bool {
        if !self.enabled {
            return false;
        }
        if input.chars().count() < self.min_input_len as usize {
            return false;
        }
        if self.clamped_weight() == 0.0 {
            return false;
        }
        crate::extension::host_pattern_matches(&self.host, host)
    }
}

/// Registry — sources accumulate in declaration order.
#[derive(Debug, Clone, Default)]
pub struct SuggestionSourceRegistry {
    specs: Vec<SuggestionSourceSpec>,
}

impl SuggestionSourceRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SuggestionSourceSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SuggestionSourceSpec>) {
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
    pub fn specs(&self) -> &[SuggestionSourceSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&SuggestionSourceSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// All sources by kind (enabled only).
    #[must_use]
    pub fn by_kind(&self, kind: SourceKind) -> Vec<&SuggestionSourceSpec> {
        self.specs
            .iter()
            .filter(|s| s.enabled && s.kind == kind)
            .collect()
    }

    /// Active sources for an omnibox query on `host`.
    #[must_use]
    pub fn active_for<'a>(
        &'a self,
        input: &str,
        host: &str,
    ) -> Vec<&'a SuggestionSourceSpec> {
        self.specs
            .iter()
            .filter(|s| s.is_active(input, host))
            .collect()
    }

    /// Total max_results budget across all enabled sources — useful
    /// for the ranker to pre-allocate a merge buffer.
    #[must_use]
    pub fn total_budget(&self) -> u32 {
        self.specs
            .iter()
            .filter(|s| s.enabled)
            .map(|s| s.max_results)
            .sum()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SuggestionSourceSpec>, String> {
    tatara_lisp::compile_typed::<SuggestionSourceSpec>(src)
        .map_err(|e| format!("failed to compile defsuggestion-source forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SuggestionSourceSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_history_with_weight_1() {
        let s = SuggestionSourceSpec::default_profile();
        assert_eq!(s.kind, SourceKind::History);
        assert!((s.weight - 1.0).abs() < 1e-5);
        assert!(s.inline);
        assert_eq!(s.max_results, 8);
    }

    #[test]
    fn clamped_weight_floors_at_zero() {
        let s = SuggestionSourceSpec {
            weight: -5.0,
            ..SuggestionSourceSpec::default_profile()
        };
        assert!(s.clamped_weight().abs() < 1e-5);
    }

    #[test]
    fn is_active_requires_enabled() {
        let s = SuggestionSourceSpec {
            enabled: false,
            ..SuggestionSourceSpec::default_profile()
        };
        assert!(!s.is_active("foo", "example.com"));
    }

    #[test]
    fn is_active_respects_min_input_len() {
        let s = SuggestionSourceSpec {
            min_input_len: 3,
            ..SuggestionSourceSpec::default_profile()
        };
        assert!(!s.is_active("ab", "example.com"));
        assert!(s.is_active("abc", "example.com"));
    }

    #[test]
    fn is_active_counts_unicode_codepoints() {
        // Japanese "hello" (5 codepoints, many bytes).
        let s = SuggestionSourceSpec {
            min_input_len: 5,
            ..SuggestionSourceSpec::default_profile()
        };
        assert!(s.is_active("こんにちは", "example.com"));
    }

    #[test]
    fn is_active_filters_by_host_glob() {
        let s = SuggestionSourceSpec {
            host: "*://*.example.com/*".into(),
            ..SuggestionSourceSpec::default_profile()
        };
        assert!(s.is_active("foo", "www.example.com"));
        assert!(!s.is_active("foo", "evil.com"));
    }

    #[test]
    fn is_active_rejects_zero_weight() {
        let s = SuggestionSourceSpec {
            weight: 0.0,
            ..SuggestionSourceSpec::default_profile()
        };
        assert!(!s.is_active("foo", "example.com"));
    }

    #[test]
    fn by_kind_filters_enabled_only() {
        let mut reg = SuggestionSourceRegistry::new();
        reg.insert(SuggestionSourceSpec::default_profile());
        reg.insert(SuggestionSourceSpec {
            name: "bookmarks".into(),
            kind: SourceKind::Bookmarks,
            ..SuggestionSourceSpec::default_profile()
        });
        reg.insert(SuggestionSourceSpec {
            name: "tabs-off".into(),
            kind: SourceKind::OpenTabs,
            enabled: false,
            ..SuggestionSourceSpec::default_profile()
        });
        assert_eq!(reg.by_kind(SourceKind::Bookmarks).len(), 1);
        assert!(reg.by_kind(SourceKind::OpenTabs).is_empty());
    }

    #[test]
    fn active_for_returns_only_matching_sources() {
        let mut reg = SuggestionSourceRegistry::new();
        reg.insert(SuggestionSourceSpec::default_profile());
        reg.insert(SuggestionSourceSpec {
            name: "gh".into(),
            host: "*://*.github.com/*".into(),
            ..SuggestionSourceSpec::default_profile()
        });
        let active = reg.active_for("foo", "www.github.com");
        assert_eq!(active.len(), 2); // default (host="*") + gh
        let other = reg.active_for("foo", "example.org");
        assert_eq!(other.len(), 1);
    }

    #[test]
    fn total_budget_sums_enabled_max_results() {
        let mut reg = SuggestionSourceRegistry::new();
        reg.insert(SuggestionSourceSpec {
            max_results: 5,
            ..SuggestionSourceSpec::default_profile()
        });
        reg.insert(SuggestionSourceSpec {
            name: "b".into(),
            max_results: 3,
            ..SuggestionSourceSpec::default_profile()
        });
        reg.insert(SuggestionSourceSpec {
            name: "c".into(),
            max_results: 100,
            enabled: false,
            ..SuggestionSourceSpec::default_profile()
        });
        assert_eq!(reg.total_budget(), 8);
    }

    #[test]
    fn source_kind_roundtrips_through_serde() {
        for k in [
            SourceKind::History,
            SourceKind::Bookmarks,
            SourceKind::OpenTabs,
            SourceKind::SearchEngines,
            SourceKind::SearchSuggest,
            SourceKind::SearchBangs,
            SourceKind::Clipboard,
            SourceKind::TopLevelDomain,
            SourceKind::LocalFiles,
            SourceKind::Llm,
            SourceKind::Custom,
        ] {
            let s = SuggestionSourceSpec {
                kind: k,
                ..SuggestionSourceSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: SuggestionSourceSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.kind, k);
        }
    }

    #[test]
    fn shape_roundtrips_through_serde() {
        for sh in [
            SuggestionShape::Text,
            SuggestionShape::Url,
            SuggestionShape::Rich,
            SuggestionShape::Action,
        ] {
            let s = SuggestionSourceSpec {
                shape: sh,
                ..SuggestionSourceSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: SuggestionSourceSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.shape, sh);
        }
    }

    #[test]
    fn registry_dedupes_on_name() {
        let mut reg = SuggestionSourceRegistry::new();
        reg.insert(SuggestionSourceSpec::default_profile());
        reg.insert(SuggestionSourceSpec {
            max_results: 99,
            ..SuggestionSourceSpec::default_profile()
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("history").unwrap().max_results, 99);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_suggestion_source_form() {
        let src = r#"
            (defsuggestion-source :name "bookmarks"
                                  :kind "bookmarks"
                                  :weight 0.75
                                  :max-results 5
                                  :min-input-len 2
                                  :host "*"
                                  :inline #f
                                  :shape "url"
                                  :priority 10)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "bookmarks");
        assert_eq!(s.kind, SourceKind::Bookmarks);
        assert!((s.weight - 0.75).abs() < 1e-5);
        assert_eq!(s.max_results, 5);
        assert!(!s.inline);
    }
}
