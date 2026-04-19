//! `(defomnibox)` — declarative URL-bar autocomplete.
//!
//! Absorbs Chrome's omnibox, Firefox's awesomebar, Safari's smart
//! search, and Vivaldi's F2 command palette into one substrate DSL:
//! the profile declares *what sources to draw from*, *which search
//! providers to expose*, and *how aggressively to surface each*.
//! Scoring + ranking is deterministic and lives here; the caller
//! supplies the raw source data (history, bookmarks, command names).
//!
//! ```lisp
//! (defomnibox :name "default"
//!             :sources (history bookmarks commands)
//!             :search-providers
//!             (
//!               (provider :name "ddg"
//!                         :shortcut "ddg"
//!                         :url "https://duckduckgo.com/?q={query}"
//!                         :label "DuckDuckGo")
//!               (provider :name "google"
//!                         :shortcut "g"
//!                         :url "https://www.google.com/search?q={query}")
//!             )
//!             :max-results 10
//!             :min-chars 1)
//! ```
//!
//! Dispatch flow (caller provides `OmniboxInput`):
//!
//! 1. If query is empty and min_chars > 0 → return an empty ranking.
//! 2. If query starts with a declared `shortcut<space>` → route the
//!    remainder to that search provider only (e.g., "ddg foo").
//! 3. Otherwise: score every enabled source, merge, sort desc, cap.
//!
//! Scoring is conservative — exact prefix > substring > case-insensitive
//! substring. No fuzzy / edit-distance yet; the V2 upgrade plugs
//! hayai in.

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Sources the omnibox can draw from. Authored as plain identifiers in
/// Lisp (`:sources (history bookmarks commands)`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum OmniboxSource {
    /// Browsing history entries.
    History,
    /// User-saved bookmarks.
    Bookmarks,
    /// `(defcommand)` names — command palette.
    Commands,
    /// Active tabs (caller provides).
    Tabs,
    /// Currently open extensions (by name + description).
    Extensions,
}

/// One search provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchProvider {
    pub name: String,
    /// URL template — the literal `{query}` substring is replaced with
    /// the URL-encoded user query.
    pub url: String,
    /// Prefix that routes the remainder to this provider only,
    /// e.g. "ddg" → user types "ddg dark mode" → only DDG fires.
    #[serde(default)]
    pub shortcut: Option<String>,
    /// Display label for the suggestion row. Defaults to `name`.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

/// Declarative omnibox profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defomnibox"))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OmniboxSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Enabled sources. Order matters only for tie-breaking (higher
    /// source priority wins on equal score).
    #[serde(default)]
    pub sources: Vec<OmniboxSource>,
    /// Search providers, in order. The first is the "default" — used
    /// when the user presses Enter without a shortcut match.
    #[serde(default)]
    pub search_providers: Vec<SearchProvider>,
    /// Maximum suggestions returned. `0` → unlimited.
    #[serde(default = "default_max_results")]
    pub max_results: usize,
    /// Minimum chars before history/bookmarks fire. Prevents wasting
    /// ranking time on single-letter noise.
    #[serde(default)]
    pub min_chars: usize,
}

fn default_max_results() -> usize {
    10
}

impl OmniboxSpec {
    /// Built-in "everything on, DDG + Google" profile. Works without
    /// any authoring.
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            description: Some("Default URL-bar autocomplete.".into()),
            sources: vec![
                OmniboxSource::History,
                OmniboxSource::Bookmarks,
                OmniboxSource::Commands,
            ],
            search_providers: vec![
                SearchProvider {
                    name: "ddg".into(),
                    url: "https://duckduckgo.com/?q={query}".into(),
                    shortcut: Some("ddg".into()),
                    label: Some("DuckDuckGo".into()),
                    icon: None,
                },
                SearchProvider {
                    name: "google".into(),
                    url: "https://www.google.com/search?q={query}".into(),
                    shortcut: Some("g".into()),
                    label: Some("Google".into()),
                    icon: None,
                },
                SearchProvider {
                    name: "github".into(),
                    url: "https://github.com/search?q={query}".into(),
                    shortcut: Some("gh".into()),
                    label: Some("GitHub".into()),
                    icon: None,
                },
            ],
            max_results: 10,
            min_chars: 1,
        }
    }
}

