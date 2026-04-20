//! `(defsmart-bookmark)` — AI-augmented bookmarks.
//!
//! **Novel** — every mainstream browser ships bookmarks as static
//! (URL, title, favicon) triples. Some extensions (Raindrop,
//! Pinboard, Instapaper) add tags + summaries on top via opaque
//! cloud services. Nobody lets the user declare, in Lisp, that
//! certain hosts become "smart bookmarks" that pull an LLM (named
//! by `(defllm-provider)` profile) to auto-tag + auto-summary +
//! auto-find-related-links on save.
//!
//! Composes with `(defllm-provider)` + `(defsummarize)` + `(defsync)`.
//!
//! ```lisp
//! (defsmart-bookmark :name            "research"
//!                    :host            "*://*.arxiv.org/*"
//!                    :llm             "local-opus"
//!                    :auto-tag        #t
//!                    :max-tags        6
//!                    :auto-summary    #t
//!                    :auto-relate     #t
//!                    :related-limit   5
//!                    :index-trigger   :on-save
//!                    :summary-template "{title} — {topics-3}. Key idea: {tl-dr}."
//!                    :related-metric  :embedding-cosine)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// When the indexing pipeline runs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum IndexTrigger {
    /// Fire once, right when the bookmark is saved.
    #[default]
    OnSave,
    /// Fire on save + re-index on a fixed cadence.
    OnSaveAndPeriodic,
    /// Never fire automatically; user hits "Index now".
    Manual,
    /// Fire only when a `(defsync)` peer pushes a new version of
    /// this bookmark — lets a fast device index for a slow one.
    OnSyncUpdate,
}

/// Similarity metric used for `auto_relate`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RelatedMetric {
    /// Cosine over LLM-provider embeddings. Default.
    #[default]
    EmbeddingCosine,
    /// BM25 on title + tags + summary.
    Bm25TitleTags,
    /// Jaccard over shared tags only.
    JaccardTags,
    /// Random (dev + test).
    Random,
}

/// What the smart-bookmark indexer fills in beyond (url, title).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum EnrichmentField {
    Title,
    Tags,
    Summary,
    Topics,
    KeyQuotes,
    /// People/orgs/places entities.
    NamedEntities,
    /// Inferred reading time in minutes.
    ReadingTime,
    /// Simple sentiment rating.
    Sentiment,
    /// BLAKE3 26-char content ID (via crate::extension::base32_16).
    ContentId,
    /// Small screenshot thumbnail.
    Thumbnail,
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsmart-bookmark"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SmartBookmarkSpec {
    pub name: String,
    /// Host glob that qualifies a page for smart indexing.
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    /// `(defllm-provider)` profile name to use.
    #[serde(default = "default_llm")]
    pub llm: String,
    /// Which enrichment fields to populate.
    #[serde(default = "default_enrich")]
    pub enrich: Vec<EnrichmentField>,
    #[serde(default = "default_auto_tag")]
    pub auto_tag: bool,
    #[serde(default = "default_max_tags")]
    pub max_tags: u32,
    /// If true, the tag set is kept to unique tokens; duplicates
    /// produced by the LLM are dropped.
    #[serde(default = "default_dedupe_tags")]
    pub dedupe_tags: bool,
    #[serde(default = "default_auto_summary")]
    pub auto_summary: bool,
    /// Summary template — tokens: `{title}`, `{tl-dr}`, `{topics-N}`
    /// (top N topics joined by ", "), `{tags}`, `{url}`.
    #[serde(default)]
    pub summary_template: Option<String>,
    /// Desired summary length in characters (soft).
    #[serde(default = "default_summary_chars")]
    pub summary_chars: u32,
    #[serde(default = "default_auto_relate")]
    pub auto_relate: bool,
    #[serde(default = "default_related_limit")]
    pub related_limit: u32,
    #[serde(default)]
    pub related_metric: RelatedMetric,
    /// Minimum similarity score for an auto-related link (0..1).
    #[serde(default = "default_related_threshold")]
    pub related_threshold: f32,
    #[serde(default)]
    pub index_trigger: IndexTrigger,
    /// Cadence for `OnSaveAndPeriodic` in hours (0 = disable periodic).
    #[serde(default)]
    pub reindex_hours: u32,
    /// Redact any enrichment field value that looks like a password
    /// / API key / 2FA secret before storing.
    #[serde(default = "default_redact")]
    pub redact_secrets: bool,
    /// Ship enrichment through `(defsync)` so other devices inherit
    /// the index without re-running the LLM.
    #[serde(default = "default_sync")]
    pub sync_enrichment: bool,
    /// `(defaudit-trail)` event emission when enrichment happens.
    #[serde(default)]
    pub audit: bool,
    /// Hosts exempt from smart indexing (banking, health, …).
    #[serde(default)]
    pub exempt_hosts: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_llm() -> String {
    "default".into()
}
fn default_enrich() -> Vec<EnrichmentField> {
    vec![
        EnrichmentField::Title,
        EnrichmentField::Tags,
        EnrichmentField::Summary,
        EnrichmentField::Topics,
        EnrichmentField::ReadingTime,
        EnrichmentField::ContentId,
    ]
}
fn default_auto_tag() -> bool {
    true
}
fn default_max_tags() -> u32 {
    6
}
fn default_dedupe_tags() -> bool {
    true
}
fn default_auto_summary() -> bool {
    true
}
fn default_summary_chars() -> u32 {
    280
}
fn default_auto_relate() -> bool {
    true
}
fn default_related_limit() -> u32 {
    5
}
fn default_related_threshold() -> f32 {
    0.6
}
fn default_redact() -> bool {
    true
}
fn default_sync() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

