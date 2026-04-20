//! `(deffind)` — declarative find-in-page.
//!
//! Absorbs Cmd+F from every browser into the substrate pattern. Each
//! profile knob (case sensitivity, whole-word matching, regex support,
//! match cap) is a field on [`FindSpec`]; the engine runs over the
//! parsed DOM and returns typed [`FindMatch`] hits so callers can
//! overlay a highlight pass, jump the viewport, or pipe through MCP.
//!
//! ```lisp
//! (deffind :name             "default"
//!          :case-sensitive   #f
//!          :whole-word       #f
//!          :regex            #f
//!          :max-matches      500)
//!
//! (deffind :name             "strict"
//!          :case-sensitive   #t
//!          :whole-word       #t
//!          :max-matches      50)
//! ```

use crate::dom::{Document, Node, NodeData};
use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Declarative find-in-page profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "deffind"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FindSpec {
    pub name: String,
    /// Case-sensitive comparison. Default false (Google-style).
    #[serde(default)]
    pub case_sensitive: bool,
    /// Require a word boundary on both sides of the match.
    #[serde(default)]
    pub whole_word: bool,
    /// Treat the query as a regex. When false, query is a literal
    /// substring (with word-boundary decoration if `whole_word`).
    #[serde(default)]
    pub regex: bool,
    /// Safety cap — a malformed regex on a huge page can produce
    /// enough hits to DoS the inspector. `0` disables the cap.
    #[serde(default = "default_max_matches")]
    pub max_matches: usize,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_max_matches() -> usize {
    500
}

impl FindSpec {
    /// Built-in default — what Cmd+F does on most sites.
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            case_sensitive: false,
            whole_word: false,
            regex: false,
            max_matches: 500,
            description: Some("Case-insensitive substring find.".into()),
        }
    }
}

/// Registry of find profiles.
#[derive(Debug, Clone, Default)]
pub struct FindRegistry {
    specs: Vec<FindSpec>,
}

impl FindRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: FindSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = FindSpec>) {
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
    pub fn specs(&self) -> &[FindSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&FindSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

/// One text hit inside the document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FindMatch {
    /// Enclosing element tag (for UI jump-to-section labels).
    pub enclosing_tag: Option<String>,
    /// Depth-first index of the containing text node (0-based) so
    /// highlighters can address the same slot across calls.
    pub text_node_index: usize,
    /// Character offset inside that text node.
    pub offset: usize,
    /// Matched text (post-case-fold if the spec asked for it —
    /// always the original casing as it appears in the DOM).
    pub matched: String,
}

/// Run a find against `doc`. Empty / unmatched queries return an
/// empty vec. Max-matches cap honored; anything beyond it is simply
/// dropped (the UI can prompt the user to narrow their search).
#[must_use]
pub fn find_in_document(doc: &Document, query: &str, spec: &FindSpec) -> Vec<FindMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let matcher = match Matcher::build(query, spec) {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut text_node_index: usize = 0;
    walk(&doc.root, None, &mut text_node_index, &matcher, spec, &mut out);
    out
}

enum Matcher {
    Literal {
        needle: String,
        case_sensitive: bool,
        whole_word: bool,
    },
    Regex(regex::Regex),
}

impl Matcher {
    fn build(query: &str, spec: &FindSpec) -> Result<Self, String> {
        if spec.regex {
            let mut builder = regex::RegexBuilder::new(query);
            builder.case_insensitive(!spec.case_sensitive);
            builder
                .build()
                .map(Matcher::Regex)
                .map_err(|e| e.to_string())
        } else {
            let needle = if spec.case_sensitive {
                query.to_owned()
            } else {
                query.to_lowercase()
            };
            Ok(Matcher::Literal {
                needle,
                case_sensitive: spec.case_sensitive,
                whole_word: spec.whole_word,
            })
        }
    }

    fn find_all(&self, haystack: &str) -> Vec<(usize, String)> {
        match self {
            Matcher::Literal {
                needle,
                case_sensitive,
                whole_word,
            } => {
                let cmp_hay = if *case_sensitive {
                    std::borrow::Cow::Borrowed(haystack)
                } else {
                    std::borrow::Cow::Owned(haystack.to_lowercase())
                };
                let mut out: Vec<(usize, String)> = Vec::new();
                let mut start = 0usize;
                while let Some(idx) = cmp_hay[start..].find(needle.as_str()) {
                    let abs = start + idx;
                    let slice = &haystack[abs..abs + needle.len()];
                    let passes = if *whole_word {
                        boundary_ok(haystack, abs, needle.len())
                    } else {
                        true
                    };
                    if passes {
                        out.push((abs, slice.to_owned()));
                    }
                    start = abs + needle.len();
                    if needle.is_empty() {
                        break;
                    }
                }
                out
            }
            Matcher::Regex(re) => re
                .find_iter(haystack)
                .map(|m| (m.start(), m.as_str().to_owned()))
                .collect(),
        }
    }
}

fn boundary_ok(text: &str, start: usize, len: usize) -> bool {
    let before = text[..start].chars().last();
    let after = text[start + len..].chars().next();
    let is_wordy = |c: Option<char>| {
        c.map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false)
    };
    !is_wordy(before) && !is_wordy(after)
}

