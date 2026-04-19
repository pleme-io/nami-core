//! Runtime state cells — Layer 2 (state half) of the React-in-Lisp arc.
//!
//! A [`StateSpec`] is a Lisp-declared, named, initially-typed value
//! that persists across evaluations. Multiple specs compose into a
//! [`StateStore`], which lives in the runtime (the browser tab,
//! typically).
//!
//! ```lisp
//! (defstate :name "counter"      :initial 0)
//! (defstate :name "dark-mode"    :initial false)
//! (defstate :name "user-name"    :initial "anonymous")
//! (defstate :name "last-seen"    :initial nil)
//! ```
//!
//! [`bind_into`] exposes every cell as a bound symbol in a
//! [`NamiEvaluator`]'s environment AND adds a `(set-state NAME VALUE)`
//! host function that mutates the store. So agent-authored effects
//! can read + write state in the same evaluator pass as template
//! expansion.
//!
//! Store access is thread-safe: the cells live behind a
//! `RwLock` inside an `Arc`, so snapshots, reads, and writes can
//! interleave freely across agents / effects / template renders.
//!
//! Pure Lisp on top, Rust for the lock semantics.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// A declarative state cell.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defstate"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StateSpec {
    pub name: String,
    /// Initial value — any JSON literal works.
    pub initial: JsonValue,
    #[serde(default)]
    pub description: Option<String>,
    /// When true, a runtime that persists state (localStorage-like)
    /// should restore this cell across sessions. Not wired in V1 —
    /// schema reserved.
    #[serde(default)]
    pub persistent: bool,
}

/// The runtime state store.
///
/// Cells are name → JSON value. Cloning the handle is `Arc::clone`
/// (cheap); writes go through the `RwLock`. One store can be shared
/// across many agents, effects, and template renders.
#[derive(Debug, Clone, Default)]
pub struct StateStore {
    cells: Arc<RwLock<BTreeMap<String, JsonValue>>>,
}

impl StateStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from declared specs. Duplicates resolve last-wins.
    #[must_use]
    pub fn from_specs(specs: &[StateSpec]) -> Self {
        let store = Self::new();
        for s in specs {
            store.set(&s.name, s.initial.clone());
        }
        store
    }

    pub fn set(&self, name: &str, value: JsonValue) {
        let mut cells = self.cells.write().expect("state lock poisoned");
        cells.insert(name.to_owned(), value);
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<JsonValue> {
        let cells = self.cells.read().expect("state lock poisoned");
        cells.get(name).cloned()
    }

    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.cells
            .read()
            .expect("state lock poisoned")
            .contains_key(name)
    }

    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.cells
            .read()
            .expect("state lock poisoned")
            .keys()
            .cloned()
            .collect()
    }

    /// Full snapshot of every cell — useful for persistence, logging,
    /// attestation, or rebuilding the evaluator env after an effect.
    #[must_use]
    pub fn snapshot(&self) -> BTreeMap<String, JsonValue> {
        self.cells.read().expect("state lock poisoned").clone()
    }

    pub fn replace_all(&self, cells: BTreeMap<String, JsonValue>) {
        let mut slot = self.cells.write().expect("state lock poisoned");
        *slot = cells;
    }
}

/// Compile a Lisp document of `(defstate …)` forms.
#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<StateSpec>, String> {
    tatara_lisp::compile_typed::<StateSpec>(src).map_err(|e| format!("{e}"))
}

/// Register the `defstate` keyword.
#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<StateSpec>();
}

// ── eval-feature bridge ─────────────────────────────────────────

