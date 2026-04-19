//! `(defnormalize)` — framework-aware DOM rewrites toward a canonical
//! semantic schema. The first step of the web-as-AST-domain plan:
//! fold every framework's idiom into a shared `n-*` vocabulary so
//! downstream transforms, scrapes, agents, and tooling author against
//! ONE shape instead of React vs Vue vs Svelte vs shadcn vs MUI vs …
//!
//! ## V1 surface
//!
//! A normalize spec matches elements via a selector, optionally gated
//! by a detected framework, and renames matches to a canonical tag:
//!
//! ```lisp
//! (defnormalize :name "shadcn-card"
//!               :framework "shadcn"
//!               :selector "[data-slot=card]"
//!               :rename-to "n-card")
//!
//! (defnormalize :name "mui-card"
//!               :framework "mui"
//!               :selector ".MuiCard-root"
//!               :rename-to "n-card")
//!
//! (defnormalize :name "generic-article"
//!               :selector "article"
//!               :rename-to "n-article")
//! ```
//!
//! Downstream tools now only need to know `n-card` / `n-article` —
//! every page, regardless of framework, yields the same semantic DOM.
//!
//! ## V2 (not yet)
//!
//! Richer rewrites: `:title-from` / `:body-from` extraction into
//! canonical attributes, `:wrap-in` for structural insertion,
//! `:emit-subtree` for full template replacement.

use crate::dom::{Document, ElementData, Node, NodeData};
use crate::framework::Detection;
use crate::selector::{OwnedContext, Selector};
use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// One canonical-form rewrite rule.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defnormalize"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizeSpec {
    pub name: String,
    /// If set, the rule only fires when the named framework was
    /// detected on the current page. Match is case-insensitive on the
    /// framework's debug name (e.g. `"shadcn"` matches `Shadcn`).
    #[serde(default)]
    pub framework: Option<String>,
    pub selector: String,
    /// New tag name for every matching element. Convention: `n-*` for
    /// semantic canonical tags (`n-card`, `n-article`, `n-nav`).
    pub rename_to: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NormalizeRegistry {
    specs: Vec<NormalizeSpec>,
}

impl NormalizeRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: NormalizeSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = NormalizeSpec>) {
        for s in specs {
            self.insert(s);
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    pub fn specs(&self) -> &[NormalizeSpec] {
        &self.specs
    }
}

/// Outcome — one hit per element actually rewritten.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NormalizeReport {
    pub hits: Vec<NormalizeHit>,
}

impl NormalizeReport {
    #[must_use]
    pub fn applied(&self) -> usize {
        self.hits.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizeHit {
    pub rule: String,
    pub from_tag: String,
    pub to_tag: String,
}

/// Apply every normalize rule whose framework gate (if any) matches the
/// current detection list. Rules without a framework fire on every page.
pub fn apply(
    doc: &mut Document,
    registry: &NormalizeRegistry,
    detections: &[Detection],
) -> NormalizeReport {
    let mut report = NormalizeReport::default();
    if registry.is_empty() {
        return report;
    }
    for spec in registry.specs() {
        if !framework_matches(spec, detections) {
            continue;
        }
        let selector = match Selector::parse(&spec.selector) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    "normalize '{}' bad selector {:?}: {e}",
                    spec.name,
                    spec.selector
                );
                continue;
            }
        };
        let mut path: Vec<OwnedContext> = Vec::new();
        rewrite(&mut doc.root, spec, &selector, &mut path, &mut report);
    }
    report
}

fn framework_matches(spec: &NormalizeSpec, detections: &[Detection]) -> bool {
    let Some(want) = spec.framework.as_deref() else {
        return true;
    };
    let want = want.to_ascii_lowercase();
    detections.iter().any(|d| {
        let name = format!("{:?}", d.framework).to_ascii_lowercase();
        name == want || name.contains(&want)
    })
}

