//! Programmable DOM transforms authored as tatara-lisp forms.
//!
//! A [`DomTransformSpec`] pairs a selector with an action: remove the
//! match, add/remove a class, set/remove an attribute, set the text
//! content, or unwrap the element (replace it with its children).
//! Selectors go through [`crate::selector::Selector`] and support
//! compound (`"div.card"`), descendant (`"article p"`), and child
//! (`"ul > li"`) combinators.
//!
//! Transforms are authored in Lisp:
//!
//! ```lisp
//! (defdom-transform :name "hide-ads"
//!                   :selector ".ad"
//!                   :action remove)
//!
//! (defdom-transform :name "reader-p-width"
//!                   :selector "article p"
//!                   :action set-attr
//!                   :arg "style=max-width: 65ch")
//!
//! (defdom-transform :name "strip-iframes-in-ads"
//!                   :selector ".ad > iframe"
//!                   :action remove)
//! ```
//!
//! Applied in order: first transform runs over the full tree, then the
//! next, etc.
//!
//! The Lisp surface is opt-in behind the `lisp` feature flag; the
//! Rust-level types and engine are always available.

use crate::dom::{Document, ElementData, Node, NodeData};
use crate::selector::{Selector, SelectorNode};
use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// A declarative DOM transform.
///
/// Authored as:
///
/// ```lisp
/// (defdom-transform :name "hide-ads" :selector ".ad" :action remove)
/// ```
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defdom-transform"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DomTransformSpec {
    pub name: String,
    pub selector: String,
    pub action: DomAction,
    /// Action-specific payload (e.g. class name for `add-class`,
    /// `name=value` for `set-attr`, the new text for `set-text`).
    #[serde(default)]
    pub arg: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// What the transform does to matching elements.
///
/// Bare symbols in Lisp: `:action remove`, `:action add-class`, etc.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DomAction {
    Remove,
    Unwrap,
    AddClass,
    RemoveClass,
    SetAttr,
    RemoveAttr,
    SetText,
}

/// Outcome of applying a batch of transforms.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransformReport {
    pub applied: Vec<TransformHit>,
}

/// One application of one transform to one element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransformHit {
    pub transform: String,
    pub action: DomAction,
    pub tag: String,
}

/// Lightweight owned snapshot of an element's selector-relevant attrs.
///
/// Used as ancestor-path elements during traversal because we can't
/// simultaneously hold `&ElementData` and `&mut` the tree. Cheap-ish
/// (three small allocations) and built once per node visited.
#[derive(Clone)]
struct PathItem {
    tag: String,
    class_attr: String,
    id: Option<String>,
}

impl PathItem {
    fn from_element(el: &ElementData) -> Self {
        Self {
            tag: el.tag.clone(),
            class_attr: el
                .get_attribute("class")
                .map(str::to_owned)
                .unwrap_or_default(),
            id: el.get_attribute("id").map(str::to_owned),
        }
    }
}

impl SelectorNode for PathItem {
    fn tag(&self) -> &str {
        &self.tag
    }
    fn has_class(&self, class: &str) -> bool {
        self.class_attr.split_whitespace().any(|c| c == class)
    }
    fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }
}

/// A compiled spec (parsed selector) ready to apply.
struct Compiled<'a> {
    spec: &'a DomTransformSpec,
    selector: Selector,
}

/// Apply a sequence of transforms to a document, in order.
pub fn apply(doc: &mut Document, transforms: &[DomTransformSpec]) -> TransformReport {
    let mut report = TransformReport::default();
    for spec in transforms {
        let selector = match Selector::parse(&spec.selector) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    "transform {} has invalid selector {:?}: {e}",
                    spec.name,
                    spec.selector
                );
                continue;
            }
        };
        let compiled = Compiled { spec, selector };
        let mut path: Vec<PathItem> = Vec::new();
        apply_one(&mut doc.root, &compiled, &mut path, &mut report);
    }
    report
}

