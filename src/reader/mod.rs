//! `(defreader)` — Mozilla-Readability-style simplified view.
//!
//! Absorbs Firefox Reader View + Safari Reader into the substrate
//! pattern: one DSL form declares content-root preferences, noise
//! strippers, and tag whitelist; the engine produces a simplified
//! Document + plain-text render + extracted metadata.
//!
//! V1 scope:
//! - Content-root detection: try `spec.roots` selectors in order,
//!   fall back to paragraph-density scoring, then `<body>`.
//! - Strip: remove any element matching `spec.strip` selectors.
//! - Keep tags: if non-empty, elements whose tag is outside the set
//!   are unwrapped (their children survive, the wrapper dies).
//! - Title: `<h1>` inside the content root → first `<h1>` anywhere →
//!   document `<title>`.
//! - Byline: `[rel=author]` / `[itemprop=author]` / `.byline`.
//! - Word count: whitespace-split of the final text render.
//!
//! ```lisp
//! (defreader :name "default"
//!            :hosts ("*")
//!            :roots ("article" "main" "[role=article]" "n-article")
//!            :strip ("nav" "aside" "footer" ".ad" "script" "style")
//!            :keep-tags ("p" "h1" "h2" "h3" "h4" "h5" "h6"
//!                        "pre" "blockquote" "figure" "img"
//!                        "ul" "ol" "li" "a" "em" "strong"))
//! ```

use crate::dom::{Document, ElementData, Node, NodeData};
use crate::selector::{OwnedContext, Selector};
use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Declarative reader-mode rule.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defreader"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReaderSpec {
    pub name: String,
    /// Host patterns — substring match against `url.host()`. Empty or
    /// `["*"]` means "every page".
    #[serde(default)]
    pub hosts: Vec<String>,
    /// Preferred content-root selectors, tried in order. First hit wins.
    #[serde(default)]
    pub roots: Vec<String>,
    /// Selectors to drop from the content root before rendering.
    #[serde(default)]
    pub strip: Vec<String>,
    /// Allow-list of tag names (lowercase). Elements outside the set
    /// are unwrapped — their children survive, the wrapper is dropped.
    /// Empty means "keep everything that wasn't stripped".
    #[serde(default)]
    pub keep_tags: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl ReaderSpec {
    /// Built-in sensible default — works on most article pages.
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            hosts: vec!["*".into()],
            roots: [
                "article",
                "main",
                "[role=article]",
                "[role=main]",
                "n-article",
                "n-main",
                "#content",
                "#main",
                ".article",
                ".post",
            ]
            .iter()
            .map(|s| (*s).into())
            .collect(),
            strip: [
                "nav",
                "aside",
                "footer",
                "header nav",
                "script",
                "style",
                "noscript",
                "iframe",
                "form",
                ".ad",
                ".ads",
                ".advertisement",
                ".sidebar",
                ".comments",
                ".related",
                "[aria-hidden=true]",
                "[role=navigation]",
                "[role=complementary]",
            ]
            .iter()
            .map(|s| (*s).into())
            .collect(),
            keep_tags: [
                "p",
                "h1",
                "h2",
                "h3",
                "h4",
                "h5",
                "h6",
                "pre",
                "code",
                "blockquote",
                "figure",
                "figcaption",
                "img",
                "ul",
                "ol",
                "li",
                "a",
                "em",
                "strong",
                "b",
                "i",
                "br",
                "hr",
                "article",
                "section",
            ]
            .iter()
            .map(|s| (*s).into())
            .collect(),
            description: Some("Readability-style simplified view.".into()),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        if self.hosts.is_empty() {
            return true;
        }
        self.hosts.iter().any(|h| h == "*" || host.contains(h))
    }
}

/// Registry of reader profiles. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct ReaderRegistry {
    specs: Vec<ReaderSpec>,
}

impl ReaderRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: ReaderSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ReaderSpec>) {
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
    pub fn specs(&self) -> &[ReaderSpec] {
        &self.specs
    }