fn rewrite(
    node: &mut Node,
    spec: &NormalizeSpec,
    selector: &Selector,
    path: &mut Vec<OwnedContext>,
    report: &mut NormalizeReport,
) {
    let pushed = if let NodeData::Element(el) = &node.data {
        path.push(OwnedContext::from_element(el));
        true
    } else {
        false
    };

    // Descend first so nested matches rewrite bottom-up.
    for child in &mut node.children {
        rewrite(child, spec, selector, path, report);
    }

    // Match this node. We use its own entry on path; selector::matches
    // expects the element-under-test to be the last context item.
    if let NodeData::Element(el) = &mut node.data {
        let full: Vec<&OwnedContext> = path.iter().collect();
        if selector.matches(&full) && el.tag != spec.rename_to {
            let from = el.tag.clone();
            el.tag = spec.rename_to.clone();
            // Stamp a tracking attribute so downstream tooling can
            // identify canonicalised elements without a second detect pass.
            set_attr(el, "data-n-from", &from);
            set_attr(el, "data-n-rule", &spec.name);
            report.hits.push(NormalizeHit {
                rule: spec.name.clone(),
                from_tag: from,
                to_tag: spec.rename_to.clone(),
            });
        }
    }

    if pushed {
        path.pop();
    }
}

fn set_attr(el: &mut ElementData, key: &str, value: &str) {
    if let Some(pair) = el.attributes.iter_mut().find(|(k, _)| k == key) {
        pair.1 = value.to_owned();
    } else {
        el.attributes.push((key.to_owned(), value.to_owned()));
    }
}

/// Compile a Lisp source of `(defnormalize …)` forms into specs.
#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<NormalizeSpec>, String> {
    tatara_lisp::compile_typed::<NormalizeSpec>(src)
        .map_err(|e| format!("failed to compile defnormalize forms: {e}"))
}

/// Registration hook so the coherence checker can see this keyword.
#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<NormalizeSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::Document;

    fn no_detections() -> Vec<Detection> {
        Vec::new()
    }

    #[test]
    fn generic_rule_renames_article_to_n_article() {
        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "generic-article".into(),
            framework: None,
            selector: "article".into(),
            rename_to: "n-article".into(),
            description: None,
        });
        let mut doc = Document::parse("<html><body><article><p>hi</p></article></body></html>");
        let report = apply(&mut doc, &reg, &no_detections());
        assert_eq!(report.applied(), 1);
        let hit = &report.hits[0];
        assert_eq!(hit.from_tag, "article");
        assert_eq!(hit.to_tag, "n-article");
        let mut saw = false;
        for n in doc.root.descendants() {
            if let Some(el) = n.as_element() {
                if el.tag == "n-article" {
                    assert_eq!(el.get_attribute("data-n-from"), Some("article"));
                    assert_eq!(el.get_attribute("data-n-rule"), Some("generic-article"));
                    saw = true;
                }
            }
        }
        assert!(saw);
    }

    #[test]
    fn framework_gate_respects_detections() {
        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "shadcn-card".into(),
            framework: Some("shadcn".into()),
            selector: "[data-slot=card]".into(),
            rename_to: "n-card".into(),
            description: None,
        });
        let mut doc = Document::parse(r#"<html><body><div data-slot="card">x</div></body></html>"#);

        // Without shadcn detected → no rewrite.
        let report = apply(&mut doc, &reg, &no_detections());
        assert_eq!(report.applied(), 0);

        // With shadcn detected → rewrite fires.
        let det = vec![Detection {
            framework: crate::framework::Framework::ShadcnRadix,
            name: "shadcn/radix",
            confidence: 0.9,
            evidence: vec![],
        }];
        let report = apply(&mut doc, &reg, &det);
        assert_eq!(report.applied(), 1);
        assert_eq!(report.hits[0].to_tag, "n-card");
    }

    #[test]
    fn idempotent_second_pass_is_noop() {
        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "article".into(),
            framework: None,
            selector: "article".into(),
            rename_to: "n-article".into(),
            description: None,
        });
        let mut doc = Document::parse("<html><body><article>x</article></body></html>");
        let first = apply(&mut doc, &reg, &no_detections());
        assert_eq!(first.applied(), 1);
        // Same rule on the now-renamed DOM shouldn't re-rename — the
        // selector `article` no longer matches `n-article`.
        let second = apply(&mut doc, &reg, &no_detections());
        assert_eq!(second.applied(), 0);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_one_form() {
        let src = r#"
            (defnormalize :name "mui-card"
                          :framework "mui"
                          :selector ".MuiCard-root"
                          :rename-to "n-card")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "mui-card");
        assert_eq!(specs[0].framework.as_deref(), Some("mui"));
        assert_eq!(specs[0].rename_to, "n-card");
    }
}