/// Walk `node` (and descendants), applying the compiled transform.
///
/// `path` is the chain of ancestor elements from root to (but NOT
/// including) `node`. When we enter an element node, we push it; when
/// we leave, we pop. Structural actions that edit `node.children`
/// read `path + [node-as-element]` to test matches.
fn apply_one(
    node: &mut Node,
    compiled: &Compiled<'_>,
    path: &mut Vec<PathItem>,
    report: &mut TransformReport,
) {
    // Push ourselves onto the path if we're an element.
    let pushed = if let NodeData::Element(el) = &node.data {
        path.push(PathItem::from_element(el));
        true
    } else {
        false
    };

    // Depth-first recurse into children (with us on the path).
    for child in &mut node.children {
        apply_one(child, compiled, path, report);
    }

    let spec = compiled.spec;

    // Now apply structural / in-place actions to our children, testing
    // the selector against (path + [child]) — since `node` IS the parent
    // context for any child we match.
    match spec.action {
        DomAction::Remove => {
            let matched_tags = drain_matches_where(&mut node.children, |child| {
                child_matches(child, path, compiled)
            });
            for tag in matched_tags {
                report.applied.push(TransformHit {
                    transform: spec.name.clone(),
                    action: spec.action,
                    tag,
                });
            }
        }
        DomAction::Unwrap => {
            let mut new_children: Vec<Node> = Vec::with_capacity(node.children.len());
            for child in std::mem::take(&mut node.children) {
                if child_matches(&child, path, compiled) {
                    let (child_tag, child_children) = match child.data {
                        NodeData::Element(e) => (e.tag, child.children),
                        _ => (String::new(), child.children),
                    };
                    report.applied.push(TransformHit {
                        transform: spec.name.clone(),
                        action: spec.action,
                        tag: child_tag,
                    });
                    new_children.extend(child_children);
                } else {
                    new_children.push(child);
                }
            }
            node.children = new_children;
        }
        DomAction::AddClass
        | DomAction::RemoveClass
        | DomAction::SetAttr
        | DomAction::RemoveAttr
        | DomAction::SetText => {
            for child in &mut node.children {
                if child_matches(child, path, compiled) {
                    if let Some(hit) = apply_in_place(child, spec) {
                        report.applied.push(hit);
                    }
                }
            }
        }
    }

    if pushed {
        path.pop();
    }
}

/// Does `child` (as the leaf) satisfy the selector, given the ancestor
/// path? Ancestors go root→parent; the child element itself is appended
/// as the last path item for matching.
fn child_matches(child: &Node, path: &[PathItem], compiled: &Compiled<'_>) -> bool {
    let NodeData::Element(child_el) = &child.data else {
        return false;
    };
    let child_item = PathItem::from_element(child_el);
    let mut full: Vec<&PathItem> = path.iter().collect();
    full.push(&child_item);
    compiled.selector.matches(&full)
}

/// Drain children matching `pred`, collecting their tag names for
/// reporting. Preserves order of surviving children.
fn drain_matches_where(
    children: &mut Vec<Node>,
    mut pred: impl FnMut(&Node) -> bool,
) -> Vec<String> {
    let mut removed = Vec::new();
    let mut i = 0;
    while i < children.len() {
        if pred(&children[i]) {
            let node = children.remove(i);
            let tag = match node.data {
                NodeData::Element(e) => e.tag,
                _ => String::new(),
            };
            removed.push(tag);
        } else {
            i += 1;
        }
    }
    removed
}

fn apply_in_place(node: &mut Node, spec: &DomTransformSpec) -> Option<TransformHit> {
    let arg = spec.arg.as_deref();
    let tag_for_hit = node.as_element().map(|e| e.tag.clone()).unwrap_or_default();

    match spec.action {
        DomAction::AddClass => {
            let class = arg?;
            if let NodeData::Element(el) = &mut node.data {
                add_class(el, class);
            }
        }
        DomAction::RemoveClass => {
            let class = arg?;
            if let NodeData::Element(el) = &mut node.data {
                remove_class(el, class);
            }
        }
        DomAction::SetAttr => {
            let (name, value) = arg?.split_once('=')?;
            if let NodeData::Element(el) = &mut node.data {
                set_attr(el, name, value);
            }
        }
        DomAction::RemoveAttr => {
            let name = arg?;
            if let NodeData::Element(el) = &mut node.data {
                el.attributes.retain(|(k, _)| k != name);
            }
        }
        DomAction::SetText => {
            let text = arg?;
            node.children = vec![Node::text(text)];
        }
        _ => return None,
    }

    Some(TransformHit {
        transform: spec.name.clone(),
        action: spec.action,
        tag: tag_for_hit,
    })
}

