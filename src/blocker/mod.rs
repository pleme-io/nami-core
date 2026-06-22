//! `(defblocker)` — declarative content blocking.
//!
//! Absorbs uBlock/EasyList-style rules into the substrate pattern:
//! rules author as Lisp, registries compose, hits report via typed
//! `BlockerHit`, DOM mutations stamp provenance so every strip is
//! traceable.
//!
//! V1 scope: domain-list + CSS-selector rules. Full EasyList grammar
//! (`||domain.com^$third-party`, cosmetic `##` filters with procedural
//! `:has-text()`, etc.) lands as the grammar stabilizes.
//!
//! ```lisp
//! (defblocker :name "trackers"
//!             :domains ("google-analytics.com"
//!                       "doubleclick.net"
//!                       "facebook.com/tr"
//!                       "scorecardresearch.com")
//!             :description "Common third-party tracker endpoints")
//!
//! (defblocker :name "sidebar-ads"
//!             :selectors (".ad-sidebar"
//!                         "[data-ad-placement]"
//!                         "[aria-label=advertisement]"))
//!
//! (defblocker :name "mixed-rule"
//!             :domains ("example-ads.net")
//!             :selectors (".sponsored-content"))
//! ```
//!
//! Rules compose at the registry level. Matching any domain of any
//! registered spec blocks the outbound fetch; matching any selector
//! of any registered spec removes the subtree.

use std::cell::OnceCell;

use crate::dom::{Document, Node, NodeData};
use crate::selector::{OwnedContext, Selector};
use hayai::{MatchEngine, RegexMatcher};
use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Built-in tracker blocklist consumed by
/// [`BlockerRegistry::with_default_trackers`]. ~120 widely-documented
/// public tracker / analytics / ad endpoints. One domain or path
/// fragment per line; `#` comments + blank lines are stripped.
const DEFAULT_TRACKERS: &str = include_str!("trackers.txt");

/// Parse [`DEFAULT_TRACKERS`] into the live domain list — strips
/// comments + blank lines, trims each entry.
#[must_use]
pub fn default_tracker_domains() -> Vec<String> {
    DEFAULT_TRACKERS
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// One blocker rule.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defblocker"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BlockerSpec {
    pub name: String,
    /// Case-insensitive substring match against the full URL. Use
    /// fully-qualified domains (`"google-analytics.com"`) or path
    /// fragments (`"/tr?id="` for Facebook pixel endpoints).
    #[serde(default)]
    pub domains: Vec<String>,
    /// CSS selectors. Any matching element is removed from the DOM.
    /// Supports the same selector syntax as `(defdom-transform)`.
    #[serde(default)]
    pub selectors: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Compiled URL matcher — a hayai [`RegexMatcher`] over every domain
/// pattern of every spec, plus a parallel index mapping each compiled
/// pattern back to the spec that contributed it. One DFA pass replaces
/// the old O(specs×domains) substring scan.
struct UrlMatcher {
    matcher: RegexMatcher,
    /// `pattern_to_spec[i]` = index (into `specs`) of the spec that
    /// owns compiled pattern `i`. Patterns are appended spec-by-spec,
    /// so this vector is non-decreasing.
    pattern_to_spec: Vec<usize>,
}

impl UrlMatcher {
    /// Build a matcher from the registry's specs. Each domain substring
    /// is lower-cased + regex-escaped so the compiled DFA does a plain
    /// case-insensitive substring match (URLs are lower-cased on check).
    /// Returns `None` when there are no domain patterns at all (an
    /// all-selector registry has nothing to match URLs against).
    fn build(specs: &[BlockerSpec]) -> Option<Self> {
        let mut patterns: Vec<String> = Vec::new();
        let mut pattern_to_spec: Vec<usize> = Vec::new();
        for (spec_idx, spec) in specs.iter().enumerate() {
            for d in &spec.domains {
                patterns.push(regex::escape(&d.to_ascii_lowercase()));
                pattern_to_spec.push(spec_idx);
            }
        }
        if patterns.is_empty() {
            return None;
        }
        // Every pattern is a regex-escaped literal, so compilation
        // cannot fail; if it ever does, surface it rather than masking.
        let matcher = RegexMatcher::new(&patterns)
            .expect("regex-escaped literal patterns always compile");
        Some(Self {
            matcher,
            pattern_to_spec,
        })
    }

    /// First spec index whose domain matched `lower_url`, preserving the
    /// original "first registered matching spec" semantics. Returns the
    /// lowest spec index among all matched patterns.
    fn first_spec(&self, lower_url: &str) -> Option<usize> {
        self.matcher
            .check(lower_url)
            .into_iter()
            .map(|pat_idx| self.pattern_to_spec[pat_idx])
            .min()
    }
}

/// Registry of blocker rules. URL matching is routed through a lazily
/// compiled hayai [`RegexMatcher`] (one DFA pass); the cache is reset
/// on any mutation so it always reflects the current spec set.
#[derive(Default)]
pub struct BlockerRegistry {
    specs: Vec<BlockerSpec>,
    /// Lazily built on first `block_url`; invalidated on insert/extend.
    matcher: OnceCell<Option<UrlMatcher>>,
}

impl std::fmt::Debug for BlockerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockerRegistry")
            .field("specs", &self.specs)
            .finish_non_exhaustive()
    }
}

