//! `(defsuggestion-ranker)` — omnibox scoring + merge strategy.
//!
//! Pairs with [`crate::suggestion_source`]. The ranker declares HOW
//! results from sources get scored and merged. Absorbs Chrome Omnibox
//! scoring (HQP relevance + shortcut boost + typed-count decay),
//! Firefox Awesome Bar frecency, Arc's "most relevant" merge, Raycast
//! fuzzy + recent-first.
//!
//! ```lisp
//! (defsuggestion-ranker :name         "default"
//!                       :strategy     :hybrid
//!                       :decay-half-life-days 14
//!                       :fuzzy-threshold 0.6
//!                       :inline-threshold 0.9
//!                       :dedupe       :by-url
//!                       :max-results  10
//!                       :prefer-inputs (exact-prefix fuzzy recent))
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Scoring strategy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RankStrategy {
    /// Last-visited wins.
    Recency,
    /// Visit count wins.
    Frequency,
    /// String-match relevance wins.
    Relevance,
    /// Firefox frecency (frequency * recency decay).
    Frecency,
    /// Chrome HQP-style — relevance + typed-count + recency.
    #[default]
    Hybrid,
    /// Alphabetical — for deterministic test pipelines.
    Alphabetic,
}

/// Dedupe strategy when the same target appears in multiple sources.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DedupePolicy {
    /// No dedupe — show everything (usually noisy).
    None,
    /// Group by URL — best URL match wins.
    #[default]
    ByUrl,
    /// Group by registrable domain.
    ByDomain,
    /// Group by title — kills "same page reached twice" noise.
    ByTitle,
    /// Group by the (kind, url) pair — preserves search vs history.
    ByKindUrl,
}

/// Match types preferred by the ranker in priority order.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum MatchInput {
    /// Exact prefix of the user input.
    ExactPrefix,
    /// Case-insensitive prefix.
    CaseInsensitivePrefix,
    /// Token-order-preserving substring match.
    SubstringOrdered,
    /// Any-order substring match.
    SubstringAny,
    /// Fuzzy subsequence match (ranked by threshold).
    Fuzzy,
    /// Most-recently-visited items even without a text match.
    Recent,
    /// Most-frequently-visited items even without a text match.
    Frequent,
}

/// Ranker profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsuggestion-ranker"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionRankerSpec {
    pub name: String,
    #[serde(default)]
    pub strategy: RankStrategy,
    /// Recency decay half-life in days — result age `t` contributes a
    /// multiplier of `0.5 ^ (t / half_life)`. 0 = no decay.
    #[serde(default = "default_decay_half_life_days")]
    pub decay_half_life_days: f32,
    /// Fuzzy-match acceptance threshold in [0.0, 1.0]. 1.0 = only
    /// exact prefix accepted; 0.0 = any sequence accepted.
    #[serde(default = "default_fuzzy_threshold")]
    pub fuzzy_threshold: f32,
    /// Minimum score to offer an inline completion (URL auto-fill).
    #[serde(default = "default_inline_threshold")]
    pub inline_threshold: f32,
    #[serde(default)]
    pub dedupe: DedupePolicy,
    /// Match-input priority order.
    #[serde(default = "default_prefer_inputs")]
    pub prefer_inputs: Vec<MatchInput>,
    /// Boost applied to typed-URL matches (Chrome "shortcut") — >= 1.
    #[serde(default = "default_typed_boost")]
    pub typed_boost: f32,
    /// Boost applied to open-tab matches — keeps "switch to tab"
    /// stable at the top.
    #[serde(default = "default_open_tab_boost")]
    pub open_tab_boost: f32,
    /// Maximum results to emit after merge.
    #[serde(default = "default_max_results")]
    pub max_results: u32,
    /// Case-sensitive matching?
    #[serde(default)]
    pub case_sensitive: bool,
    /// Which (defsuggestion-source) names this ranker applies to.
    /// Empty = all sources.
    #[serde(default)]
    pub source_names: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_decay_half_life_days() -> f32 {
    14.0
}
fn default_fuzzy_threshold() -> f32 {
    0.6
}
fn default_inline_threshold() -> f32 {
    0.9
}
fn default_prefer_inputs() -> Vec<MatchInput> {
    vec![
        MatchInput::ExactPrefix,
        MatchInput::CaseInsensitivePrefix,
        MatchInput::SubstringOrdered,
        MatchInput::Fuzzy,
        MatchInput::Recent,
    ]
}
fn default_typed_boost() -> f32 {
    1.5
}
fn default_open_tab_boost() -> f32 {
    1.25
}
fn default_max_results() -> u32 {
    10
}
fn default_enabled() -> bool {
    true
}

