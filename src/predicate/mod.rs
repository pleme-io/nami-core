//! Lisp-authored DOM predicates.
//!
//! A predicate is a named boolean check over a document + its detected
//! frameworks + embedded state. Agents, plans, and transforms can
//! reference predicates to gate their actions on observable page
//! conditions without hand-rolling evaluation logic.
//!
//! Four body shapes (user fills exactly one per predicate):
//!
//! ```lisp
//! ; selector match — optionally with count bounds
//! (defpredicate :name "likely-article"
//!               :selector "article"
//!               :min 1)
//!
//! ; "no ads on page"
//! (defpredicate :name "ad-free"
//!               :selector ".ad"
//!               :max 0)
//!
//! ; specific framework was detected (anywhere, any confidence)
//! (defpredicate :name "is-shadcn"
//!               :framework "shadcn-radix")
//!
//! ; specific embedded state blob present
//! (defpredicate :name "has-next-data"
//!               :state-kind "next-data")
//!
//!
//! ; composition — references OTHER predicates by name
//! (defpredicate :name "clean-reading"
//!               :all ("likely-article" "ad-free"))
//!
//! (defpredicate :name "problematic"
//!               :any ("has-paywall" "has-tracker-heavy"))
//!
//! (defpredicate :name "first-party"
//!               :none ("has-gtm" "has-shopify"))
//! ```
//!
//! Exactly one of `:selector | :framework | :state-kind | :all | :any
//! | :none` should be set per predicate. The evaluator picks the
//! first one present (in that order). Composition references are
//! recursively resolved + cycle-detected.
//!
//! Pure Lisp-layer abstraction: Rust owns evaluation, Lisp owns the
//! composition surface.

use crate::dom::Document;
use crate::framework::Detection;
use crate::selector::Selector;
use crate::state::StateBlob;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// A named predicate. Exactly one of the body fields should be set.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defpredicate"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PredicateSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,

    // ── body: match a selector, with optional count bounds ──
    #[serde(default)]
    pub selector: Option<String>,
    /// Minimum number of matches (inclusive). Default 1 if `selector`
    /// is set and `min` isn't.
    #[serde(default)]
    pub min: Option<usize>,
    /// Maximum number of matches (inclusive).
    #[serde(default)]
    pub max: Option<usize>,

    // ── body: framework present by kebab-case name ──
    #[serde(default)]
    pub framework: Option<String>,

    // ── body: embedded state blob of a given kind present ──
    #[serde(default)]
    pub state_kind: Option<String>,

    // ── body: composition over predicate names ──
    #[serde(default)]
    pub all: Vec<String>,
    #[serde(default)]
    pub any: Vec<String>,
    #[serde(default)]
    pub none: Vec<String>,
}

/// Which of the body fields is populated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Body {
    Match,
    Framework,
    State,
    All,
    Any,
    None,
    Empty,
}

impl PredicateSpec {
    #[must_use]
    pub fn body(&self) -> Body {
        if self.selector.is_some() {
            Body::Match
        } else if self.framework.is_some() {
            Body::Framework
        } else if self.state_kind.is_some() {
            Body::State
        } else if !self.all.is_empty() {
            Body::All
        } else if !self.any.is_empty() {
            Body::Any
        } else if !self.none.is_empty() {
            Body::None
        } else {
            Body::Empty
        }
    }
}

/// Everything an evaluator needs about the page under test.
///
/// Borrows are cheap + all inputs are derivable from a single parse
/// (`Document::parse`, `framework::detect`, `state::extract`), so
/// callers typically prepare them once per page and evaluate many
/// predicates against the same context.
pub struct EvalContext<'a> {
    pub doc: &'a Document,
    pub detections: &'a [Detection],
    pub state: &'a [StateBlob],
}

/// Index of predicates by name.
#[derive(Debug, Clone, Default)]
pub struct PredicateRegistry {
    specs: Vec<PredicateSpec>,
}