/// Registry of omnibox profiles.
#[derive(Debug, Clone, Default)]
pub struct OmniboxRegistry {
    specs: Vec<OmniboxSpec>,
}

impl OmniboxRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: OmniboxSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = OmniboxSpec>) {
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
    pub fn specs(&self) -> &[OmniboxSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&OmniboxSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

// ─── input from caller ───────────────────────────────────────────

/// One history-ish entry the caller provides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryItem {
    pub title: String,
    pub url: String,
    /// Times visited. Higher = more weight in ranking.
    pub visit_count: u32,
}

/// One bookmark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkItem {
    pub title: String,
    pub url: String,
    pub tags: Vec<String>,
}

/// One command palette entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandItem {
    pub name: String,
    pub description: Option<String>,
    pub bound_keys: Vec<String>,
}

/// One open tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabItem {
    pub title: String,
    pub url: String,
}

/// One installed extension summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionItem {
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
}

/// Everything the ranker needs from the caller. Each field is
/// consulted only if the profile enables its source.
#[derive(Debug, Clone, Default)]
pub struct OmniboxInput<'a> {
    pub history: &'a [HistoryItem],
    pub bookmarks: &'a [BookmarkItem],
    pub commands: &'a [CommandItem],
    pub tabs: &'a [TabItem],
    pub extensions: &'a [ExtensionItem],
}

// ─── output ──────────────────────────────────────────────────────

/// Suggestion category.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SuggestionKind {
    History,
    Bookmark,
    Command,
    Tab,
    Extension,
    Search,
    /// Direct navigation (query looks like a URL).
    Navigate,
}

/// One ranked suggestion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Suggestion {
    pub kind: SuggestionKind,
    pub label: String,
    #[serde(default)]
    pub detail: Option<String>,
    /// Action token the UI dispatches on Enter. Examples:
    /// `navigate:https://…`, `run:command-name`, `tab:switch:INDEX`.
    pub action: String,
    pub score: f32,
}

// ─── ranking ─────────────────────────────────────────────────────

/// Run the omnibox against `query`.
#[must_use]
pub fn rank<'a>(query: &str, spec: &OmniboxSpec, input: OmniboxInput<'a>) -> Vec<Suggestion> {
    let q = query.trim();
    if q.is_empty() || q.chars().count() < spec.min_chars {
        return Vec::new();
    }

    // Shortcut routing — if the query starts with a provider shortcut
    // followed by a space, emit only that search suggestion.
    if let Some((shortcut, remainder)) = q.split_once(' ') {
        if let Some(p) = spec
            .search_providers
            .iter()
            .find(|p| p.shortcut.as_deref() == Some(shortcut))
        {
            return vec![make_search_suggestion(p, remainder, 1.0)];
        }
    }

    let mut out: Vec<Suggestion> = Vec::new();
    let max = if spec.max_results == 0 {
        usize::MAX
    } else {
        spec.max_results
    };

    // Direct-URL shortcut: if the query parses as a bare URL or has a
    // scheme, surface a Navigate suggestion at the top.
    if looks_like_url(q) {
        out.push(Suggestion {
            kind: SuggestionKind::Navigate,
            label: q.to_owned(),
            detail: Some("Open as URL".into()),
            action: format!("navigate:{}", normalize_url_guess(q)),
            score: 0.99,
        });
    }

    for src in &spec.sources {
        match src {
            OmniboxSource::History => push_history(&mut out, q, input.history),
            OmniboxSource::Bookmarks => push_bookmarks(&mut out, q, input.bookmarks),
            OmniboxSource::Commands => push_commands(&mut out, q, input.commands),
            OmniboxSource::Tabs => push_tabs(&mut out, q, input.tabs),
            OmniboxSource::Extensions => push_extensions(&mut out, q, input.extensions),
        }
    }

    // Append search-provider suggestions at the tail, lower priority
    // than exact-matches but always present so the user can always
    // Enter to search.
    for (i, p) in spec.search_providers.iter().enumerate() {
        let base = 0.20 - (i as f32 * 0.01);
        out.push(make_search_suggestion(p, q, base.max(0.05)));
    }

    // Sort descending by score; stable for deterministic output.
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(max);
    out
}

