//! `(deffeed)` — declarative RSS/Atom subscriptions.
//!
//! Absorbs Opera Reader, NetNewsWire, Feedly's OPML subscribe model,
//! and Firefox Live Bookmarks. A feed is a named URL + cadence +
//! category + storage cache; the fetcher trait is pluggable so the
//! substrate spec can drive any HTTP pipeline (namimado's reqwest,
//! todoku, or a test stub).
//!
//! ```lisp
//! (deffeed :name          "hn"
//!          :url           "https://hnrss.org/frontpage"
//!          :cadence-seconds 900
//!          :category      "news"
//!          :max-items     50
//!          :storage       "feeds")
//!
//! (deffeed :name          "arxiv-cs-pl"
//!          :url           "https://export.arxiv.org/rss/cs.PL"
//!          :cadence-seconds 3600
//!          :category      "research"
//!          :enabled       #t)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Feed subscription.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "deffeed"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FeedSpec {
    pub name: String,
    pub url: String,
    /// Poll cadence in seconds. Clamped to `[60, 86400]` at apply.
    /// Zero = use default (900s = 15min).
    #[serde(default = "default_cadence")]
    pub cadence_seconds: u64,
    /// Freeform grouping — "news", "research", "friends", etc.
    #[serde(default)]
    pub category: Option<String>,
    /// Cap on items retained per feed. `0` = unlimited.
    #[serde(default = "default_max_items")]
    pub max_items: usize,
    /// Storage name for cache (should match a (defstorage) decl).
    #[serde(default = "default_storage")]
    pub storage: String,
    /// Inactive feeds skip fetch but stay in the registry — supports
    /// the "pause subscription" flow without uninstall.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_cadence() -> u64 {
    900
}
fn default_max_items() -> usize {
    100
}
fn default_storage() -> String {
    "feeds".into()
}
fn default_enabled() -> bool {
    true
}

const MIN_CADENCE: u64 = 60;
const MAX_CADENCE: u64 = 86_400;

impl FeedSpec {
    #[must_use]
    pub fn clamped_cadence(&self) -> u64 {
        let c = if self.cadence_seconds == 0 {
            default_cadence()
        } else {
            self.cadence_seconds
        };
        c.clamp(MIN_CADENCE, MAX_CADENCE)
    }

    /// Validate structural requirements — non-empty name + url.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("feed name is empty".into());
        }
        if self.url.trim().is_empty() {
            return Err(format!("feed '{}' has empty url", self.name));
        }
        Ok(())
    }
}

/// One item parsed from a feed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeedItem {
    /// Item URL (link target).
    pub url: String,
    pub title: String,
    /// Item `guid`/`id` — stable across updates. When absent upstream,
    /// fall back to the URL.
    pub guid: String,
    /// Published-at unix seconds. `0` = unknown.
    #[serde(default)]
    pub published_at: i64,
    /// Author name if present.
    #[serde(default)]
    pub author: Option<String>,
    /// Summary HTML / plaintext.
    #[serde(default)]
    pub summary: Option<String>,
    /// Category tags from the item.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Registry of feeds.
#[derive(Debug, Clone, Default)]
pub struct FeedRegistry {
    specs: Vec<FeedSpec>,
}

