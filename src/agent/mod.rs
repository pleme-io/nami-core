//! Lisp-authored agents — the composition of every primitive we've
//! shipped into a single end-to-end loop.
//!
//! An agent is a named bundle: "when TRIGGER fires, if PREDICATE
//! holds, apply PLAN-OR-TRANSFORM." Stored as pure data; evaluated
//! against an [`EvalContext`] + applied via the existing transform
//! engine.
//!
//! ```lisp
//! ; named predicate + plan first (see predicate, plan modules)
//! (defpredicate :name "likely-article" :selector "article" :min 1)
//! (defplan      :name "reader-mode"   :apply ("hide-ads" "flag-images"))
//!
//! ; then the agent:
//! (defagent :name "auto-reader-mode"
//!           :on "page-load"
//!           :when "likely-article"
//!           :apply "reader-mode")
//!
//! ; ad-free agent that runs ALWAYS (no :when):
//! (defagent :name "cleanup"
//!           :on "page-load"
//!           :applies ("strip-scripts" "hide-ads"))
//! ```
//!
//! Design: **two-phase** execution.
//!
//!   Phase 1 — `decide`:  read-only, produces `Vec<Decision>`. No
//!                        tree mutation. This is exactly what a WASM
//!                        agent would do in the future host API —
//!                        read DOM, read state, emit names of things
//!                        to apply.
//!
//!   Phase 2 — `apply`:   mutable, consumes decisions + the real
//!                        transform engine.
//!
//! Splitting phases keeps the read-side immutable for the duration
//! of evaluation (safer, easier to reason about, maps cleanly to the
//! wasmtime host-function shape that's coming) and lets us produce a
//! structured report BEFORE committing any mutation.
//!
//! Trigger strings are open-ended. V1 runtime only fires agents with
//! `:on "page-load"`, but the schema allows `"interval"`,
//! `"user-invoked"`, `"on-navigate"`, etc. — those activate when the
//! WASM host lands.

use crate::plan::PlanRegistry;
use crate::predicate::{EvalContext, PredicateRegistry};
use crate::transform::{self, DomTransformSpec, TransformReport};
use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// A named agent: a predicate-gated action bundle.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defagent"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Trigger identifier. V1 runtime fires `"page-load"` only; other
    /// strings stored but not evaluated until the WASM host lands.
    pub on: String,
    /// Name of a predicate that must evaluate true. Absent = always
    /// run.
    #[serde(default)]
    pub when: Option<String>,
    /// Single plan or transform name to apply. Resolved plan-first:
    /// if `apply` names a plan, it expands; otherwise it's treated
    /// as a transform name.
    #[serde(default)]
    pub apply: Option<String>,
    /// Alternatively, a list of names applied in order. Mutually
    /// exclusive with `apply` — pick one.
    #[serde(default)]
    pub applies: Vec<String>,
    /// Free-form categorization.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Read-only decision produced by phase 1.
#[derive(Debug, Clone)]
pub struct Decision {
    pub agent: String,
    pub fired: bool,
    /// Non-empty only when `fired` is true — the ordered list of
    /// transform names the agent wants to apply.
    pub transforms: Vec<String>,
    /// `None` when the agent fired, else a human-readable reason.
    pub skipped_reason: Option<String>,
}

/// Post-apply record: the report of an agent that actually ran.
#[derive(Debug, Clone)]
pub struct AgentRunReport {
    pub agent: String,
    pub applied: usize,
    pub missing_transforms: Vec<String>,
    pub engine_report: TransformReport,
}

/// Index of agents by name.
#[derive(Debug, Clone, Default)]
pub struct AgentRegistry {
    agents: Vec<AgentSpec>,
}

impl AgentRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: AgentSpec) {
        self.agents.retain(|a| a.name != spec.name);
        self.agents.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = AgentSpec>) {
        for s in specs {
            self.insert(s);
        }
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&AgentSpec> {
        self.agents.iter().find(|a| a.name == name)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.agents.len()
    }

    /// Iterate agents with a given trigger (e.g. `"page-load"`).
    pub fn for_trigger<'a>(&'a self, trigger: &'a str) -> impl Iterator<Item = &'a AgentSpec> + 'a {
        self.agents.iter().filter(move |a| a.on == trigger)
    }
}

/// Phase 1: evaluate each matching agent's predicate + resolve its
/// plan/transform references into an ordered transform-name list.
/// Read-only; safe to run many times, compose with agents, etc.
pub fn decide(
    registry: &AgentRegistry,
    trigger: &str,
    predicates: &PredicateRegistry,
    plans: &PlanRegistry,
    cx: &EvalContext<'_>,
) -> Vec<Decision> {
    let mut out = Vec::new();
    for spec in registry.for_trigger(trigger) {
        out.push(decide_one(spec, predicates, plans, cx));
    }
    out
}