impl SuggestionRankerSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            strategy: RankStrategy::Hybrid,
            decay_half_life_days: 14.0,
            fuzzy_threshold: 0.6,
            inline_threshold: 0.9,
            dedupe: DedupePolicy::ByUrl,
            prefer_inputs: default_prefer_inputs(),
            typed_boost: 1.5,
            open_tab_boost: 1.25,
            max_results: 10,
            case_sensitive: false,
            source_names: vec![],
            enabled: true,
            description: Some(
                "Default ranker — Hybrid strategy, 14-day half-life, by-url dedupe, 10 results.".into(),
            ),
        }
    }

    /// Recency-decay multiplier for an entry whose age is `age_days`.
    /// Returns a value in (0, 1].
    #[must_use]
    pub fn decay(&self, age_days: f32) -> f32 {
        if self.decay_half_life_days <= 0.0 {
            return 1.0;
        }
        let age = age_days.max(0.0);
        0.5f32.powf(age / self.decay_half_life_days)
    }

    /// Is a candidate's raw `score` above the fuzzy-accept floor?
    #[must_use]
    pub fn accept_fuzzy(&self, score: f32) -> bool {
        score >= self.fuzzy_threshold.clamp(0.0, 1.0)
    }

    /// Should the ranker offer this candidate as an inline completion?
    #[must_use]
    pub fn offer_inline(&self, score: f32) -> bool {
        score >= self.inline_threshold.clamp(0.0, 1.0)
    }

    /// Does this ranker apply to `source_name`?
    #[must_use]
    pub fn applies_to(&self, source_name: &str) -> bool {
        self.source_names.is_empty()
            || self.source_names.iter().any(|n| n == source_name)
    }

    /// Clamp a score into [0.0, 1.0] — useful for caller to invoke
    /// before comparing against thresholds.
    #[must_use]
    pub fn clamp_score(score: f32) -> f32 {
        score.clamp(0.0, 1.0)
    }
}

/// Registry — usually one ranker is active; but host-scoped rankers
/// (e.g. "ranker for github.com" favouring Frequency) are supported.
#[derive(Debug, Clone, Default)]
pub struct SuggestionRankerRegistry {
    specs: Vec<SuggestionRankerSpec>,
}

