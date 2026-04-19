//! Named plans — composable bundles of Lisp-authored transforms.
//!
//! A plan is a named list of **transform names** and/or **sub-plan
//! names**, applied in order. Sub-plans expand recursively; cycles are
//! detected and reported.
//!
//! ```lisp
//! (defdom-transform :name "hide-ads"     :selector ".ad"  :action remove)
//! (defdom-transform :name "flag-images"  :selector "img"  :action add-class :arg "nami-alt")
//! (defdom-transform :name "strip-scripts" :selector "script" :action remove)
//!
//! (defplan :name "reader-mode"
//!          :apply ("hide-ads" "flag-images"))
//!
//! (defplan :name "full-cleanup"
//!          :apply ("reader-mode" "strip-scripts"))
//! ```
//!
//! Resolution rule: for each entry in `:apply`, look up in the plan
//! registry first (recursive expand); if not a plan, treat as a raw
//! transform name. Cycles (plan A references plan B which references
//! plan A) return `Err`.
//!
//! Plans live entirely in the Lisp layer — they produce a selection
//! of transform names, which then run through the unchanged engine.
//! The principle holds: Lisp composes, Rust executes.

use crate::transform::DomTransformSpec;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// A named composition of transforms.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defplan"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlanSpec {
    pub name: String,
    /// Names of transforms or sub-plans to apply, in order.
    pub apply: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Free-form tags for filtering / categorization.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Index of plans by name.
#[derive(Debug, Clone, Default)]
pub struct PlanRegistry {
    plans: Vec<PlanSpec>,
}

impl PlanRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, plan: PlanSpec) {
        self.plans.retain(|p| p.name != plan.name);
        self.plans.push(plan);
    }

    pub fn extend(&mut self, plans: impl IntoIterator<Item = PlanSpec>) {
        for p in plans {
            self.insert(p);
        }
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&PlanSpec> {
        self.plans.iter().find(|p| p.name == name)
    }

    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.plans.iter().map(|p| p.name.as_str()).collect()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plans.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.plans.len()
    }

    /// Expand a plan name into an ordered list of transform names,
    /// recursively descending into sub-plans. Duplicates are preserved
    /// (intentional — a transform listed twice in two sub-plans runs
    /// twice, which is often what the user wants for idempotent
    /// actions like `add-class`).
    ///
    /// Returns `Err` if:
    ///   - the plan name is not registered
    ///   - a cycle is detected during expansion
    pub fn resolve(&self, plan_name: &str) -> Result<Vec<String>, String> {
        let mut visited: HashSet<&str> = HashSet::new();
        let mut out: Vec<String> = Vec::new();
        self.resolve_into(plan_name, &mut visited, &mut out)?;
        Ok(out)
    }

    fn resolve_into<'a>(
        &'a self,
        plan_name: &'a str,
        visited: &mut HashSet<&'a str>,
        out: &mut Vec<String>,
    ) -> Result<(), String> {
        let plan = self
            .get(plan_name)
            .ok_or_else(|| format!("unknown plan: {plan_name}"))?;
        if !visited.insert(plan.name.as_str()) {
            return Err(format!("plan cycle: {plan_name} already in expansion path"));
        }
        for entry in &plan.apply {
            if self.get(entry).is_some() {
                self.resolve_into(entry.as_str(), visited, out)?;
            } else {
                // Not a plan — treat as a transform name.
                out.push(entry.clone());
            }
        }
        visited.remove(plan.name.as_str());
        Ok(())
    }

    /// Given a plan name + a full set of transform specs, return the
    /// subset of specs selected by the plan, preserving plan order.
    /// Transforms named by the plan but not present in `all` are
    /// silently dropped (with a tracing warning) rather than erroring
    /// — lets users keep partial plans around during authoring.
    pub fn select_transforms(
        &self,
        plan_name: &str,
        all: &[DomTransformSpec],
    ) -> Result<Vec<DomTransformSpec>, String> {
        let names = self.resolve(plan_name)?;
        let mut out = Vec::with_capacity(names.len());
        for name in &names {
            match all.iter().find(|t| t.name == *name) {
                Some(spec) => out.push(spec.clone()),
                None => tracing::warn!("plan {plan_name} references unknown transform {name}"),
            }
        }
        Ok(out)
    }
}

/// Compile a Lisp document of `(defplan …)` forms.
#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<PlanSpec>, String> {
    tatara_lisp::compile_typed::<PlanSpec>(src).map_err(|e| format!("{e}"))
}