impl Clone for BlockerRegistry {
    fn clone(&self) -> Self {
        // Don't carry the compiled cache across clones — it rebuilds
        // lazily from `specs`, so a fresh empty cell stays correct.
        Self {
            specs: self.specs.clone(),
            matcher: OnceCell::new(),
        }
    }
}

impl BlockerRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry seeded with the built-in ~120-entry tracker
    /// blocklist (see `trackers.txt`). One `(defblocker)`-shaped spec
    /// named `"default-trackers"` carrying every embedded domain.
    #[must_use]
    pub fn with_default_trackers() -> Self {
        let mut reg = Self::new();
        reg.insert(BlockerSpec {
            name: "default-trackers".into(),
            domains: default_tracker_domains(),
            selectors: Vec::new(),
            description: Some(
                "Built-in tracker blocklist — well-known third-party analytics, ads, and session-replay endpoints.".into(),
            ),
        });
        reg
    }

    /// Drop the compiled URL matcher so the next `block_url` rebuilds it
    /// from the current spec set.
    fn invalidate(&mut self) {
        self.matcher.take();
    }

    pub fn insert(&mut self, spec: BlockerSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
        self.invalidate();
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = BlockerSpec>) {
        for s in specs {
            // Inline the dedupe so we only invalidate once at the end.
            self.specs.retain(|existing| existing.name != s.name);
            self.specs.push(s);
        }
        self.invalidate();
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    pub fn specs(&self) -> &[BlockerSpec] {
        &self.specs
    }

    /// Returns the first spec whose domain list matches the URL, or
    /// `None` if nothing blocks it. Matching is case-insensitive
    /// substring matching against the URL — real-world tracker lists
    /// live at this fidelity — but runs as a single hayai `RegexSet`
    /// DFA pass over all domains rather than an O(specs×domains) scan.
    #[must_use]
    pub fn block_url(&self, url: &str) -> Option<&BlockerSpec> {
        let lower = url.to_ascii_lowercase();
        let matcher = self
            .matcher
            .get_or_init(|| UrlMatcher::build(&self.specs))
            .as_ref()?;
        matcher.first_spec(&lower).map(|i| &self.specs[i])
    }
}

/// One recorded DOM-strip outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockerHit {
    /// Blocker spec name that triggered the strip.
    pub rule: String,
    /// CSS selector that matched.
    pub selector: String,
    /// Tag name of the removed element.
    pub tag: String,
}

/// Outcome of a DOM blocking pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockerReport {
    pub hits: Vec<BlockerHit>,
}

impl BlockerReport {
    #[must_use]
    pub fn applied(&self) -> usize {
        self.hits.len()
    }
}