    /// First spec whose host pattern matches. Typical V1 usage: the
    /// registry holds one `default` profile (no host filter) plus
    /// zero or more per-host overrides; the matcher returns the
    /// most-specific hit.
    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&ReaderSpec> {
        // Prefer host-specific rules over wildcards.
        let specific = self
            .specs
            .iter()
            .find(|s| !s.hosts.is_empty() && !s.hosts.iter().any(|h| h == "*") && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.matches_host(host)))
    }
}

/// Extracted simplified content.
#[derive(Debug, Clone)]
pub struct ReaderOutput {
    pub spec_name: String,
    pub title: Option<String>,
    pub byline: Option<String>,
    /// A fresh Document wrapping the simplified content.
    pub content: Document,
    /// Whitespace-normalized plain text of `content`.
    pub text: String,
    pub word_count: usize,
}

/// Run the reader against a parsed document.
///
/// The source `doc` is **not** mutated — we clone the selected
/// subtree and rebuild a fresh Document. This keeps the reader
/// compatible with every other pass in the pipeline (normalize,
/// blocker, transform) which still needs the original DOM.
pub fn apply_reader(doc: &Document, spec: &ReaderSpec) -> ReaderOutput {
    let doc_title = doc.title();
    let byline = extract_byline(doc);

    // 1. Find the content root.
    let root = find_content_root(doc, &spec.roots)
        .or_else(|| doc.query_selector("body"))
        .unwrap_or(&doc.root);

    // 2. Clone the subtree.
    let mut cloned = root.clone();

    // 3. Strip.
    let compiled_strip: Vec<(String, Selector)> = spec
        .strip
        .iter()
        .filter_map(|sel| Selector::parse(sel).ok().map(|s| (sel.clone(), s)))
        .collect();
    let mut path: Vec<OwnedContext> = Vec::new();
    strip_matches(&mut cloned, &compiled_strip, &mut path);

    // 4. Unwrap tags not in the keep set.
    if !spec.keep_tags.is_empty() {
        let keep: std::collections::HashSet<String> =
            spec.keep_tags.iter().map(|t| t.to_ascii_lowercase()).collect();
        cloned = unwrap_outside_keep(cloned, &keep);
    }

    // 5. Pick a title: prefer an <h1> inside the selected subtree,
    //    then the document <title>, then None.
    let title = first_heading_text(&cloned).or(doc_title);

    // 6. Build a fresh Document wrapping the simplified content.
    let content = wrap_as_document(&cloned, title.as_deref());
    let text = normalize_whitespace(&content.root.text_content());
    let word_count = text.split_whitespace().count();

    ReaderOutput {
        spec_name: spec.name.clone(),
        title,
        byline,
        content,
        text,
        word_count,
    }
}

// ─── content root selection ──────────────────────────────────────

fn find_content_root<'a>(doc: &'a Document, roots: &[String]) -> Option<&'a Node> {
    for sel in roots {
        let Ok(compiled) = Selector::parse(sel) else {
            continue;
        };
        if let Some(node) = find_first_matching(&doc.root, &compiled, &mut Vec::new()) {
            return Some(node);
        }
    }
    best_paragraph_density(doc)
}

fn find_first_matching<'a>(
    node: &'a Node,
    sel: &Selector,
    path: &mut Vec<OwnedContext>,
) -> Option<&'a Node> {
    let pushed = if let NodeData::Element(el) = &node.data {
        path.push(OwnedContext::from_element(el));
        let refs: Vec<&OwnedContext> = path.iter().collect();
        if sel.matches(&refs) {
            path.pop();
            return Some(node);
        }
        true
    } else {
        false
    };
    for child in &node.children {
        if let Some(hit) = find_first_matching(child, sel, path) {
            if pushed {
                path.pop();
            }
            return Some(hit);
        }
    }
    if pushed {
        path.pop();
    }
    None
}

fn best_paragraph_density(doc: &Document) -> Option<&Node> {
    let mut best: Option<(&Node, usize)> = None;
    for node in doc.root.descendants() {
        let Some(el) = node.as_element() else { continue };
        // Only score block-ish containers.
        let tag = el.tag.to_ascii_lowercase();
        if !matches!(
            tag.as_str(),
            "div" | "section" | "article" | "main" | "body"
        ) {
            continue;
        }
        let score = score_paragraph_density(node);
        if score < 280 {
            continue; // too small to be the main content
        }
        match best {
            Some((_, b)) if score <= b => {}
            _ => best = Some((node, score)),
        }
    }
    best.map(|(n, _)| n)
}