impl PredicateRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: PredicateSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = PredicateSpec>) {
        for s in specs {
            self.insert(s);
        }
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&PredicateSpec> {
        self.specs.iter().find(|p| p.name == name)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// Evaluate a named predicate. Unknown name → `Err`. Cycle in
    /// references → `Err`.
    pub fn evaluate(&self, name: &str, cx: &EvalContext<'_>) -> Result<bool, String> {
        let mut visiting: HashSet<&str> = HashSet::new();
        self.eval_named(name, cx, &mut visiting)
    }

    fn eval_named<'a>(
        &'a self,
        name: &'a str,
        cx: &EvalContext<'_>,
        visiting: &mut HashSet<&'a str>,
    ) -> Result<bool, String> {
        let spec = self
            .get(name)
            .ok_or_else(|| format!("unknown predicate: {name}"))?;
        if !visiting.insert(spec.name.as_str()) {
            return Err(format!(
                "predicate cycle: {name} already in evaluation path"
            ));
        }
        let result = self.eval_spec(spec, cx, visiting);
        visiting.remove(spec.name.as_str());
        result
    }

    fn eval_spec<'a>(
        &'a self,
        spec: &'a PredicateSpec,
        cx: &EvalContext<'_>,
        visiting: &mut HashSet<&'a str>,
    ) -> Result<bool, String> {
        match spec.body() {
            Body::Match => {
                let sel_str = spec.selector.as_deref().unwrap();
                let parsed = Selector::parse(sel_str)
                    .map_err(|e| format!("{name}: {e}", name = spec.name))?;
                let count = count_matches(&parsed, cx.doc);
                let min = spec.min.unwrap_or(1);
                if count < min {
                    return Ok(false);
                }
                if let Some(max) = spec.max {
                    if count > max {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Body::Framework => {
                let want = spec.framework.as_deref().unwrap();
                // Match either the display name (`"shadcn/radix"`) or
                // the serde kebab-case name (`"shadcn-radix"`) so
                // users can write whichever is ergonomic.
                Ok(cx.detections.iter().any(|d| {
                    if d.framework.name() == want {
                        return true;
                    }
                    let serde_name = serde_json::to_value(d.framework)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned));
                    serde_name.is_some_and(|s| s == want)
                }))
            }
            Body::State => {
                let want = spec.state_kind.as_deref().unwrap();
                let want_kebab = want.to_ascii_lowercase();
                Ok(cx.state.iter().any(|b| {
                    let as_kind = serde_json::to_value(b.kind)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned));
                    as_kind.map(|s| s == want_kebab).unwrap_or(false)
                }))
            }
            Body::All => {
                for ref_name in &spec.all {
                    if !self.eval_named(ref_name, cx, visiting)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Body::Any => {
                for ref_name in &spec.any {
                    if self.eval_named(ref_name, cx, visiting)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Body::None => {
                for ref_name in &spec.none {
                    if self.eval_named(ref_name, cx, visiting)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Body::Empty => Err(format!(
                "predicate {} has no body — set exactly one of :selector | :framework | :state-kind | :all | :any | :none",
                spec.name
            )),
        }
    }
}

/// Walk the document, counting elements that satisfy the selector.
fn count_matches(selector: &Selector, doc: &Document) -> usize {
    let mut count = 0;
    let mut path: Vec<PathItem> = Vec::new();
    count_walk(&doc.root, selector, &mut path, &mut count);
    count
}

fn count_walk(
    node: &crate::dom::Node,
    selector: &Selector,
    path: &mut Vec<PathItem>,
    count: &mut usize,
) {
    let pushed = if let crate::dom::NodeData::Element(el) = &node.data {
        path.push(PathItem::from_element(el));
        true
    } else {
        false
    };

    // Test THIS node as the leaf of the path.
    if matches!(node.data, crate::dom::NodeData::Element(_)) {
        let path_refs: Vec<&PathItem> = path.iter().collect();
        if selector.matches(&path_refs) {
            *count += 1;
        }
    }

    for child in &node.children {
        count_walk(child, selector, path, count);
    }

    if pushed {
        path.pop();
    }
}

// Canonical ancestor-path element lives in `selector::OwnedContext`.
type PathItem = crate::selector::OwnedContext;

/// Compile a Lisp document of `(defpredicate …)` forms.
#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<PredicateSpec>, String> {
    tatara_lisp::compile_typed::<PredicateSpec>(src).map_err(|e| format!("{e}"))
}

/// Register the `defpredicate` keyword in the global tatara registry.
#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<PredicateSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::Framework;
    use crate::state::StateKind;

    fn ctx<'a>(
        doc: &'a Document,
        detections: &'a [Detection],
        state: &'a [StateBlob],
    ) -> EvalContext<'a> {
        EvalContext {
            doc,
            detections,
            state,
        }
    }

    fn detection(framework: Framework, confidence: f32) -> Detection {
        Detection {
            framework,
            name: framework.name(),
            confidence,
            evidence: vec![],
        }
    }

    fn spec_match(name: &str, sel: &str, min: Option<usize>, max: Option<usize>) -> PredicateSpec {
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

    fn spec_framework(name: &str, fw: &str) -> PredicateSpec {
        PredicateSpec {
            name: name.into(),
            description: None,
            selector: None,
            min: None,
            max: None,
            framework: Some(fw.into()),
            state_kind: None,
            all: vec![],
            any: vec![],
            none: vec![],
        }
    }

    fn spec_composite(name: &str, all: &[&str], any: &[&str], none: &[&str]) -> PredicateSpec {
        PredicateSpec {
            name: name.into(),
            description: None,
            selector: None,
            min: None,
            max: None,
            framework: None,
            state_kind: None,
            all: all.iter().map(|s| s.to_string()).collect(),
            any: any.iter().map(|s| s.to_string()).collect(),
            none: none.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn selector_min_one_is_default() {
        let doc = Document::parse(r#"<html><body><article>…</article></body></html>"#);
        let mut reg = PredicateRegistry::new();
        reg.insert(spec_match("has-article", "article", None, None));
        assert!(reg.evaluate("has-article", &ctx(&doc, &[], &[])).unwrap());
    }

    #[test]
    fn selector_min_threshold() {
        let doc = Document::parse(r#"<html><body><p>a</p><p>b</p></body></html>"#);
        let mut reg = PredicateRegistry::new();
        reg.insert(spec_match("two-plus-ps", "p", Some(2), None));
        reg.insert(spec_match("three-plus-ps", "p", Some(3), None));
        let cx = ctx(&doc, &[], &[]);
        assert!(reg.evaluate("two-plus-ps", &cx).unwrap());
        assert!(!reg.evaluate("three-plus-ps", &cx).unwrap());
    }

    #[test]
    fn selector_max_threshold() {
        let doc = Document::parse(r#"<html><body><div class="ad"></div></body></html>"#);
        let mut reg = PredicateRegistry::new();
        reg.insert(spec_match("ad-free", ".ad", Some(0), Some(0)));
        assert!(!reg.evaluate("ad-free", &ctx(&doc, &[], &[])).unwrap());

        let clean = Document::parse(r#"<html><body><p>clean</p></body></html>"#);
        assert!(reg.evaluate("ad-free", &ctx(&clean, &[], &[])).unwrap());
    }

    #[test]
    fn framework_predicate_checks_detection_list() {
        let doc = Document::parse("<html><body></body></html>");
        let mut reg = PredicateRegistry::new();
        reg.insert(spec_framework("is-shadcn", "shadcn-radix"));
        let shadcn = [detection(Framework::ShadcnRadix, 0.8)];
        let tailwind = [detection(Framework::Tailwind, 0.8)];
        let yes = ctx(&doc, &shadcn, &[]);
        let no = ctx(&doc, &tailwind, &[]);
        assert!(reg.evaluate("is-shadcn", &yes).unwrap());
        assert!(!reg.evaluate("is-shadcn", &no).unwrap());
    }

    #[test]
    fn state_kind_predicate_checks_state_list() {
        let doc = Document::parse("<html><body></body></html>");
        let blob = StateBlob {
            kind: StateKind::NextData,
            id: Some("__NEXT_DATA__".into()),
            value: None,
            raw: None,
            bytes: 0,
        };
        let mut reg = PredicateRegistry::new();
        let mut spec = spec_match("_", "_", None, None);
        spec.selector = None;
        spec.state_kind = Some("next-data".into());
        spec.name = "has-next-data".into();
        reg.insert(spec);
        assert!(
            reg.evaluate(
                "has-next-data",
                &ctx(&doc, &[], std::slice::from_ref(&blob))
            )
            .unwrap()
        );
        assert!(!reg.evaluate("has-next-data", &ctx(&doc, &[], &[])).unwrap());
    }

    #[test]
    fn composition_all_and_any() {
        let doc = Document::parse(r#"<html><body><article>…</article></body></html>"#);
        let mut reg = PredicateRegistry::new();
        reg.insert(spec_match("has-article", "article", None, None));
        reg.insert(spec_match("has-p", "p", None, None));
        reg.insert(spec_composite(
            "article-and-p",
            &["has-article", "has-p"],
            &[],
            &[],
        ));
        reg.insert(spec_composite(
            "article-or-p",
            &[],
            &["has-article", "has-p"],
            &[],
        ));

        let cx = ctx(&doc, &[], &[]);
        assert!(!reg.evaluate("article-and-p", &cx).unwrap());
        assert!(reg.evaluate("article-or-p", &cx).unwrap());
    }

    #[test]
    fn composition_none() {
        let doc = Document::parse(r#"<html><body><article>…</article></body></html>"#);
        let mut reg = PredicateRegistry::new();
        reg.insert(spec_match("has-paywall", ".paywall", None, None));
        reg.insert(spec_match("has-ad", ".ad", None, None));
        reg.insert(spec_composite(
            "clean",
            &[],
            &[],
            &["has-paywall", "has-ad"],
        ));
        assert!(reg.evaluate("clean", &ctx(&doc, &[], &[])).unwrap());
    }

    #[test]
    fn unknown_predicate_errors() {
        let doc = Document::parse("<html></html>");
        let reg = PredicateRegistry::new();
        assert!(reg.evaluate("ghost", &ctx(&doc, &[], &[])).is_err());
    }

    #[test]
    fn cycle_detection() {
        let doc = Document::parse("<html></html>");
        let mut reg = PredicateRegistry::new();
        reg.insert(spec_composite("a", &["b"], &[], &[]));
        reg.insert(spec_composite("b", &["a"], &[], &[]));
        let err = reg.evaluate("a", &ctx(&doc, &[], &[])).unwrap_err();
        assert!(err.contains("cycle"));
    }

    #[test]
    fn empty_predicate_errors() {
        let doc = Document::parse("<html></html>");
        let mut reg = PredicateRegistry::new();
        reg.insert(PredicateSpec {
            name: "empty".into(),
            description: None,
            selector: None,
            min: None,
            max: None,
            framework: None,
            state_kind: None,
            all: vec![],
            any: vec![],
            none: vec![],
        });
        let err = reg.evaluate("empty", &ctx(&doc, &[], &[])).unwrap_err();
        assert!(err.contains("no body"));
    }

    #[test]
    fn reader_mode_composition_real_page_shape() {
        // likely-article = has-article AND min-500-chars (approximated by p count).
        let doc = Document::parse(
            r#"<html><body><article>
                 <h1>Title</h1>
                 <p>A sufficiently long paragraph. …</p>
                 <p>Another paragraph. …</p>
                 <p>And one more. …</p>
               </article></body></html>"#,
        );
        let mut reg = PredicateRegistry::new();
        reg.insert(spec_match("has-article", "article", None, None));
        reg.insert(spec_match("prose-heavy", "p", Some(3), None));
        reg.insert(spec_composite(
            "likely-article",
            &["has-article", "prose-heavy"],
            &[],
            &[],
        ));
        assert!(
            reg.evaluate("likely-article", &ctx(&doc, &[], &[]))
                .unwrap()
        );
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_round_trip_predicate_specs() {
        let src = r#"
            (defpredicate :name "likely-article" :selector "article" :min 1)
            (defpredicate :name "ad-free"        :selector ".ad"     :max 0)
            (defpredicate :name "is-nextjs"      :framework "next.js")
            (defpredicate :name "has-next-data"  :state-kind "next-data")
            (defpredicate :name "clean-reading"  :all ("likely-article" "ad-free"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 5);
        assert_eq!(specs[0].selector.as_deref(), Some("article"));
        assert_eq!(specs[1].max, Some(0));
        assert_eq!(specs[2].framework.as_deref(), Some("next.js"));
        assert_eq!(specs[3].state_kind.as_deref(), Some("next-data"));
        assert_eq!(specs[4].all, vec!["likely-article", "ad-free"]);
    }
}
