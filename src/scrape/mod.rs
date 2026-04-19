//! Lisp-native DOM scraping / dissection.
//!
//! The dual of [`crate::transform`]: instead of mutating the tree,
//! walks it and *collects* matches. Authored as tatara-lisp:
//!
//! ```lisp
//! (defscrape :name "hn-titles"
//!            :selector ".titleline > a"
//!            :extract text)
//!
//! (defscrape :name "hn-links"
//!            :selector ".titleline > a"
//!            :extract attr
//!            :attr "href")
//!
//! (defscrape :name "all-images"
//!            :selector "img"
//!            :extract attrs)
//! ```
//!
//! Each scrape returns a list of [`ScrapeHit`] carrying the ancestor
//! path (for disambiguation), tag name, and the extracted value. Hits
//! serialize to JSON so CLI tools and MCP servers can ship them
//! straight to consumers.
//!
//! Selector grammar is the full [`crate::selector::Selector`]
//! (compound / descendant / child / universal). No tree mutation
//! happens during scrape — safe to run alongside transforms.

use crate::dom::{Document, Node, NodeData};
use crate::selector::{OwnedContext, Selector};
use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// A declarative scrape: selector + what to extract from each match.
///
/// ```lisp
/// (defscrape :name "titles" :selector "article h2" :extract text)
/// ```
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defscrape"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeSpec {
    pub name: String,
    pub selector: String,
    pub extract: ExtractKind,
    /// Which attribute to return when `extract = attr`. Ignored otherwise.
    #[serde(default)]
    pub attr: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// What to pull from each matched element.
///
/// Bare symbols in Lisp: `:extract text`, `:extract attr`, etc.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExtractKind {
    /// Concatenated text content of the element and its descendants.
    Text,
    /// The value of a single named attribute (requires `:attr`).
    Attr,
    /// The element's tag name.
    Tag,
    /// Every attribute key/value pair on the element.
    Attrs,
}

/// One match of one scrape.
#[derive(Debug, Clone, Serialize)]
pub struct ScrapeHit {
    pub scrape: String,
    pub tag: String,
    /// Ancestor tag chain root → parent (excludes the matched element).
    pub path: Vec<String>,
    pub value: ScrapeValue,
}

/// The extracted value.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScrapeValue {
    Text { text: String },
    Attr { name: String, value: Option<String> },
    Tag { tag: String },
    Attrs { attrs: Vec<(String, String)> },
}

/// Apply every scrape to a document; return flat list of hits.
///
/// Order: outer loop is the scrape specs (in author order); inner loop
/// is depth-first tree traversal. Invalid selectors are logged and
/// skipped, not errors.
pub fn scrape(doc: &Document, specs: &[ScrapeSpec]) -> Vec<ScrapeHit> {
    let mut hits = Vec::new();
    for spec in specs {
        let selector = match Selector::parse(&spec.selector) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    "scrape {} has invalid selector {:?}: {e}",
                    spec.name,
                    spec.selector
                );
                continue;
            }
        };
        let mut path: Vec<PathItem> = Vec::new();
        walk(&doc.root, spec, &selector, &mut path, &mut hits);
    }
    hits
}

/// Same as [`scrape`], but written in streaming style for large docs.
pub fn scrape_stream<F: FnMut(ScrapeHit)>(doc: &Document, specs: &[ScrapeSpec], mut emit: F) {
    for spec in specs {
        let Ok(selector) = Selector::parse(&spec.selector) else {
            continue;
        };
        let mut path: Vec<PathItem> = Vec::new();
        walk_emit(&doc.root, spec, &selector, &mut path, &mut emit);
    }
}

// Canonical ancestor-path element lives in `selector::OwnedContext`.
type PathItem = OwnedContext;

fn walk(
    node: &Node,
    spec: &ScrapeSpec,
    selector: &Selector,
    path: &mut Vec<PathItem>,
    hits: &mut Vec<ScrapeHit>,
) {
    let pushed = if let NodeData::Element(el) = &node.data {
        path.push(PathItem::from_element(el));
        true
    } else {
        false
    };

    // Test matches among children of `node` so `path + [child]` is the
    // selector test vector.
    for child in &node.children {
        if let NodeData::Element(child_el) = &child.data {
            let child_item = PathItem::from_element(child_el);
            let mut full: Vec<&PathItem> = path.iter().collect();
            full.push(&child_item);
            if selector.matches(&full) {
                if let Some(hit) = extract_hit(spec, child, path) {
                    hits.push(hit);
                }
            }
        }
        walk(child, spec, selector, path, hits);
    }

    if pushed {
        path.pop();
    }
}

