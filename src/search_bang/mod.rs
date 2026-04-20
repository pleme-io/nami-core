//! `(defsearch-bang)` — DuckDuckGo/Kagi-style `!bang` shortcuts.
//!
//! A bang is a prefix (`!`) + short trigger that routes the query to
//! a target engine. `!gh rust async` → GitHub search for "rust async".
//! DDG ships 13,000+ of these; Kagi has its own curated list. One
//! Lisp form per bang lets you customise the whole surface.
//!
//! ```lisp
//! (defsearch-bang :trigger "gh"
//!                 :engine  "github"
//!                 :url     "https://github.com/search?q=%s&type=code"
//!                 :category :code
//!                 :priority 10
//!                 :description "GitHub code search")
//! ```
//!
//! `engine` can be omitted when `url` is set directly — useful for
//! one-shot bangs that don't warrant a full engine profile.

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Position of the bang token in the query.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum BangPosition {
    /// `!trigger query` (default — DuckDuckGo convention).
    #[default]
    Leading,
    /// `query !trigger` (DuckDuckGo also accepts this).
    Trailing,
    /// Either position is accepted.
    Either,
}

/// Category — absorbs same taxonomy as (defsearch-engine).
pub use crate::search_engine::SearchCategory;

/// Bang profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsearch-bang"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchBangSpec {
    /// Trigger text (without the leading `!`). e.g. `"gh"`.
    pub trigger: String,
    /// Optional name of a `(defsearch-engine)` to route through.
    /// Ignored when `url` is also set.
    #[serde(default)]
    pub engine: Option<String>,
    /// Direct URL template with `%s` / `{query}` substitution.
    /// Takes precedence over `engine`.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub position: BangPosition,
    /// If true the bang is applied even when the trigger starts with
    /// uppercase letters (`!GH`). Default true (case-insensitive).
    #[serde(default = "default_case_insensitive")]
    pub case_insensitive: bool,
    #[serde(default)]
    pub category: SearchCategory,
    /// Priority — higher wins when two bangs share a trigger.
    #[serde(default)]
    pub priority: i32,
    /// Favicon URL for omnibox rendering.
    #[serde(default)]
    pub favicon: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_case_insensitive() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

impl SearchBangSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            trigger: "g".into(),
            engine: None,
            url: Some("https://www.google.com/search?q=%s".into()),
            position: BangPosition::Leading,
            case_insensitive: true,
            category: SearchCategory::Web,
            priority: 0,
            favicon: None,
            enabled: true,
            description: Some("Default bang — `!g` routes to Google.".into()),
        }
    }

    /// Detect this bang in `input`. Returns the remaining query text
    /// (bang stripped) if matched, else None.
    #[must_use]
    pub fn detect<'a>(&self, input: &'a str) -> Option<&'a str> {
        let token = format!("!{}", self.trigger);
        let matches = |tok: &str, s: &str| -> bool {
            if self.case_insensitive {
                s.eq_ignore_ascii_case(tok)
            } else {
                s == tok
            }
        };

        match self.position {
            BangPosition::Leading => detect_leading(input, &token, matches),
            BangPosition::Trailing => detect_trailing(input, &token, matches),
            BangPosition::Either => detect_leading(input, &token, matches)
                .or_else(|| detect_trailing(input, &token, matches)),
        }
    }
}

fn detect_leading<'a>(
    input: &'a str,
    token: &str,
    matches: impl Fn(&str, &str) -> bool,
) -> Option<&'a str> {
    let trimmed = input.trim_start();
    let (first, rest) = trimmed.split_once(char::is_whitespace).unwrap_or((trimmed, ""));
    if matches(token, first) {
        Some(rest.trim_start())
    } else {
        None
    }
}

fn detect_trailing<'a>(
    input: &'a str,
    token: &str,
    matches: impl Fn(&str, &str) -> bool,
) -> Option<&'a str> {
    let trimmed = input.trim_end();
    let (rest, last) = trimmed.rsplit_once(char::is_whitespace).unwrap_or(("", trimmed));
    if matches(token, last) {
        Some(rest.trim_end())
    } else {
        None
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct SearchBangRegistry {
    specs: Vec<SearchBangSpec>,
}

impl SearchBangRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SearchBangSpec) {
        self.specs.retain(|s| s.trigger != spec.trigger);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SearchBangSpec>) {
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
    pub fn specs(&self) -> &[SearchBangSpec] {
        &self.specs
    }

    /// Scan `input` against every enabled bang. Returns the first
    /// match with the highest priority. Tie-break is insertion order
    /// (first-in wins among equals).
    #[must_use]
    pub fn detect<'a, 'b>(&'a self, input: &'b str) -> Option<BangMatch<'a, 'b>> {
        self.specs
            .iter()
            .filter(|s| s.enabled)
            .filter_map(|s| s.detect(input).map(|rest| (s, rest)))
            .max_by_key(|(s, _)| s.priority)
            .map(|(spec, rest)| BangMatch { spec, remaining: rest })
    }

    #[must_use]
    pub fn by_trigger(&self, trigger: &str) -> Option<&SearchBangSpec> {
        let want = trigger.trim_start_matches('!');
        self.specs.iter().find(|s| s.trigger == want)
    }
}

