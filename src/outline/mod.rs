//! `(defoutline)` — table-of-contents extraction.
//!
//! Pairs with [`crate::reader`]: the reader simplifies a page; the
//! outline extractor walks the simplified (or original) DOM and
//! builds a hierarchical TOC from `<h1>..<h6>` plus any elements
//! matching a user-supplied selector list. Absorbs Firefox Reader's
//! inline outline, Chrome Reading Mode's TOC sidebar, Safari
//! Reader's article structure.
//!
//! ```lisp
//! (defoutline :name          "default"
//!             :min-level     1
//!             :max-level     4
//!             :include       ("h1" "h2" "h3" "h4")
//!             :exclude       (".footnote" "[aria-hidden=true]")
//!             :generate-ids  #t
//!             :flatten       #f)
//! ```
//!
//! `generate-ids` writes a stable slug onto any matched element
//! missing an `id` attribute so deep-link anchors work. `flatten`
//! collapses the nested tree into one flat list (easier to render
//! but loses reading-depth information).

use crate::dom::{Document, Node, NodeData};
use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Outline-extraction profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defoutline"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OutlineSpec {
    pub name: String,
    /// Minimum heading level to include (1 = `<h1>`).
    #[serde(default = "default_min_level")]
    pub min_level: u8,
    /// Maximum heading level to include. 1..=6 valid; clamped.
    #[serde(default = "default_max_level")]
    pub max_level: u8,
    /// Extra tag/attribute selectors to include beyond `<hN>`. Empty
    /// keeps only heading tags.
    #[serde(default)]
    pub include: Vec<String>,
    /// Exclude hits under any matching selector. Applied AFTER
    /// include so authors can scope out fine-grained noise.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Stamp a slug id onto any matched element missing one.
    #[serde(default = "default_generate_ids")]
    pub generate_ids: bool,
    /// Return a flat list instead of a nested tree.
    #[serde(default)]
    pub flatten: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_min_level() -> u8 {
    1
}
fn default_max_level() -> u8 {
    4
}
fn default_generate_ids() -> bool {
    true
}

impl OutlineSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            min_level: 1,
            max_level: 4,
            include: vec![],
            exclude: vec![".footnote".into(), "[aria-hidden=true]".into()],
            generate_ids: true,
            flatten: false,
            description: Some("Default outline — h1..h4, generate ids, nested.".into()),
        }
    }
}

/// One outline entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutlineEntry {
    /// Heading level 1..=6.
    pub level: u8,
    /// Visible text (whitespace-normalized, trimmed).
    pub text: String,
    /// Element id (existing or generated). Empty when not generatable.
    pub id: String,
    /// Nested children (populated when `flatten = false`).
    #[serde(default)]
    pub children: Vec<OutlineEntry>,
}

/// Walk `doc` under `spec` and produce the outline.
#[must_use]
pub fn extract_outline(doc: &Document, spec: &OutlineSpec) -> Vec<OutlineEntry> {
    let min_level = spec.min_level.clamp(1, 6);
    let max_level = spec.max_level.clamp(min_level, 6);

    let mut flat: Vec<OutlineEntry> = Vec::new();
    let mut counter: usize = 0;
    walk_for_headings(&doc.root, min_level, max_level, spec, &mut flat, &mut counter);

    if spec.flatten {
        flat
    } else {
        nest_entries(flat)
    }
}

fn walk_for_headings(
    node: &Node,
    min_level: u8,
    max_level: u8,
    spec: &OutlineSpec,
    out: &mut Vec<OutlineEntry>,
    counter: &mut usize,
) {
    // Depth-first traversal; headings emit in reading order.
    if let NodeData::Element(el) = &node.data {
        if excluded(el.tag.as_str(), el, &spec.exclude) {
            return;
        }
        let tag = el.tag.to_ascii_lowercase();
        let level = heading_level(&tag);
        let in_band = level.is_some_and(|l| l >= min_level && l <= max_level);
        let included_selector = !in_band && included_by_extra(&tag, el, &spec.include);
        if in_band || included_selector {
            let text = normalize_text(&node.text_content());
            if !text.is_empty() {
                let id = resolve_id(el, &text, spec.generate_ids, counter);
                let eff_level = level.unwrap_or(1);
                out.push(OutlineEntry {
                    level: eff_level,
                    text,
                    id,
                    children: Vec::new(),
                });
            }
        }
    }
    for c in &node.children {
        walk_for_headings(c, min_level, max_level, spec, out, counter);
    }
}