fn walk_emit<F: FnMut(ScrapeHit)>(
    node: &Node,
    spec: &ScrapeSpec,
    selector: &Selector,
    path: &mut Vec<PathItem>,
    emit: &mut F,
) {
    let pushed = if let NodeData::Element(el) = &node.data {
        path.push(PathItem::from_element(el));
        true
    } else {
        false
    };

    for child in &node.children {
        if let NodeData::Element(child_el) = &child.data {
            let child_item = PathItem::from_element(child_el);
            let mut full: Vec<&PathItem> = path.iter().collect();
            full.push(&child_item);
            if selector.matches(&full) {
                if let Some(hit) = extract_hit(spec, child, path) {
                    emit(hit);
                }
            }
        }
        walk_emit(child, spec, selector, path, emit);
    }

    if pushed {
        path.pop();
    }
}

fn extract_hit(spec: &ScrapeSpec, node: &Node, path: &[PathItem]) -> Option<ScrapeHit> {
    let NodeData::Element(el) = &node.data else {
        return None;
    };
    let value = match spec.extract {
        ExtractKind::Text => ScrapeValue::Text {
            text: node.text_content().trim().to_owned(),
        },
        ExtractKind::Attr => {
            let name = spec.attr.as_deref()?;
            ScrapeValue::Attr {
                name: name.to_owned(),
                value: el.get_attribute(name).map(str::to_owned),
            }
        }
        ExtractKind::Tag => ScrapeValue::Tag {
            tag: el.tag.clone(),
        },
        ExtractKind::Attrs => ScrapeValue::Attrs {
            attrs: el.attributes.clone(),
        },
    };
    Some(ScrapeHit {
        scrape: spec.name.clone(),
        tag: el.tag.clone(),
        path: path.iter().map(|p| p.tag.clone()).collect(),
        value,
    })
}

/// Compile a Lisp document of `(defscrape …)` forms.
#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ScrapeSpec>, String> {
    tatara_lisp::compile_typed::<ScrapeSpec>(src).map_err(|e| format!("{e}"))
}