/// Outcome of detecting a bang in an omnibox input.
#[derive(Debug, Clone, Copy)]
pub struct BangMatch<'a, 'b> {
    pub spec: &'a SearchBangSpec,
    /// The query text with the bang token stripped.
    pub remaining: &'b str,
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SearchBangSpec>, String> {
    tatara_lisp::compile_typed::<SearchBangSpec>(src)
        .map_err(|e| format!("failed to compile defsearch-bang forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SearchBangSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_triggers_on_g() {
        let s = SearchBangSpec::default_profile();
        assert_eq!(s.trigger, "g");
        assert!(s.case_insensitive);
    }

    #[test]
    fn detect_leading_strips_bang_token() {
        let s = SearchBangSpec::default_profile();
        assert_eq!(s.detect("!g rust async"), Some("rust async"));
    }

    #[test]
    fn detect_case_insensitive_by_default() {
        let s = SearchBangSpec::default_profile();
        assert_eq!(s.detect("!G rust"), Some("rust"));
    }

    #[test]
    fn detect_case_sensitive_when_flag_off() {
        let s = SearchBangSpec {
            case_insensitive: false,
            ..SearchBangSpec::default_profile()
        };
        assert_eq!(s.detect("!G rust"), None);
        assert_eq!(s.detect("!g rust"), Some("rust"));
    }

    #[test]
    fn detect_returns_none_when_bang_is_inline() {
        let s = SearchBangSpec::default_profile();
        // No bang at a position boundary — returns None.
        assert!(s.detect("rust!g").is_none());
    }

    #[test]
    fn detect_trailing_strips_bang() {
        let s = SearchBangSpec {
            position: BangPosition::Trailing,
            ..SearchBangSpec::default_profile()
        };
        assert_eq!(s.detect("rust async !g"), Some("rust async"));
    }

    #[test]
    fn detect_either_accepts_both_positions() {
        let s = SearchBangSpec {
            position: BangPosition::Either,
            ..SearchBangSpec::default_profile()
        };
        assert_eq!(s.detect("!g foo"), Some("foo"));
        assert_eq!(s.detect("foo !g"), Some("foo"));
    }

    #[test]
    fn detect_leading_handles_leading_whitespace() {
        let s = SearchBangSpec::default_profile();
        assert_eq!(s.detect("   !g   rust   "), Some("rust   "));
    }

    #[test]
    fn registry_picks_highest_priority_bang_on_collision() {
        let mut reg = SearchBangRegistry::new();
        reg.insert(SearchBangSpec {
            trigger: "a".into(),
            priority: 0,
            description: Some("low".into()),
            ..SearchBangSpec::default_profile()
        });
        // Different trigger `b` so both stay in the registry.
        reg.insert(SearchBangSpec {
            trigger: "b".into(),
            priority: 100,
            description: Some("high".into()),
            ..SearchBangSpec::default_profile()
        });
        // Only the `b` bang matches — highest priority.
        let m = reg.detect("!b rust").unwrap();
        assert_eq!(m.spec.trigger, "b");
        assert_eq!(m.remaining, "rust");
    }

    #[test]
    fn registry_dedupes_by_trigger() {
        let mut reg = SearchBangRegistry::new();
        reg.insert(SearchBangSpec {
            trigger: "g".into(),
            priority: 0,
            ..SearchBangSpec::default_profile()
        });
        reg.insert(SearchBangSpec {
            trigger: "g".into(),
            priority: 50,
            ..SearchBangSpec::default_profile()
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.by_trigger("g").unwrap().priority, 50);
    }

    #[test]
    fn disabled_bangs_are_not_detected() {
        let mut reg = SearchBangRegistry::new();
        reg.insert(SearchBangSpec {
            trigger: "g".into(),
            enabled: false,
            ..SearchBangSpec::default_profile()
        });
        assert!(reg.detect("!g rust").is_none());
    }

    #[test]
    fn by_trigger_strips_leading_bang() {
        let mut reg = SearchBangRegistry::new();
        reg.insert(SearchBangSpec::default_profile());
        assert!(reg.by_trigger("g").is_some());
        assert!(reg.by_trigger("!g").is_some());
    }

    #[test]
    fn position_roundtrips_through_serde() {
        for p in [
            BangPosition::Leading,
            BangPosition::Trailing,
            BangPosition::Either,
        ] {
            let s = SearchBangSpec {
                position: p,
                ..SearchBangSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: SearchBangSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.position, p);
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_search_bang_form() {
        let src = r#"
            (defsearch-bang :trigger "gh"
                            :engine "github"
                            :url "https://github.com/search?q=%s&type=code"
                            :position "leading"
                            :case-insensitive #t
                            :category "code"
                            :priority 10)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.trigger, "gh");
        assert_eq!(s.engine.as_deref(), Some("github"));
        assert_eq!(s.position, BangPosition::Leading);
        assert_eq!(s.priority, 10);
    }
}
