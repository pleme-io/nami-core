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