fn decide_one(
    spec: &AgentSpec,
    predicates: &PredicateRegistry,
    plans: &PlanRegistry,
    cx: &EvalContext<'_>,
) -> Decision {
    // Gate on predicate.
    if let Some(name) = &spec.when {
        match predicates.evaluate(name, cx) {
            Ok(true) => {}
            Ok(false) => {
                return Decision {
                    agent: spec.name.clone(),
                    fired: false,
                    transforms: vec![],
                    skipped_reason: Some(format!("predicate {name} was false")),
                };
            }
            Err(e) => {
                return Decision {
                    agent: spec.name.clone(),
                    fired: false,
                    transforms: vec![],
                    skipped_reason: Some(format!("predicate error: {e}")),
                };
            }
        }
    }

    // Resolve apply / applies into a flat name list.
    let entries: Vec<String> = if !spec.applies.is_empty() {
        spec.applies.clone()
    } else if let Some(one) = &spec.apply {
        vec![one.clone()]
    } else {
        return Decision {
            agent: spec.name.clone(),
            fired: false,
            transforms: vec![],
            skipped_reason: Some("agent has no :apply or :applies".into()),
        };
    };

    // Each entry might be a plan (expand) or a transform name (pass
    // through).
    let mut transforms = Vec::new();
    for entry in entries {
        if plans.get(&entry).is_some() {
            match plans.resolve(&entry) {
                Ok(names) => transforms.extend(names),
                Err(e) => {
                    return Decision {
                        agent: spec.name.clone(),
                        fired: false,
                        transforms: vec![],
                        skipped_reason: Some(format!("plan {entry} resolve error: {e}")),
                    };
                }
            }
        } else {
            transforms.push(entry);
        }
    }

    Decision {
        agent: spec.name.clone(),
        fired: true,
        transforms,
        skipped_reason: None,
    }
}

/// Phase 2: apply decided transforms to the document. Only decisions
/// with `fired: true` actually touch the tree. Missing transform
/// names are recorded per-agent (not errors).
pub fn apply(
    doc: &mut crate::dom::Document,
    decisions: &[Decision],
    transforms: &[DomTransformSpec],
) -> Vec<AgentRunReport> {
    let mut out = Vec::new();
    for d in decisions {
        if !d.fired {
            continue;
        }
        let mut selected = Vec::with_capacity(d.transforms.len());
        let mut missing = Vec::new();
        for name in &d.transforms {
            match transforms.iter().find(|t| t.name == *name) {
                Some(spec) => selected.push(spec.clone()),
                None => missing.push(name.clone()),
            }
        }
        let report = transform::apply(doc, &selected);
        let applied = report.applied.len();
        out.push(AgentRunReport {
            agent: d.agent.clone(),
            applied,
            missing_transforms: missing,
            engine_report: report,
        });
    }
    out
}

/// Convenience: do both phases for the `"page-load"` trigger.
pub fn run_page_load(
    doc: &mut crate::dom::Document,
    registry: &AgentRegistry,
    predicates: &PredicateRegistry,
    plans: &PlanRegistry,
    transforms: &[DomTransformSpec],
    detections: &[crate::framework::Detection],
    state: &[crate::state::StateBlob],
) -> (Vec<Decision>, Vec<AgentRunReport>) {
    let cx = EvalContext {
        doc,
        detections,
        state,
    };
    let decisions = decide(registry, "page-load", predicates, plans, &cx);
    let reports = apply(doc, &decisions, transforms);
    (decisions, reports)
}

/// Compile a Lisp document of `(defagent …)` forms.
#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<AgentSpec>, String> {
    tatara_lisp::compile_typed::<AgentSpec>(src).map_err(|e| format!("{e}"))
}

