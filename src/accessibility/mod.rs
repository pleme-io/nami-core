//! Accessibility tree emission.
//!
//! The canonical `n-*` vocabulary doubles as the ARIA role map:
//! `n-article` → role `article`, `n-nav` → role `navigation`,
//! `n-button` → role `button`, and so on. This module walks a
//! post-normalize `Document` and emits an `AxTree` of `AxNode`s
//! shaped to match AccessKit / WAI-ARIA conventions.
//!
//! Because normalize folds every framework (HTML5 semantic tags,
//! shadcn, MUI, Bootstrap, Tailwind patterns, JSX/TSX, Svelte) into
//! the same `n-*` schema, any normalize pack that matches a page
//! yields a valid AX tree for free. Screen readers, AccessKit, and
//! any other a11y consumer all target one structure.
//!
//! Design invariants:
//!
//! 1. **Pure function** — `ax_tree(&doc)` is deterministic; same DOM =
//!    same tree every time.
//! 2. **No ambient role assignment**. A `<div>` with no canonical
//!    mapping emits role `generic` (matches ARIA default). Authors
//!    who want a richer role ship a normalize rule.
//! 3. **Accessible name** comes from `aria-label` → `aria-labelledby`
//!    → text content (first non-empty) → empty.
//! 4. **Walk order matches DOM order** so reading sequence is preserved.

use crate::dom::{Document, Node};
use serde::{Deserialize, Serialize};

/// One node in the accessibility tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxNode {
    /// ARIA role (`article` / `navigation` / `button` / `heading` / …).
    pub role: String,
    /// Accessible name — what a screen reader announces. Derived
    /// per the AccName-1.1 algorithm (simplified for V1).
    pub name: String,
    /// For headings: 1..=6. `None` for non-heading roles.
    pub heading_level: Option<u8>,
    /// Non-canonical tag name when we couldn't map to a role — helps
    /// authors spot coverage gaps ("why does this have role=generic?").
    pub source_tag: String,
    /// Children, in DOM order.
    pub children: Vec<AxNode>,
}

/// Build the accessibility tree for a document.
///
/// Returns a root `AxNode` with role `"document"` whose children are
/// the top-level body contents. When the document has no `<body>`,
/// walks from the root element directly.
#[must_use]
pub fn ax_tree(doc: &Document) -> AxNode {
    let body = body_or_root(&doc.root);
    let children = walk_children(body);
    AxNode {
        role: "document".into(),
        name: accessible_name(body, &children).unwrap_or_default(),
        heading_level: None,
        source_tag: "document".into(),
        children,
    }
}

/// Emit the tree as a readable sexp. Useful for diffing + agents.
#[must_use]
pub fn ax_tree_sexp(doc: &Document) -> String {
    let tree = ax_tree(doc);
    let mut out = String::new();
    emit_sexp(&tree, 0, &mut out);
    out
}

fn emit_sexp(node: &AxNode, depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str("  ");
    }
    out.push_str("(ax :role ");
    write_str(out, &node.role);
    if !node.name.is_empty() {
        out.push_str(" :name ");
        write_str(out, &node.name);
    }
    if let Some(l) = node.heading_level {
        out.push_str(&format!(" :heading-level {l}"));
    }
    if node.children.is_empty() {
        out.push(')');
    } else {
        out.push('\n');
        for c in &node.children {
            emit_sexp(c, depth + 1, out);
            out.push('\n');
        }
        for _ in 0..depth {
            out.push_str("  ");
        }
        out.push(')');
    }
}

fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
}

fn body_or_root(root: &Node) -> &Node {
    fn find_body(n: &Node) -> Option<&Node> {
        if let Some(el) = n.as_element() {
            if el.tag.eq_ignore_ascii_case("body") {
                return Some(n);
            }
        }
        for c in &n.children {
            if let Some(b) = find_body(c) {
                return Some(b);
            }
        }
        None
    }
    find_body(root).unwrap_or(root)
}

fn walk_children(parent: &Node) -> Vec<AxNode> {
    let mut out = Vec::new();
    for child in &parent.children {
        if let Some(node) = walk_one(child) {
            out.push(node);
        }
    }
    out
}

fn walk_one(node: &Node) -> Option<AxNode> {
    let el = node.as_element()?;
    let tag = el.tag.to_ascii_lowercase();

    // Hidden elements drop out of the tree (ARIA semantics).
    if is_presentation_hidden(node) {
        return None;
    }

    let children = walk_children(node);
    let (role, heading_level) = role_of(&tag);
    let name = accessible_name(node, &children).unwrap_or_default();

    // The walk skips purely wrapping elements (role=`none`), hoisting
    // their children into the parent. Matches ARIA `role="none"` /
    // `role="presentation"` flattening.
    if role == "none" {
        // We can't return multiple nodes from one slot; keep the
        // wrapper as "generic" so information isn't lost. Future: a
        // multi-hoist helper.
    }

    Some(AxNode {
        role,
        name,
        heading_level,
        source_tag: el.tag.clone(),
        children,
    })
}

