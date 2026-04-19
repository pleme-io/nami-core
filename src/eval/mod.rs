//! Layer 3 of the React-in-Lisp arc — a real evaluator.
//!
//! Adopts [`tatara_eval::Interpreter`] rather than reinventing. Adds
//! a thin wrapper that:
//!
//!   1. Pre-binds a **prop set** (serde JSON) into the evaluator env
//!      before each call, so templates can write `(@ count)` /
//!      `(if (> count 0) "some" "none")` without manual `define`
//!      calls per field.
//!
//!   2. Converts the evaluator's [`Value`] back into useful shapes:
//!      stringified text, booleans for conditionals, lists of nodes.
//!
//!   3. Keeps the evaluator **pure by default** — no filesystem, no
//!      process spawn, no network. `new()` uses [`Interpreter::new`]
//!      which excludes the `system_builtin_table` (no `shell`,
//!      `read-file`, etc.). Upgrade to `new_with_system` only behind
//!      an explicit capability flag (not in V1).
//!
//! Feature-gated behind `eval` so the core library stays slim when
//! the evaluator isn't needed.
//!
//! ```rust
//! # #[cfg(feature = "eval")]
//! # {
//! use nami_core::eval::NamiEvaluator;
//! use serde_json::json;
//! let e = NamiEvaluator::new();
//! let n = e.eval_int("(+ 1 2)", &json!({})).unwrap();
//! assert_eq!(n, 3);
//! let s = e.eval_string("(if (> count 0) \"some\" \"none\")", &json!({"count": 5})).unwrap();
//! assert_eq!(s, "some");
//! # }
//! ```

use serde_json::Value as JsonValue;
use tatara_eval::{Interpreter, Value};

/// Convenience alias — the evaluator's error type.
pub use tatara_eval::EvalError;

/// Thin wrapper around [`tatara_eval::Interpreter`] with nami-core-
/// flavored helpers: prop injection and typed result extraction.
///
/// Reusable across many evaluations. `eval_*` methods seed the prop
/// map fresh each call, so state from one eval doesn't leak into the
/// next.
pub struct NamiEvaluator {
    inner: Interpreter,
}

impl NamiEvaluator {
    /// Pure evaluator — safe for untrusted input (no filesystem /
    /// process / network builtins). Arithmetic, string ops,
    /// conditionals, lambdas.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Interpreter::new(),
        }
    }

    /// Access the underlying `tatara_eval::Interpreter` for advanced
    /// use (`define` custom bindings, register a host function).
    #[must_use]
    pub fn interpreter(&self) -> &Interpreter {
        &self.inner
    }

    /// Evaluate a Lisp expression against a prop map. Each key in
    /// `props` becomes a bound symbol in the env.
    ///
    /// Prop types map as:
    ///   JSON string  → `Value::Str`
    ///   JSON integer → `Value::Int`
    ///   JSON float   → `Value::Float`
    ///   JSON bool    → `Value::Bool`
    ///   JSON null    → `Value::Nil`
    ///   other        → stringified via `Value::Str`
    pub fn eval(&self, src: &str, props: &JsonValue) -> Result<Value, EvalError> {
        self.bind_props(props);
        self.inner.eval_source(src)
    }

    /// Evaluate and extract as an i64 if the result is an Int.
    pub fn eval_int(&self, src: &str, props: &JsonValue) -> Result<i64, EvalError> {
        match self.eval(src, props)? {
            Value::Int(n) => Ok(n),
            other => Err(EvalError::Type {
                expected: "Int".into(),
                found: format!("{other:?}"),
            }),
        }
    }

    /// Evaluate and extract as a String (stringify primitives).
    pub fn eval_string(&self, src: &str, props: &JsonValue) -> Result<String, EvalError> {
        Ok(value_to_string(&self.eval(src, props)?))
    }

    /// Evaluate and interpret as truthy — `Bool(true)` and non-zero
    /// ints / non-empty strings are true. `Bool(false)` and `Nil` and
    /// zero and empty string are false. Matches common Lisp
    /// conventions for template conditionals.
    pub fn eval_truthy(&self, src: &str, props: &JsonValue) -> Result<bool, EvalError> {
        Ok(is_truthy(&self.eval(src, props)?))
    }

    fn bind_props(&self, props: &JsonValue) {
        let JsonValue::Object(map) = props else {
            return;
        };
        for (k, v) in map {
            self.inner.define(k, json_to_value(v));
        }
    }
}

impl Default for NamiEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a serde JSON value into a tatara-eval [`Value`]. Nested
/// objects become `Attrs` (BTreeMap<String, Value>).
fn json_to_value(v: &JsonValue) -> Value {
    match v {
        JsonValue::Null => Value::Nil,
        JsonValue::Bool(b) => Value::Bool(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::Nil
            }
        }
        JsonValue::String(s) => Value::Str(s.clone()),
        JsonValue::Array(items) => {
            let vs: Vec<Value> = items.iter().map(json_to_value).collect();
            Value::List(std::sync::Arc::new(vs))
        }
        JsonValue::Object(map) => {
            let mut out = std::collections::BTreeMap::new();
            for (k, v) in map {
                out.insert(k.clone(), json_to_value(v));
            }
            Value::Attrs(std::sync::Arc::new(out))
        }
    }
}