/// Register the `defagent` keyword in the global tatara registry.
#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<AgentSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::Document;
    use crate::plan::PlanSpec;
    use crate::predicate::PredicateSpec;
    use crate::transform::{DomAction, DomTransformSpec};

    fn pred(name: &str, sel: &str, min: Option<usize>, max: Option<usize>) -> PredicateSpec {
        PredicateSpec {
            name: name.into(),
            description: None,
            selector: Some(sel.into()),
            min,
            max,
            framework: None,
            state_kind: None,
            all: vec![],
            any: vec![],
            none: vec![],
        }
    }

    fn xform(name: &str, sel: &str, action: DomAction) -> DomTransformSpec {
        DomTransformSpec {
            name: name.into(),
            selector: sel.into(),
            action,
            arg: None,
            description: None,
        }
    }

    fn plan(name: &str, apply: &[&str]) -> PlanSpec {
        PlanSpec {
            name: name.into(),
            apply: apply.iter().map(|s| s.to_string()).collect(),
            description: None,
            tags: vec![],
        }
    }

    fn agent(
        name: &str,
        on: &str,
        when: Option<&str>,
        apply: Option<&str>,
        applies: &[&str],
    ) -> AgentSpec {
        AgentSpec {
            name: name.into(),
            description: None,
            on: on.into(),
            when: when.map(str::to_owned),
            apply: apply.map(str::to_owned),
            applies: applies.iter().map(|s| s.to_string()).collect(),
            tags: vec![],
        }
    }

    #[test]
    fn agent_gated_by_predicate_fires_when_condition_holds() {
        let mut preds = PredicateRegistry::new();
        preds.insert(pred("has-ads", ".ad", None, None));
        let plans = PlanRegistry::new();
        let mut agents = AgentRegistry::new();
        agents.insert(agent(
            "ad-stripper",
            "page-load",
            Some("has-ads"),
            Some("hide-ads"),
            &[],
        ));

        let doc = Document::parse(r#"<html><body><div class="ad">x</div></body></html>"#);
        let cx = EvalContext {
            doc: &doc,
            detections: &[],
            state: &[],
        };
        let decisions = decide(&agents, "page-load", &preds, &plans, &cx);
        assert_eq!(decisions.len(), 1);
        assert!(decisions[0].fired);
        assert_eq!(decisions[0].transforms, vec!["hide-ads"]);
    }

    #[test]
    fn agent_gated_by_predicate_skips_when_condition_false() {
        let mut preds = PredicateRegistry::new();
        preds.insert(pred("has-ads", ".ad", None, None));
        let plans = PlanRegistry::new();
        let mut agents = AgentRegistry::new();
        agents.insert(agent(
            "ad-stripper",
            "page-load",
            Some("has-ads"),
            Some("hide-ads"),
            &[],
        ));

        let clean = Document::parse(r#"<html><body><p>clean</p></body></html>"#);
        let cx = EvalContext {
            doc: &clean,
            detections: &[],
            state: &[],
        };
        let decisions = decide(&agents, "page-load", &preds, &plans, &cx);
        assert!(!decisions[0].fired);
        assert!(
            decisions[0]
                .skipped_reason
                .as_ref()
                .unwrap()
                .contains("false")
        );
    }

    #[test]
    fn agent_without_when_always_fires() {
        let preds = PredicateRegistry::new();
        let plans = PlanRegistry::new();
        let mut agents = AgentRegistry::new();
        agents.insert(agent(
            "always",
            "page-load",
            None,
            Some("strip-scripts"),
            &[],
        ));
        let doc = Document::parse("<html></html>");
        let cx = EvalContext {
            doc: &doc,
            detections: &[],
            state: &[],
        };
        let decisions = decide(&agents, "page-load", &preds, &plans, &cx);
        assert!(decisions[0].fired);
    }

    #[test]
    fn agent_applies_plan_expands_to_transforms() {
        let preds = PredicateRegistry::new();
        let mut plans = PlanRegistry::new();
        plans.insert(plan("reader-mode", &["hide-ads", "flag-images"]));
        let mut agents = AgentRegistry::new();
        agents.insert(agent(
            "auto-reader",
            "page-load",
            None,
            Some("reader-mode"),
            &[],
        ));

        let doc = Document::parse("<html></html>");
        let cx = EvalContext {
            doc: &doc,
            detections: &[],
            state: &[],
        };
        let decisions = decide(&agents, "page-load", &preds, &plans, &cx);
        assert_eq!(decisions[0].transforms, vec!["hide-ads", "flag-images"]);
    }

    #[test]
    fn agent_applies_list_mixes_plans_and_transforms() {
        let preds = PredicateRegistry::new();
        let mut plans = PlanRegistry::new();
        plans.insert(plan("cleanup", &["hide-ads"]));
        let mut agents = AgentRegistry::new();
        agents.insert(agent(
            "double-duty",
            "page-load",
            None,
            None,
            &["cleanup", "flag-images"],
        ));

        let doc = Document::parse("<html></html>");
        let cx = EvalContext {
            doc: &doc,
            detections: &[],
            state: &[],
        };
        let decisions = decide(&agents, "page-load", &preds, &plans, &cx);
        // "cleanup" expands to "hide-ads"; "flag-images" passes through.
        assert_eq!(decisions[0].transforms, vec!["hide-ads", "flag-images"]);
    }

    #[test]
    fn non_matching_trigger_skips_agent_entirely() {
        let preds = PredicateRegistry::new();
        let plans = PlanRegistry::new();
        let mut agents = AgentRegistry::new();
        agents.insert(agent("on-click", "user-invoked", None, Some("noop"), &[]));
        let doc = Document::parse("<html></html>");
        let cx = EvalContext {
            doc: &doc,
            detections: &[],
            state: &[],
        };
        let decisions = decide(&agents, "page-load", &preds, &plans, &cx);
        assert!(decisions.is_empty());
    }

    #[test]
    fn full_end_to_end_likely_article_triggers_reader_mode() {
        // Predicates
        let mut preds = PredicateRegistry::new();
        preds.insert(pred("has-article", "article", None, None));
        preds.insert(pred("prose-heavy", "p", Some(2), None));
        preds.insert(PredicateSpec {
            name: "likely-article".into(),
            description: None,
            selector: None,
            min: None,
            max: None,
            framework: None,
            state_kind: None,
            all: vec!["has-article".into(), "prose-heavy".into()],
            any: vec![],
            none: vec![],
        });

        // Plan
        let mut plans = PlanRegistry::new();
        plans.insert(plan("reader-mode", &["hide-ads", "flag-images"]));

        // Transforms
        let transforms = vec![
            xform("hide-ads", ".ad", DomAction::Remove),
            xform("flag-images", "img", DomAction::AddClass),
        ];

        // Agent
        let mut agents = AgentRegistry::new();
        agents.insert(agent(
            "auto-reader-mode",
            "page-load",
            Some("likely-article"),
            Some("reader-mode"),
            &[],
        ));

        // Article-shaped page with an ad and images.
        let mut doc = Document::parse(
            r#"<html><body><article>
                 <h1>Title</h1>
                 <p>First paragraph of real content.</p>
                 <p>Second paragraph.</p>
                 <div class="ad">ad</div>
                 <img src="x.png">
               </article></body></html>"#,
        );

        let (decisions, reports) =
            run_page_load(&mut doc, &agents, &preds, &plans, &transforms, &[], &[]);
        assert_eq!(decisions.len(), 1);
        assert!(decisions[0].fired);
        assert_eq!(reports.len(), 1);
        // hide-ads removed the ad; flag-images added a class to img.
        assert!(reports[0].applied >= 1);
        // Ad is gone from the tree.
        let ad_count = doc
            .root
            .descendants()
            .filter(|n| n.as_element().is_some_and(|e| e.has_class("ad")))
            .count();
        assert_eq!(ad_count, 0);
    }

    #[test]
    fn missing_transform_names_reported_not_errored() {
        let preds = PredicateRegistry::new();
        let plans = PlanRegistry::new();
        let mut agents = AgentRegistry::new();
        agents.insert(agent(
            "incomplete",
            "page-load",
            None,
            Some("does-not-exist"),
            &[],
        ));
        let mut doc = Document::parse("<html></html>");
        let transforms: Vec<DomTransformSpec> = vec![];
        let (_decisions, reports) =
            run_page_load(&mut doc, &agents, &preds, &plans, &transforms, &[], &[]);
        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].missing_transforms,
            vec!["does-not-exist".to_string()]
        );
        assert_eq!(reports[0].applied, 0);
    }

    #[test]
    fn predicate_error_records_skip_reason() {
        let preds = PredicateRegistry::new();
        let plans = PlanRegistry::new();
        let mut agents = AgentRegistry::new();
        agents.insert(agent(
            "bad-pred",
            "page-load",
            Some("ghost"),
            Some("x"),
            &[],
        ));
        let doc = Document::parse("<html></html>");
        let cx = EvalContext {
            doc: &doc,
            detections: &[],
            state: &[],
        };
        let decisions = decide(&agents, "page-load", &preds, &plans, &cx);
        assert!(!decisions[0].fired);
        assert!(
            decisions[0]
                .skipped_reason
                .as_ref()
                .unwrap()
                .contains("predicate error")
        );
    }

    #[test]
    fn registry_replace_by_name() {
        let mut reg = AgentRegistry::new();
        reg.insert(agent("x", "page-load", None, Some("a"), &[]));
        reg.insert(agent("x", "interval", None, Some("b"), &[]));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("x").unwrap().on, "interval");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_round_trip_agent_specs() {
        let src = r#"
            (defagent :name "reader"
                      :on "page-load"
                      :when "likely-article"
                      :apply "reader-mode"
                      :description "auto reader mode for article-shaped pages"
                      :tags ("reader" "content"))
            (defagent :name "cleanup-all"
                      :on "page-load"
                      :applies ("strip-scripts" "hide-ads"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].when.as_deref(), Some("likely-article"));
        assert_eq!(specs[0].apply.as_deref(), Some("reader-mode"));
        assert_eq!(specs[0].tags, vec!["reader", "content"]);
        assert_eq!(specs[1].applies, vec!["strip-scripts", "hide-ads"]);
    }
}