fn heading_level(tag: &str) -> Option<u8> {
    match tag {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

fn excluded(
    tag: &str,
    el: &crate::dom::ElementData,
    patterns: &[String],
) -> bool {
    patterns.iter().any(|p| element_matches(tag, el, p))
}

fn included_by_extra(
    tag: &str,
    el: &crate::dom::ElementData,
    patterns: &[String],
) -> bool {
    patterns.iter().any(|p| element_matches(tag, el, p))
}

/// Tiny selector subset — `tag`, `.class`, `#id`, `[attr=value]`.
/// Enough for outline authoring; richer selectors can compile through
/// `crate::selector::Selector` in a V2.
fn element_matches(tag: &str, el: &crate::dom::ElementData, pattern: &str) -> bool {
    let p = pattern.trim();
    if p.is_empty() {
        return false;
    }
    if let Some(rest) = p.strip_prefix('.') {
        return el.has_class(rest);
    }
    if let Some(rest) = p.strip_prefix('#') {
        return el.id() == Some(rest);
    }
    if let Some(rest) = p.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return attr_match(el, rest);
    }
    tag.eq_ignore_ascii_case(p)
}

fn attr_match(el: &crate::dom::ElementData, body: &str) -> bool {
    if let Some((name, value)) = body.split_once('=') {
        let name = name.trim();
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        el.get_attribute(name) == Some(value)
    } else {
        el.get_attribute(body.trim()).is_some()
    }
}

fn normalize_text(s: &str) -> String {
    let mut out = String::new();
    let mut last_ws = false;
    for ch in s.chars() {
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
    out.trim().to_owned()
}

fn resolve_id(
    el: &crate::dom::ElementData,
    text: &str,
    generate: bool,
    counter: &mut usize,
) -> String {
    if let Some(existing) = el.id() {
        if !existing.is_empty() {
            return existing.to_owned();
        }
    }
    if !generate {
        return String::new();
    }
    *counter += 1;
    let mut slug = slugify(text);
    if slug.is_empty() {
        slug = format!("h-{counter}");
    } else {
        // Ensure uniqueness across the document by appending the
        // counter when the slug alone would likely collide.
        slug.push('-');
        slug.push_str(&counter.to_string());
    }
    slug
}

fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = false;
    for ch in s.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            out.push(lower);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_owned()
}

fn nest_entries(flat: Vec<OutlineEntry>) -> Vec<OutlineEntry> {
    let mut roots: Vec<OutlineEntry> = Vec::new();
    for entry in flat {
        insert_by_level(&mut roots, entry);
    }
    roots
}

fn insert_by_level(roots: &mut Vec<OutlineEntry>, entry: OutlineEntry) {
    // Find the last root that's lower-level (i.e. a parent heading);
    // recurse into its children to continue the hunt.
    if let Some(last) = roots.last_mut() {
        if last.level < entry.level {
            insert_by_level(&mut last.children, entry);
            return;
        }
    }
    roots.push(entry);
}

/// Registry of outline profiles.
#[derive(Debug, Clone, Default)]
pub struct OutlineRegistry {
    specs: Vec<OutlineSpec>,
}

impl OutlineRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: OutlineSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = OutlineSpec>) {
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
    pub fn specs(&self) -> &[OutlineSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&OutlineSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<OutlineSpec>, String> {
    tatara_lisp::compile_typed::<OutlineSpec>(src)
        .map_err(|e| format!("failed to compile defoutline forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<OutlineSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn article() -> Document {
        Document::parse(
            r#"<html><body>
                <h1>Top</h1>
                <p>intro</p>
                <h2>Section One</h2>
                <p>body one</p>
                <h3 id="sub-explicit">Sub A</h3>
                <p>sub a</p>
                <h2>Section Two</h2>
                <h3 aria-hidden="true">Hidden sub</h3>
                <p class="footnote">not really a sub</p>
                <h3>Sub B</h3>
              </body></html>"#,
        )
    }

    #[test]
    fn default_profile_builds_nested_tree() {
        let doc = article();
        let spec = OutlineSpec::default_profile();
        let tree = extract_outline(&doc, &spec);
        assert_eq!(tree.len(), 1);
        let top = &tree[0];
        assert_eq!(top.level, 1);
        assert_eq!(top.text, "Top");
        // Two h2 children, each with its own h3.
        assert_eq!(top.children.len(), 2);
        assert_eq!(top.children[0].text, "Section One");
        assert_eq!(top.children[0].children[0].text, "Sub A");
        assert_eq!(top.children[1].text, "Section Two");
        assert_eq!(top.children[1].children[0].text, "Sub B");
    }

    #[test]
    fn exclude_filters_hidden_and_footnote() {
        let doc = article();
        let spec = OutlineSpec::default_profile();
        let tree = extract_outline(&doc, &spec);
        let section_two = &tree[0].children[1];
        for child in &section_two.children {
            assert_ne!(child.text, "Hidden sub");
            assert_ne!(child.text, "not really a sub");
        }
    }

    #[test]
    fn flatten_returns_sequential_list() {
        let doc = article();
        let spec = OutlineSpec {
            flatten: true,
            ..OutlineSpec::default_profile()
        };
        let flat = extract_outline(&doc, &spec);
        let texts: Vec<&str> = flat.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["Top", "Section One", "Sub A", "Section Two", "Sub B"]
        );
        // Depth preserved via `level` even when flattened.
        assert_eq!(flat[0].level, 1);
        assert_eq!(flat[2].level, 3);
    }

    #[test]
    fn min_max_level_clamps_range() {
        let doc = article();
        let spec = OutlineSpec {
            min_level: 2,
            max_level: 2,
            flatten: true,
            ..OutlineSpec::default_profile()
        };
        let flat = extract_outline(&doc, &spec);
        let texts: Vec<&str> = flat.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, vec!["Section One", "Section Two"]);
    }

    #[test]
    fn generate_ids_creates_slugs_when_missing() {
        let doc = article();
        let spec = OutlineSpec::default_profile();
        let tree = extract_outline(&doc, &spec);
        // The h1 "Top" had no id; it should have a slug now.
        assert!(!tree[0].id.is_empty());
        assert!(tree[0].id.contains("top"));
        // Existing id preserved verbatim.
        let sub_a = &tree[0].children[0].children[0];
        assert_eq!(sub_a.id, "sub-explicit");
    }

    #[test]
    fn generate_ids_off_leaves_empty_strings() {
        let doc = Document::parse("<html><body><h1>Plain</h1></body></html>");
        let spec = OutlineSpec {
            generate_ids: false,
            ..OutlineSpec::default_profile()
        };
        let tree = extract_outline(&doc, &spec);
        assert_eq!(tree[0].id, "");
    }

    #[test]
    fn include_pattern_surfaces_non_heading_element() {
        // Custom include lets a `.lede` paragraph count as a top-level entry.
        let html = r#"<html><body>
                        <p class="lede">A lede for the article</p>
                        <h1>Body Heading</h1>
                      </body></html>"#;
        let doc = Document::parse(html);
        let spec = OutlineSpec {
            include: vec![".lede".into()],
            flatten: true,
            ..OutlineSpec::default_profile()
        };
        let flat = extract_outline(&doc, &spec);
        let texts: Vec<&str> = flat.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, vec!["A lede for the article", "Body Heading"]);
    }