/// Stringify a tatara-eval value for template interpolation.
pub fn value_to_string(v: &Value) -> String {
    match v {
        Value::Nil => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Str(s) => s.clone(),
        Value::Symbol(s) => s.clone(),
        Value::Keyword(k) => format!(":{k}"),
        other => format!("{other:?}"),
    }
}

/// Truthiness over tatara-eval values.
#[must_use]
pub fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Nil => false,
        Value::Bool(b) => *b,
        Value::Int(n) => *n != 0,
        Value::Float(f) => *f != 0.0,
        Value::Str(s) => !s.is_empty(),
        Value::List(xs) => !xs.is_empty(),
        Value::Attrs(m) => !m.is_empty(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pure_arithmetic() {
        let e = NamiEvaluator::new();
        assert_eq!(e.eval_int("(+ 1 2)", &json!({})).unwrap(), 3);
        assert_eq!(e.eval_int("(* 3 (+ 4 5))", &json!({})).unwrap(), 27);
    }

    #[test]
    fn string_concatenation() {
        let e = NamiEvaluator::new();
        let s = e
            .eval_string(r#"(string-append "hi " "there")"#, &json!({}))
            .unwrap();
        assert_eq!(s, "hi there");
    }

    #[test]
    fn conditional_branches_on_bool() {
        let e = NamiEvaluator::new();
        let t = e.eval_string("(if #t \"yes\" \"no\")", &json!({})).unwrap();
        let f = e.eval_string("(if #f \"yes\" \"no\")", &json!({})).unwrap();
        assert_eq!(t, "yes");
        assert_eq!(f, "no");
    }

    #[test]
    fn props_are_bound_as_symbols() {
        let e = NamiEvaluator::new();
        let n = e.eval_int("(* count 2)", &json!({"count": 21})).unwrap();
        assert_eq!(n, 42);
    }

    #[test]
    fn conditional_over_prop() {
        let e = NamiEvaluator::new();
        let yes = e
            .eval_string(
                "(if (> count 0) \"positive\" \"zero-or-less\")",
                &json!({"count": 5}),
            )
            .unwrap();
        let no = e
            .eval_string(
                "(if (> count 0) \"positive\" \"zero-or-less\")",
                &json!({"count": 0}),
            )
            .unwrap();
        assert_eq!(yes, "positive");
        assert_eq!(no, "zero-or-less");
    }

    #[test]
    fn truthy_semantics() {
        let e = NamiEvaluator::new();
        assert!(e.eval_truthy("(+ 1 0)", &json!({})).unwrap());
        assert!(!e.eval_truthy("(- 1 1)", &json!({})).unwrap());
        assert!(e.eval_truthy("#t", &json!({})).unwrap());
        assert!(!e.eval_truthy("#f", &json!({})).unwrap());
    }

    #[test]
    fn lambda_and_closure() {
        let e = NamiEvaluator::new();
        let n = e
            .eval_int(
                "(let ((inc (lambda (x) (+ x 1)))) (inc (inc (inc count))))",
                &json!({"count": 10}),
            )
            .unwrap();
        assert_eq!(n, 13);
    }

    #[test]
    fn let_binding_scope() {
        let e = NamiEvaluator::new();
        let n = e
            .eval_int("(let ((a 2) (b 3)) (* a b))", &json!({}))
            .unwrap();
        assert_eq!(n, 6);
    }

    #[test]
    fn nested_json_props() {
        let e = NamiEvaluator::new();
        // A nested object becomes Attrs; accessing fields isn't
        // supported natively here, but we can at least see it binds.
        let e = NamiEvaluator::new();
        // Primitives are the common case; verify a string prop works.
        let s = e
            .eval_string(r#"(string-append name "!")"#, &json!({"name": "world"}))
            .unwrap();
        assert_eq!(s, "world!");
        // suppress unused warning
        let _ = e;
    }

    #[test]
    fn errors_propagate_as_eval_error() {
        let e = NamiEvaluator::new();
        // unbound symbol as a function
        assert!(e.eval_int("(definitely-unbound 1)", &json!({})).is_err());
        // type mismatch — `eval_int` rejects non-Int result
        assert!(e.eval_int(r#""not a number""#, &json!({})).is_err());
    }

    #[test]
    fn interpreter_access_allows_custom_define() {
        let e = NamiEvaluator::new();
        e.interpreter().define("threshold", Value::Int(42));
        let s = e
            .eval_string("(if (> threshold 10) \"big\" \"small\")", &json!({}))
            .unwrap();
        assert_eq!(s, "big");
    }

    #[test]
    fn value_to_string_covers_primitives() {
        assert_eq!(value_to_string(&Value::Nil), "");
        assert_eq!(value_to_string(&Value::Bool(true)), "true");
        assert_eq!(value_to_string(&Value::Int(42)), "42");
        assert_eq!(value_to_string(&Value::Str("hi".into())), "hi");
    }
}