impl SmartBookmarkSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            llm: "default".into(),
            enrich: default_enrich(),
            auto_tag: true,
            max_tags: 6,
            dedupe_tags: true,
            auto_summary: true,
            summary_template: None,
            summary_chars: 280,
            auto_relate: true,
            related_limit: 5,
            related_metric: RelatedMetric::EmbeddingCosine,
            related_threshold: 0.6,
            index_trigger: IndexTrigger::OnSave,
            reindex_hours: 0,
            redact_secrets: true,
            sync_enrichment: true,
            audit: false,
            exempt_hosts: vec![],
            enabled: true,
            description: Some(
                "Default smart bookmark — on-save index: 6 tags + 280-char summary + top-5 related (cosine ≥ 0.6).".into(),
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
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    #[must_use]
    pub fn captures(&self, field: EnrichmentField) -> bool {
        self.enrich.contains(&field)
    }

    /// Clamp `related_threshold` into `[0.0, 1.0]`.
    #[must_use]
    pub fn clamped_threshold(&self) -> f32 {
        self.related_threshold.clamp(0.0, 1.0)
    }

    /// Cap a raw tag vec to `max_tags` + dedupe (case-insensitive)
    /// if requested.
    #[must_use]
    pub fn shape_tags(&self, raw: Vec<String>) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for t in raw {
            let trimmed = t.trim();
            if trimmed.is_empty() {
                continue;
            }
            if self.dedupe_tags {
                if out.iter().any(|e| e.eq_ignore_ascii_case(trimmed)) {
                    continue;
                }
            }
            out.push(trimmed.to_owned());
            if self.max_tags != 0 && out.len() as u32 >= self.max_tags {
                break;
            }
        }
        out
    }

    /// Render the summary for (title, tl_dr, topics, tags, url)
    /// using the configured template. Returns None if no template.
    #[must_use]
    pub fn render_summary(
        &self,
        title: &str,
        tl_dr: &str,
        topics: &[String],
        tags: &[String],
        url: &str,
    ) -> Option<String> {
        let t = self.summary_template.as_deref()?;
        let mut out = t
            .replace("{title}", title)
            .replace("{tl-dr}", tl_dr)
            .replace("{tags}", &tags.join(", "))
            .replace("{url}", url);
        // {topics-N} — top N topics joined by ", ".
        // Scan for "{topics-" occurrences.
        while let Some(start) = out.find("{topics-") {
            let after = &out[start + "{topics-".len()..];
            let end_idx = match after.find('}') {
                Some(i) => i,
                None => break,
            };
            let n_str = &after[..end_idx];
            let n: usize = n_str.parse().unwrap_or(3);
            let joined = topics.iter().take(n).cloned().collect::<Vec<_>>().join(", ");
            let full = format!("{{topics-{n_str}}}");
            out = out.replace(&full, &joined);
        }
        Some(out)
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct SmartBookmarkRegistry {
    specs: Vec<SmartBookmarkSpec>,
}

impl SmartBookmarkRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SmartBookmarkSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SmartBookmarkSpec>) {
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
    pub fn specs(&self) -> &[SmartBookmarkSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&SmartBookmarkSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host) && !s.is_exempt(host));
        specific.or_else(|| {
            self.specs
                .iter()
                .find(|s| s.enabled && s.matches_host(host) && !s.is_exempt(host))
        })
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SmartBookmarkSpec>, String> {
    tatara_lisp::compile_typed::<SmartBookmarkSpec>(src)
        .map_err(|e| format!("failed to compile defsmart-bookmark forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SmartBookmarkSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_covers_tag_summary_related() {
        let s = SmartBookmarkSpec::default_profile();
        assert!(s.auto_tag);
        assert!(s.auto_summary);
        assert!(s.auto_relate);
        assert_eq!(s.max_tags, 6);
        assert_eq!(s.index_trigger, IndexTrigger::OnSave);
    }

    #[test]
    fn captures_default_enrichment_fields() {
        let s = SmartBookmarkSpec::default_profile();
        assert!(s.captures(EnrichmentField::Title));
        assert!(s.captures(EnrichmentField::Tags));
        assert!(s.captures(EnrichmentField::Summary));
        assert!(s.captures(EnrichmentField::ContentId));
        assert!(!s.captures(EnrichmentField::Thumbnail));
    }

    #[test]
    fn clamped_threshold_bounds() {
        let over = SmartBookmarkSpec {
            related_threshold: 2.5,
            ..SmartBookmarkSpec::default_profile()
        };
        assert!((over.clamped_threshold() - 1.0).abs() < 1e-5);
        let neg = SmartBookmarkSpec {
            related_threshold: -0.3,
            ..SmartBookmarkSpec::default_profile()
        };
        assert!(neg.clamped_threshold().abs() < 1e-5);
    }

    #[test]
    fn shape_tags_caps_at_max() {
        let s = SmartBookmarkSpec {
            max_tags: 3,
            dedupe_tags: false,
            ..SmartBookmarkSpec::default_profile()
        };
        let raw = (0..10).map(|i| format!("t{i}")).collect();
        let shaped = s.shape_tags(raw);
        assert_eq!(shaped.len(), 3);
        assert_eq!(shaped, vec!["t0", "t1", "t2"]);
    }

    #[test]
    fn shape_tags_unlimited_when_max_zero() {
        let s = SmartBookmarkSpec {
            max_tags: 0,
            dedupe_tags: false,
            ..SmartBookmarkSpec::default_profile()
        };
        let raw = (0..200).map(|i| format!("t{i}")).collect();
        assert_eq!(s.shape_tags(raw).len(), 200);
    }

    #[test]
    fn shape_tags_dedupes_case_insensitive() {
        let s = SmartBookmarkSpec {
            max_tags: 10,
            dedupe_tags: true,
            ..SmartBookmarkSpec::default_profile()
        };
        let raw = vec![
            "Rust".into(),
            "rust".into(),
            "Lisp".into(),
            "RUST".into(),
            "lisp".into(),
        ];
        let shaped = s.shape_tags(raw);
        assert_eq!(shaped, vec!["Rust", "Lisp"]);
    }

    #[test]
    fn shape_tags_trims_whitespace_and_drops_empty() {
        let s = SmartBookmarkSpec {
            dedupe_tags: false,
            ..SmartBookmarkSpec::default_profile()
        };
        let raw = vec!["  rust  ".into(), "".into(), "   ".into(), "lisp".into()];
        let shaped = s.shape_tags(raw);
        assert_eq!(shaped, vec!["rust", "lisp"]);
    }

    #[test]
    fn render_summary_substitutes_title_tldr_tags_url() {
        let s = SmartBookmarkSpec {
            summary_template: Some("{title} — {tl-dr} ({tags}) <{url}>".into()),
            ..SmartBookmarkSpec::default_profile()
        };
        let out = s
            .render_summary(
                "Attention Is All You Need",
                "Transformers",
                &[],
                &["ml".into(), "nlp".into()],
                "https://arxiv.org/abs/1706.03762",
            )
            .unwrap();
        assert_eq!(
            out,
            "Attention Is All You Need — Transformers (ml, nlp) <https://arxiv.org/abs/1706.03762>"
        );
    }

    #[test]
    fn render_summary_topics_n_token_picks_top_n() {
        let s = SmartBookmarkSpec {
            summary_template: Some("topics: {topics-3}".into()),
            ..SmartBookmarkSpec::default_profile()
        };
        let topics = vec![
            "attention".into(),
            "transformers".into(),
            "nlp".into(),
            "training".into(),
            "rnn".into(),
        ];
        let out = s.render_summary("t", "d", &topics, &[], "u").unwrap();
        assert_eq!(out, "topics: attention, transformers, nlp");
    }

    #[test]
    fn render_summary_topics_n_cap_exceeds_input() {
        let s = SmartBookmarkSpec {
            summary_template: Some("{topics-10}".into()),
            ..SmartBookmarkSpec::default_profile()
        };
        let topics = vec!["a".into(), "b".into()];
        assert_eq!(
            s.render_summary("t", "d", &topics, &[], "u").unwrap(),
            "a, b"
        );
    }

    #[test]
    fn render_summary_none_when_no_template() {
        let s = SmartBookmarkSpec::default_profile();
        assert!(s.render_summary("a", "b", &[], &[], "u").is_none());
    }

    #[test]
    fn index_trigger_roundtrips_through_serde() {
        for t in [
            IndexTrigger::OnSave,
            IndexTrigger::OnSaveAndPeriodic,
            IndexTrigger::Manual,
            IndexTrigger::OnSyncUpdate,
        ] {
            let s = SmartBookmarkSpec {
                index_trigger: t,
                ..SmartBookmarkSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: SmartBookmarkSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.index_trigger, t);
        }
    }

    #[test]
    fn related_metric_roundtrips_through_serde() {
        for m in [
            RelatedMetric::EmbeddingCosine,
            RelatedMetric::Bm25TitleTags,
            RelatedMetric::JaccardTags,
            RelatedMetric::Random,
        ] {
            let s = SmartBookmarkSpec {
                related_metric: m,
                ..SmartBookmarkSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: SmartBookmarkSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.related_metric, m);
        }
    }

    #[test]
    fn enrichment_field_roundtrips_through_serde() {
        let all = vec![
            EnrichmentField::Title,
            EnrichmentField::Tags,
            EnrichmentField::Summary,
            EnrichmentField::Topics,
            EnrichmentField::KeyQuotes,
            EnrichmentField::NamedEntities,
            EnrichmentField::ReadingTime,
            EnrichmentField::Sentiment,
            EnrichmentField::ContentId,
            EnrichmentField::Thumbnail,
        ];
        let s = SmartBookmarkSpec {
            enrich: all.clone(),
            ..SmartBookmarkSpec::default_profile()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: SmartBookmarkSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.enrich, all);
    }

    #[test]
    fn registry_prefers_specific_host_and_skips_exempt() {
        let mut reg = SmartBookmarkRegistry::new();
        reg.insert(SmartBookmarkSpec::default_profile());
        reg.insert(SmartBookmarkSpec {
            name: "arxiv".into(),
            host: "*://*.arxiv.org/*".into(),
            llm: "opus".into(),
            ..SmartBookmarkSpec::default_profile()
        });
        reg.insert(SmartBookmarkSpec {
            name: "no-bank".into(),
            exempt_hosts: vec!["*://*.bank.com/*".into()],
            ..SmartBookmarkSpec::default_profile()
        });
        let arxiv = reg.resolve("www.arxiv.org").unwrap();
        assert_eq!(arxiv.name, "arxiv");
        assert_eq!(arxiv.llm, "opus");
        // Bank falls to 'default' (not 'no-bank' — that one exempts)
        let bank = reg.resolve("my.bank.com").unwrap();
        assert_eq!(bank.name, "default");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_smart_bookmark_form() {
        let src = r#"
            (defsmart-bookmark :name "research"
                               :host "*://*.arxiv.org/*"
                               :llm "local-opus"
                               :auto-tag #t
                               :max-tags 6
                               :auto-summary #t
                               :auto-relate #t
                               :related-limit 5
                               :index-trigger "on-save"
                               :related-metric "embedding-cosine"
                               :related-threshold 0.7
                               :sync-enrichment #t
                               :audit #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.llm, "local-opus");
        assert_eq!(s.max_tags, 6);
        assert!((s.related_threshold - 0.7).abs() < 1e-5);
        assert!(s.audit);
    }
}
