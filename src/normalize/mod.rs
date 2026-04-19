//! `(defnormalize)` — framework-aware DOM rewrites toward a canonical
//! semantic schema. The first step of the web-as-AST-domain plan:
//! fold every framework's idiom into a shared `n-*` vocabulary so
//! downstream transforms, scrapes, agents, and tooling author against
//! ONE shape instead of React vs Vue vs Svelte vs shadcn vs MUI vs …
//!
//! ## Surface
//!
//! A normalize spec matches elements via a selector, optionally gated
//! by a detected framework, and rewrites matches:
//!
//! | field             | effect                                          |
//! | ----------------- | ----------------------------------------------- |
//! | `:rename-to`      | new tag name (required)                         |
//! | `:set-attrs`      | list of `(KEY VALUE)` pairs to add/overwrite    |
//! | `:remove-attrs`   | list of attribute names to strip                |
//! | `:framework`      | only fire when detection matches                |
//!
//! ```lisp
//! ;; Inbound fold: shadcn → canonical.
//! (defnormalize :name "shadcn-card-in"
//!               :framework "shadcn"
//!               :selector "[data-slot=card]"
//!               :rename-to "n-card")
//!
//! ;; Outbound emit: canonical → shadcn-shaped DOM.
//! (defnormalize :name "shadcn-card-out"
//!               :selector "n-card"
//!               :rename-to "div"
//!               :set-attrs (("data-slot" "card")))
//!
//! ;; Strip framework-specific debris after normalization.
//! (defnormalize :name "strip-mui-debris"
//!               :selector "[data-n-rule=mui-card]"
//!               :remove-attrs ("data-testid"))
//! ```
//!
//! Run a pair of `*-in` + `*-out` rule sets and you get framework
//! conversion: `<article>` → `n-article` → `<div data-slot=article>`.

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
    /// Attributes to add or overwrite on every rewritten element.
    /// Enables outbound emit patterns (canonical → framework-shaped):
    /// `:set-attrs (("data-slot" "card") ("role" "article"))`.
    #[serde(default)]
    pub set_attrs: Vec<(String, String)>,
    /// Attributes to strip from every rewritten element.
    #[serde(default)]
    pub remove_attrs: Vec<String>,
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
    let want = want.trim().to_ascii_lowercase();
    detections.iter().any(|d| {
        // Accept any of: Debug enum name, Detection.name (canonical
        // string like "shadcn/radix"), substring either way. Gives
        // authors flexibility — `"shadcn"`, `"shadcn/radix"`, and
        // `"ShadcnRadix"` all match.
        let debug_name = format!("{:?}", d.framework).to_ascii_lowercase();
        let canonical = d.name.to_ascii_lowercase();
        debug_name == want
            || debug_name.contains(&want)
            || canonical == want
            || canonical.contains(&want)
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
        let matched = selector.matches(&full);
        let needs_rename = el.tag != spec.rename_to;
        let has_attr_work = !spec.set_attrs.is_empty() || !spec.remove_attrs.is_empty();
        if matched && (needs_rename || has_attr_work) {
            let from = el.tag.clone();
            if needs_rename {
                el.tag = spec.rename_to.clone();
                set_attr(el, "data-n-from", &from);
                set_attr(el, "data-n-rule", &spec.name);
            }
            for name in &spec.remove_attrs {
                el.attributes.retain(|(k, _)| k != name);
            }
            for (k, v) in &spec.set_attrs {
                set_attr(el, k, v);
            }
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
            set_attrs: vec![],
            remove_attrs: vec![],
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
            set_attrs: vec![],
            remove_attrs: vec![],
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
            set_attrs: vec![],
            remove_attrs: vec![],
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

    #[test]
    fn framework_match_accepts_canonical_slash_name() {
        // Historically "shadcn/radix" was the user-facing name; the
        // Debug enum is ShadcnRadix. Both spellings should gate
        // rules correctly.
        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "shadcn-card".into(),
            framework: Some("shadcn/radix".into()),
            selector: "[data-slot=card]".into(),
            rename_to: "n-card".into(),
            set_attrs: vec![],
            remove_attrs: vec![],
            description: None,
        });
        reg.insert(NormalizeSpec {
            name: "shadcn-alt".into(),
            framework: Some("shadcn".into()),
            selector: "[data-slot=tab]".into(),
            rename_to: "n-tab".into(),
            set_attrs: vec![],
            remove_attrs: vec![],
            description: None,
        });
        let det = vec![Detection {
            framework: crate::framework::Framework::ShadcnRadix,
            name: "shadcn/radix",
            confidence: 0.9,
            evidence: vec![],
        }];
        let mut doc = Document::parse(
            r#"<html><body>
               <div data-slot="card">c</div>
               <div data-slot="tab">t</div>
            </body></html>"#,
        );
        let report = apply(&mut doc, &reg, &det);
        // Both rules fire — canonical slash name AND substring match.
        assert_eq!(report.applied(), 2);
    }

    #[test]
    fn multiple_rules_fire_independently() {
        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "a".into(),
            framework: None,
            selector: "article".into(),
            rename_to: "n-article".into(),
            set_attrs: vec![],
            remove_attrs: vec![],
            description: None,
        });
        reg.insert(NormalizeSpec {
            name: "b".into(),
            framework: None,
            selector: "nav".into(),
            rename_to: "n-nav".into(),
            set_attrs: vec![],
            remove_attrs: vec![],
            description: None,
        });
        let mut doc = Document::parse(
            "<html><body><article>x</article><nav>n</nav></body></html>",
        );
        let report = apply(&mut doc, &reg, &no_detections());
        assert_eq!(report.applied(), 2);
        // Both tags present in rewritten form.
        let tags: Vec<String> = doc
            .root
            .descendants()
            .filter_map(|n| n.as_element().map(|e| e.tag.clone()))
            .collect();
        assert!(tags.iter().any(|t| t == "n-article"));
        assert!(tags.iter().any(|t| t == "n-nav"));
    }

    #[test]
    fn provenance_attrs_stamped_on_every_hit() {
        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "rule-x".into(),
            framework: None,
            selector: "section".into(),
            rename_to: "n-section".into(),
            set_attrs: vec![],
            remove_attrs: vec![],
            description: None,
        });
        let mut doc = Document::parse(
            "<html><body><section>a</section><section>b</section></body></html>",
        );
        apply(&mut doc, &reg, &no_detections());
        let mut counted = 0;
        for n in doc.root.descendants() {
            if let Some(el) = n.as_element() {
                if el.tag == "n-section" {
                    assert_eq!(el.get_attribute("data-n-from"), Some("section"));
                    assert_eq!(el.get_attribute("data-n-rule"), Some("rule-x"));
                    counted += 1;
                }
            }
        }
        assert_eq!(counted, 2);
    }

    #[test]
    fn empty_registry_is_noop() {
        let reg = NormalizeRegistry::new();
        let mut doc = Document::parse("<html><body><article>x</article></body></html>");
        let report = apply(&mut doc, &reg, &no_detections());
        assert_eq!(report.applied(), 0);
        // Original <article> intact.
        let tags: Vec<String> = doc
            .root
            .descendants()
            .filter_map(|n| n.as_element().map(|e| e.tag.clone()))
            .collect();
        assert!(tags.iter().any(|t| t == "article"));
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

    #[test]
    fn set_attrs_adds_new_attribute_on_rewrite() {
        // Outbound emit pattern: canonical n-card → shadcn-shaped div.
        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "emit-shadcn-card".into(),
            framework: None,
            selector: "n-card".into(),
            rename_to: "div".into(),
            set_attrs: vec![
                ("data-slot".into(), "card".into()),
                ("role".into(), "article".into()),
            ],
            remove_attrs: vec![],
            description: None,
        });
        let mut doc = Document::parse("<html><body><n-card>x</n-card></body></html>");
        let report = apply(&mut doc, &reg, &no_detections());
        assert_eq!(report.applied(), 1);
        let mut found = false;
        for n in doc.root.descendants() {
            if let Some(el) = n.as_element() {
                if el.tag == "div"
                    && el.get_attribute("data-slot") == Some("card")
                    && el.get_attribute("role") == Some("article")
                {
                    found = true;
                }
            }
        }
        assert!(found, "emit rule didn't add shadcn attributes");
    }

    #[test]
    fn remove_attrs_strips_specified_keys() {
        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "strip-testids".into(),
            framework: None,
            selector: "[data-testid]".into(),
            rename_to: "section".into(),
            set_attrs: vec![],
            remove_attrs: vec!["data-testid".into(), "data-telemetry".into()],
            description: None,
        });
        let mut doc = Document::parse(
            r#"<html><body><section data-testid="x" data-telemetry="y" class="k">ok</section></body></html>"#,
        );
        let report = apply(&mut doc, &reg, &no_detections());
        assert_eq!(report.applied(), 1);
        for n in doc.root.descendants() {
            if let Some(el) = n.as_element() {
                if el.tag == "section" && el.get_attribute("class") == Some("k") {
                    assert!(el.get_attribute("data-testid").is_none());
                    assert!(el.get_attribute("data-telemetry").is_none());
                }
            }
        }
    }

    #[test]
    fn attr_only_rule_matches_without_renaming() {
        // When rename_to == current tag, rule still fires if attr work
        // is requested — enables "add semantic aria attrs" rules.
        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "aria-article".into(),
            framework: None,
            selector: "n-article".into(),
            rename_to: "n-article".into(),
            set_attrs: vec![("aria-label".into(), "article".into())],
            remove_attrs: vec![],
            description: None,
        });
        let mut doc = Document::parse("<html><body><n-article>x</n-article></body></html>");
        let report = apply(&mut doc, &reg, &no_detections());
        assert_eq!(report.applied(), 1);
        let mut found = false;
        for n in doc.root.descendants() {
            if let Some(el) = n.as_element() {
                if el.tag == "n-article"
                    && el.get_attribute("aria-label") == Some("article")
                {
                    found = true;
                }
            }
        }
        assert!(found);
    }

    #[test]
    fn roundtrip_html5_to_canonical_to_shadcn() {
        // Two-pass framework conversion: generic <article> → n-article
        // → shadcn-style <div data-slot=article>. End-to-end proof.
        let mut reg = NormalizeRegistry::new();
        // Pass 1: inbound.
        reg.insert(NormalizeSpec {
            name: "html5-article-in".into(),
            framework: None,
            selector: "article".into(),
            rename_to: "n-article".into(),
            set_attrs: vec![],
            remove_attrs: vec![],
            description: None,
        });
        // Pass 2: outbound emit.
        reg.insert(NormalizeSpec {
            name: "shadcn-article-out".into(),
            framework: None,
            selector: "n-article".into(),
            rename_to: "div".into(),
            set_attrs: vec![("data-slot".into(), "article".into())],
            remove_attrs: vec![],
            description: None,
        });
        let mut doc = Document::parse("<html><body><article>hi</article></body></html>");
        let report = apply(&mut doc, &reg, &no_detections());
        // article→n-article, then n-article→div. Two hits.
        assert_eq!(report.applied(), 2);
        let mut found_div = false;
        let mut saw_article = false;
        for n in doc.root.descendants() {
            if let Some(el) = n.as_element() {
                if el.tag == "article" {
                    saw_article = true;
                }
                if el.tag == "div" && el.get_attribute("data-slot") == Some("article") {
                    found_div = true;
                }
            }
        }
        assert!(!saw_article, "source article should have been rewritten");
        assert!(found_div, "target shadcn-shaped div not emitted");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_set_attrs_and_remove_attrs() {
        let src = r#"
            (defnormalize :name "emit-shadcn-card"
                          :selector "n-card"
                          :rename-to "div"
                          :set-attrs (("data-slot" "card") ("role" "article"))
                          :remove-attrs ("data-testid" "data-telemetry"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].set_attrs.len(), 2);
        assert_eq!(specs[0].set_attrs[0].0, "data-slot");
        assert_eq!(specs[0].set_attrs[0].1, "card");
        assert_eq!(specs[0].remove_attrs.len(), 2);
        assert_eq!(specs[0].remove_attrs[0], "data-testid");
    }
}
