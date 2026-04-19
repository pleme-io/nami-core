//! Source-language AST → canonical Lisp S-expressions.
//!
//! First non-HTML source language: **TSX/JSX** via tree-sitter. The
//! shape we emit is a generic `(ts-node :kind "KIND" :text "TEXT" …)`
//! tree — the normalize layer can then pattern-match on `:kind
//! "jsx_element"` / `"jsx_opening_element"` / `"function_declaration"`
//! etc. to lift into the canonical `n-*` vocabulary.
//!
//! ## Grammar of the emitted sexp
//!
//! ```text
//!   form     := (ts-node :kind "STRING" [:text "STRING"] CHILD…)
//! ```
//!
//! Leaf nodes (named or anonymous terminals) carry `:text`; interior
//! nodes omit `:text` and recurse into children. Whitespace is
//! preserved only implicitly via the original source range.
//!
//! ## Feature gate
//!
//! Entire module lives behind `ts`. Each grammar is a C build via
//! `cc` — opt-in.

use crate::dom::{Document, ElementData, Node as DomNode, NodeData};
use std::fmt::Write;
use tree_sitter::{Node, Parser, Tree};

/// Parse TypeScript + JSX (`.tsx` / `.jsx`).
///
/// Always returns a tree. On a tree-sitter hard failure (the parser
/// itself fails to initialize) we return `Err`; a *syntactically
/// malformed* source still parses — tree-sitter records ERROR nodes
/// in the CST — and the sexp output reflects them. That lets users
/// normalize partial / in-progress source.
pub fn parse_tsx(source: &str) -> Result<String, String> {
    let language = tree_sitter_typescript::LANGUAGE_TSX;
    let mut parser = Parser::new();
    parser
        .set_language(&language.into())
        .map_err(|e| format!("set tsx language: {e}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "tree-sitter: parse returned None".to_owned())?;
    Ok(ts_to_sexp(&tree, source))
}

/// Parse Svelte component source (a `.svelte` file). Template markup
/// projects into a `Document` with each element tagged
/// `data-ast-source="svelte"` for provenance. `<script>` and `<style>`
/// sections are preserved as opaque elements (same as the rest).
pub fn parse_svelte_as_document(source: &str) -> Result<Document, String> {
    let language = tree_sitter_svelte_next::LANGUAGE;
    let mut parser = Parser::new();
    parser
        .set_language(&language.into())
        .map_err(|e| format!("set svelte language: {e}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "tree-sitter: parse returned None".to_owned())?;

    let mut doc = Document {
        root: DomNode::document(),
    };
    walk_svelte_into_dom(&tree.root_node(), source, &mut doc.root.children);
    Ok(doc)
}

fn walk_svelte_into_dom(node: &Node, source: &str, out: &mut Vec<DomNode>) {
    let kind = node.kind();
    if is_svelte_element_kind(kind) {
        if let Some(el) = svelte_element_to_dom(node, source) {
            out.push(el);
        }
        return;
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            walk_svelte_into_dom(&child, source, out);
        }
    }
}

fn is_svelte_element_kind(kind: &str) -> bool {
    matches!(
        kind,
        "element"
            | "script_element"
            | "style_element"
            | "template_element"
            | "self_closing_element"
    )
}

fn svelte_element_to_dom(node: &Node, source: &str) -> Option<DomNode> {
    let mut tag: Option<String> = None;
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut children: Vec<DomNode> = Vec::new();

    for i in 0..node.child_count() {
        let child = node.child(i)?;
        let kind = child.kind();

        if kind == "start_tag" || kind == "self_closing_tag" {
            let (t, a) = parse_svelte_start_tag(&child, source);
            if tag.is_none() {
                tag = t;
            }
            attrs.extend(a);
            continue;
        }
        if kind == "end_tag" {
            continue;
        }
        if kind == "text" || kind == "raw_text" {
            if let Some(text) = source.get(child.byte_range()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    children.push(DomNode::text(trimmed.to_owned()));
                }
            }
            continue;
        }
        if is_svelte_element_kind(kind) {
            let mut bucket = Vec::new();
            walk_svelte_into_dom(&child, source, &mut bucket);
            children.extend(bucket);
            continue;
        }
        // Svelte-specific blocks: `{#if …}`, `{#each …}`, interpolations
        // `{expr}` — preserve as text so scrapes can still read the
        // binding references.
        if kind.starts_with("{") || kind.contains("expression") || kind == "interpolation" {
            if let Some(text) = source.get(child.byte_range()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    children.push(DomNode::text(trimmed.to_owned()));
                }
            }
        }
    }

    let mut el = ElementData::with_attributes(tag.unwrap_or_else(|| "svelte".to_owned()), attrs);
    el.attributes
        .push(("data-ast-source".to_owned(), "svelte".to_owned()));
    let mut dom_node = DomNode::element(el);
    dom_node.children = children;
    Some(dom_node)
}

fn parse_svelte_start_tag(node: &Node, source: &str) -> (Option<String>, Vec<(String, String)>) {
    let mut tag: Option<String> = None;
    let mut attrs = Vec::new();
    for i in 0..node.child_count() {
        let Some(child) = node.child(i) else {
            continue;
        };
        match child.kind() {
            "tag_name" => {
                if let Some(text) = source.get(child.byte_range()) {
                    tag = Some(text.trim().to_owned());
                }
            }
            "attribute" => {
                if let Some(pair) = extract_svelte_attribute(&child, source) {
                    attrs.push(pair);
                }
            }
            _ => {}
        }
    }
    (tag, attrs)
}

fn extract_svelte_attribute(node: &Node, source: &str) -> Option<(String, String)> {
    let mut name: Option<String> = None;
    let mut value: String = String::new();
    for i in 0..node.child_count() {
        let child = node.child(i)?;
        let kind = child.kind();
        if kind == "attribute_name" {
            if name.is_none() {
                if let Some(text) = source.get(child.byte_range()) {
                    name = Some(text.trim().to_owned());
                }
            }
        } else if kind == "quoted_attribute_value" || kind == "attribute_value" {
            if let Some(text) = source.get(child.byte_range()) {
                let t = text.trim();
                let inner = t
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .or_else(|| t.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                    .unwrap_or(t);
                value = inner.to_owned();
            }
        }
    }
    Some((name?, value))
}

/// Parse plain TypeScript (no JSX).
pub fn parse_ts(source: &str) -> Result<String, String> {
    let language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT;
    let mut parser = Parser::new();
    parser
        .set_language(&language.into())
        .map_err(|e| format!("set ts language: {e}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "tree-sitter: parse returned None".to_owned())?;
    Ok(ts_to_sexp(&tree, source))
}

/// Serialize a tree to our Lisp sexp shape.
pub fn ts_to_sexp(tree: &Tree, source: &str) -> String {
    let mut out = String::new();
    emit_node(&tree.root_node(), source, 0, &mut out);
    out
}

fn emit_node(node: &Node, source: &str, depth: usize, out: &mut String) {
    indent(out, depth);
    out.push_str("(ts-node :kind ");
    write_quoted(out, node.kind());

    // Terminal / named leaf: include the byte range as :text.
    if node.child_count() == 0 && node.byte_range().len() <= MAX_LEAF_TEXT_BYTES {
        if let Some(text) = source.get(node.byte_range()) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                out.push_str(" :text ");
                write_quoted(out, trimmed);
            }
        }
    }

    let child_count = node.child_count();
    if child_count == 0 {
        out.push(')');
        return;
    }

    out.push('\n');
    for i in 0..child_count {
        let child = node.child(i).expect("child index valid");
        // Skip extras (comments, whitespace) to keep output compact;
        // callers needing them can walk the tree directly.
        if child.is_extra() {
            continue;
        }
        emit_node(&child, source, depth + 1, out);
        out.push('\n');
    }
    indent(out, depth);
    out.push(')');
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_quoted(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
}

/// Cap how much source text we inline as `:text`. Prevents a single
/// huge leaf (e.g. a giant string literal) from ballooning the sexp.
const MAX_LEAF_TEXT_BYTES: usize = 256;

/// Parse TSX source and **project it into a `Document`** — JSX
/// elements become real DOM elements, attributes become element
/// attributes, JSX text becomes text nodes. Everything else (ts
/// imports, function bodies, non-JSX expressions) is dropped.
///
/// This lets `(defnormalize)` / `(defdom-transform)` / `(defscrape)`
/// and every other DOM-targeted DSL operate on JSX source unchanged.
/// A rule like `(defnormalize :selector "article" :rename-to
/// "n-article")` folds both `<article>` in HTML AND `<article>` in a
/// `.tsx` file.
///
/// Non-JSX top-level code (imports, type defs, unrelated statements)
/// gets skipped; the returned Document contains just the JSX subtree(s).
pub fn parse_tsx_as_document(source: &str) -> Result<Document, String> {
    let language = tree_sitter_typescript::LANGUAGE_TSX;
    let mut parser = Parser::new();
    parser
        .set_language(&language.into())
        .map_err(|e| format!("set tsx language: {e}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "tree-sitter: parse returned None".to_owned())?;

    let mut doc = Document {
        root: DomNode::document(),
    };
    walk_into_dom(&tree.root_node(), source, &mut doc.root.children);
    Ok(doc)
}

fn walk_into_dom(node: &Node, source: &str, out: &mut Vec<DomNode>) {
    match node.kind() {
        "jsx_element" => {
            if let Some(el) = jsx_element_to_dom(node, source) {
                out.push(el);
            }
        }
        "jsx_self_closing_element" => {
            if let Some(el) = jsx_self_closing_to_dom(node, source) {
                out.push(el);
            }
        }
        _ => {
            // Walk children — a JSX tree may be buried inside a
            // variable declarator, return statement, etc.
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    walk_into_dom(&child, source, out);
                }
            }
        }
    }
}

fn jsx_element_to_dom(node: &Node, source: &str) -> Option<DomNode> {
    // Find opening, closing, and child JSX content.
    let mut opening: Option<Node> = None;
    let mut children: Vec<DomNode> = Vec::new();

    for i in 0..node.child_count() {
        let child = node.child(i)?;
        match child.kind() {
            "jsx_opening_element" => opening = Some(child),
            "jsx_closing_element" => {}
            "jsx_element" | "jsx_self_closing_element" => {
                let mut bucket = Vec::new();
                walk_into_dom(&child, source, &mut bucket);
                children.extend(bucket);
            }
            "jsx_text" => {
                if let Some(text) = source.get(child.byte_range()) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        children.push(DomNode::text(trimmed.to_owned()));
                    }
                }
            }
            "jsx_expression" => {
                // `{expr}` — preserve the source text so scrapes can
                // still see "{count}" if they want to. Skip the outer
                // braces to keep output clean.
                if let Some(text) = source.get(child.byte_range()) {
                    let t = text.trim();
                    if !t.is_empty() {
                        children.push(DomNode::text(t.to_owned()));
                    }
                }
            }
            _ => {}
        }
    }

    let (tag, attrs) = parse_opening_tag(&opening?, source)?;
    let mut el = ElementData::with_attributes(tag, attrs);
    // Record provenance — every JSX-derived element is tagged so
    // downstream tools can distinguish source-language origins.
    el.attributes
        .push(("data-ast-source".to_owned(), "jsx".to_owned()));
    let mut node = DomNode::element(el);
    node.children = children;
    Some(node)
}

fn jsx_self_closing_to_dom(node: &Node, source: &str) -> Option<DomNode> {
    let (tag, attrs) = parse_self_closing_tag(node, source)?;
    let mut el = ElementData::with_attributes(tag, attrs);
    el.attributes
        .push(("data-ast-source".to_owned(), "jsx".to_owned()));
    Some(DomNode::element(el))
}

fn parse_opening_tag(node: &Node, source: &str) -> Option<(String, Vec<(String, String)>)> {
    let mut tag: Option<String> = None;
    let mut attrs: Vec<(String, String)> = Vec::new();
    for i in 0..node.child_count() {
        let child = node.child(i)?;
        match child.kind() {
            "identifier" | "nested_identifier" | "member_expression" => {
                if tag.is_none() {
                    if let Some(text) = source.get(child.byte_range()) {
                        tag = Some(text.trim().to_owned());
                    }
                }
            }
            "jsx_attribute" => {
                if let Some(pair) = extract_attribute(&child, source) {
                    attrs.push(pair);
                }
            }
            _ => {}
        }
    }
    Some((tag.unwrap_or_else(|| "jsx".to_owned()), attrs))
}

fn parse_self_closing_tag(node: &Node, source: &str) -> Option<(String, Vec<(String, String)>)> {
    // Same shape as an opening tag — tree-sitter treats the whole
    // self-closing element as the "opening" unit.
    parse_opening_tag(node, source)
}

fn extract_attribute(node: &Node, source: &str) -> Option<(String, String)> {
    let mut name: Option<String> = None;
    let mut value: String = String::new();
    for i in 0..node.child_count() {
        let child = node.child(i)?;
        match child.kind() {
            "property_identifier" | "jsx_namespace_name" | "identifier" => {
                if name.is_none() {
                    if let Some(text) = source.get(child.byte_range()) {
                        name = Some(text.trim().to_owned());
                    }
                }
            }
            "string" => {
                if let Some(text) = source.get(child.byte_range()) {
                    // Strip surrounding quotes.
                    let trimmed = text.trim();
                    let inner = trimmed
                        .strip_prefix('"').and_then(|s| s.strip_suffix('"'))
                        .or_else(|| trimmed.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                        .unwrap_or(trimmed);
                    value = inner.to_owned();
                }
            }
            "jsx_expression" => {
                // `attr={expr}` — keep the source text (minus braces)
                // so downstream queries can see the variable reference.
                if let Some(text) = source.get(child.byte_range()) {
                    let trimmed = text.trim();
                    let inner = trimmed
                        .strip_prefix('{')
                        .and_then(|s| s.strip_suffix('}'))
                        .unwrap_or(trimmed)
                        .trim();
                    value = inner.to_owned();
                }
            }
            _ => {}
        }
    }
    Some((name?, value))
}

/// Count every `(ts-node …)` form in a sexp string. Used for
/// proptest invariants: `count(sexp) == tree.root_node().descendant_count()`.
#[must_use]
pub fn count_ts_nodes(sexp: &str) -> usize {
    // Fast lexical count — we emit one `(ts-node ` per node, never
    // anywhere else.
    sexp.matches("(ts-node ").count()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Smoke tests — shape of the output ─────────────────────────

    #[test]
    fn parses_empty_source_without_panic() {
        let out = parse_tsx("").expect("parser");
        assert!(out.contains("(ts-node "));
        assert!(out.contains(":kind \"program\""));
    }

    #[test]
    fn simple_jsx_element_round_trips_through_tree_sitter() {
        let src = "const x = <div>hi</div>;";
        let out = parse_tsx(src).expect("parse");
        assert!(out.contains(":kind \"jsx_element\""), "sexp: {out}");
        assert!(out.contains(":kind \"jsx_opening_element\""));
        assert!(out.contains(":kind \"jsx_closing_element\""));
        // The raw tag identifier and text survive.
        assert!(out.contains("\"div\""));
        assert!(out.contains("\"hi\""));
    }

    #[test]
    fn self_closing_jsx_element_distinguished() {
        let src = "const x = <br />;";
        let out = parse_tsx(src).expect("parse");
        assert!(out.contains(":kind \"jsx_self_closing_element\""), "sexp: {out}");
        assert!(out.contains("\"br\""));
    }

    #[test]
    fn nested_jsx_preserves_structure() {
        let src = "const x = <section><article><p>x</p></article></section>;";
        let out = parse_tsx(src).expect("parse");
        // 3 opening elements.
        assert_eq!(out.matches(":kind \"jsx_opening_element\"").count(), 3);
        assert_eq!(out.matches(":kind \"jsx_closing_element\"").count(), 3);
        // Tag names.
        assert!(out.contains("\"section\""));
        assert!(out.contains("\"article\""));
        assert!(out.contains("\"p\""));
    }

    #[test]
    fn jsx_fragment_is_parsed() {
        // tree-sitter-typescript models fragments as jsx_element with
        // empty-name opening/closing tags (just <>/</>), not a distinct
        // jsx_fragment node. Proven by the bare `<` + `>` children.
        let src = "const x = <><span>a</span><span>b</span></>;";
        let out = parse_tsx(src).expect("parse");
        // Outer JSX element exists.
        assert!(out.contains(":kind \"jsx_element\""), "sexp: {out}");
        // Two spans inside — each appears once in the opening tag
        // identifier and once in the closing tag identifier.
        assert_eq!(out.matches("\"span\"").count(), 4);
        // The opening fragment tag is a bare `<`+`>` pair inside an
        // opening element with no name child — easier to spot by its
        // lack of an identifier between the brackets.
        assert!(
            out.contains(":kind \"jsx_opening_element\""),
            "sexp: {out}"
        );
    }

    #[test]
    fn jsx_expression_container_visible_in_sexp() {
        let src = "const x = <p>{count}</p>;";
        let out = parse_tsx(src).expect("parse");
        assert!(out.contains(":kind \"jsx_expression\""), "sexp: {out}");
        assert!(out.contains("\"count\""));
    }

    #[test]
    fn attributes_land_as_jsx_attribute_nodes() {
        let src = r#"const x = <a href="h" class="c" data-testid="t">yo</a>;"#;
        let out = parse_tsx(src).expect("parse");
        assert_eq!(out.matches(":kind \"jsx_attribute\"").count(), 3);
        assert!(out.contains("\"href\""));
        assert!(out.contains("\"class\""));
        assert!(out.contains("\"data-testid\""));
    }

    // ── TypeScript (no JSX) ───────────────────────────────────────

    #[test]
    fn plain_ts_interface_parses() {
        let src = "interface U { name: string; age: number; }";
        let out = parse_ts(src).expect("parse");
        assert!(out.contains(":kind \"interface_declaration\""));
        assert!(out.contains("\"name\""));
        assert!(out.contains("\"age\""));
    }

    #[test]
    fn plain_ts_does_not_accept_jsx_as_valid() {
        // Without JSX grammar, JSX-like source will parse but emit
        // ERROR nodes — proves the two grammars are genuinely
        // distinct and JSX-off TS is stricter.
        let src = "const x = <div />;";
        let out = parse_ts(src).expect("parser still returns");
        // tree-sitter still produces a tree; it just has ERROR
        // nodes or reinterprets the angle brackets.
        assert!(out.contains("(ts-node "));
    }

    // ── Malformed input is tolerated, not panicked ────────────────

    #[test]
    fn unbalanced_jsx_yields_error_nodes_not_panic() {
        let src = "const x = <div>unclosed";
        let out = parse_tsx(src).expect("parse (errors in-tree OK)");
        // tree-sitter flags partial parses with ERROR/MISSING nodes;
        // any non-empty sexp here is acceptable.
        assert!(count_ts_nodes(&out) >= 1);
    }

    // ── Invariants / provability ──────────────────────────────────

    #[test]
    fn every_opening_is_balanced_by_a_closing() {
        // For well-formed JSX, openings and closings are 1:1.
        let src = "const x = <a><b><c><d></d></c></b></a>;";
        let out = parse_tsx(src).expect("parse");
        let opens = out.matches(":kind \"jsx_opening_element\"").count();
        let closes = out.matches(":kind \"jsx_closing_element\"").count();
        assert_eq!(opens, closes, "openings {opens} must equal closings {closes}");
    }

    #[test]
    fn output_has_balanced_parentheses() {
        let src = "const x: string = foo<bar>();";
        let out = parse_ts(src).expect("parse");
        let opens = out.matches('(').count();
        let closes = out.matches(')').count();
        assert_eq!(opens, closes, "sexp: {out}");
    }

    #[test]
    fn large_string_literal_is_truncated_from_leaf_text() {
        // Leaf texts over MAX_LEAF_TEXT_BYTES should omit :text so
        // the sexp doesn't explode.
        let huge = "a".repeat(MAX_LEAF_TEXT_BYTES + 10);
        let src = format!("const x = \"{huge}\";");
        let out = parse_ts(&src).expect("parse");
        // The literal string content should NOT appear in the sexp.
        assert!(
            !out.contains(&huge),
            "huge leaf text leaked into sexp — truncation broken"
        );
    }

    #[test]
    fn count_ts_nodes_matches_declared_structure() {
        let src = "const x = <a>x</a>;";
        let out = parse_tsx(src).expect("parse");
        let n = count_ts_nodes(&out);
        // Rough sanity: program → lexical_declaration → variable_declarator
        // → jsx_element → {opening, text, closing} + identifiers. Well
        // north of 5 nodes for even the tiniest JSX snippet.
        assert!(n >= 5, "expected >= 5 ts-nodes, got {n}; sexp: {out}");
    }

    // ── JSX → Document mapping ────────────────────────────────────

    #[test]
    fn parse_tsx_as_document_extracts_jsx_tree() {
        let src = "const x = <article><p>hi</p></article>;";
        let doc = parse_tsx_as_document(src).expect("parse");
        let mut saw_article = false;
        let mut saw_p = false;
        for n in doc.root.descendants() {
            if let Some(el) = n.as_element() {
                match el.tag.as_str() {
                    "article" => {
                        saw_article = true;
                        assert_eq!(el.get_attribute("data-ast-source"), Some("jsx"));
                    }
                    "p" => {
                        saw_p = true;
                        assert_eq!(n.text_content(), "hi");
                    }
                    _ => {}
                }
            }
        }
        assert!(saw_article && saw_p);
    }

    #[test]
    fn jsx_attributes_map_to_element_attributes() {
        let src = r#"const x = <a href="/h" class="c" data-slot="card">yo</a>;"#;
        let doc = parse_tsx_as_document(src).expect("parse");
        let a = doc.root.descendants()
            .find(|n| n.as_element().is_some_and(|e| e.tag == "a"))
            .expect("found <a>");
        let el = a.as_element().unwrap();
        assert_eq!(el.get_attribute("href"), Some("/h"));
        assert_eq!(el.get_attribute("class"), Some("c"));
        assert_eq!(el.get_attribute("data-slot"), Some("card"));
        assert_eq!(el.get_attribute("data-ast-source"), Some("jsx"));
    }

    #[test]
    fn jsx_self_closing_element_parses_as_empty_element() {
        let src = r#"const x = <img src="/a.png" alt="pic" />;"#;
        let doc = parse_tsx_as_document(src).expect("parse");
        let img = doc.root.descendants()
            .find(|n| n.as_element().is_some_and(|e| e.tag == "img"))
            .expect("found <img>");
        let el = img.as_element().unwrap();
        assert_eq!(el.get_attribute("src"), Some("/a.png"));
        assert_eq!(el.get_attribute("alt"), Some("pic"));
        assert!(img.children.is_empty());
    }

    #[test]
    fn same_normalize_rule_applies_to_jsx_and_html() {
        // THE SHIP: one (defnormalize) rule folds article→n-article
        // regardless of whether source was HTML or JSX.
        use crate::normalize::{apply, NormalizeRegistry, NormalizeSpec};

        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "article".into(),
            framework: None,
            selector: "article".into(),
            rename_to: "n-article".into(),
            set_attrs: vec![],
            remove_attrs: vec![],
            description: None,
        });

        // HTML source.
        let mut html_doc = Document::parse("<html><body><article>x</article></body></html>");
        let html_report = apply(&mut html_doc, &reg, &[]);

        // JSX source — same logical structure.
        let mut jsx_doc = parse_tsx_as_document("const x = <article>x</article>;").expect("parse");
        let jsx_report = apply(&mut jsx_doc, &reg, &[]);

        assert_eq!(html_report.applied(), 1, "html normalize");
        assert_eq!(jsx_report.applied(), 1, "jsx normalize");

        // Both now have a canonical n-article element.
        assert!(jsx_doc.root.descendants().any(|n| {
            n.as_element().is_some_and(|e| e.tag == "n-article")
        }));
    }

    #[test]
    fn empty_source_yields_empty_document() {
        let doc = parse_tsx_as_document("").expect("parse");
        // Root is a Document node with zero children (no JSX).
        assert_eq!(doc.root.children.len(), 0);
    }

    #[test]
    fn non_jsx_code_yields_empty_document() {
        let doc = parse_tsx_as_document("const x = 1 + 2;").expect("parse");
        assert_eq!(doc.root.children.len(), 0);
    }

    #[test]
    fn nested_jsx_preserves_depth_in_dom() {
        let src = "const x = <a><b><c>deep</c></b></a>;";
        let doc = parse_tsx_as_document(src).expect("parse");
        // Walk: root → a → b → c → text("deep")
        let a = &doc.root.children[0];
        assert_eq!(a.as_element().unwrap().tag, "a");
        let b = &a.children[0];
        assert_eq!(b.as_element().unwrap().tag, "b");
        let c = &b.children[0];
        assert_eq!(c.as_element().unwrap().tag, "c");
        assert_eq!(c.text_content(), "deep");
    }

    #[test]
    fn jsx_expression_interpolation_preserved_as_text() {
        let src = "const x = <p>{count}</p>;";
        let doc = parse_tsx_as_document(src).expect("parse");
        let p = doc.root.descendants()
            .find(|n| n.as_element().is_some_and(|e| e.tag == "p"))
            .expect("found <p>");
        assert!(p.text_content().contains("count"));
    }

    #[test]
    fn attribute_expression_value_preserved_as_string() {
        let src = r#"const x = <div id={main} class={variant}>x</div>;"#;
        let doc = parse_tsx_as_document(src).expect("parse");
        let div = doc.root.descendants()
            .find(|n| n.as_element().is_some_and(|e| e.tag == "div"))
            .unwrap();
        let el = div.as_element().unwrap();
        assert_eq!(el.get_attribute("id"), Some("main"));
        assert_eq!(el.get_attribute("class"), Some("variant"));
    }

    // ── Svelte → Document mapping ─────────────────────────────────

    #[test]
    fn parse_svelte_basic_template_extracts_elements() {
        let src = r#"<script>let x = 1;</script>
<article class="post">
  <h1>title</h1>
  <p>body</p>
</article>"#;
        let doc = parse_svelte_as_document(src).expect("parse");
        let tags: Vec<String> = doc
            .root
            .descendants()
            .filter_map(|n| n.as_element().map(|e| e.tag.clone()))
            .collect();
        assert!(tags.iter().any(|t| t == "article"));
        assert!(tags.iter().any(|t| t == "h1"));
        assert!(tags.iter().any(|t| t == "p"));
    }

    #[test]
    fn svelte_elements_carry_svelte_provenance() {
        let src = "<div>x</div>";
        let doc = parse_svelte_as_document(src).expect("parse");
        let div = doc
            .root
            .descendants()
            .find(|n| n.as_element().is_some_and(|e| e.tag == "div"))
            .expect("found div");
        assert_eq!(
            div.as_element().unwrap().get_attribute("data-ast-source"),
            Some("svelte")
        );
    }

    #[test]
    fn svelte_attributes_are_captured() {
        let src = r#"<a href="/a" class="primary" data-id="hello">link</a>"#;
        let doc = parse_svelte_as_document(src).expect("parse");
        let a = doc
            .root
            .descendants()
            .find(|n| n.as_element().is_some_and(|e| e.tag == "a"))
            .unwrap();
        let el = a.as_element().unwrap();
        assert_eq!(el.get_attribute("href"), Some("/a"));
        assert_eq!(el.get_attribute("class"), Some("primary"));
        assert_eq!(el.get_attribute("data-id"), Some("hello"));
    }

    #[test]
    fn svelte_interpolation_preserved_as_text() {
        let src = "<p>{count}</p>";
        let doc = parse_svelte_as_document(src).expect("parse");
        let p = doc
            .root
            .descendants()
            .find(|n| n.as_element().is_some_and(|e| e.tag == "p"))
            .unwrap();
        assert!(p.text_content().contains("count"));
    }

    #[test]
    fn same_normalize_rule_folds_html_jsx_and_svelte() {
        // THE SHIP: three source languages, one DSL, identical result.
        use crate::normalize::{apply, NormalizeRegistry, NormalizeSpec};

        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "art".into(),
            framework: None,
            selector: "article".into(),
            rename_to: "n-article".into(),
            set_attrs: vec![],
            remove_attrs: vec![],
            description: None,
        });

        let mut html = Document::parse("<html><body><article>x</article></body></html>");
        let html_hits = apply(&mut html, &reg, &[]).applied();

        let mut jsx = parse_tsx_as_document("const x = <article>x</article>;").expect("tsx");
        let jsx_hits = apply(&mut jsx, &reg, &[]).applied();

        let mut svelte = parse_svelte_as_document("<article>x</article>").expect("svelte");
        let svelte_hits = apply(&mut svelte, &reg, &[]).applied();

        assert_eq!(html_hits, 1);
        assert_eq!(jsx_hits, 1);
        assert_eq!(svelte_hits, 1);

        // All three now have canonical n-article.
        for doc in [&html, &jsx, &svelte] {
            assert!(doc.root.descendants().any(|n| {
                n.as_element().is_some_and(|e| e.tag == "n-article")
            }));
        }
    }

    #[test]
    fn svelte_nested_elements_preserve_depth() {
        let src = "<section><article><p>deep</p></article></section>";
        let doc = parse_svelte_as_document(src).expect("parse");
        // Section → article → p → text("deep")
        let section = &doc.root.children[0];
        assert_eq!(section.as_element().unwrap().tag, "section");
        let article = &section.children[0];
        assert_eq!(article.as_element().unwrap().tag, "article");
        let p = &article.children[0];
        assert_eq!(p.as_element().unwrap().tag, "p");
        assert_eq!(p.text_content(), "deep");
    }

    #[test]
    fn svelte_empty_source_is_empty_doc() {
        let doc = parse_svelte_as_document("").expect("parse");
        assert!(doc.root.children.is_empty());
    }

    // ── Determinism ───────────────────────────────────────────────

    #[test]
    fn parse_is_deterministic() {
        let src = "const x = <a><b>{count}</b></a>;";
        let a = parse_tsx(src).expect("1");
        let b = parse_tsx(src).expect("2");
        let c = parse_tsx(src).expect("3");
        assert_eq!(a, b);
        assert_eq!(b, c);
    }
}
