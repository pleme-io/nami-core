//! Programmable DOM transforms authored as tatara-lisp forms.
//!
//! A [`DomTransformSpec`] pairs a simple CSS selector with an action:
//! remove the match, add/remove a class, set/remove an attribute, set
//! the text content, unwrap the element (replace it with its children),
//! or collect it for external consumption.
//!
//! Transforms are authored in Lisp:
//!
//! ```lisp
//! (defdom-transform :name "hide-ads"
//!                   :selector ".ad"
//!                   :action remove)
//!
//! (defdom-transform :name "reader-mode-width"
//!                   :selector "article"
//!                   :action set-attr
//!                   :arg "style=max-width: 65ch")
//!
//! (defdom-transform :name "flag-images-missing-alt"
//!                   :selector "img"
//!                   :action add-class
//!                   :arg "missing-alt")
//! ```
//!
//! Applied in order: first transform runs over the full tree, then the
//! next, etc. Selectors piggyback on [`SimpleMatcher`] (tag / `.class`
//! / `#id`) for V1; richer combinator support can graduate later.
//!
//! The Lisp surface is opt-in behind the `lisp` feature flag. The
//! Rust-level types and engine are always available so namimado /
//! other consumers can build specs programmatically without the Lisp
//! compile step.

use crate::dom::{Document, ElementData, Node, NodeData};
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
/// Bare symbols in Lisp:
///
/// ```lisp
/// :action remove
/// :action add-class
/// :action set-attr
/// ```
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DomAction {
    /// Delete matching elements from the tree.
    Remove,
    /// Replace matching elements with their children (strip wrapper).
    Unwrap,
    /// Add a class to matching elements. `arg` = class name.
    AddClass,
    /// Remove a class from matching elements. `arg` = class name.
    RemoveClass,
    /// Set an attribute. `arg` = `"name=value"` (first `=` splits).
    SetAttr,
    /// Remove an attribute. `arg` = attribute name.
    RemoveAttr,
    /// Replace the text content. `arg` = new text.
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

/// Apply a sequence of transforms to a document, in order.
pub fn apply(doc: &mut Document, transforms: &[DomTransformSpec]) -> TransformReport {
    let mut report = TransformReport::default();
    for spec in transforms {
        apply_one(&mut doc.root, spec, &mut report);
    }
    report
}

fn apply_one(node: &mut Node, spec: &DomTransformSpec, report: &mut TransformReport) {
    // Depth-first: transform descendants first, so actions that mutate
    // `children` in place (remove / unwrap) apply to the post-transform
    // subtree, not stale structure.
    for child in &mut node.children {
        apply_one(child, spec, report);
    }

    // Structural actions operate on this node's children (so matched
    // elements are transformed relative to their parent).
    match spec.action {
        DomAction::Remove => {
            let before = node.children.len();
            node.children
                .retain(|c| !matches_element(c, &spec.selector));
            let removed = before - node.children.len();
            for _ in 0..removed {
                report.applied.push(TransformHit {
                    transform: spec.name.clone(),
                    action: spec.action,
                    tag: selector_tag(&spec.selector),
                });
            }
        }
        DomAction::Unwrap => {
            let mut new_children: Vec<Node> = Vec::with_capacity(node.children.len());
            for child in std::mem::take(&mut node.children) {
                if matches_element(&child, &spec.selector) {
                    report.applied.push(TransformHit {
                        transform: spec.name.clone(),
                        action: spec.action,
                        tag: child
                            .as_element()
                            .map(|e| e.tag.clone())
                            .unwrap_or_default(),
                    });
                    new_children.extend(child.children);
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
            // In-place actions: apply to matching element children only
            // (not the node itself — root / document / text siblings).
            for child in &mut node.children {
                if matches_element(child, &spec.selector) {
                    if let Some(hit) = apply_in_place(child, spec) {
                        report.applied.push(hit);
                    }
                }
            }
        }
    }
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

/// Simple selector match: `"tag"`, `".class"`, `"#id"` (single atom).
///
/// Kept local to the transform engine so the feature works without
/// pulling `Document::query_selector_all`'s allocator pattern. Tests
/// verify it stays in sync with `tree::SimpleMatcher`'s behaviour.
fn matches_element(node: &Node, selector: &str) -> bool {
    let NodeData::Element(el) = &node.data else {
        return false;
    };
    let sel = selector.trim();
    if let Some(class) = sel.strip_prefix('.') {
        el.has_class(class)
    } else if let Some(id) = sel.strip_prefix('#') {
        el.id() == Some(id)
    } else {
        el.tag.eq_ignore_ascii_case(sel)
    }
}

fn selector_tag(selector: &str) -> String {
    selector.trim_start_matches(['.', '#']).to_owned()
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

    #[test]
    fn remove_action_drops_matching_elements() {
        let mut doc = parse(
            r#"<html><body><div class="ad">a</div><p>ok</p><div class="ad">b</div></body></html>"#,
        );
        let spec = DomTransformSpec {
            name: "hide-ads".into(),
            selector: ".ad".into(),
            action: DomAction::Remove,
            arg: None,
            description: None,
        };
        let report = apply(&mut doc, &[spec]);
        assert_eq!(count_elements(&doc, "div"), 0);
        assert_eq!(count_elements(&doc, "p"), 1);
        assert_eq!(report.applied.len(), 2);
    }

    #[test]
    fn add_class_is_idempotent() {
        let mut doc =
            parse(r#"<html><body><img src="x"><img src="y" class="missing-alt"></body></html>"#);
        let spec = DomTransformSpec {
            name: "flag".into(),
            selector: "img".into(),
            action: DomAction::AddClass,
            arg: Some("missing-alt".into()),
            description: None,
        };
        apply(&mut doc, &[spec.clone()]);
        let imgs: Vec<_> = doc
            .root
            .descendants()
            .filter_map(|n| n.as_element().filter(|e| e.tag == "img"))
            .collect();
        assert_eq!(imgs.len(), 2);
        assert!(imgs.iter().all(|e| e.has_class("missing-alt")));
        // Second application — still only one "missing-alt" per element.
        apply(&mut doc, &[spec]);
        let imgs: Vec<_> = doc
            .root
            .descendants()
            .filter_map(|n| n.as_element().filter(|e| e.tag == "img"))
            .collect();
        for e in imgs {
            let classes = e.get_attribute("class").unwrap_or_default();
            assert_eq!(classes.matches("missing-alt").count(), 1);
        }
    }

    #[test]
    fn set_attr_updates_existing_and_creates_missing() {
        let mut doc = parse(r#"<html><body><a href="old">x</a></body></html>"#);
        apply(
            &mut doc,
            &[DomTransformSpec {
                name: "rewrite".into(),
                selector: "a".into(),
                action: DomAction::SetAttr,
                arg: Some("href=https://new".into()),
                description: None,
            }],
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
            &[DomTransformSpec {
                name: "unwrap".into(),
                selector: ".wrap".into(),
                action: DomAction::Unwrap,
                arg: None,
                description: None,
            }],
        );
        assert_eq!(count_elements(&doc, "div"), 0);
        assert_eq!(count_elements(&doc, "p"), 1);
    }

    #[test]
    fn set_text_replaces_content() {
        let mut doc = parse(r#"<html><body><h1>old</h1></body></html>"#);
        apply(
            &mut doc,
            &[DomTransformSpec {
                name: "title".into(),
                selector: "h1".into(),
                action: DomAction::SetText,
                arg: Some("new".into()),
                description: None,
            }],
        );
        let h1 = doc
            .root
            .descendants()
            .find(|n| n.as_element().is_some_and(|e| e.tag == "h1"))
            .unwrap();
        assert_eq!(h1.text_content(), "new");
    }

    #[test]
    fn transforms_compose_in_order() {
        let mut doc = parse(
            r#"<html><body><div class="ad">x</div><img src="a"><img src="b" class="missing-alt"></body></html>"#,
        );
        apply(
            &mut doc,
            &[
                DomTransformSpec {
                    name: "hide-ads".into(),
                    selector: ".ad".into(),
                    action: DomAction::Remove,
                    arg: None,
                    description: None,
                },
                DomTransformSpec {
                    name: "flag".into(),
                    selector: "img".into(),
                    action: DomAction::AddClass,
                    arg: Some("missing-alt".into()),
                    description: None,
                },
            ],
        );
        assert_eq!(count_elements(&doc, "div"), 0);
        let imgs: Vec<_> = doc
            .root
            .descendants()
            .filter_map(|n| n.as_element().filter(|e| e.tag == "img"))
            .collect();
        assert!(imgs.iter().all(|e| e.has_class("missing-alt")));
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_round_trip_parses_transforms() {
        let src = r#"
            (defdom-transform :name "hide-ads" :selector ".ad" :action remove)
            (defdom-transform :name "flag-alt" :selector "img" :action add-class :arg "missing-alt")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].action, DomAction::Remove);
        assert_eq!(specs[1].action, DomAction::AddClass);
        assert_eq!(specs[1].arg.as_deref(), Some("missing-alt"));
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_compiled_transforms_mutate_dom() {
        let src = r#"
            (defdom-transform :name "hide-ads" :selector ".ad" :action remove)
        "#;
        let specs = compile(src).unwrap();
        let mut doc = parse(r#"<html><body><div class="ad">a</div><p>ok</p></body></html>"#);
        apply(&mut doc, &specs);
        assert_eq!(count_elements(&doc, "div"), 0);
        assert_eq!(count_elements(&doc, "p"), 1);
    }
}