fn add_class(el: &mut ElementData, class: &str) {
    if el.has_class(class) {
        return;
    }
    let existing = el
        .get_attribute("class")
        .map(str::to_owned)
        .unwrap_or_default();
    let new_val = if existing.is_empty() {
        class.to_owned()
    } else {
        format!("{existing} {class}")
    };
    set_attr(el, "class", &new_val);
}

fn remove_class(el: &mut ElementData, class: &str) {
    let Some(current) = el.get_attribute("class") else {
        return;
    };
    let filtered: Vec<&str> = current.split_whitespace().filter(|c| *c != class).collect();
    if filtered.is_empty() {
        el.attributes.retain(|(k, _)| k != "class");
    } else {
        let new_val = filtered.join(" ");
        set_attr(el, "class", &new_val);
    }
}

fn set_attr(el: &mut ElementData, name: &str, value: &str) {
    if let Some((_, v)) = el.attributes.iter_mut().find(|(k, _)| k == name) {
        *v = value.to_owned();
    } else {
        el.attributes.push((name.to_owned(), value.to_owned()));
    }
}

/// Compile a Lisp document of `(defdom-transform …)` forms into typed specs.
#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<DomTransformSpec>, String> {
    tatara_lisp::compile_typed::<DomTransformSpec>(src).map_err(|e| format!("{e}"))
}