fn push_history(out: &mut Vec<Suggestion>, q: &str, items: &[HistoryItem]) {
    let ql = q.to_ascii_lowercase();
    for h in items {
        if let Some(score) = score_text(&ql, &h.title, &h.url, None) {
            // Weight recency/frequency into the score: visit_count
            // contributes up to +0.10.
            let bonus = (f32::from(h.visit_count.min(100) as u16)) / 1000.0;
            out.push(Suggestion {
                kind: SuggestionKind::History,
                label: if h.title.is_empty() { h.url.clone() } else { h.title.clone() },
                detail: Some(h.url.clone()),
                action: format!("navigate:{}", h.url),
                score: score + bonus,
            });
        }
    }
}

fn push_bookmarks(out: &mut Vec<Suggestion>, q: &str, items: &[BookmarkItem]) {
    let ql = q.to_ascii_lowercase();
    for b in items {
        let tag_blob = b.tags.join(" ");
        let extra = (!tag_blob.is_empty()).then_some(tag_blob.as_str());
        if let Some(score) = score_text(&ql, &b.title, &b.url, extra) {
            out.push(Suggestion {
                kind: SuggestionKind::Bookmark,
                label: b.title.clone(),
                detail: Some(b.url.clone()),
                action: format!("navigate:{}", b.url),
                // Bookmarks get a small prior — the user explicitly curated them.
                score: score + 0.05,
            });
        }
    }
}

fn push_commands(out: &mut Vec<Suggestion>, q: &str, items: &[CommandItem]) {
    let ql = q.to_ascii_lowercase();
    for c in items {
        let descr = c.description.as_deref().unwrap_or("");
        if let Some(score) = score_text(&ql, &c.name, descr, None) {
            let bound = if c.bound_keys.is_empty() {
                String::new()
            } else {
                format!(" [{}]", c.bound_keys.join(", "))
            };
            out.push(Suggestion {
                kind: SuggestionKind::Command,
                label: format!("{}{}", c.name, bound),
                detail: c.description.clone(),
                action: format!("run:{}", c.name),
                score,
            });
        }
    }
}

fn push_tabs(out: &mut Vec<Suggestion>, q: &str, items: &[TabItem]) {
    let ql = q.to_ascii_lowercase();
    for (i, t) in items.iter().enumerate() {
        if let Some(score) = score_text(&ql, &t.title, &t.url, None) {
            out.push(Suggestion {
                kind: SuggestionKind::Tab,
                label: t.title.clone(),
                detail: Some(t.url.clone()),
                action: format!("tab:switch:{i}"),
                score: score + 0.10, // switching is cheaper than opening
            });
        }
    }
}

fn push_extensions(out: &mut Vec<Suggestion>, q: &str, items: &[ExtensionItem]) {
    let ql = q.to_ascii_lowercase();
    for e in items {
        let descr = e.description.as_deref().unwrap_or("");
        if let Some(score) = score_text(&ql, &e.name, descr, None) {
            out.push(Suggestion {
                kind: SuggestionKind::Extension,
                label: e.name.clone(),
                detail: e.description.clone(),
                action: format!("extension:toggle:{}", e.name),
                score,
            });
        }
    }
}

fn make_search_suggestion(p: &SearchProvider, q: &str, score: f32) -> Suggestion {
    let url = p.url.replace("{query}", &url_encode(q));
    let label = p.label.clone().unwrap_or_else(|| p.name.clone());
    Suggestion {
        kind: SuggestionKind::Search,
        label: format!("{label}: {q}"),
        detail: Some(url.clone()),
        action: format!("navigate:{url}"),
        score,
    }
}