/// Bind every state cell as a symbol in the evaluator's env AND
/// register a `(set-state NAME VALUE)` host function that mutates
/// the store. Call before evaluating effects / templates that
/// should have state access.
#[cfg(feature = "eval")]
pub fn bind_into(evaluator: &crate::eval::NamiEvaluator, store: &StateStore) {
    use std::sync::Arc;
    use tatara_eval::{Arity, Builtin, Value};

    let interpreter = evaluator.interpreter();

    // Read pass: snapshot every cell and bind as a symbol.
    for (name, value) in store.snapshot() {
        interpreter.define(name, crate::eval::json_to_value(&value));
    }

    // Write path: expose a `(set-state NAME VALUE)` builtin.
    let store_handle = store.clone();
    let set_state = Builtin {
        name: "set-state".into(),
        arity: Arity::Exact(2),
        func: Arc::new(move |args: &[Value]| -> tatara_eval::Result<Value> {
            let name = match &args[0] {
                Value::Str(s) => s.clone(),
                Value::Symbol(s) => s.clone(),
                other => {
                    return Err(tatara_eval::EvalError::Type {
                        expected: "string or symbol".into(),
                        found: format!("{other:?}"),
                    });
                }
            };
            let json = crate::eval::value_to_json(&args[1]);
            store_handle.set(&name, json);
            Ok(args[1].clone())
        }),
    };
    interpreter.define("set-state", Value::Builtin(Arc::new(set_state)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn store_defaults_and_get_set() {
        let store = StateStore::new();
        store.set("counter", json!(0));
        store.set("dark", json!(true));
        assert_eq!(store.get("counter"), Some(json!(0)));
        assert_eq!(store.get("dark"), Some(json!(true)));
        assert!(store.has("counter"));
        assert!(!store.has("missing"));
    }

    #[test]
    fn store_from_specs_seeds_initial_values() {
        let specs = vec![
            StateSpec {
                name: "count".into(),
                initial: json!(42),
                description: None,
                persistent: false,
            },
            StateSpec {
                name: "dark".into(),
                initial: json!(false),
                description: None,
                persistent: true,
            },
        ];
        let store = StateStore::from_specs(&specs);
        assert_eq!(store.get("count"), Some(json!(42)));
        assert_eq!(store.get("dark"), Some(json!(false)));
    }

    #[test]
    fn store_snapshot_is_a_copy() {
        let store = StateStore::new();
        store.set("a", json!(1));
        let snap = store.snapshot();
        store.set("a", json!(999));
        assert_eq!(snap.get("a"), Some(&json!(1)));
        assert_eq!(store.get("a"), Some(json!(999)));
    }

    #[test]
    fn store_clone_shares_cells() {
        let a = StateStore::new();
        let b = a.clone();
        a.set("x", json!(10));
        assert_eq!(b.get("x"), Some(json!(10)));
    }

    #[cfg(feature = "eval")]
    #[test]
    fn bind_into_exposes_symbols() {
        use crate::eval::NamiEvaluator;
        let store = StateStore::new();
        store.set("counter", json!(7));
        let e = NamiEvaluator::new();
        bind_into(&e, &store);
        let n = e.eval_int("(+ counter 3)", &json!({})).unwrap();
        assert_eq!(n, 10);
    }

    #[cfg(feature = "eval")]
    #[test]
    fn set_state_builtin_writes_through() {
        use crate::eval::NamiEvaluator;
        let store = StateStore::new();
        store.set("counter", json!(0));
        let e = NamiEvaluator::new();
        bind_into(&e, &store);
        // Increment the counter via set-state.
        e.eval_int("(set-state \"counter\" (+ counter 1))", &json!({}))
            .unwrap();
        assert_eq!(store.get("counter"), Some(json!(1)));
    }

    #[cfg(feature = "eval")]
    #[test]
    fn set_state_can_write_strings() {
        use crate::eval::NamiEvaluator;
        let store = StateStore::new();
        store.set("name", json!("anonymous"));
        let e = NamiEvaluator::new();
        bind_into(&e, &store);
        e.eval_string(
            r#"(set-state "name" (string-append "hello " name))"#,
            &json!({}),
        )
        .unwrap();
        assert_eq!(store.get("name"), Some(json!("hello anonymous")));
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_round_trip_state_specs() {
        let src = r#"
            (defstate :name "counter"   :initial 0   :description "click count")
            (defstate :name "dark-mode" :initial #f  :persistent #t)
            (defstate :name "user-name" :initial "anonymous")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].initial, json!(0));
        assert_eq!(specs[1].initial, json!(false));
        assert!(specs[1].persistent);
        assert_eq!(specs[2].initial, json!("anonymous"));
    }
}