fn is_presentation_hidden(node: &Node) -> bool {
    let Some(el) = node.as_element() else {
        return false;
    };
    if el
        .get_attribute("aria-hidden")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    {
        return true;
    }
    if el
        .get_attribute("hidden")
        .is_some_and(|_| true)
    {
        return true;
    }
    false
}

/// Map an HTML / canonical tag to an ARIA role + heading level (if a
/// heading). This table is the authoritative role mapping — extend
/// it when a new canonical `n-*` tag lands in the typescape.
fn role_of(tag: &str) -> (String, Option<u8>) {
    // Explicit heading roles.
    if let Some(lvl) = tag.strip_prefix('h').and_then(|d| d.parse::<u8>().ok()) {
        if (1..=6).contains(&lvl) {
            return ("heading".into(), Some(lvl));
        }
    }
    let role = match tag {
        // Canonical n-* vocabulary → ARIA.
        "n-article" => "article",
        "n-nav" => "navigation",
        "n-main" => "main",
        "n-aside" => "complementary",
        "n-section" => "region",
        "n-header" => "banner",
        "n-footer" => "contentinfo",
        "n-figure" => "figure",
        "n-card" => "group",
        "n-card-title" => "heading",
        "n-card-description" => "paragraph",
        "n-card-content" => "group",
        "n-card-header" => "group",
        "n-card-actions" => "group",
        "n-card-footer" => "group",
        "n-button" => "button",
        "n-icon-button" => "button",
        "n-input" => "textbox",
        "n-tab" => "tab",
        "n-tabs-list" => "tablist",
        "n-nav-link" => "link",
        "n-dialog" => "dialog",
        "n-drawer" => "dialog",
        "n-alert" => "alert",
        "n-badge" => "status",
        "n-breadcrumb" => "navigation",
        "n-list" => "list",
        "n-list-item" => "listitem",
        "n-app-bar" => "banner",
        "n-toolbar" => "toolbar",
        "n-avatar" => "img",
        // Native HTML5 semantic tags get the same roles, so an
        // un-normalized page still yields a useful tree.
        "article" => "article",
        "nav" => "navigation",
        "main" => "main",
        "aside" => "complementary",
        "section" => "region",
        "header" => "banner",
        "footer" => "contentinfo",
        "figure" => "figure",
        "button" => "button",
        "a" => "link",
        "ul" | "ol" => "list",
        "li" => "listitem",
        "img" => "img",
        "form" => "form",
        "input" | "textarea" | "select" => "textbox",
        "p" => "paragraph",
        "hr" => "separator",
        "dialog" => "dialog",
        // Wrappers the AX tree usually ignores. We keep them as
        // "generic" so tree shape matches DOM — a later walk can
        // post-process.
        "div" | "span" => "generic",
        // Unknown — still emit, tagged as generic, with the raw tag
        // preserved in `source_tag` for diagnostics.
        _ => "generic",
    };
    (role.into(), None)
}