fn walk(
    node: &Node,
    enclosing_tag: Option<&str>,
    text_idx: &mut usize,
    matcher: &Matcher,
    spec: &FindSpec,
    out: &mut Vec<FindMatch>,
) {
    match &node.data {
        NodeData::Text(t) => {
            let this_idx = *text_idx;
            *text_idx += 1;
            let hits = matcher.find_all(t);
            for (offset, matched) in hits {
                if spec.max_matches > 0 && out.len() >= spec.max_matches {
                    return;
                }
                out.push(FindMatch {
                    enclosing_tag: enclosing_tag.map(str::to_owned),
                    text_node_index: this_idx,
                    offset,
                    matched,
                });
            }
        }
        NodeData::Element(el) => {
            // Skip nodes whose text is meaningless for user-facing
            // search — script/style/comment payloads would pollute
            // results.
            let tag = el.tag.to_ascii_lowercase();
            if matches!(tag.as_str(), "script" | "style" | "noscript" | "template") {
                return;
            }
            for child in &node.children {
                if spec.max_matches > 0 && out.len() >= spec.max_matches {
                    return;
                }
                walk(child, Some(&tag), text_idx, matcher, spec, out);
            }
        }
        NodeData::Document => {
            for child in &node.children {
                walk(child, enclosing_tag, text_idx, matcher, spec, out);
            }
        }
        NodeData::Comment(_) => {}
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<FindSpec>, String> {
    tatara_lisp::compile_typed::<FindSpec>(src)
        .map_err(|e| format!("failed to compile deffind forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<FindSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Document {
        Document::parse(
            r#"<html><body>
                <h1>Rust Browser</h1>
                <p>Rust is memory-safe and rust-the-fungus isn't.</p>
                <script>var secret = "rust"</script>
                <p class="footnote">See also: trust, gust.</p>
              </body></html>"#,
        )
    }

    #[test]
    fn empty_query_returns_empty() {
        let doc = sample();
        let spec = FindSpec::default_profile();
        assert!(find_in_document(&doc, "", &spec).is_empty());
    }

    #[test]
    fn substring_match_case_insensitive() {
        let doc = sample();
        let spec = FindSpec::default_profile();
        let hits = find_in_document(&doc, "rust", &spec);
        // "Rust" in h1 + "Rust" + "rust-the-fungus" in p — script
        // content is excluded. "trust"/"gust" contain "rust"/"gust"
        // so with substring matching they also hit.
        assert!(hits.iter().any(|h| h.matched == "Rust"));
        assert!(hits.iter().any(|h| h.matched == "rust"));
        // Script is skipped.
        assert!(!hits.iter().any(|h| h.enclosing_tag.as_deref() == Some("script")));
    }

    #[test]
    fn case_sensitive_respected() {
        let doc = sample();
        let spec = FindSpec {
            case_sensitive: true,
            ..FindSpec::default_profile()
        };
        let rust_cap = find_in_document(&doc, "Rust", &spec);
        // Only capitalized occurrences.
        assert!(rust_cap.iter().all(|h| h.matched == "Rust"));
    }

    #[test]
    fn whole_word_rejects_substring_hits() {
        let doc = Document::parse("<p>trust the rust</p>");
        let spec = FindSpec {
            whole_word: true,
            ..FindSpec::default_profile()
        };
        let hits = find_in_document(&doc, "rust", &spec);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].matched, "rust");
    }

    #[test]
    fn whole_word_also_handles_hyphens_as_boundaries() {
        let doc = Document::parse("<p>rust-lang site</p>");
        let spec = FindSpec {
            whole_word: true,
            ..FindSpec::default_profile()
        };
        let hits = find_in_document(&doc, "rust", &spec);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn regex_mode_supports_alternation() {
        let doc = sample();
        let spec = FindSpec {
            regex: true,
            case_sensitive: true,
            ..FindSpec::default_profile()
        };
        let hits = find_in_document(&doc, "Rust|gust", &spec);
        assert!(hits.iter().any(|h| h.matched == "Rust"));
        assert!(hits.iter().any(|h| h.matched == "gust"));
    }

    #[test]
    fn malformed_regex_returns_empty_not_panics() {
        let doc = sample();
        let spec = FindSpec {
            regex: true,
            ..FindSpec::default_profile()
        };
        let hits = find_in_document(&doc, "[unbalanced", &spec);
        assert!(hits.is_empty());
    }

    #[test]
    fn max_matches_caps_results() {
        let long = "a ".repeat(2000);
        let html = format!("<p>{long}</p>");
        let doc = Document::parse(&html);
        let spec = FindSpec {
            max_matches: 10,
            ..FindSpec::default_profile()
        };
        let hits = find_in_document(&doc, "a", &spec);
        assert_eq!(hits.len(), 10);
    }

    #[test]
    fn script_and_style_content_skipped() {
        let doc = Document::parse(
            r#"<html><body>
                <p>visible</p>
                <script>invisible</script>
                <style>.x { color: invisible; }</style>
              </body></html>"#,
        );
        let spec = FindSpec::default_profile();
        let hits = find_in_document(&doc, "invisible", &spec);
        assert!(hits.is_empty());
    }

    #[test]
    fn matches_carry_enclosing_tag_and_offset() {
        let doc = Document::parse("<p>hello world hello again</p>");
        let spec = FindSpec {
            case_sensitive: true,
            ..FindSpec::default_profile()
        };
        let hits = find_in_document(&doc, "hello", &spec);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].offset, 0);
        assert_eq!(hits[0].enclosing_tag.as_deref(), Some("p"));
        assert_eq!(hits[1].offset, 12);
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = FindRegistry::new();
        reg.insert(FindSpec::default_profile());
        reg.insert(FindSpec {
            whole_word: true,
            ..FindSpec::default_profile()
        });
        assert_eq!(reg.len(), 1);
        assert!(reg.get("default").unwrap().whole_word);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_find_form() {
        let src = r#"
            (deffind :name "strict"
                     :case-sensitive #t
                     :whole-word     #t
                     :regex          #f
                     :max-matches    50)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "strict");
        assert!(s.case_sensitive);
        assert!(s.whole_word);
        assert!(!s.regex);
        assert_eq!(s.max_matches, 50);
    }
}