fn score_paragraph_density(node: &Node) -> usize {
    let mut total = 0usize;
    for n in node.descendants() {
        let Some(el) = n.as_element() else { continue };
        let t = el.tag.to_ascii_lowercase();
        if matches!(t.as_str(), "p" | "pre" | "blockquote" | "li") {
            total += n.text_content().trim().len();
        }
    }
    total
}

// ─── strip ───────────────────────────────────────────────────────

fn strip_matches(node: &mut Node, rules: &[(String, Selector)], path: &mut Vec<OwnedContext>) {
    let pushed = if let NodeData::Element(el) = &node.data {
        path.push(OwnedContext::from_element(el));
        true
    } else {
        false
    };

    // Recurse first.
    for child in &mut node.children {
        strip_matches(child, rules, path);
    }

    // Scan own children for strip-list matches.
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
        let drop = rules.iter().any(|(_, s)| s.matches(&full));
        if drop {
            node.children.remove(i);
        } else {
            i += 1;
        }
    }

    if pushed {
        path.pop();
    }
}

// ─── unwrap outside keep set ─────────────────────────────────────

fn unwrap_outside_keep(node: Node, keep: &std::collections::HashSet<String>) -> Node {
    let Node { data, children } = node;
    let mut new_children = Vec::with_capacity(children.len());
    for child in children {
        match child.data {
            NodeData::Element(ref el) if !keep.contains(&el.tag.to_ascii_lowercase()) => {
                // Unwrap: recurse into children and splice them in.
                let recursed = unwrap_outside_keep(child, keep);
                new_children.extend(recursed.children);
            }
            _ => {
                new_children.push(unwrap_outside_keep(child, keep));
            }
        }
    }
    Node {
        data,
        children: new_children,
    }
}

// ─── metadata extraction ─────────────────────────────────────────

fn extract_byline(doc: &Document) -> Option<String> {
    let selectors = [
        "[rel=author]",
        "[itemprop=author]",
        ".byline",
        ".author",
    ];
    for sel in selectors {
        let Ok(compiled) = Selector::parse(sel) else {
            continue;
        };
        if let Some(node) = find_first_matching(&doc.root, &compiled, &mut Vec::new()) {
            let t = normalize_whitespace(&node.text_content());
            if !t.is_empty() && t.len() < 200 {
                return Some(t);
            }
        }
    }
    None
}