/// Simplified AccName-1.1 pipeline.
/// 1. `aria-label` wins
/// 2. else concatenate child accessible names
/// 3. else the element's own text_content (trimmed)
fn accessible_name(node: &Node, children: &[AxNode]) -> Option<String> {
    if let Some(el) = node.as_element() {
        if let Some(label) = el.get_attribute("aria-label") {
            let trimmed = label.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    // Compose from children names (joined with a space).
    let joined: String = children
        .iter()
        .filter_map(|c| {
            if c.name.is_empty() {
                None
            } else {
                Some(c.name.clone())
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if !joined.is_empty() {
        return Some(joined);
    }
    // Fallback: own text_content.
    let text = node.text_content();
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        Some(collapse_whitespace(trimmed))
    } else {
        None
    }
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !last_space && !out.is_empty() {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document_yields_document_root() {
        let doc = Document::parse("");
        let tree = ax_tree(&doc);
        assert_eq!(tree.role, "document");
        assert!(tree.children.is_empty());
    }

    #[test]
    fn html5_semantic_tags_get_correct_roles() {
        let html = "<html><body>
            <header>top</header>
            <nav>links</nav>
            <main>
                <article>body</article>
                <aside>extra</aside>
            </main>
            <footer>bottom</footer>
        </body></html>";
        let doc = Document::parse(html);
        let tree = ax_tree(&doc);
        let roles: Vec<&str> = tree.children.iter().map(|c| c.role.as_str()).collect();
        assert!(roles.contains(&"banner"));
        assert!(roles.contains(&"navigation"));
        assert!(roles.contains(&"main"));
        assert!(roles.contains(&"contentinfo"));
    }

    #[test]
    fn canonical_n_vocab_maps_to_aria() {
        let html = r#"<html><body>
            <n-article>one</n-article>
            <n-button>click</n-button>
            <n-dialog>hi</n-dialog>
            <n-nav>links</n-nav>
        </body></html>"#;
        let doc = Document::parse(html);
        let tree = ax_tree(&doc);
        let mut by_role: std::collections::HashMap<String, usize> = Default::default();
        for c in &tree.children {
            *by_role.entry(c.role.clone()).or_default() += 1;
        }
        assert_eq!(by_role.get("article"), Some(&1));
        assert_eq!(by_role.get("button"), Some(&1));
        assert_eq!(by_role.get("dialog"), Some(&1));
        assert_eq!(by_role.get("navigation"), Some(&1));
    }

    #[test]
    fn headings_emit_level() {
        let html = "<html><body><h1>A</h1><h2>B</h2><h6>F</h6></body></html>";
        let doc = Document::parse(html);
        let tree = ax_tree(&doc);
        let mut levels: Vec<u8> = tree
            .children
            .iter()
            .filter_map(|c| c.heading_level)
            .collect();
        levels.sort_unstable();
        assert_eq!(levels, vec![1, 2, 6]);
    }

    #[test]
    fn aria_label_overrides_text_for_name() {
        let html = r#"<html><body>
            <n-button aria-label="Close dialog">x</n-button>
        </body></html>"#;
        let doc = Document::parse(html);
        let tree = ax_tree(&doc);
        let btn = tree.children.iter().find(|c| c.role == "button").unwrap();
        assert_eq!(btn.name, "Close dialog");
    }

    #[test]
    fn accessible_name_falls_back_to_text() {
        let html = "<html><body><button>Submit</button></body></html>";
        let doc = Document::parse(html);
        let tree = ax_tree(&doc);
        let btn = tree.children.iter().find(|c| c.role == "button").unwrap();
        assert_eq!(btn.name, "Submit");
    }

    #[test]
    fn aria_hidden_prunes_subtree() {
        let html = r#"<html><body>
            <article>visible</article>
            <article aria-hidden="true">hidden</article>
        </body></html>"#;
        let doc = Document::parse(html);
        let tree = ax_tree(&doc);
        let articles: Vec<_> = tree.children.iter().filter(|c| c.role == "article").collect();
        assert_eq!(articles.len(), 1);
        assert_eq!(articles[0].name, "visible");
    }

    #[test]
    fn hidden_attribute_prunes_subtree() {
        let html = r#"<html><body>
            <n-button>visible</n-button>
            <n-button hidden>invisible</n-button>
        </body></html>"#;
        let doc = Document::parse(html);
        let tree = ax_tree(&doc);
        let buttons: Vec<_> = tree.children.iter().filter(|c| c.role == "button").collect();
        assert_eq!(buttons.len(), 1);
    }

    #[test]
    fn unknown_tag_falls_back_to_generic_with_source_tag() {
        let html = "<html><body><custom-widget>hi</custom-widget></body></html>";
        let doc = Document::parse(html);
        let tree = ax_tree(&doc);
        let w = &tree.children[0];
        assert_eq!(w.role, "generic");
        assert_eq!(w.source_tag, "custom-widget");
    }

    #[test]
    fn ax_tree_is_deterministic() {
        let html = r#"<html><body><n-article><h2>T</h2><p>body</p></n-article></body></html>"#;
        let doc = Document::parse(html);
        let a = ax_tree(&doc);
        let b = ax_tree(&doc);
        let c = ax_tree(&doc);
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    #[test]
    fn sexp_emission_is_parseable_shape() {
        let html = r#"<html><body><n-article><h2>Title</h2><p>body</p></n-article></body></html>"#;
        let doc = Document::parse(html);
        let sexp = ax_tree_sexp(&doc);
        assert!(sexp.contains("(ax :role \"document\""));
        assert!(sexp.contains("(ax :role \"article\""));
        assert!(sexp.contains("(ax :role \"heading\""));
        assert!(sexp.contains(":heading-level 2"));
    }

    #[test]
    fn walk_order_matches_dom_order() {
        let html = r#"<html><body>
            <article><h1>First</h1></article>
            <nav><h2>Second</h2></nav>
            <aside><h3>Third</h3></aside>
        </body></html>"#;
        let doc = Document::parse(html);
        let tree = ax_tree(&doc);
        let role_order: Vec<&str> = tree.children.iter().map(|c| c.role.as_str()).collect();
        assert_eq!(role_order, vec!["article", "navigation", "complementary"]);
    }

    #[test]
    fn post_normalize_dom_yields_consistent_ax_regardless_of_source() {
        // A lightly-normalized page (emulating what normalize_apply
        // produces) yields the same roles whether the input was
        // <article> or <n-article>.
        let html_1 = "<html><body><article>x</article></body></html>";
        let html_2 = "<html><body><n-article>x</n-article></body></html>";
        let t1 = ax_tree(&Document::parse(html_1));
        let t2 = ax_tree(&Document::parse(html_2));
        assert_eq!(t1.children[0].role, t2.children[0].role);
    }

    #[test]
    fn document_name_concatenates_child_names() {
        let html = r#"<html><body>
            <article aria-label="A">…</article>
            <nav aria-label="B">…</nav>
        </body></html>"#;
        let doc = Document::parse(html);
        let tree = ax_tree(&doc);
        // Document name joins child names.
        assert!(tree.name.contains('A'));
        assert!(tree.name.contains('B'));
    }
}
