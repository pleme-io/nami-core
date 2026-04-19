//! Effects — Layer 2 (reactivity half) of the React-in-Lisp arc.
//!
//! An effect is "when TRIGGER fires, if PREDICATE holds, evaluate
//! this Lisp expression." The expression can read + write the
//! [`StateStore`] via pre-bound symbols + the `(set-state NAME VALUE)`
//! host function.
//!
//! ```lisp
//! (defstate :name "counter" :initial 0)
//!
//! (defeffect :name "bump-on-load"
//!            :on "page-load"
//!            :run "(set-state \"counter\" (+ counter 1))")
//!
//! (defeffect :name "greet-once-first-visit"
//!            :on "page-load"
//!            :when "first-visit"       ; predicate
//!            :run "(set-state \"greeted\" #t)")
//! ```
//!
//! Two-phase execution, same shape as [`crate::agent`]:
//!
//!   Phase 1 — `decide`: read-only evaluation of predicates; emits
//!             `Vec<EffectDecision>` listing which effects want to
//!             fire and what their run expressions are. Identical in
//!             shape to what a WASM host will produce.
//!
//!   Phase 2 — `apply`:  runs the `:run` expressions through a shared
//!             evaluator bound to the state store; each successful
//!             run mutates the store in place.
//!
//! Separating phases lets callers aggregate decisions across many
//! effect sources, audit them, or defer application.

use crate::predicate::{EvalContext, PredicateRegistry};
use crate::store::StateStore;
use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// A named effect.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defeffect"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EffectSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Trigger identifier. V1 fires `"page-load"` only; other
    /// strings stored but not evaluated until the WASM host lands.
    pub on: String,
    /// Optional predicate gating execution.
    #[serde(default)]
    pub when: Option<String>,
    /// Lisp source of the expression to evaluate when the effect
    /// fires. The expression can reference any state cell as a bound
    /// symbol and mutate the store via `(set-state NAME VALUE)`.
    pub run: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Registry of effect specs.
#[derive(Debug, Clone, Default)]
pub struct EffectRegistry {
    effects: Vec<EffectSpec>,
}

impl EffectRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: EffectSpec) {
        self.effects.retain(|e| e.name != spec.name);
        self.effects.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = EffectSpec>) {
        for s in specs {
            self.insert(s);
        }
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&EffectSpec> {
        self.effects.iter().find(|e| e.name == name)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.effects.len()
    }

    /// Iterate effects with a given trigger (e.g. `"page-load"`).
    pub fn for_trigger<'a>(
        &'a self,
        trigger: &'a str,
    ) -> impl Iterator<Item = &'a EffectSpec> + 'a {
        self.effects.iter().filter(move |e| e.on == trigger)
    }
}

/// Read-only decision from phase 1.
#[derive(Debug, Clone)]
pub struct EffectDecision {
    pub effect: String,
    pub fired: bool,
    pub run: Option<String>,
    pub skipped_reason: Option<String>,
}

/// Post-run record of phase 2.
#[derive(Debug, Clone)]
pub struct EffectRunReport {
    pub effect: String,
    pub ok: bool,
    pub error: Option<String>,
}

/// Phase 1: evaluate predicates, emit decisions. Read-only.
pub fn decide(
    registry: &EffectRegistry,
    trigger: &str,
    predicates: &PredicateRegistry,
    cx: &EvalContext<'_>,
) -> Vec<EffectDecision> {
    let mut out = Vec::new();
    for spec in registry.for_trigger(trigger) {
        out.push(decide_one(spec, predicates, cx));
    }
    out
}

fn decide_one(
    spec: &EffectSpec,
    predicates: &PredicateRegistry,
    cx: &EvalContext<'_>,
) -> EffectDecision {
    if let Some(name) = &spec.when {
        match predicates.evaluate(name, cx) {
            Ok(true) => {}
            Ok(false) => {
                return EffectDecision {
                    effect: spec.name.clone(),
                    fired: false,
                    run: None,
                    skipped_reason: Some(format!("predicate {name} was false")),
                };
            }
            Err(e) => {
                return EffectDecision {
                    effect: spec.name.clone(),
                    fired: false,
                    run: None,
                    skipped_reason: Some(format!("predicate error: {e}")),
                };
            }
        }
    }
    EffectDecision {
        effect: spec.name.clone(),
        fired: true,
        run: Some(spec.run.clone()),
        skipped_reason: None,
    }
}

/// Phase 2: execute each fired decision's `run` expression against a
/// shared evaluator bound to the state store. Requires the `eval`
/// feature.
#[cfg(feature = "eval")]
pub fn apply(store: &StateStore, decisions: &[EffectDecision]) -> Vec<EffectRunReport> {
    use crate::eval::NamiEvaluator;
    use serde_json::json;

    let mut reports = Vec::with_capacity(decisions.len());
    for d in decisions {
        if !d.fired {
            continue;
        }
        let Some(src) = d.run.as_ref() else {
            continue;
        };
        // Fresh evaluator per effect so state changes between effects
        // aren't captured by stale symbol bindings. Cost is tiny
        // (interpreter::new is cheap) and semantics are clearer.
        let evaluator = NamiEvaluator::new();
        crate::store::bind_into(&evaluator, store);
        match evaluator.eval(src, &json!({})) {
            Ok(_) => reports.push(EffectRunReport {
                effect: d.effect.clone(),
                ok: true,
                error: None,
            }),
            Err(e) => reports.push(EffectRunReport {
                effect: d.effect.clone(),
                ok: false,
                error: Some(format!("{e}")),
            }),
        }
    }
    reports
}