/// Register the `defplan` keyword in the global tatara registry.
#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<PlanSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::DomAction;

    fn plan(name: &str, apply: &[&str]) -> PlanSpec {
        PlanSpec {
            name: name.into(),
            apply: apply.iter().map(|s| s.to_string()).collect(),
            description: None,
            tags: vec![],
        }
    }

    fn t(name: &str, sel: &str) -> DomTransformSpec {
        DomTransformSpec {
            name: name.into(),
            selector: sel.into(),
            action: DomAction::Remove,
            arg: None,
            description: None,
        }
    }

    #[test]
    fn resolve_flat_plan() {
        let mut reg = PlanRegistry::new();
        reg.insert(plan("reader", &["hide-ads", "flag-images"]));
        let out = reg.resolve("reader").unwrap();
        assert_eq!(out, vec!["hide-ads", "flag-images"]);
    }

    #[test]
    fn resolve_nested_plan() {
        let mut reg = PlanRegistry::new();
        reg.insert(plan("reader", &["hide-ads", "flag-images"]));
        reg.insert(plan("cleanup", &["reader", "strip-scripts"]));
        let out = reg.resolve("cleanup").unwrap();
        assert_eq!(out, vec!["hide-ads", "flag-images", "strip-scripts"]);
    }

    #[test]
    fn resolve_deep_nesting() {
        let mut reg = PlanRegistry::new();
        reg.insert(plan("leaf", &["a"]));
        reg.insert(plan("mid", &["leaf", "b"]));
        reg.insert(plan("top", &["mid", "c"]));
        let out = reg.resolve("top").unwrap();
        assert_eq!(out, vec!["a", "b", "c"]);
    }

    #[test]
    fn resolve_unknown_plan_errors() {
        let reg = PlanRegistry::new();
        let err = reg.resolve("ghost").unwrap_err();
        assert!(err.contains("unknown plan"));
    }

    #[test]
    fn resolve_detects_cycle() {
        let mut reg = PlanRegistry::new();
        reg.insert(plan("a", &["b"]));
        reg.insert(plan("b", &["a"]));
        let err = reg.resolve("a").unwrap_err();
        assert!(err.contains("cycle"), "expected cycle error, got {err}");
    }

    #[test]
    fn resolve_detects_self_cycle() {
        let mut reg = PlanRegistry::new();
        reg.insert(plan("me", &["me"]));
        let err = reg.resolve("me").unwrap_err();
        assert!(err.contains("cycle"));
    }

    #[test]
    fn plan_entries_that_are_not_plans_are_treated_as_transforms() {
        // "hide-ads" isn't a plan — it's just a transform name.
        let mut reg = PlanRegistry::new();
        reg.insert(plan("reader", &["hide-ads", "flag-images"]));
        let out = reg.resolve("reader").unwrap();
        // Both come through as raw transform names.
        assert_eq!(out, vec!["hide-ads", "flag-images"]);
    }

    #[test]
    fn duplicate_entries_preserved() {
        // Idempotent actions like add-class are fine to run twice.
        let mut reg = PlanRegistry::new();
        reg.insert(plan("reader", &["hide-ads", "flag-images"]));
        reg.insert(plan("double", &["reader", "reader"]));
        let out = reg.resolve("double").unwrap();
        assert_eq!(
            out,
            vec!["hide-ads", "flag-images", "hide-ads", "flag-images"]
        );
    }

    #[test]
    fn select_transforms_filters_and_orders() {
        let mut reg = PlanRegistry::new();
        reg.insert(plan("reader", &["hide-ads", "flag-images"]));
        let all = vec![
            t("strip-scripts", "script"),
            t("flag-images", "img"),
            t("hide-ads", ".ad"),
        ];
        let selected = reg.select_transforms("reader", &all).unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].name, "hide-ads");
        assert_eq!(selected[1].name, "flag-images");
    }

    #[test]
    fn select_transforms_drops_missing_silently() {
        let mut reg = PlanRegistry::new();
        reg.insert(plan("reader", &["hide-ads", "ghost"]));
        let all = vec![t("hide-ads", ".ad")];
        let selected = reg.select_transforms("reader", &all).unwrap();
        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn registry_insert_and_replace() {
        let mut reg = PlanRegistry::new();
        reg.insert(plan("p", &["a"]));
        reg.insert(plan("p", &["b", "c"]));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("p").unwrap().apply, vec!["b", "c"]);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_round_trip_plan_spec() {
        let src = r#"
            (defplan :name "reader-mode"
                     :apply ("hide-ads" "flag-images")
                     :description "declutter for reading")
            (defplan :name "full-cleanup"
                     :apply ("reader-mode" "strip-scripts")
                     :tags ("deep" "destructive"))
        "#;
        let plans = compile(src).unwrap();
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].name, "reader-mode");
        assert_eq!(plans[0].apply, vec!["hide-ads", "flag-images"]);
        assert_eq!(plans[1].tags, vec!["deep", "destructive"]);
    }
}
