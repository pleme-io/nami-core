//! Computed / memoized state — `(defderived …)`.
//!
//! A [`DerivedSpec`] is a named lazy computation over state cells.
//! Think Svelte's `$:` reactive declarations or Solid's `createMemo`,
//! authored as Lisp:
//!
//! ```lisp
//! (defstate :name "price" :initial 10)
//! (defstate :name "quantity" :initial 3)
//!
//! (defderived :name "subtotal"
//!             :inputs ("price" "quantity")
//!             :compute "(* price quantity)")
//!
//! (defderived :name "tax"
//!             :inputs ("subtotal" "tax-rate")
//!             :compute "(* subtotal tax-rate)")
//!
//! (defderived :name "total"
//!             :inputs ("subtotal" "tax")
//!             :compute "(+ subtotal tax)")
//! ```
//!
//! Derivations compose. Each `:compute` is a Lisp expression that
//! sees its declared inputs as bound symbols; the result is any JSON
//! primitive. Derived values are computed on demand by
//! [`DerivedRegistry::evaluate`] — a caller that wants React-style
//! memoization can wrap the registry with their own cache.
//!
//! Pure Lisp-layer abstraction. Needs the `eval` feature since
//! computations are evaluated via [`NamiEvaluator`].

use crate::store::StateStore;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Declarative computed value.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defderived"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DerivedSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Names of state cells or other derived values this computation
    /// reads. Declarations are informative for tooling + dependency
    /// tracking; the evaluator pre-binds EVERY cell in the store
    /// anyway, so a typo here won't silently drop a read.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Lisp expression that produces the derived value. Sees all
    /// state cells as bound symbols, plus any other derived values
    /// already computed on this registry.
    pub compute: String,
}

/// Index of derived computations by name.
#[derive(Debug, Clone, Default)]
pub struct DerivedRegistry {
    specs: Vec<DerivedSpec>,
}

impl DerivedRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: DerivedSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = DerivedSpec>) {
        for s in specs {
            self.insert(s);
        }
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&DerivedSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.specs.iter().map(|s| s.name.as_str()).collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// Evaluate a single derivation against the current state store.
    #[cfg(feature = "eval")]
    pub fn evaluate(&self, name: &str, store: &StateStore) -> Result<JsonValue, String> {
        let spec = self
            .get(name)
            .ok_or_else(|| format!("unknown derived: {name}"))?;
        evaluate_spec(spec, store, Some(self), &mut Default::default())
    }

    /// Evaluate every derivation in registration order, returning a
    /// fresh JSON map `name → value`. Later derivations can reference
    /// earlier ones (they're bound into the evaluator env as symbols
    /// as soon as they're computed). Errors at any step short-circuit.
    #[cfg(feature = "eval")]
    pub fn evaluate_all(
        &self,
        store: &StateStore,
    ) -> Result<serde_json::Map<String, JsonValue>, String> {
        use std::collections::HashSet;
        let mut out = serde_json::Map::new();
        let mut visiting: HashSet<String> = HashSet::new();
        for spec in &self.specs {
            let v = evaluate_spec(spec, store, Some(self), &mut visiting)?;
            out.insert(spec.name.clone(), v);
        }
        Ok(out)
    }
}

/// Evaluate one spec. If `reg` is given, earlier derivations are
/// recursively available; simple cycle detection via `visiting`.
///
/// Guarantees `visiting` is returned to its pre-call state on every
/// exit path (Ok or Err) so sibling evaluations don't see phantom
/// cycles.
#[cfg(feature = "eval")]
fn evaluate_spec(
    spec: &DerivedSpec,
    store: &StateStore,
    reg: Option<&DerivedRegistry>,
    visiting: &mut std::collections::HashSet<String>,
) -> Result<JsonValue, String> {
    if !visiting.insert(spec.name.clone()) {
        return Err(format!(
            "derived cycle: {} already in evaluation path",
            spec.name
        ));
    }
    let result = evaluate_spec_inner(spec, store, reg, visiting);
    visiting.remove(&spec.name);
    result
}

#[cfg(feature = "eval")]
fn evaluate_spec_inner(
    spec: &DerivedSpec,
    store: &StateStore,
    reg: Option<&DerivedRegistry>,
    visiting: &mut std::collections::HashSet<String>,
) -> Result<JsonValue, String> {
    use crate::eval::NamiEvaluator;
    use serde_json::json;

    let evaluator = NamiEvaluator::new();
    crate::store::bind_into(&evaluator, store);

    // Also bind any OTHER derivations as symbols, computing them on
    // demand so a derivation can reference another. Cycles in the
    // reference graph are detected via `visiting` and silently
    // skipped here (the direct evaluation path reports them).
    if let Some(reg) = reg {
        for other in &reg.specs {
            if other.name == spec.name {
                continue;
            }
            if visiting.contains(&other.name) {
                // Already being evaluated further up the call stack;
                // skip to avoid reporting a spurious cycle for a
                // sibling that doesn't actually reference this one.
                continue;
            }
            if let Ok(v) = evaluate_spec(other, store, Some(reg), visiting) {
                evaluator
                    .interpreter()
                    .define(&other.name, crate::eval::json_to_value(&v));
            }
        }
    }

    let raw = evaluator
        .eval(&spec.compute, &json!({}))
        .map_err(|e| format!("derived {} compute: {e}", spec.name))?;

    Ok(crate::eval::value_to_json(&raw))
}