/// Convenience: phase 1 + phase 2 for `"page-load"`.
#[cfg(feature = "eval")]
pub fn run_page_load(
    store: &StateStore,
    effects: &EffectRegistry,
    predicates: &PredicateRegistry,
    cx: &EvalContext<'_>,
) -> (Vec<EffectDecision>, Vec<EffectRunReport>) {
    let decisions = decide(effects, "page-load", predicates, cx);
    let reports = apply(store, &decisions);
    (decisions, reports)
}

/// Compile a Lisp document of `(defeffect …)` forms.
#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<EffectSpec>, String> {
    tatara_lisp::compile_typed::<EffectSpec>(src).map_err(|e| format!("{e}"))
}

/// Register the `defeffect` keyword.
#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<EffectSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::Document;
    use crate::predicate::PredicateSpec;
    use serde_json::json;

    fn effect(name: &str, on: &str, when: Option<&str>, run: &str) -> EffectSpec {
        EffectSpec {
            name: name.into(),
            description: None,
            on: on.into(),
            when: when.map(str::to_owned),
            run: run.into(),
            tags: vec![],
        }
    }

    fn ctx<'a>(doc: &'a Document) -> EvalContext<'a> {
        EvalContext {
            doc,
            detections: &[],
            state: &[],
        }
    }

    #[test]
    fn effect_trigger_filters() {
        let mut reg = EffectRegistry::new();
        reg.insert(effect("a", "page-load", None, "()"));
        reg.insert(effect("b", "interval", None, "()"));
        let preds = PredicateRegistry::new();
        let doc = Document::parse("<html></html>");
        let decisions = decide(&reg, "page-load", &preds, &ctx(&doc));
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].effect, "a");
    }

    #[test]
    fn effect_gated_by_predicate_skips_false() {
        let mut preds = PredicateRegistry::new();
        preds.insert(PredicateSpec {
            name: "has-article".into(),
            description: None,
            selector: Some("article".into()),
            min: None,
            max: None,
            framework: None,
            state_kind: None,
            all: vec![],
            any: vec![],
            none: vec![],
        });
        let mut reg = EffectRegistry::new();
        reg.insert(effect("e", "page-load", Some("has-article"), "()"));
        let doc = Document::parse("<html><body><p>no article</p></body></html>");
        let decisions = decide(&reg, "page-load", &preds, &ctx(&doc));
        assert!(!decisions[0].fired);
    }

    #[cfg(feature = "eval")]
    #[test]
    fn counter_increments_via_set_state() {
        // End-to-end reactivity: state cell + effect that mutates it.
        let store = StateStore::from_specs(&[StateSpec {
            name: "counter".into(),
            initial: json!(0),
            description: None,
            persistent: false,
        }]);
        let mut reg = EffectRegistry::new();
        reg.insert(effect(
            "bump",
            "page-load",
            None,
            r#"(set-state "counter" (+ counter 1))"#,
        ));
        let preds = PredicateRegistry::new();
        let doc = Document::parse("<html></html>");

        // First run → 1
        let (_, reports) = run_page_load(&store, &reg, &preds, &ctx(&doc));
        assert!(reports[0].ok);
        assert_eq!(store.get("counter"), Some(json!(1)));

        // Second run → 2 (state persisted across invocations)
        let (_, reports) = run_page_load(&store, &reg, &preds, &ctx(&doc));
        assert!(reports[0].ok);
        assert_eq!(store.get("counter"), Some(json!(2)));
    }

    #[cfg(feature = "eval")]
    #[test]
    fn effect_error_captured_in_report() {
        let store = StateStore::new();
        let mut reg = EffectRegistry::new();
        reg.insert(effect("bad", "page-load", None, "(nope)"));
        let preds = PredicateRegistry::new();
        let doc = Document::parse("<html></html>");
        let (_, reports) = run_page_load(&store, &reg, &preds, &ctx(&doc));
        assert_eq!(reports.len(), 1);
        assert!(!reports[0].ok);
        assert!(reports[0].error.is_some());
    }

    #[cfg(feature = "eval")]
    #[test]
    fn effects_run_in_registered_order() {
        let store = StateStore::from_specs(&[StateSpec {
            name: "seq".into(),
            initial: json!(0),
            description: None,
            persistent: false,
        }]);
        let mut reg = EffectRegistry::new();
        reg.insert(effect("first", "page-load", None, r#"(set-state "seq" 1)"#));
        reg.insert(effect(
            "second",
            "page-load",
            None,
            r#"(set-state "seq" 2)"#,
        ));
        reg.insert(effect("third", "page-load", None, r#"(set-state "seq" 3)"#));
        let preds = PredicateRegistry::new();
        let doc = Document::parse("<html></html>");
        run_page_load(&store, &reg, &preds, &ctx(&doc));
        // Third wins.
        assert_eq!(store.get("seq"), Some(json!(3)));
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_round_trip_effect_specs() {
        let src = r#"
            (defeffect :name "bump-on-load"
                       :on "page-load"
                       :run "(set-state \"counter\" (+ counter 1))")
            (defeffect :name "greet-once"
                       :on "page-load"
                       :when "first-visit"
                       :run "(set-state \"greeted\" #t)"
                       :tags ("greeting"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[1].when.as_deref(), Some("first-visit"));
        assert_eq!(specs[1].tags, vec!["greeting"]);
    }

    use crate::store::StateSpec;
}