impl FeedRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: FeedSpec) -> Result<(), String> {
        spec.validate()?;
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
        Ok(())
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = FeedSpec>) {
        for s in specs {
            if let Err(e) = self.insert(s.clone()) {
                tracing::warn!("deffeed '{}' rejected: {}", s.name, e);
            }
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

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.specs.len();
        self.specs.retain(|s| s.name != name);
        self.specs.len() < before
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
    pub fn specs(&self) -> &[FeedSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&FeedSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    #[must_use]
    pub fn enabled_specs(&self) -> Vec<&FeedSpec> {
        self.specs.iter().filter(|s| s.enabled).collect()
    }

    /// All distinct categories present, sorted.
    #[must_use]
    pub fn categories(&self) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for s in &self.specs {
            if let Some(cat) = &s.category {
                if !cat.is_empty() {
                    set.insert(cat.clone());
                }
            }
        }
        set.into_iter().collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<FeedSpec>, String> {
    tatara_lisp::compile_typed::<FeedSpec>(src)
        .map_err(|e| format!("failed to compile deffeed forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<FeedSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> FeedSpec {
        FeedSpec {
            name: name.into(),
            url: format!("https://example.com/{name}.rss"),
            cadence_seconds: 900,
            category: Some("news".into()),
            max_items: 50,
            storage: "feeds".into(),
            enabled: true,
            description: None,
        }
    }

    #[test]
    fn clamped_cadence_respects_bounds() {
        let too_low = FeedSpec {
            cadence_seconds: 10,
            ..sample("x")
        };
        assert_eq!(too_low.clamped_cadence(), MIN_CADENCE);
        let too_high = FeedSpec {
            cadence_seconds: 999_999,
            ..sample("x")
        };
        assert_eq!(too_high.clamped_cadence(), MAX_CADENCE);
        let ok = FeedSpec {
            cadence_seconds: 3600,
            ..sample("x")
        };
        assert_eq!(ok.clamped_cadence(), 3600);
    }

    #[test]
    fn clamped_cadence_zero_uses_default() {
        let zero = FeedSpec {
            cadence_seconds: 0,
            ..sample("x")
        };
        assert_eq!(zero.clamped_cadence(), 900);
    }

    #[test]
    fn validate_rejects_empty_fields() {
        let empty_name = FeedSpec {
            name: String::new(),
            ..sample("x")
        };
        assert!(empty_name.validate().is_err());
        let empty_url = FeedSpec {
            url: String::new(),
            ..sample("x")
        };
        assert!(empty_url.validate().is_err());
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = FeedRegistry::new();
        reg.insert(sample("hn")).unwrap();
        reg.insert(FeedSpec {
            url: "https://new-url/".into(),
            ..sample("hn")
        })
        .unwrap();
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].url, "https://new-url/");
    }

    #[test]
    fn registry_extend_drops_invalid_silently() {
        let mut reg = FeedRegistry::new();
        reg.extend(vec![
            sample("good"),
            FeedSpec {
                url: String::new(),
                ..sample("bad")
            },
        ]);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].name, "good");
    }

    #[test]
    fn set_enabled_and_remove() {
        let mut reg = FeedRegistry::new();
        reg.insert(sample("a")).unwrap();
        reg.insert(sample("b")).unwrap();
        assert!(reg.set_enabled("a", false));
        assert!(!reg.specs()[0].enabled);
        assert!(reg.remove("b"));
        assert_eq!(reg.len(), 1);
        assert!(!reg.remove("nonexistent"));
    }

    #[test]
    fn enabled_specs_filters_paused_feeds() {
        let mut reg = FeedRegistry::new();
        reg.insert(sample("on")).unwrap();
        reg.insert(FeedSpec {
            enabled: false,
            ..sample("off")
        })
        .unwrap();
        let enabled = reg.enabled_specs();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name, "on");
    }

    #[test]
    fn categories_returns_sorted_distinct() {
        let mut reg = FeedRegistry::new();
        reg.insert(FeedSpec {
            category: Some("research".into()),
            ..sample("a")
        })
        .unwrap();
        reg.insert(FeedSpec {
            category: Some("news".into()),
            ..sample("b")
        })
        .unwrap();
        reg.insert(FeedSpec {
            category: Some("news".into()),
            ..sample("c")
        })
        .unwrap();
        reg.insert(FeedSpec {
            category: None,
            ..sample("d")
        })
        .unwrap();
        assert_eq!(
            reg.categories(),
            vec!["news".to_owned(), "research".to_owned()]
        );
    }

    #[test]
    fn feed_item_roundtrips_through_json() {
        let item = FeedItem {
            url: "https://example.com/post".into(),
            title: "Hello".into(),
            guid: "post-1".into(),
            published_at: 1_700_000_000,
            author: Some("Jane".into()),
            summary: Some("<p>Body</p>".into()),
            tags: vec!["rust".into(), "browser".into()],
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: FeedItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back, item);
    }

    #[test]
    fn default_cadence_is_15_minutes() {
        assert_eq!(default_cadence(), 900);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_feed_form() {
        let src = r#"
            (deffeed :name            "hn"
                     :url             "https://hnrss.org/frontpage"
                     :cadence-seconds 1800
                     :category        "news"
                     :max-items       50
                     :storage         "feeds")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "hn");
        assert_eq!(s.url, "https://hnrss.org/frontpage");
        assert_eq!(s.cadence_seconds, 1800);
        assert_eq!(s.category.as_deref(), Some("news"));
        assert_eq!(s.max_items, 50);
    }
}