impl SuggestionRankerRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SuggestionRankerSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SuggestionRankerSpec>) {
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
    pub fn specs(&self) -> &[SuggestionRankerSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&SuggestionRankerSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// The ranker responsible for `source_name`. Returns the first
    /// enabled ranker that applies to this source — source-specific
    /// rankers are not ordered; last-wins is handled by insertion
    /// dedupe.
    #[must_use]
    pub fn for_source(&self, source_name: &str) -> Option<&SuggestionRankerSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.source_names.is_empty() && s.applies_to(source_name));
        specific.or_else(|| {
            self.specs
                .iter()
                .find(|s| s.enabled && s.source_names.is_empty())
        })
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SuggestionRankerSpec>, String> {
    tatara_lisp::compile_typed::<SuggestionRankerSpec>(src)
        .map_err(|e| format!("failed to compile defsuggestion-ranker forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SuggestionRankerSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_hybrid_with_14_day_half_life() {
        let s = SuggestionRankerSpec::default_profile();
        assert_eq!(s.strategy, RankStrategy::Hybrid);
        assert!((s.decay_half_life_days - 14.0).abs() < 1e-5);
        assert_eq!(s.dedupe, DedupePolicy::ByUrl);
    }

    #[test]
    fn decay_zero_days_is_one() {
        let s = SuggestionRankerSpec::default_profile();
        assert!((s.decay(0.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn decay_half_life_produces_point_five() {
        let s = SuggestionRankerSpec::default_profile();
        assert!((s.decay(14.0) - 0.5).abs() < 1e-5);
        assert!((s.decay(28.0) - 0.25).abs() < 1e-5);
    }

    #[test]
    fn decay_zero_half_life_is_no_decay() {
        let s = SuggestionRankerSpec {
            decay_half_life_days: 0.0,
            ..SuggestionRankerSpec::default_profile()
        };
        assert!((s.decay(1000.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn decay_negative_age_treated_as_zero() {
        let s = SuggestionRankerSpec::default_profile();
        // Clock-skew case — should not blow up scores.
        assert!((s.decay(-7.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn accept_fuzzy_respects_threshold() {
        let s = SuggestionRankerSpec {
            fuzzy_threshold: 0.7,
            ..SuggestionRankerSpec::default_profile()
        };
        assert!(!s.accept_fuzzy(0.5));
        assert!(s.accept_fuzzy(0.7));
        assert!(s.accept_fuzzy(0.99));
    }

    #[test]
    fn offer_inline_requires_high_score() {
        let s = SuggestionRankerSpec::default_profile();
        assert!(!s.offer_inline(0.5));
        assert!(s.offer_inline(0.95));
    }

    #[test]
    fn applies_to_empty_source_names_matches_all() {
        let s = SuggestionRankerSpec::default_profile();
        assert!(s.applies_to("history"));
        assert!(s.applies_to("bookmarks"));
    }

    #[test]
    fn applies_to_named_sources_is_exclusive() {
        let s = SuggestionRankerSpec {
            source_names: vec!["history".into()],
            ..SuggestionRankerSpec::default_profile()
        };
        assert!(s.applies_to("history"));
        assert!(!s.applies_to("bookmarks"));
    }

    #[test]
    fn clamp_score_bounds() {
        assert!((SuggestionRankerSpec::clamp_score(2.5) - 1.0).abs() < 1e-5);
        assert!(SuggestionRankerSpec::clamp_score(-0.5).abs() < 1e-5);
        assert!((SuggestionRankerSpec::clamp_score(0.5) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn for_source_prefers_source_specific_ranker() {
        let mut reg = SuggestionRankerRegistry::new();
        reg.insert(SuggestionRankerSpec::default_profile());
        reg.insert(SuggestionRankerSpec {
            name: "history-heavy".into(),
            strategy: RankStrategy::Frecency,
            source_names: vec!["history".into()],
            ..SuggestionRankerSpec::default_profile()
        });
        assert_eq!(
            reg.for_source("history").unwrap().strategy,
            RankStrategy::Frecency
        );
        // Non-matching source falls back to the global default.
        assert_eq!(
            reg.for_source("bookmarks").unwrap().name,
            "default"
        );
    }

    #[test]
    fn for_source_returns_none_when_only_disabled_exist() {
        let mut reg = SuggestionRankerRegistry::new();
        reg.insert(SuggestionRankerSpec {
            enabled: false,
            ..SuggestionRankerSpec::default_profile()
        });
        assert!(reg.for_source("anything").is_none());
    }

    #[test]
    fn strategy_roundtrips_through_serde() {
        for s in [
            RankStrategy::Recency,
            RankStrategy::Frequency,
            RankStrategy::Relevance,
            RankStrategy::Frecency,
            RankStrategy::Hybrid,
            RankStrategy::Alphabetic,
        ] {
            let spec = SuggestionRankerSpec {
                strategy: s,
                ..SuggestionRankerSpec::default_profile()
            };
            let json = serde_json::to_string(&spec).unwrap();
            let back: SuggestionRankerSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.strategy, s);
        }
    }

    #[test]
    fn dedupe_policy_roundtrips_through_serde() {
        for d in [
            DedupePolicy::None,
            DedupePolicy::ByUrl,
            DedupePolicy::ByDomain,
            DedupePolicy::ByTitle,
            DedupePolicy::ByKindUrl,
        ] {
            let s = SuggestionRankerSpec {
                dedupe: d,
                ..SuggestionRankerSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: SuggestionRankerSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.dedupe, d);
        }
    }

    #[test]
    fn match_input_roundtrips_through_serde() {
        let s = SuggestionRankerSpec {
            prefer_inputs: vec![
                MatchInput::ExactPrefix,
                MatchInput::CaseInsensitivePrefix,
                MatchInput::SubstringOrdered,
                MatchInput::SubstringAny,
                MatchInput::Fuzzy,
                MatchInput::Recent,
                MatchInput::Frequent,
            ],
            ..SuggestionRankerSpec::default_profile()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: SuggestionRankerSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.prefer_inputs.len(), 7);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_suggestion_ranker_form() {
        let src = r#"
            (defsuggestion-ranker :name "default"
                                  :strategy "hybrid"
                                  :decay-half-life-days 14
                                  :fuzzy-threshold 0.6
                                  :inline-threshold 0.9
                                  :dedupe "by-url"
                                  :max-results 10
                                  :typed-boost 1.5
                                  :open-tab-boost 1.25
                                  :prefer-inputs ("exact-prefix" "fuzzy" "recent"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.strategy, RankStrategy::Hybrid);
        assert_eq!(s.dedupe, DedupePolicy::ByUrl);
        assert_eq!(s.prefer_inputs.len(), 3);
        assert!((s.typed_boost - 1.5).abs() < 1e-5);
    }
}