/// Register the `defdom-transform` keyword in the global tatara registry.
#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<DomTransformSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(html: &str) -> Document {
        Document::parse(html)
    }

    fn count_elements(doc: &Document, tag: &str) -> usize {
        doc.root
            .descendants()
            .filter(|n| n.as_element().is_some_and(|e| e.tag == tag))
            .count()
    }

    fn spec(name: &str, sel: &str, act: DomAction, arg: Option<&str>) -> DomTransformSpec {
        DomTransformSpec {
            name: name.into(),
            selector: sel.into(),
            action: act,
            arg: arg.map(str::to_owned),
            description: None,
        }
    }

    #[test]
    fn remove_action_drops_matching_elements() {
        let mut doc = parse(
            r#"<html><body><div class="ad">a</div><p>ok</p><div class="ad">b</div></body></html>"#,
        );
        let report = apply(
            &mut doc,
            &[spec("hide-ads", ".ad", DomAction::Remove, None)],
        );
        assert_eq!(count_elements(&doc, "div"), 0);
        assert_eq!(count_elements(&doc, "p"), 1);
        assert_eq!(report.applied.len(), 2);
    }

    #[test]
    fn compound_selector_is_tag_and_class() {
        let mut doc = parse(
            r#"<html><body><div class="card">keep</div><p class="card">strip</p></body></html>"#,
        );
        apply(&mut doc, &[spec("x", "p.card", DomAction::Remove, None)]);
        assert_eq!(count_elements(&doc, "p"), 0);
        assert_eq!(count_elements(&doc, "div"), 1);
    }

    #[test]
    fn descendant_combinator_only_matches_in_context() {
        // <article><p>A</p></article><p>B</p>
        // "article p" should only hit A, not the free-standing B.
        let mut doc = parse(r#"<html><body><article><p>A</p></article><p>B</p></body></html>"#);
        let report = apply(&mut doc, &[spec("a", "article p", DomAction::Remove, None)]);
        assert_eq!(report.applied.len(), 1);
        assert_eq!(count_elements(&doc, "p"), 1);
    }

    #[test]
    fn child_combinator_only_matches_direct_children() {
        // <ul><li>A</li><div><li>B</li></div></ul>
        // "ul > li" matches A (direct child) but not B (grandchild via div).
        let mut doc =
            parse(r#"<html><body><ul><li>A</li><div><li>B</li></div></ul></body></html>"#);
        let report = apply(&mut doc, &[spec("c", "ul > li", DomAction::Remove, None)]);
        assert_eq!(report.applied.len(), 1);
        assert_eq!(count_elements(&doc, "li"), 1);
    }

    #[test]
    fn add_class_is_idempotent() {
        let mut doc = parse(r#"<html><body><img src="x"></body></html>"#);
        let s = spec("f", "img", DomAction::AddClass, Some("needs-alt"));
        apply(&mut doc, &[s.clone()]);
        apply(&mut doc, &[s]);
        let img = doc
            .root
            .descendants()
            .find_map(|n| n.as_element().filter(|e| e.tag == "img"))
            .unwrap();
        assert_eq!(img.get_attribute("class"), Some("needs-alt"));
    }

    #[test]
    fn set_attr_updates_existing_and_creates_missing() {
        let mut doc = parse(r#"<html><body><a href="old">x</a></body></html>"#);
        apply(
            &mut doc,
            &[spec(
                "rw",
                "a",
                DomAction::SetAttr,
                Some("href=https://new"),
            )],
        );
        let a = doc
            .root
            .descendants()
            .find_map(|n| n.as_element().filter(|e| e.tag == "a"))
            .unwrap();
        assert_eq!(a.get_attribute("href"), Some("https://new"));
    }

    #[test]
    fn unwrap_replaces_with_children() {
        let mut doc = parse(r#"<html><body><div class="wrap"><p>inner</p></div></body></html>"#);
        apply(
            &mut doc,
            &[spec("unwrap", ".wrap", DomAction::Unwrap, None)],
        );
        assert_eq!(count_elements(&doc, "div"), 0);
        assert_eq!(count_elements(&doc, "p"), 1);
    }

    #[test]
    fn set_text_replaces_content() {
        let mut doc = parse(r#"<html><body><h1>old</h1></body></html>"#);
        apply(
            &mut doc,
            &[spec("t", "h1", DomAction::SetText, Some("new"))],
        );
        let h1 = doc
            .root
            .descendants()
            .find(|n| n.as_element().is_some_and(|e| e.tag == "h1"))
            .unwrap();
        assert_eq!(h1.text_content(), "new");
    }

    #[test]
    fn universal_selector_matches_any_element() {
        let mut doc = parse(r#"<html><body><div></div><span></span></body></html>"#);
        apply(
            &mut doc,
            &[spec("tag", "*", DomAction::AddClass, Some("seen"))],
        );
        // Every element — html, head, body, div, span — gets the class.
        let seen: Vec<_> = doc
            .root
            .descendants()
            .filter_map(|n| n.as_element())
            .filter(|e| e.has_class("seen"))
            .collect();
        assert!(seen.len() >= 4); // at least body, div, span, html
    }

    #[test]
    fn invalid_selector_warns_and_skips_rest_of_spec() {
        let mut doc = parse(r#"<html><body><p>ok</p></body></html>"#);
        let report = apply(
            &mut doc,
            &[
                spec("bad", "", DomAction::Remove, None),
                spec("good", "p", DomAction::Remove, None),
            ],
        );
        assert_eq!(report.applied.len(), 1);
        assert_eq!(count_elements(&doc, "p"), 0);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_round_trip_parses_transforms() {
        let src = r#"
            (defdom-transform :name "hide-ads" :selector ".ad" :action remove)
            (defdom-transform :name "flag-alt" :selector "img" :action add-class :arg "missing-alt")
            (defdom-transform :name "iframe-in-ad" :selector ".ad > iframe" :action remove)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[2].selector, ".ad > iframe");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_compiled_transforms_mutate_dom_with_combinators() {
        let src = r#"
            (defdom-transform :name "ad-iframes" :selector ".ad > iframe" :action remove)
        "#;
        let specs = compile(src).unwrap();
        let mut doc = parse(
            r#"<html><body><div class="ad"><iframe>1</iframe></div><iframe>2</iframe></body></html>"#,
        );
        apply(&mut doc, &specs);
        // Only the iframe INSIDE the .ad is removed.
        assert_eq!(count_elements(&doc, "iframe"), 1);
        // The .ad div still exists, now empty.
        assert_eq!(count_elements(&doc, "div"), 1);
    }
}