    #[test]
    fn normalize_text_collapses_whitespace() {
        assert_eq!(normalize_text("  a\n  b\tc  "), "a b c");
    }

    #[test]
    fn slugify_produces_url_safe_output() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("   spaced   "), "spaced");
        assert_eq!(slugify("multi--dash"), "multi-dash");
    }

    #[test]
    fn nest_entries_handles_depth_jumps() {
        // h1 → h3 (skips h2). h3 becomes a child of h1.
        let html = "<html><body><h1>A</h1><h3>C</h3></body></html>";
        let doc = Document::parse(html);
        let spec = OutlineSpec::default_profile();
        let tree = extract_outline(&doc, &spec);
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].level, 3);
    }

    #[test]
    fn attr_selector_matches_value() {
        let doc = Document::parse(r#"<html><body><h2 aria-hidden="true">x</h2></body></html>"#);
        let spec = OutlineSpec {
            exclude: vec!["[aria-hidden=true]".into()],
            flatten: true,
            ..OutlineSpec::default_profile()
        };
        let flat = extract_outline(&doc, &spec);
        assert!(flat.is_empty());
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = OutlineRegistry::new();
        reg.insert(OutlineSpec::default_profile());
        reg.insert(OutlineSpec {
            flatten: true,
            ..OutlineSpec::default_profile()
        });
        assert_eq!(reg.len(), 1);
        assert!(reg.specs()[0].flatten);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_outline_form() {
        let src = r#"
            (defoutline :name     "strict"
                        :min-level 2
                        :max-level 3
                        :include  (".lede")
                        :exclude  (".footnote")
                        :generate-ids #f
                        :flatten  #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "strict");
        assert_eq!(s.min_level, 2);
        assert_eq!(s.max_level, 3);
        assert!(!s.generate_ids);
        assert!(s.flatten);
    }
}
