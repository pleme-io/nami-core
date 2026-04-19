//! Inline Lisp macros — pages embed `<l-eval>` or
//! `<script type="application/tatara-lisp">` elements whose text is
//! tatara-lisp source. At navigate time, [`expand`] evaluates each
//! block with a shared [`NamiEvaluator`] and splices the result back
//! into the DOM in-place.
//!
//! ## Grammar
//!
//! ```html
//! <l-eval>(string-append "hello " "world")</l-eval>
//!
//! <l-eval>(elem "h2" (string-append "visit #" "3"))</l-eval>
//!
//! <script type="application/tatara-lisp">
//!   (elem "em" "attention")
//! </script>
//! ```
//!
//! ## Host builtins
//!
//! A small DOM-emit surface is pre-registered on every evaluator used
//! by this module:
//!
//! | builtin         | shape                      | emits |
//! | --------------- | -------------------------- | ----- |
//! | `text-node`     | `(text-node STR)`          | `(text "STR")` |
//! | `elem`          | `(elem TAG TEXT)`          | `(element :tag "TAG" (text "TEXT"))` |
//!
//! These are enough to cover ~90% of inline macros. For richer trees,
//! author a `(defcomponent …)` in your rc file and call
//! `(render-component "Name" props)` — the V2 builtin.
//!
//! ## Output
//!
//! The evaluator returns a string. That string is interpreted as:
//!
//! 1. If it parses as `(element …)` / `(text …)` / `(document …)` →
//!    parse via `sexp_to_dom` and splice the resulting children.
//! 2. Otherwise it's treated as plain text and becomes a text node.
//!
//! Evaluation errors log a warning and leave the source element empty
//! — the tolerant "log and skip" pattern used across the substrate.
//!
//! ## Ordering
//!
//! Runs *before* framework-alias / transform / component passes so
//! emitted subtrees participate in every downstream transformation.
//! Children are expanded depth-first so nested `<l-eval>` macros
//! compose bottom-up.

use crate::dom::{Document, Node, NodeData};
use crate::eval::NamiEvaluator;
use serde_json::json;
use std::sync::Arc;
use tatara_eval::{Arity, Builtin, Value};

/// Summary of one expansion pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExpandReport {
    /// Macros evaluated successfully.
    pub evaluated: usize,
    /// Macros whose body failed to eval (logged, then spliced empty).
    pub failed: usize,
}

/// Expand every inline Lisp macro in `doc` in place. Returns a report
/// of how many fired + failed.
///
/// Installs the DOM-emit builtins (`elem`, `text-node`) on the
/// evaluator if not already present. Safe to call repeatedly on the
/// same evaluator.
pub fn expand(doc: &mut Document, eval: &NamiEvaluator) -> ExpandReport {
    install_host_builtins(eval);
    let mut report = ExpandReport::default();
    expand_children(&mut doc.root.children, eval, &mut report);
    report
}

fn install_host_builtins(eval: &NamiEvaluator) {
    let interp = eval.interpreter();

    // (text-node STR) → "(text \"STR\")"
    interp.define(
        "text-node",
        Value::Builtin(Arc::new(Builtin {
            name: "text-node".into(),
            arity: Arity::Exact(1),
            func: Arc::new(|args: &[Value]| {
                let s = value_to_string(&args[0]);
                Ok(Value::Str(format!("(text {})", escape_sexp_string(&s))))
            }),
        })),
    );

    // (elem TAG TEXT) → "(element :tag \"TAG\" (text \"TEXT\"))"
    interp.define(
        "elem",
        Value::Builtin(Arc::new(Builtin {
            name: "elem".into(),
            arity: Arity::Exact(2),
            func: Arc::new(|args: &[Value]| {
                let tag = value_to_string(&args[0]);
                let inner = value_to_string(&args[1]);
                Ok(Value::Str(format!(
                    "(element :tag {} (text {}))",
                    escape_sexp_string(&tag),
                    escape_sexp_string(&inner)
                )))
            }),
        })),
    );
}

fn value_to_string(v: &Value) -> String {
    crate::eval::value_to_string(v)
}

fn escape_sexp_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn expand_children(children: &mut Vec<Node>, eval: &NamiEvaluator, report: &mut ExpandReport) {
    let mut i = 0;
    while i < children.len() {
        // Expand nested macros bottom-up so an outer macro sees fully
        // resolved inner ones (important if an outer macro later reads
        // its own text body — today it doesn't, but behaviour should
        // be correct for that composition).
        expand_children(&mut children[i].children, eval, report);

        if is_macro_element(&children[i]) {
            let src = gather_source(&children[i]);
            let replacement = evaluate_macro(&src, eval, report);
            let replacement_len = replacement.len();
            children.splice(i..=i, replacement);
            i += replacement_len;
        } else {
            i += 1;
        }
    }
}

fn is_macro_element(node: &Node) -> bool {
    let Some(el) = node.as_element() else {
        return false;
    };
    let tag = el.tag.to_ascii_lowercase();
    if tag == "l-eval" {
        return true;
    }
    if tag == "script" {
        if let Some(ty) = el.get_attribute("type") {
            let ty = ty.trim().to_ascii_lowercase();
            return matches!(
                ty.as_str(),
                "application/tatara-lisp" | "text/tatara-lisp" | "text/lisp" | "application/lisp"
            );
        }
    }
    false
}

fn gather_source(node: &Node) -> String {
    let mut out = String::new();
    collect_text(node, &mut out);
    out
}