/// Score a query against a label + url + optional extra blob. Returns
/// None if nothing matches. Priority:
///   1.0 — exact label match (case-insensitive)
///   0.9 — label starts with query
///   0.7 — URL starts with query
///   0.5 — label contains query
///   0.3 — URL contains query
///   0.2 — extra (tags/description) contains query
fn score_text(q_lower: &str, label: &str, url: &str, extra: Option<&str>) -> Option<f32> {
    let label_l = label.to_ascii_lowercase();
    let url_l = url.to_ascii_lowercase();
    if label_l == q_lower {
        return Some(1.0);
    }
    if label_l.starts_with(q_lower) {
        return Some(0.9);
    }
    if url_l.starts_with(q_lower) {
        return Some(0.7);
    }
    if label_l.contains(q_lower) {
        return Some(0.5);
    }
    if url_l.contains(q_lower) {
        return Some(0.3);
    }
    if let Some(e) = extra {
        if e.to_ascii_lowercase().contains(q_lower) {
            return Some(0.2);
        }
    }
    None
}

fn looks_like_url(q: &str) -> bool {
    q.contains("://")
        || q.starts_with("localhost")
        || q.starts_with("127.")
        || (q.contains('.') && !q.contains(' '))
}

fn normalize_url_guess(q: &str) -> String {
    if q.contains("://") {
        q.to_owned()
    } else {
        format!("https://{q}")
    }
}