/// Compile a Lisp document of `(defderived …)` forms.
#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<DerivedSpec>, String> {
    tatara_lisp::compile_typed::<DerivedSpec>(src).map_err(|e| format!("{e}"))
}

/// Register the `defderived` keyword.
#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<DerivedSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{StateSpec, StateStore};
    use serde_json::json;

    fn derived(name: &str, inputs: &[&str], compute: &str) -> DerivedSpec {
        DerivedSpec {
            name: name.into(),
            description: None,
            inputs: inputs.iter().map(|s| s.to_string()).collect(),
            compute: compute.into(),
        }
    }

    fn state_store(cells: &[(&str, JsonValue)]) -> StateStore {
        let specs: Vec<StateSpec> = cells
            .iter()
            .map(|(n, v)| StateSpec {
                name: (*n).into(),
                initial: v.clone(),
                description: None,
                persistent: false,
            })
            .collect();
        StateStore::from_specs(&specs)
    }

    #[cfg(feature = "eval")]
    #[test]
    fn simple_arithmetic_derivation() {
        let store = state_store(&[("price", json!(10)), ("quantity", json!(3))]);
        let mut reg = DerivedRegistry::new();
        reg.insert(derived(
            "subtotal",
            &["price", "quantity"],
            "(* price quantity)",
        ));
        let v = reg.evaluate("subtotal", &store).unwrap();
        assert_eq!(v, json!(30));
    }

    #[cfg(feature = "eval")]
    #[test]
    fn derivation_reacts_to_state_change() {
        let store = state_store(&[("count", json!(5))]);
        let mut reg = DerivedRegistry::new();
        reg.insert(derived("doubled", &["count"], "(* count 2)"));
        assert_eq!(reg.evaluate("doubled", &store).unwrap(), json!(10));
        store.set("count", json!(7));
        assert_eq!(reg.evaluate("doubled", &store).unwrap(), json!(14));
    }

    #[cfg(feature = "eval")]
    #[test]
    fn derivation_chains() {
        // subtotal = price * quantity
        // tax = subtotal * rate
        // total = subtotal + tax
        let store = state_store(&[
            ("price", json!(100)),
            ("quantity", json!(2)),
            ("rate", json!(0)), // integer rate for clean arithmetic
        ]);
        let mut reg = DerivedRegistry::new();
        reg.insert(derived(
            "subtotal",
            &["price", "quantity"],
            "(* price quantity)",
        ));
        reg.insert(derived("tax", &["subtotal", "rate"], "(* subtotal rate)"));
        reg.insert(derived("total", &["subtotal", "tax"], "(+ subtotal tax)"));
        let all = reg.evaluate_all(&store).unwrap();
        assert_eq!(all["subtotal"], json!(200));
        assert_eq!(all["tax"], json!(0));
        assert_eq!(all["total"], json!(200));
    }

    #[cfg(feature = "eval")]
    #[test]
    fn derivation_can_reference_other_derivations() {
        let store = state_store(&[("base", json!(4))]);
        let mut reg = DerivedRegistry::new();
        reg.insert(derived("square", &["base"], "(* base base)"));
        reg.insert(derived("square-plus-one", &["square"], "(+ square 1)"));
        let v = reg.evaluate("square-plus-one", &store).unwrap();
        assert_eq!(v, json!(17));
    }

    #[cfg(feature = "eval")]
    #[test]
    fn unknown_derived_errors() {
        let store = state_store(&[]);
        let reg = DerivedRegistry::new();
        assert!(reg.evaluate("ghost", &store).is_err());
    }

    #[cfg(feature = "eval")]
    #[test]
    fn string_operation_derivation() {
        let store = state_store(&[("first", json!("Jane")), ("last", json!("Doe"))]);
        let mut reg = DerivedRegistry::new();
        reg.insert(derived(
            "full-name",
            &["first", "last"],
            r#"(string-append first " " last)"#,
        ));
        assert_eq!(
            reg.evaluate("full-name", &store).unwrap(),
            json!("Jane Doe")
        );
    }

    #[cfg(feature = "eval")]
    #[test]
    fn conditional_derivation() {
        let store = state_store(&[("count", json!(7))]);
        let mut reg = DerivedRegistry::new();
        reg.insert(derived(
            "parity",
            &["count"],
            r#"(if (= (- count (* 2 (/ count 2))) 0) "even" "odd")"#,
        ));
        assert_eq!(reg.evaluate("parity", &store).unwrap(), json!("odd"));
    }

    #[cfg(feature = "eval")]
    #[test]
    fn registry_replace_by_name() {
        let mut reg = DerivedRegistry::new();
        reg.insert(derived("x", &["a"], "(* a 2)"));
        reg.insert(derived("x", &["a"], "(* a 3)"));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("x").unwrap().compute, "(* a 3)");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_round_trip_derived_specs() {
        let src = r#"
            (defderived :name "subtotal"
                        :inputs ("price" "quantity")
                        :compute "(* price quantity)"
                        :description "line subtotal before tax")
            (defderived :name "total"
                        :inputs ("subtotal" "tax")
                        :compute "(+ subtotal tax)")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].inputs, vec!["price", "quantity"]);
        assert_eq!(specs[1].compute, "(+ subtotal tax)");
    }
}