fn collect_text(node: &Node, buf: &mut String) {
    if let NodeData::Text(t) = &node.data {
        buf.push_str(t);
    }
    for c in &node.children {
        collect_text(c, buf);
    }
}

fn evaluate_macro(src: &str, eval: &NamiEvaluator, report: &mut ExpandReport) -> Vec<Node> {
    if src.trim().is_empty() {
        report.evaluated += 1;
        return Vec::new();
    }

    let result = match eval.eval_string(src, &json!({})) {
        Ok(s) => {
            report.evaluated += 1;
            s
        }
        Err(e) => {
            tracing::warn!("inline-lisp macro eval failed: {e}");
            report.failed += 1;
            return Vec::new();
        }
    };

    parse_output(&result)
}

/// Interpret the evaluator's stringified output as either:
///   1. an `(element …)` / `(text …)` / `(document …)` S-expression →
///      parse with sexp_to_dom and return the resulting children;
///   2. plain text → a single text node.
fn parse_output(result: &str) -> Vec<Node> {
    let trimmed = result.trim_start();
    if trimmed.starts_with("(element ")
        || trimmed.starts_with("(element(")
        || trimmed.starts_with("(text ")
        || trimmed.starts_with("(document")
    {
        let wrapped = if trimmed.starts_with("(document") {
            result.to_owned()
        } else {
            format!("(document {result})")
        };
        match crate::lisp::sexp_to_dom(&wrapped) {
            Ok(doc) => doc.root.children,
            Err(e) => {
                tracing::warn!("inline-lisp output wasn't parseable sexp: {e} — falling back to text");
                vec![Node::text(result.to_owned())]
            }
        }
    } else {
        vec![Node::text(result.to_owned())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::Document;

    fn eval() -> NamiEvaluator {
        NamiEvaluator::new()
    }

    #[test]
    fn plain_text_output_replaces_l_eval_element() {
        let html = r#"<html><body><p>before <l-eval>(+ 1 2)</l-eval> after</p></body></html>"#;
        let mut doc = Document::parse(html);
        let report = expand(&mut doc, &eval());
        assert_eq!(report.evaluated, 1);
        assert_eq!(report.failed, 0);
        // The l-eval element is gone; "3" appears as text inside <p>.
        let text = doc.text_content();
        assert!(text.contains("before"));
        assert!(text.contains("3"));
        assert!(text.contains("after"));
        assert!(!text.contains("l-eval"));
    }

    #[test]
    fn elem_builtin_produces_real_dom() {
        // `(elem "span" "yo")` expands to the sexp form sexp_to_dom
        // understands. The <l-eval> is replaced with a real <span>.
        let html = r#"<html><body><div><l-eval>(elem "span" "yo")</l-eval></div></body></html>"#;
        let mut doc = Document::parse(html);
        let report = expand(&mut doc, &eval());
        assert_eq!(report.evaluated, 1);
        let mut found = false;
        for n in doc.root.descendants() {
            if let Some(el) = n.as_element() {
                if el.tag == "span" {
                    assert_eq!(n.text_content(), "yo");
                    found = true;
                }
            }
        }
        assert!(found, "span not spliced into DOM");
    }

    #[test]
    fn elem_composes_with_string_append() {
        // Real compile-time computation inside the macro.
        let html = r#"<html><body><l-eval>(elem "h2" (string-append "visit #" "3"))</l-eval></body></html>"#;
        let mut doc = Document::parse(html);
        expand(&mut doc, &eval());
        let mut found = false;
        for n in doc.root.descendants() {
            if let Some(el) = n.as_element() {
                if el.tag == "h2" {
                    assert_eq!(n.text_content(), "visit #3");
                    found = true;
                }
            }
        }
        assert!(found);
    }

    #[test]
    fn script_type_tatara_lisp_is_recognized() {
        let html = r#"<html><body><script type="application/tatara-lisp">(string-append "a" "b")</script></body></html>"#;
        let mut doc = Document::parse(html);
        let report = expand(&mut doc, &eval());
        assert_eq!(report.evaluated, 1);
        let text = doc.text_content();
        assert!(text.contains("ab"), "text was: {text}");
    }

    #[test]
    fn malformed_macro_body_logs_and_splices_empty() {
        let html = r#"<html><body><p>keep <l-eval>(unbalanced</l-eval> me</p></body></html>"#;
        let mut doc = Document::parse(html);
        let report = expand(&mut doc, &eval());
        assert_eq!(report.evaluated, 0);
        assert_eq!(report.failed, 1);
        // "keep" + " me" still present; l-eval element removed entirely.
        let text = doc.text_content();
        assert!(text.contains("keep"));
        assert!(text.contains("me"));
        assert!(!text.contains("l-eval"));
    }

    #[test]
    fn empty_macro_is_noop_but_counted() {
        let html = r#"<html><body><l-eval>   </l-eval></body></html>"#;
        let mut doc = Document::parse(html);
        let report = expand(&mut doc, &eval());
        assert_eq!(report.evaluated, 1);
        assert_eq!(report.failed, 0);
        // l-eval element replaced with nothing.
        for n in doc.root.descendants() {
            if let Some(el) = n.as_element() {
                assert_ne!(el.tag, "l-eval");
            }
        }
    }

    #[test]
    fn no_macros_means_no_work() {
        let html = r#"<html><body><p>nothing here</p></body></html>"#;
        let mut doc = Document::parse(html);
        let report = expand(&mut doc, &eval());
        assert_eq!(report.evaluated, 0);
        assert_eq!(report.failed, 0);
    }
}