/// Register the `defscrape` keyword in the global tatara registry.
#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ScrapeSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(html: &str) -> Document {
        Document::parse(html)
    }

    fn spec(name: &str, sel: &str, extract: ExtractKind, attr: Option<&str>) -> ScrapeSpec {
        ScrapeSpec {
            name: name.into(),
            selector: sel.into(),
            extract,
            attr: attr.map(str::to_owned),
            description: None,
        }
    }

    #[test]
    fn extract_text_of_each_match() {
        let doc = parse(r#"<html><body><h2>First</h2><h2>Second</h2></body></html>"#);
        let hits = scrape(&doc, &[spec("titles", "h2", ExtractKind::Text, None)]);
        assert_eq!(hits.len(), 2);
        assert!(matches!(&hits[0].value, ScrapeValue::Text { text } if text == "First"));
        assert!(matches!(&hits[1].value, ScrapeValue::Text { text } if text == "Second"));
    }

    #[test]
    fn extract_attribute() {
        let doc = parse(
            r#"<html><body><a href="https://a">x</a><a href="https://b">y</a></body></html>"#,
        );
        let hits = scrape(&doc, &[spec("links", "a", ExtractKind::Attr, Some("href"))]);
        assert_eq!(hits.len(), 2);
        match &hits[0].value {
            ScrapeValue::Attr { name, value } => {
                assert_eq!(name, "href");
                assert_eq!(value.as_deref(), Some("https://a"));
            }
            other => panic!("expected Attr, got {other:?}"),
        }
    }

    #[test]
    fn extract_tag() {
        let doc = parse(r#"<html><body><p>x</p><span>y</span></body></html>"#);
        let hits = scrape(&doc, &[spec("tags", "*", ExtractKind::Tag, None)]);
        // Universal selector matches every element; we filter out the
        // implicit <html>, <head>, <body> wrappers by counting only
        // matches we know we authored.
        let tags: Vec<_> = hits
            .iter()
            .map(|h| match &h.value {
                ScrapeValue::Tag { tag } => tag.clone(),
                _ => String::new(),
            })
            .collect();
        assert!(tags.contains(&"p".to_string()));
        assert!(tags.contains(&"span".to_string()));
    }

    #[test]
    fn extract_all_attributes() {
        let doc = parse(r#"<html><body><img src="x.png" alt="hello"></body></html>"#);
        let hits = scrape(&doc, &[spec("imgs", "img", ExtractKind::Attrs, None)]);
        assert_eq!(hits.len(), 1);
        match &hits[0].value {
            ScrapeValue::Attrs { attrs } => {
                let as_map: std::collections::HashMap<_, _> = attrs.iter().cloned().collect();
                assert_eq!(as_map.get("src").map(String::as_str), Some("x.png"));
                assert_eq!(as_map.get("alt").map(String::as_str), Some("hello"));
            }
            other => panic!("expected Attrs, got {other:?}"),
        }
    }

    #[test]
    fn scraping_respects_descendant_combinator() {
        // The <a> inside article should be collected; the free-standing <a> should not.
        let doc = parse(
            r#"<html><body><article><a href="in">in</a></article><a href="out">out</a></body></html>"#,
        );
        let hits = scrape(
            &doc,
            &[spec(
                "article-links",
                "article a",
                ExtractKind::Attr,
                Some("href"),
            )],
        );
        assert_eq!(hits.len(), 1);
        match &hits[0].value {
            ScrapeValue::Attr { value, .. } => assert_eq!(value.as_deref(), Some("in")),
            other => panic!("expected Attr, got {other:?}"),
        }
    }

    #[test]
    fn path_is_captured_for_context() {
        let doc = parse(r#"<html><body><article><h2>X</h2></article></body></html>"#);
        let hits = scrape(&doc, &[spec("h2s", "h2", ExtractKind::Text, None)]);
        assert_eq!(hits.len(), 1);
        // Path goes root → … → article (parent of h2).
        assert!(hits[0].path.iter().any(|t| t == "article"));
        assert!(hits[0].path.iter().any(|t| t == "body"));
    }

    #[test]
    fn multiple_specs_compose_in_one_call() {
        let doc =
            parse(r#"<html><body><h2>Title</h2><a href="https://example">Link</a></body></html>"#);
        let hits = scrape(
            &doc,
            &[
                spec("t", "h2", ExtractKind::Text, None),
                spec("l", "a", ExtractKind::Attr, Some("href")),
            ],
        );
        assert_eq!(hits.len(), 2);
        // Outer loop is specs; first spec's hit comes first.
        assert_eq!(hits[0].scrape, "t");
        assert_eq!(hits[1].scrape, "l");
    }

    #[test]
    fn invalid_selector_skips_spec() {
        let doc = parse(r#"<html><body><p>ok</p></body></html>"#);
        let hits = scrape(
            &doc,
            &[
                spec("bad", "", ExtractKind::Text, None),
                spec("good", "p", ExtractKind::Text, None),
            ],
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].scrape, "good");
    }

    #[test]
    fn hit_serializes_as_json() {
        let doc = parse(r#"<html><body><h2>Hi</h2></body></html>"#);
        let hits = scrape(&doc, &[spec("t", "h2", ExtractKind::Text, None)]);
        let json = serde_json::to_string(&hits[0]).unwrap();
        assert!(json.contains(r#""scrape":"t""#));
        assert!(json.contains(r#""tag":"h2""#));
        assert!(json.contains(r#""kind":"text""#));
        assert!(json.contains(r#""text":"Hi""#));
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_round_trip_scrape_specs() {
        let src = r#"
            (defscrape :name "titles" :selector "h2" :extract text)
            (defscrape :name "links" :selector "a" :extract attr :attr "href")
            (defscrape :name "everything" :selector "*" :extract tag)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].extract, ExtractKind::Text);
        assert_eq!(specs[1].extract, ExtractKind::Attr);
        assert_eq!(specs[1].attr.as_deref(), Some("href"));
        assert_eq!(specs[2].extract, ExtractKind::Tag);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_compiled_scrape_extracts_real_values() {
        let src = r#"
            (defscrape :name "titles" :selector "h2" :extract text)
        "#;
        let specs = compile(src).unwrap();
        let doc = parse(r#"<html><body><h2>A</h2><h2>B</h2><h2>C</h2></body></html>"#);
        let hits = scrape(&doc, &specs);
        assert_eq!(hits.len(), 3);
    }
}