fn first_heading_text(node: &Node) -> Option<String> {
    for n in node.descendants() {
        let Some(el) = n.as_element() else { continue };
        let t = el.tag.to_ascii_lowercase();
        if t == "h1" {
            let text = normalize_whitespace(&n.text_content());
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

fn wrap_as_document(subtree: &Node, title: Option<&str>) -> Document {
    let mut body = Node::element(ElementData::new("body"));
    if let Some(t) = title {
        let mut h1 = Node::element(ElementData::new("h1"));
        h1.append_child(Node::text(t));
        // Only emit a synthetic <h1> if the subtree doesn't already
        // have its own h1 — otherwise readers see the title twice.
        if first_heading_text(subtree).is_none() {
            body.append_child(h1);
        }
    }
    // If the subtree itself is body, splice; otherwise nest.
    if let Some(el) = subtree.as_element() {
        if el.tag.eq_ignore_ascii_case("body") {
            for c in &subtree.children {
                body.append_child(c.clone());
            }
        } else {
            body.append_child(subtree.clone());
        }
    } else {
        for c in &subtree.children {
            body.append_child(c.clone());
        }
    }
    let mut html = Node::element(ElementData::new("html"));
    html.append_child(body);
    let mut root = Node::document();
    root.append_child(html);
    Document { root }
}

fn normalize_whitespace(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_ws = false;
    for ch in input.chars() {
        if ch.is_whitespace() {
            if !last_ws && !out.is_empty() {
                out.push(' ');
            }
            last_ws = true;
        } else {
            out.push(ch);
            last_ws = false;
        }
    }
    out.trim_end().to_owned()
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ReaderSpec>, String> {
    tatara_lisp::compile_typed::<ReaderSpec>(src)
        .map_err(|e| format!("failed to compile defreader forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ReaderSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_article_html() -> &'static str {
        r#"<html>
              <head><title>Example Article</title></head>
              <body>
                <nav><a href='/'>Home</a></nav>
                <header class='site-header'>Banner noise</header>
                <article>
                  <h1>My Article Title</h1>
                  <p class='byline'>By Author Name</p>
                  <p>First paragraph of body text, long enough to rank above the
                     nav and sidebar in paragraph-density scoring. The sidebar is
                     tight; this paragraph is where the article content lives.</p>
                  <p>Second paragraph adds more signal so density stays high.</p>
                  <aside class='ad'>buy now</aside>
                </article>
                <aside class='sidebar'>
                  <h3>Related</h3>
                  <ul><li>Other article</li></ul>
                </aside>
                <footer>© 2026</footer>
                <script>console.log('x')</script>
              </body>
            </html>"#
    }

    #[test]
    fn default_profile_strips_nav_and_keeps_article() {
        let doc = Document::parse(sample_article_html());
        let spec = ReaderSpec::default_profile();
        let out = apply_reader(&doc, &spec);

        // Title came through.
        assert_eq!(out.title.as_deref(), Some("My Article Title"));

        // Text has the two paragraphs, no nav, no footer, no script.
        assert!(out.text.contains("First paragraph"));
        assert!(out.text.contains("Second paragraph"));
        assert!(!out.text.contains("Home"));
        assert!(!out.text.contains("© 2026"));
        assert!(!out.text.contains("console.log"));
        assert!(!out.text.contains("buy now"));
        // Sidebar content also dropped — either by strip or because
        // the content root was the <article> node, which excludes
        // ancestor siblings.
        assert!(!out.text.contains("Related"));
    }

    #[test]
    fn article_selector_preferred_over_density_fallback() {
        // Two blocks, but `article` exists — it wins deterministically.
        let html = r#"<html><body>
            <div>irrelevant div with enough paragraph text to
               otherwise trigger the density heuristic. Lorem ipsum
               dolor sit amet, consectetur adipiscing elit, sed do
               eiusmod tempor incididunt ut labore et dolore magna
               aliqua ut enim ad minim veniam quis nostrud exercitation
               ullamco laboris nisi ut aliquip ex ea commodo consequat
               duis aute irure dolor in reprehenderit in voluptate
               velit esse cillum dolore eu fugiat nulla pariatur.</div>
            <article>
              <h1>Real</h1>
              <p>Chosen because article beats density.</p>
            </article>
          </body></html>"#;
        let doc = Document::parse(html);
        let out = apply_reader(&doc, &ReaderSpec::default_profile());
        assert_eq!(out.title.as_deref(), Some("Real"));
        assert!(out.text.contains("Chosen because"));
        assert!(!out.text.contains("irrelevant div"));
    }

    #[test]
    fn density_fallback_picks_the_paragraph_heavy_container() {
        // No <article> / <main> / role= hooks → density heuristic
        // must pick the content div over the ad sidebar.
        let html = r#"<html><body>
            <div class='sidebar'><p>SIDEBAR_MARKER</p></div>
            <div class='content'>
              <h1>Density Winner</h1>
              <p>Paragraph one is long enough to clear the 280-character
                 density threshold by itself. It keeps talking about the
                 substance of the article, nothing cleverer than that,
                 because the fallback is just "where is the text".</p>
              <p>Paragraph two padding padding padding padding padding
                 padding padding padding padding padding padding padding
                 padding padding padding padding padding padding.</p>
            </div>
          </body></html>"#;
        let doc = Document::parse(html);
        let spec = ReaderSpec {
            roots: vec![], // force density fallback
            ..ReaderSpec::default_profile()
        };
        let out = apply_reader(&doc, &spec);
        assert!(out.text.contains("substance of the article"));
        assert!(!out.text.contains("SIDEBAR_MARKER"));
    }

    #[test]
    fn keep_tags_unwrap_outside_whitelist() {
        let html = r#"<html><body>
            <article>
              <h1>Title</h1>
              <div class='wrapper'>
                <span class='accent'><p>inner paragraph</p></span>
              </div>
            </article>
          </body></html>"#;
        let doc = Document::parse(html);
        let spec = ReaderSpec {
            keep_tags: ["p", "h1", "article"].iter().map(|s| (*s).into()).collect(),
            strip: vec![],
            ..ReaderSpec::default_profile()
        };
        let out = apply_reader(&doc, &spec);
        let html = out.content.root.to_html();
        // <div> and <span> unwrapped, <p> and <h1> kept.
        assert!(html.contains("<p>inner paragraph</p>"));
        assert!(!html.contains("<div"));
        assert!(!html.contains("<span"));
    }

    #[test]
    fn byline_extracted_from_rel_author() {
        let html = r#"<html><body>
            <article>
              <h1>T</h1>
              <p><a rel='author' href='#'>Jane Doe</a></p>
              <p>body</p>
            </article>
          </body></html>"#;
        let doc = Document::parse(html);
        let out = apply_reader(&doc, &ReaderSpec::default_profile());
        assert_eq!(out.byline.as_deref(), Some("Jane Doe"));
    }

    #[test]
    fn word_count_matches_whitespace_split() {
        let html = r#"<html><body><article><h1>t</h1>
            <p>one two three four five</p></article></body></html>"#;
        let doc = Document::parse(html);
        let out = apply_reader(&doc, &ReaderSpec::default_profile());
        // "t" (title) + 5 body words, but title only emits once —
        // the synthetic <h1> only runs when no h1 exists in subtree.
        assert!(out.word_count >= 5);
    }

    #[test]
    fn empty_strip_list_is_harmless() {
        let doc = Document::parse("<html><body><article><h1>t</h1><p>body</p></article></body></html>");
        let spec = ReaderSpec {
            strip: vec![],
            ..ReaderSpec::default_profile()
        };
        let out = apply_reader(&doc, &spec);
        assert!(out.text.contains("body"));
    }

    #[test]
    fn registry_resolves_host_specific_over_wildcard() {
        let mut reg = ReaderRegistry::new();
        reg.insert(ReaderSpec {
            name: "default".into(),
            hosts: vec!["*".into()],
            ..ReaderSpec::default_profile()
        });
        reg.insert(ReaderSpec {
            name: "github".into(),
            hosts: vec!["github.com".into()],
            ..ReaderSpec::default_profile()
        });
        let hit = reg.resolve("github.com").unwrap();
        assert_eq!(hit.name, "github");
        let fallback = reg.resolve("example.org").unwrap();
        assert_eq!(fallback.name, "default");
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = ReaderRegistry::new();
        reg.insert(ReaderSpec {
            name: "x".into(),
            strip: vec!["a".into()],
            ..ReaderSpec::default_profile()
        });
        reg.insert(ReaderSpec {
            name: "x".into(),
            strip: vec!["b".into()],
            ..ReaderSpec::default_profile()
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].strip, vec!["b"]);
    }

    #[test]
    fn normalize_whitespace_collapses_runs_and_trims() {
        assert_eq!(normalize_whitespace("  a   b\n\tc  "), "a b c");
        assert_eq!(normalize_whitespace(""), "");
    }

    #[test]
    fn host_wildcard_matches_any_host() {
        let s = ReaderSpec::default_profile();
        assert!(s.matches_host("example.com"));
        assert!(s.matches_host(""));
    }

    #[test]
    fn empty_hosts_matches_everything() {
        let s = ReaderSpec {
            hosts: vec![],
            ..ReaderSpec::default_profile()
        };
        assert!(s.matches_host("anything"));
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_reader_form() {
        let src = r#"
            (defreader :name "blog"
                       :hosts ("blog.example.com")
                       :roots ("article" "main")
                       :strip (".ad" "aside")
                       :keep-tags ("p" "h1" "h2"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "blog");
        assert_eq!(specs[0].hosts, vec!["blog.example.com"]);
        assert_eq!(specs[0].roots, vec!["article", "main"]);
        assert_eq!(specs[0].strip, vec![".ad", "aside"]);
        assert_eq!(specs[0].keep_tags, vec!["p", "h1", "h2"]);
    }
}
