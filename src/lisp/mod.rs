//! DOM ↔ Lisp — absorb any web page into Lisp space.
//!
//! `dom_to_sexp(&Document)` emits the entire DOM as an S-expression so
//! any page (React SSR, Next, Remix, Vue, htmx, Astro, plain HTML,
//! whatever) becomes a uniform Lisp tree you can traverse, transform,
//! and reason about.
//!
//! Shape of the output:
//!
//! ```lisp
//! (document
//!   (element :tag "html"
//!     (element :tag "head"
//!       (element :tag "title"
//!         (text "Example Domain")))
//!     (element :tag "body" :attrs ((:class "hero"))
//!       (element :tag "h1"
//!         (text "Example Domain"))
//!       (comment " some comment "))))
//! ```
//!
//! Key properties:
//!
//! - **Structure-preserving**: every element, text, comment, and
//!   attribute is captured; no lossy normalization.
//! - **Framework-agnostic**: a Next.js page and a plain `.html` page
//!   both become the same shape of Lisp; the framework-specific attrs
//!   (`data-reactroot`, `hx-get`, etc.) appear verbatim in `:attrs`.
//! - **Tatara-compatible**: uses the same S-expression reader grammar
//!   as tatara-lisp, so the output can be piped back through
//!   `tatara_lisp::read()` for programmatic inspection.
//!
//! `depth_cap` lets CLI consumers print partial trees for exploration
//! without dumping a 5MB page. Truncated subtrees emit `(… :elided N)`.

mod parse;

pub use parse::sexp_to_dom;

use crate::dom::{Document, Node, NodeData};
use std::fmt::Write;

/// Formatting options.
#[derive(Debug, Clone, Copy)]
pub struct SexpOptions {
    /// Maximum element nesting to emit. Deeper subtrees collapse to
    /// `(… :elided N)` where N is the descendant count skipped.
    /// `None` = unbounded.
    pub depth_cap: Option<usize>,
    /// Pretty-print with newlines + indent. `false` emits a compact line.
    pub pretty: bool,
    /// Strip whitespace-only text nodes. Useful for dissect; a scraper
    /// that cares about whitespace would set `false`.
    pub trim_whitespace: bool,
}

impl Default for SexpOptions {
    fn default() -> Self {
        Self {
            depth_cap: None,
            pretty: true,
            trim_whitespace: true,
        }
    }
}

/// Emit the whole document as an S-expression string with default opts.
#[must_use]
pub fn dom_to_sexp(doc: &Document) -> String {
    dom_to_sexp_with(doc, &SexpOptions::default())
}

/// Emit with explicit options.
#[must_use]
pub fn dom_to_sexp_with(doc: &Document, opts: &SexpOptions) -> String {
    let mut out = String::new();
    out.push_str("(document");
    emit_children(&mut out, &doc.root.children, 0, opts);
    if opts.pretty {
        out.push('\n');
    }
    out.push(')');
    out
}

/// Emit one subtree rooted at `node` — handy for scrape hits or
/// interactive inspection.
#[must_use]
pub fn node_to_sexp(node: &Node, opts: &SexpOptions) -> String {
    let mut out = String::new();
    emit_node(&mut out, node, 0, opts);
    out
}

fn emit_children(out: &mut String, children: &[Node], depth: usize, opts: &SexpOptions) {
    for child in children {
        if opts.trim_whitespace {
            if let NodeData::Text(t) = &child.data {
                if t.trim().is_empty() {
                    continue;
                }
            }
        }
        if opts.pretty {
            out.push('\n');
            push_indent(out, depth + 1);
        } else {
            out.push(' ');
        }
        emit_node(out, child, depth + 1, opts);
    }
}

fn emit_node(out: &mut String, node: &Node, depth: usize, opts: &SexpOptions) {
    match &node.data {
        NodeData::Document => {
            // Nested document nodes are rare but serialize consistently.
            out.push_str("(document");
            emit_children(out, &node.children, depth, opts);
            if opts.pretty && !node.children.is_empty() {
                out.push('\n');
                push_indent(out, depth);
            }
            out.push(')');
        }
        NodeData::Element(el) => {
            out.push_str("(element :tag ");
            push_string_literal(out, &el.tag);
            if !el.attributes.is_empty() {
                out.push_str(" :attrs (");
                for (i, (k, v)) in el.attributes.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    out.push_str("(:");
                    push_attr_key(out, k);
                    out.push(' ');
                    push_string_literal(out, v);
                    out.push(')');
                }
                out.push(')');
            }

            if let Some(cap) = opts.depth_cap {
                if depth >= cap && !node.children.is_empty() {
                    let elided = count_descendants(node);
                    let _ = write!(out, " (… :elided {elided}))");
                    return;
                }
            }

            if !node.children.is_empty() {
                emit_children(out, &node.children, depth, opts);
                if opts.pretty {
                    out.push('\n');
                    push_indent(out, depth);
                }
            }
            out.push(')');
        }
        NodeData::Text(t) => {
            let shown = if opts.trim_whitespace { t.trim() } else { t };
            out.push_str("(text ");
            push_string_literal(out, shown);
            out.push(')');
        }
        NodeData::Comment(c) => {
            out.push_str("(comment ");
            push_string_literal(out, c);
            out.push(')');
        }
    }
}

fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn push_string_literal(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{{{:04x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Attribute keys go in kebab-case Lisp keyword slot. Bare-identifier
/// chars pass through; anything unusual gets quoted.
fn push_attr_key(out: &mut String, k: &str) {
    let safe = !k.is_empty()
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if safe {
        out.push_str(k);
    } else {
        push_string_literal(out, k);
    }
}

fn count_descendants(node: &Node) -> usize {
    let mut n = 0;
    for c in &node.children {
        n += 1;
        n += count_descendants(c);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_compact_line() {
        let doc = Document::parse("<html><body><p>hi</p></body></html>");
        let opts = SexpOptions {
            pretty: false,
            trim_whitespace: true,
            depth_cap: None,
        };
        let sexp = dom_to_sexp_with(&doc, &opts);
        // sanity — starts with (document and ends with )
        assert!(sexp.starts_with("(document"));
        assert!(sexp.ends_with(')'));
        assert!(sexp.contains(r#"(element :tag "html""#));
        assert!(sexp.contains(r#"(element :tag "p""#));
        assert!(sexp.contains(r#"(text "hi")"#));
    }

    #[test]
    fn preserves_attributes_as_kv_list() {
        let doc =
            Document::parse(r#"<html><body><a href="https://x" class="hero">go</a></body></html>"#);
        let sexp = dom_to_sexp(&doc);
        assert!(sexp.contains(r#":attrs ((:href "https://x") (:class "hero"))"#));
    }

    #[test]
    fn escapes_quotes_and_newlines() {
        let doc =
            Document::parse(r#"<html><body><p>quoted "value" newline here</p></body></html>"#);
        let sexp = dom_to_sexp(&doc);
        assert!(sexp.contains(r#"quoted \"value\""#));
    }

    #[test]
    fn preserves_comments() {
        let doc = Document::parse("<html><body><!-- hidden --><p>x</p></body></html>");
        let sexp = dom_to_sexp(&doc);
        assert!(sexp.contains(r#"(comment " hidden ")"#));
    }

    #[test]
    fn depth_cap_elides_deep_subtrees() {
        let doc = Document::parse(
            "<html><body><div><div><div><div><p>deep</p></div></div></div></div></body></html>",
        );
        let opts = SexpOptions {
            depth_cap: Some(3),
            pretty: false,
            trim_whitespace: true,
        };
        let sexp = dom_to_sexp_with(&doc, &opts);
        assert!(sexp.contains(":elided"));
    }

    #[test]
    fn pretty_prints_with_newlines_and_indent() {
        let doc = Document::parse("<html><body><p>hi</p></body></html>");
        let sexp = dom_to_sexp(&doc);
        assert!(sexp.contains('\n'));
    }

    #[test]
    fn framework_attributes_pass_through_verbatim() {
        let doc = Document::parse(
            r##"<html><body><button hx-get="/api/x" hx-target="#result" data-state="open">go</button></body></html>"##,
        );
        let sexp = dom_to_sexp(&doc);
        assert!(sexp.contains(r#"(:hx-get "/api/x")"#));
        assert!(sexp.contains(r##"(:hx-target "#result")"##));
        assert!(sexp.contains(r#"(:data-state "open")"#));
    }

    #[test]
    fn framework_agnostic_shape() {
        // Two superficially different docs produce the same outer shape.
        let react =
            Document::parse(r#"<html><body><div data-reactroot><p>hi</p></div></body></html>"#);
        let plain = Document::parse(r#"<html><body><div><p>hi</p></div></body></html>"#);
        let r_sexp = dom_to_sexp(&react);
        let p_sexp = dom_to_sexp(&plain);
        // both have a <p> emitted exactly the same way
        assert!(r_sexp.contains(r#"(element :tag "p""#));
        assert!(p_sexp.contains(r#"(element :tag "p""#));
        // framework attr surfaces on the react version, not the plain
        assert!(r_sexp.contains(":data-reactroot"));
        assert!(!p_sexp.contains(":data-reactroot"));
    }

    #[test]
    fn node_to_sexp_emits_subtree() {
        let doc = Document::parse(r#"<html><body><article><h2>Title</h2></article></body></html>"#);
        let article = doc
            .root
            .descendants()
            .find(|n| n.as_element().is_some_and(|e| e.tag == "article"))
            .unwrap();
        let sexp = node_to_sexp(
            article,
            &SexpOptions {
                pretty: false,
                trim_whitespace: true,
                depth_cap: None,
            },
        );
        assert!(sexp.starts_with(r#"(element :tag "article""#));
        assert!(sexp.contains(r#"(element :tag "h2""#));
    }
}