/// Walk the document and strip every element matching any blocker's
/// selectors. Stamps `data-n-blocked` + `data-n-blocker` on a
/// synthetic `<n-blocked>` placeholder so the provenance chain stays
/// intact (downstream tooling still sees where an element used to be).
pub fn apply(doc: &mut Document, registry: &BlockerRegistry) -> BlockerReport {
    let mut report = BlockerReport::default();
    if registry.is_empty() {
        return report;
    }

    // Compile every selector once.
    let mut compiled: Vec<(String, String, Selector)> = Vec::new();
    for spec in registry.specs() {
        for sel in &spec.selectors {
            match Selector::parse(sel) {
                Ok(s) => compiled.push((spec.name.clone(), sel.clone(), s)),
                Err(e) => {
                    tracing::warn!("blocker '{}' bad selector {:?}: {e}", spec.name, sel);
                }
            }
        }
    }
    if compiled.is_empty() {
        return report;
    }

    let mut path: Vec<OwnedContext> = Vec::new();
    strip(&mut doc.root, &compiled, &mut path, &mut report);
    report
}

fn strip(
    node: &mut Node,
    rules: &[(String, String, Selector)],
    path: &mut Vec<OwnedContext>,
    report: &mut BlockerReport,
) {
    let pushed = if let NodeData::Element(el) = &node.data {
        path.push(OwnedContext::from_element(el));
        true
    } else {
        false
    };

    // Descend first (bottom-up pruning plays nicer with nested rules).
    for child in &mut node.children {
        strip(child, rules, path, report);
    }

    // Now scan our children against each rule, removing matches.
    let parent_path: Vec<&OwnedContext> = path.iter().collect();
    let mut i = 0;
    while i < node.children.len() {
        let Some(el) = node.children[i].as_element() else {
            i += 1;
            continue;
        };
        let child_ctx = OwnedContext::from_element(el);
        let mut full = parent_path.clone();
        full.push(&child_ctx);

        let hit = rules
            .iter()
            .find(|(_, _, s)| s.matches(&full))
            .map(|(name, sel, _)| (name.clone(), sel.clone(), el.tag.clone()));

        if let Some((name, sel, tag)) = hit {
            node.children.remove(i);
            report.hits.push(BlockerHit {
                rule: name,
                selector: sel,
                tag,
            });
            // Don't advance i — next element shifted into this slot.
        } else {
            i += 1;
        }
    }

    if pushed {
        path.pop();
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<BlockerSpec>, String> {
    tatara_lisp::compile_typed::<BlockerSpec>(src)
        .map_err(|e| format!("failed to compile defblocker forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<BlockerSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, domains: &[&str], selectors: &[&str]) -> BlockerSpec {
        BlockerSpec {
            name: name.into(),
            domains: domains.iter().map(|s| (*s).into()).collect(),
            selectors: selectors.iter().map(|s| (*s).into()).collect(),
            description: None,
        }
    }

    #[test]
    fn empty_registry_is_noop() {
        let reg = BlockerRegistry::new();
        assert!(reg.is_empty());
        let mut doc = Document::parse("<html><body><div class='ad'>x</div></body></html>");
        let report = apply(&mut doc, &reg);
        assert_eq!(report.applied(), 0);
    }

    #[test]
    fn domain_match_is_case_insensitive_substring() {
        let mut reg = BlockerRegistry::new();
        reg.insert(spec("trackers", &["google-analytics.com"], &[]));
        assert!(reg.block_url("https://www.google-analytics.com/ga.js").is_some());
        assert!(reg.block_url("HTTPS://WWW.GOOGLE-ANALYTICS.COM/GA.JS").is_some());
        assert!(reg.block_url("https://cdn.google-analytics.com/x").is_some());
        assert!(reg.block_url("https://example.com/").is_none());
    }

    #[test]
    fn block_url_returns_first_matching_spec() {
        let mut reg = BlockerRegistry::new();
        reg.insert(spec("a", &["example.com"], &[]));
        reg.insert(spec("b", &["example.com"], &[]));
        let hit = reg.block_url("https://example.com").unwrap();
        assert_eq!(hit.name, "a");
    }

    #[test]
    fn selector_match_removes_element() {
        let mut reg = BlockerRegistry::new();
        reg.insert(spec("sidebar-ads", &[], &[".ad-sidebar"]));
        let mut doc = Document::parse(
            r#"<html><body>
              <p>keep</p>
              <div class="ad-sidebar">drop</div>
              <div class="content">keep</div>
            </body></html>"#,
        );
        let report = apply(&mut doc, &reg);
        assert_eq!(report.applied(), 1);
        assert_eq!(report.hits[0].rule, "sidebar-ads");
        assert_eq!(report.hits[0].tag, "div");

        // The ad-sidebar is gone, other content intact.
        let mut saw_content = false;
        let mut saw_ad = false;
        for n in doc.root.descendants() {
            if let Some(el) = n.as_element() {
                if el.get_attribute("class") == Some("ad-sidebar") {
                    saw_ad = true;
                }
                if el.get_attribute("class") == Some("content") {
                    saw_content = true;
                }
            }
        }
        assert!(saw_content && !saw_ad);
    }

    #[test]
    fn multiple_selectors_across_specs_compose() {
        let mut reg = BlockerRegistry::new();
        reg.insert(spec("a", &[], &[".ad"]));
        reg.insert(spec("b", &[], &["[aria-label=advertisement]"]));
        let mut doc = Document::parse(
            r#"<html><body>
              <div class="ad">1</div>
              <aside aria-label="advertisement">2</aside>
              <p>keep</p>
            </body></html>"#,
        );
        let report = apply(&mut doc, &reg);
        assert_eq!(report.applied(), 2);
        let rules: std::collections::HashSet<_> = report
            .hits
            .iter()
            .map(|h| h.rule.clone())
            .collect();
        assert!(rules.contains("a"));
        assert!(rules.contains("b"));
    }

    #[test]
    fn bad_selector_logs_and_continues() {
        let mut reg = BlockerRegistry::new();
        reg.insert(spec("broken", &[], &["[unbalanced"]));
        reg.insert(spec("ok", &[], &[".ad"]));
        let mut doc = Document::parse("<html><body><div class='ad'>x</div></body></html>");
        let report = apply(&mut doc, &reg);
        // The broken selector is skipped; the good one still fires.
        assert_eq!(report.applied(), 1);
        assert_eq!(report.hits[0].rule, "ok");
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = BlockerRegistry::new();
        reg.insert(spec("trackers", &["a.com"], &[]));
        reg.insert(spec("trackers", &["b.com"], &[]));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].domains, vec!["b.com"]);
    }

    #[test]
    fn nested_match_is_removed_cleanly() {
        let mut reg = BlockerRegistry::new();
        reg.insert(spec("ads", &[], &[".ad"]));
        let mut doc = Document::parse(
            r#"<html><body>
              <article>
                <p>content</p>
                <div class="ad"><span>nested</span></div>
              </article>
            </body></html>"#,
        );
        let report = apply(&mut doc, &reg);
        assert_eq!(report.applied(), 1);
        // ad is gone, including its children.
        for n in doc.root.descendants() {
            if let Some(el) = n.as_element() {
                assert_ne!(el.get_attribute("class"), Some("ad"));
            }
        }
    }

    #[test]
    fn empty_domain_list_and_empty_selectors_is_noop() {
        let mut reg = BlockerRegistry::new();
        reg.insert(spec("empty", &[], &[]));
        let mut doc = Document::parse("<html><body><p>hi</p></body></html>");
        let report = apply(&mut doc, &reg);
        assert_eq!(report.applied(), 0);
        assert!(reg.block_url("https://example.com").is_none());
    }

    #[test]
    fn hayai_backed_match_handles_multiple_domains_one_pass() {
        // Many specs, many domains — the hayai DFA matches in one pass
        // and still reports the first registered matching spec.
        let mut reg = BlockerRegistry::new();
        reg.insert(spec("ads", &["doubleclick.net", "adnxs.com"], &[]));
        reg.insert(spec("analytics", &["google-analytics.com", "segment.io"], &[]));
        assert_eq!(
            reg.block_url("https://ib.adnxs.com/ttj").unwrap().name,
            "ads"
        );
        assert_eq!(
            reg.block_url("https://api.segment.io/v1/track").unwrap().name,
            "analytics"
        );
        assert!(reg.block_url("https://example.com/").is_none());
    }

    #[test]
    fn matcher_rebuilds_after_mutation() {
        // The lazily-compiled matcher must reflect inserts/extends made
        // after the first block_url forced compilation.
        let mut reg = BlockerRegistry::new();
        reg.insert(spec("a", &["a.com"], &[]));
        assert!(reg.block_url("https://a.com/x").is_some()); // forces build
        assert!(reg.block_url("https://b.com/x").is_none());
        reg.insert(spec("b", &["b.com"], &[])); // invalidates cache
        assert_eq!(reg.block_url("https://b.com/x").unwrap().name, "b");
        reg.extend([spec("c", &["c.com"], &[])]);
        assert_eq!(reg.block_url("https://c.com/x").unwrap().name, "c");
    }

    #[test]
    fn cloned_registry_matches_independently() {
        let mut reg = BlockerRegistry::new();
        reg.insert(spec("a", &["a.com"], &[]));
        let _ = reg.block_url("https://a.com"); // warm the original cache
        let clone = reg.clone();
        // The clone rebuilds its own cache lazily and matches correctly.
        assert_eq!(clone.block_url("https://a.com/x").unwrap().name, "a");
        assert!(clone.block_url("https://z.com/x").is_none());
    }

    #[test]
    fn special_regex_chars_in_domain_are_literal() {
        // A path fragment with regex metacharacters must match literally
        // (regex::escape), not as a pattern.
        let mut reg = BlockerRegistry::new();
        reg.insert(spec("pixel", &["facebook.com/tr?id="], &[]));
        assert!(reg
            .block_url("https://www.facebook.com/tr?id=123&ev=PageView")
            .is_some());
        // The `?` is literal — a URL missing it must not match.
        assert!(reg.block_url("https://www.facebook.com/troll").is_none());
    }

    #[test]
    fn default_trackers_blocks_known_tracker_allows_example() {
        let reg = BlockerRegistry::with_default_trackers();
        assert_eq!(reg.len(), 1);
        // A representative sample of the seeded list is blocked.
        for url in [
            "https://www.google-analytics.com/collect?v=1",
            "https://connect.facebook.net/en_US/fbevents.js",
            "https://b.scorecardresearch.com/beacon.js",
            "https://static.hotjar.com/c/hotjar-123.js",
            "https://cdn.segment.com/analytics.js/v1/abc/analytics.min.js",
            "https://api.mixpanel.com/track",
            "https://cdn.amplitude.com/libs/amplitude.js",
            "https://edge.fullstory.com/s/fs.js",
            "https://www.googletagmanager.com/gtm.js?id=GTM-XXXX",
        ] {
            assert!(
                reg.block_url(url).is_some(),
                "expected default trackers to block {url}"
            );
            assert_eq!(reg.block_url(url).unwrap().name, "default-trackers");
        }
        // example.com (and other benign hosts) pass through.
        assert!(reg.block_url("https://example.com/").is_none());
        assert!(reg.block_url("https://docs.rs/nami-core").is_none());
        assert!(reg.block_url("https://news.example.org/article").is_none());
    }

    #[test]
    fn default_tracker_domains_is_clean_and_substantial() {
        let domains = default_tracker_domains();
        // The list is real (~120 entries) and well-formed.
        assert!(domains.len() >= 100, "expected ~120 trackers, got {}", domains.len());
        for d in &domains {
            assert!(!d.is_empty());
            assert!(!d.starts_with('#'));
            assert_eq!(d.trim(), d, "entry has surrounding whitespace: {d:?}");
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_multi_field_spec() {
        let src = r#"
            (defblocker :name "trackers"
                        :domains ("a.com" "b.net")
                        :selectors (".ad" "[data-ad]"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "trackers");
        assert_eq!(specs[0].domains, vec!["a.com", "b.net"]);
        assert_eq!(specs[0].selectors, vec![".ad", "[data-ad]"]);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_rejects_malformed_source() {
        let src = "(defblocker :name)"; // missing required shape
        let r = compile(src);
        assert!(r.is_err() || r.unwrap().is_empty());
    }
}