/// Minimal URL-encoder for query strings — just enough to make the
/// `{query}` template safe. Encodes spaces as `+` (per form-encoded),
/// and percent-escapes `&`, `?`, `#`, `=`, `+`, and non-ASCII bytes.
#[must_use]
pub fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            ' ' => out.push('+'),
            c if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') => out.push(c),
            c => {
                for b in c.to_string().bytes() {
                    out.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    out
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<OmniboxSpec>, String> {
    tatara_lisp::compile_typed::<OmniboxSpec>(src)
        .map_err(|e| format!("failed to compile defomnibox forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<OmniboxSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(title: &str, url: &str, visits: u32) -> HistoryItem {
        HistoryItem {
            title: title.into(),
            url: url.into(),
            visit_count: visits,
        }
    }

    fn b(title: &str, url: &str, tags: &[&str]) -> BookmarkItem {
        BookmarkItem {
            title: title.into(),
            url: url.into(),
            tags: tags.iter().map(|s| (*s).into()).collect(),
        }
    }

    fn c(name: &str, desc: Option<&str>) -> CommandItem {
        CommandItem {
            name: name.into(),
            description: desc.map(str::to_owned),
            bound_keys: vec![],
        }
    }

    #[test]
    fn empty_query_returns_empty() {
        let spec = OmniboxSpec::default_profile();
        let out = rank("", &spec, OmniboxInput::default());
        assert!(out.is_empty());
    }

    #[test]
    fn min_chars_gates_short_queries() {
        let spec = OmniboxSpec {
            min_chars: 3,
            ..OmniboxSpec::default_profile()
        };
        assert!(rank("ab", &spec, OmniboxInput::default()).is_empty());
        assert!(!rank("abc", &spec, OmniboxInput::default()).is_empty());
    }

    #[test]
    fn history_substring_matches_rank_with_visit_bonus() {
        let spec = OmniboxSpec::default_profile();
        let hist = vec![
            h("Rust Docs", "https://doc.rust-lang.org", 50),
            h("Example", "https://example.com", 1),
        ];
        let input = OmniboxInput {
            history: &hist,
            ..Default::default()
        };
        let out = rank("rust", &spec, input);
        // First real result (after any synthetic navigate stub) must
        // be the Rust Docs entry.
        let first_history = out
            .iter()
            .find(|s| s.kind == SuggestionKind::History)
            .unwrap();
        assert_eq!(first_history.label, "Rust Docs");
        // And the visit-count bonus lifts it above a 1-visit substring hit.
        assert!(first_history.score > 0.9);
    }

    #[test]
    fn bookmark_match_scores_above_history_on_tie() {
        let spec = OmniboxSpec::default_profile();
        let hist = vec![h("Docs", "https://docs.example.com", 1)];
        let book = vec![b("Docs", "https://docs.example.com", &[])];
        let out = rank(
            "docs",
            &spec,
            OmniboxInput {
                history: &hist,
                bookmarks: &book,
                ..Default::default()
            },
        );
        // Find the first history + first bookmark; bookmark should score higher.
        let hs = out.iter().find(|s| s.kind == SuggestionKind::History).unwrap();
        let bs = out.iter().find(|s| s.kind == SuggestionKind::Bookmark).unwrap();
        assert!(bs.score >= hs.score);
    }

    #[test]
    fn command_matches_by_name_and_description() {
        let spec = OmniboxSpec::default_profile();
        let cmds = vec![
            c("reader:toggle", Some("Flip reader view")),
            c("reload", None),
        ];
        let out = rank(
            "flip",
            &spec,
            OmniboxInput {
                commands: &cmds,
                ..Default::default()
            },
        );
        // Description-match scores — lower than label but present.
        assert!(out.iter().any(|s| matches!(s.kind, SuggestionKind::Command)
            && s.label.starts_with("reader:toggle")));
    }

    #[test]
    fn shortcut_routes_to_specific_provider() {
        let spec = OmniboxSpec::default_profile();
        let out = rank("ddg dark mode", &spec, OmniboxInput::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, SuggestionKind::Search);
        assert!(out[0].label.contains("DuckDuckGo"));
        assert!(out[0].action.contains("dark+mode"));
        // Google should NOT fire when DDG shortcut matched.
        assert!(!out.iter().any(|s| s.label.contains("Google")));
    }

    #[test]
    fn direct_url_emits_navigate_suggestion() {
        let spec = OmniboxSpec::default_profile();
        let out = rank("example.com", &spec, OmniboxInput::default());
        let nav = out
            .iter()
            .find(|s| s.kind == SuggestionKind::Navigate)
            .unwrap();
        assert_eq!(nav.action, "navigate:https://example.com");
    }

    #[test]
    fn direct_url_with_scheme_preserves_it() {
        let spec = OmniboxSpec::default_profile();
        let out = rank("http://localhost:8080", &spec, OmniboxInput::default());
        let nav = out
            .iter()
            .find(|s| s.kind == SuggestionKind::Navigate)
            .unwrap();
        assert_eq!(nav.action, "navigate:http://localhost:8080");
    }

    #[test]
    fn search_providers_always_at_tail() {
        let spec = OmniboxSpec::default_profile();
        let out = rank("totally-absent-token", &spec, OmniboxInput::default());
        // All three providers should still render as Search suggestions.
        let search_count = out.iter().filter(|s| s.kind == SuggestionKind::Search).count();
        assert_eq!(search_count, 3);
    }

    #[test]
    fn max_results_cap_is_honored() {
        let spec = OmniboxSpec {
            max_results: 2,
            ..OmniboxSpec::default_profile()
        };
        let hist: Vec<HistoryItem> = (0..20)
            .map(|i| h(&format!("Rust {i}"), &format!("https://r{i}.example/"), 1))
            .collect();
        let out = rank(
            "rust",
            &spec,
            OmniboxInput {
                history: &hist,
                ..Default::default()
            },
        );
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn scoring_exact_beats_prefix_beats_contains() {
        assert_eq!(score_text("rust", "rust", "", None), Some(1.0));
        assert_eq!(score_text("rust", "Rust docs", "", None), Some(0.9));
        assert_eq!(score_text("rust", "about rust", "", None), Some(0.5));
        assert!(score_text("rust", "something", "", None).is_none());
    }

    #[test]
    fn url_encode_encodes_spaces_and_reserved() {
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("a&b"), "a%26b");
        assert_eq!(url_encode("a=b"), "a%3Db");
        assert_eq!(url_encode("abc"), "abc");
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = OmniboxRegistry::new();
        reg.insert(OmniboxSpec::default_profile());
        reg.insert(OmniboxSpec {
            max_results: 5,
            ..OmniboxSpec::default_profile()
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("default").unwrap().max_results, 5);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_minimal_form() {
        let src = r#"
            (defomnibox :name "minimal"
                        :sources (history)
                        :max-results 5
                        :min-chars 2)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "minimal");
        assert_eq!(s.max_results, 5);
        assert_eq!(s.min_chars, 2);
        assert_eq!(s.sources, vec![OmniboxSource::History]);
    }
}
