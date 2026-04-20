//! `(defdom-diff)` — Lisp-native DOM diffing.
//!
//! **Novel** — every JS framework has virtual-DOM diffs under the
//! hood, but no browser exposes DOM diffs as a TYPED, declaratively
//! rulable domain. This DSL declares *which* subtrees to watch,
//! *which* ops to emit, *when* to throttle, *what* attrs to ignore,
//! and lets the author observe a stream of typed `DomOp` values —
//! InsertNode / RemoveNode / ReplaceNode / SetAttr / RemoveAttr /
//! SetText — rooted at a stable `DomPath`.
//!
//! The diff algorithm itself is a pure function over an
//! `AbstractNode` tree (tag + attrs + children + text) — self-
//! contained, no dependencies on `dom::Document`. A consumer snapshots
//! `AbstractNode` trees before + after whatever mutation, calls
//! `diff(&before, &after)` and gets back a canonical `DomDiff`.
//!
//! ```lisp
//! (defdom-diff :name "watch-forms"
//!              :host "*"
//!              :watch-tags ("form" "input" "textarea")
//!              :emit-ops (insert-node remove-node set-attr set-text)
//!              :ignore-attrs ("data-react-id" "data-ssr-tick")
//!              :max-ops-per-second 60
//!              :stop-at-depth 8)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Which kind of DOM mutation a `DomOp` represents.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum DomOpKind {
    InsertNode,
    RemoveNode,
    ReplaceNode,
    SetAttr,
    RemoveAttr,
    SetText,
}

/// Path from the root of the diffed tree to a node. Each `usize` is
/// the child index at that depth. Empty = the root itself.
pub type DomPath = Vec<usize>;

/// One canonical DOM mutation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DomOp {
    InsertNode {
        path: DomPath,
        tag: String,
    },
    RemoveNode {
        path: DomPath,
        tag: String,
    },
    /// Entire node replaced (tag changed at the same position).
    ReplaceNode {
        path: DomPath,
        old_tag: String,
        new_tag: String,
    },
    SetAttr {
        path: DomPath,
        name: String,
        value: String,
    },
    RemoveAttr {
        path: DomPath,
        name: String,
    },
    /// Text content (combined-child text) changed.
    SetText {
        path: DomPath,
        value: String,
    },
}

impl DomOp {
    #[must_use]
    pub fn kind(&self) -> DomOpKind {
        match self {
            DomOp::InsertNode { .. } => DomOpKind::InsertNode,
            DomOp::RemoveNode { .. } => DomOpKind::RemoveNode,
            DomOp::ReplaceNode { .. } => DomOpKind::ReplaceNode,
            DomOp::SetAttr { .. } => DomOpKind::SetAttr,
            DomOp::RemoveAttr { .. } => DomOpKind::RemoveAttr,
            DomOp::SetText { .. } => DomOpKind::SetText,
        }
    }

    #[must_use]
    pub fn path(&self) -> &DomPath {
        match self {
            DomOp::InsertNode { path, .. }
            | DomOp::RemoveNode { path, .. }
            | DomOp::ReplaceNode { path, .. }
            | DomOp::SetAttr { path, .. }
            | DomOp::RemoveAttr { path, .. }
            | DomOp::SetText { path, .. } => path,
        }
    }
}

/// Canonical diff result.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomDiff {
    pub ops: Vec<DomOp>,
}

impl DomDiff {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    #[must_use]
    pub fn count_of(&self, kind: DomOpKind) -> usize {
        self.ops.iter().filter(|o| o.kind() == kind).count()
    }
}

/// Minimal abstract DOM node the diffing algorithm operates on.
/// Decoupled from `dom::Document` so the algorithm is pure + testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbstractNode {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<AbstractNode>,
}

impl AbstractNode {
    #[must_use]
    pub fn new(tag: &str) -> Self {
        Self {
            tag: tag.to_owned(),
            attrs: vec![],
            text: String::new(),
            children: vec![],
        }
    }

    #[must_use]
    pub fn with_attr(mut self, k: &str, v: &str) -> Self {
        self.attrs.push((k.to_owned(), v.to_owned()));
        self
    }

    #[must_use]
    pub fn with_text(mut self, t: &str) -> Self {
        self.text = t.to_owned();
        self
    }

    #[must_use]
    pub fn with_child(mut self, c: AbstractNode) -> Self {
        self.children.push(c);
        self
    }
}

/// Diffing knobs — what to skip, where to stop.
#[derive(Debug, Clone, Default)]
pub struct DiffConfig {
    /// Skip these attribute names (e.g. framework-generated noise).
    pub ignore_attrs: Vec<String>,
    /// Skip nodes whose tag is in this list (e.g. `<script>`).
    pub ignore_tags: Vec<String>,
    /// Stop descending past this depth. `None` = unlimited.
    pub stop_at_depth: Option<u32>,
}

/// Canonical pure diff: compare `before` and `after` tree, produce
/// a `DomDiff`. Order of ops is deterministic (depth-first, same-
/// level index ascending).
#[must_use]
pub fn diff(before: &AbstractNode, after: &AbstractNode, cfg: &DiffConfig) -> DomDiff {
    let mut ops = Vec::new();
    diff_into(&[], before, after, cfg, 0, &mut ops);
    DomDiff { ops }
}

fn diff_into(
    path: &[usize],
    a: &AbstractNode,
    b: &AbstractNode,
    cfg: &DiffConfig,
    depth: u32,
    ops: &mut Vec<DomOp>,
) {
    if cfg.ignore_tags.iter().any(|t| t == &a.tag) && cfg.ignore_tags.iter().any(|t| t == &b.tag) {
        return;
    }
    let p = path.to_vec();
    if a.tag != b.tag {
        ops.push(DomOp::ReplaceNode {
            path: p,
            old_tag: a.tag.clone(),
            new_tag: b.tag.clone(),
        });
        return;
    }
    diff_attrs(&p, a, b, cfg, ops);
    if a.text != b.text {
        ops.push(DomOp::SetText {
            path: p.clone(),
            value: b.text.clone(),
        });
    }
    if let Some(max) = cfg.stop_at_depth {
        if depth >= max {
            return;
        }
    }
    let la = a.children.len();
    let lb = b.children.len();
    let shared = la.min(lb);
    for i in 0..shared {
        let mut child_path = p.clone();
        child_path.push(i);
        diff_into(&child_path, &a.children[i], &b.children[i], cfg, depth + 1, ops);
    }
    // Extra children on the after side → inserted.
    for i in la..lb {
        let mut ip = p.clone();
        ip.push(i);
        ops.push(DomOp::InsertNode {
            path: ip,
            tag: b.children[i].tag.clone(),
        });
    }
    // Extra children on the before side → removed.
    for i in lb..la {
        let mut rp = p.clone();
        rp.push(i);
        ops.push(DomOp::RemoveNode {
            path: rp,
            tag: a.children[i].tag.clone(),
        });
    }
}

fn diff_attrs(
    path: &DomPath,
    a: &AbstractNode,
    b: &AbstractNode,
    cfg: &DiffConfig,
    ops: &mut Vec<DomOp>,
) {
    let ignored = |name: &str| cfg.ignore_attrs.iter().any(|x| x == name);
    // Present in `a`, missing or changed in `b`.
    for (k, va) in &a.attrs {
        if ignored(k) {
            continue;
        }
        match b.attrs.iter().find(|(bk, _)| bk == k) {
            None => ops.push(DomOp::RemoveAttr {
                path: path.clone(),
                name: k.clone(),
            }),
            Some((_, vb)) if vb != va => ops.push(DomOp::SetAttr {
                path: path.clone(),
                name: k.clone(),
                value: vb.clone(),
            }),
            _ => {}
        }
    }
    // New attrs in `b`.
    for (k, vb) in &b.attrs {
        if ignored(k) {
            continue;
        }
        if !a.attrs.iter().any(|(ak, _)| ak == k) {
            ops.push(DomOp::SetAttr {
                path: path.clone(),
                name: k.clone(),
                value: vb.clone(),
            });
        }
    }
}

/// Declarative DOM-diff watcher spec.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defdom-diff"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DomDiffSpec {
    pub name: String,
    /// Host glob — `"*"` watches every page.
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    /// CSS-tag names the watcher restricts to. Empty = all tags.
    #[serde(default)]
    pub watch_tags: Vec<String>,
    /// Op kinds to emit. Empty = all.
    #[serde(default)]
    pub emit_ops: Vec<DomOpKind>,
    /// Attribute names to ignore (noise filters — framework-generated
    /// data-* keys, etc.).
    #[serde(default)]
    pub ignore_attrs: Vec<String>,
    /// Tag names to ignore entirely (never traversed).
    #[serde(default)]
    pub ignore_tags: Vec<String>,
    /// Stop descending past this depth. `0` = unlimited.
    #[serde(default)]
    pub stop_at_depth: u32,
    /// Max ops per second the watcher will deliver. `0` = unlimited.
    #[serde(default)]
    pub max_ops_per_second: u32,
    /// Forward emitted ops to (defaudit-trail) as a DomMutation event.
    #[serde(default)]
    pub forward_to_audit: bool,
    /// Privacy-first: disabled until explicitly opted-in.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_enabled() -> bool {
    false
}

impl DomDiffSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            watch_tags: vec![],
            emit_ops: vec![],
            ignore_attrs: vec![],
            ignore_tags: vec!["script".into(), "style".into()],
            stop_at_depth: 0,
            max_ops_per_second: 0,
            forward_to_audit: false,
            enabled: false,
            description: Some(
                "DOM-diff watcher — disabled (privacy-first). Enable in rc file.".into(),
            ),
        }
    }

    #[must_use]
    pub fn watchful_forms() -> Self {
        Self {
            name: "watchful-forms".into(),
            enabled: true,
            watch_tags: vec!["form".into(), "input".into(), "textarea".into(), "select".into()],
            emit_ops: vec![
                DomOpKind::InsertNode,
                DomOpKind::RemoveNode,
                DomOpKind::SetAttr,
                DomOpKind::SetText,
            ],
            ignore_attrs: vec!["data-react-id".into(), "data-ssr-tick".into()],
            max_ops_per_second: 60,
            forward_to_audit: true,
            description: Some(
                "Watch form inputs for attribute / value / child changes at 60 ops/sec.".into(),
            ),
            ..Self::default_profile()
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    #[must_use]
    pub fn watches_tag(&self, tag: &str) -> bool {
        self.watch_tags.is_empty() || self.watch_tags.iter().any(|t| t == tag)
    }

    #[must_use]
    pub fn emits(&self, kind: DomOpKind) -> bool {
        self.emit_ops.is_empty() || self.emit_ops.contains(&kind)
    }

    /// Projected `DiffConfig` for the diff algorithm.
    #[must_use]
    pub fn as_diff_config(&self) -> DiffConfig {
        DiffConfig {
            ignore_attrs: self.ignore_attrs.clone(),
            ignore_tags: self.ignore_tags.clone(),
            stop_at_depth: (self.stop_at_depth > 0).then_some(self.stop_at_depth),
        }
    }

    /// Filter a raw diff down to just the ops this spec wants to emit.
    #[must_use]
    pub fn filter_ops(&self, diff: &DomDiff) -> Vec<DomOp> {
        diff.ops
            .iter()
            .filter(|op| self.emits(op.kind()))
            .cloned()
            .collect()
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct DomDiffRegistry {
    specs: Vec<DomDiffSpec>,
}

impl DomDiffRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: DomDiffSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = DomDiffSpec>) {
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

    #[must_use]
    pub fn specs(&self) -> &[DomDiffSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&DomDiffSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<DomDiffSpec>, String> {
    tatara_lisp::compile_typed::<DomDiffSpec>(src)
        .map_err(|e| format!("failed to compile defdom-diff forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<DomDiffSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> DiffConfig {
        DiffConfig::default()
    }

    #[test]
    fn identical_trees_produce_empty_diff() {
        let a = AbstractNode::new("div")
            .with_attr("class", "x")
            .with_text("hello");
        let b = a.clone();
        let d = diff(&a, &b, &cfg());
        assert!(d.is_empty());
    }

    #[test]
    fn tag_change_emits_replace() {
        let a = AbstractNode::new("div");
        let b = AbstractNode::new("span");
        let d = diff(&a, &b, &cfg());
        assert_eq!(d.len(), 1);
        assert_eq!(d.ops[0].kind(), DomOpKind::ReplaceNode);
        assert_eq!(d.ops[0].path(), &Vec::<usize>::new());
    }

    #[test]
    fn attr_add_and_remove() {
        let a = AbstractNode::new("div").with_attr("id", "a");
        let b = AbstractNode::new("div").with_attr("class", "b");
        let d = diff(&a, &b, &cfg());
        assert_eq!(d.count_of(DomOpKind::RemoveAttr), 1);
        assert_eq!(d.count_of(DomOpKind::SetAttr), 1);
    }

    #[test]
    fn attr_change_emits_set() {
        let a = AbstractNode::new("input").with_attr("value", "old");
        let b = AbstractNode::new("input").with_attr("value", "new");
        let d = diff(&a, &b, &cfg());
        assert_eq!(d.len(), 1);
        match &d.ops[0] {
            DomOp::SetAttr { name, value, .. } => {
                assert_eq!(name, "value");
                assert_eq!(value, "new");
            }
            o => panic!("expected SetAttr, got {o:?}"),
        }
    }

    #[test]
    fn ignore_attrs_masks_noise() {
        let a = AbstractNode::new("div").with_attr("data-react-id", "0.1");
        let b = AbstractNode::new("div").with_attr("data-react-id", "0.2");
        let c = DiffConfig {
            ignore_attrs: vec!["data-react-id".into()],
            ..DiffConfig::default()
        };
        assert!(diff(&a, &b, &c).is_empty());
    }

    #[test]
    fn child_insert_at_end() {
        let a = AbstractNode::new("ul").with_child(AbstractNode::new("li"));
        let b = AbstractNode::new("ul")
            .with_child(AbstractNode::new("li"))
            .with_child(AbstractNode::new("li"));
        let d = diff(&a, &b, &cfg());
        assert_eq!(d.count_of(DomOpKind::InsertNode), 1);
        assert_eq!(d.ops[0].path(), &vec![1]);
    }

    #[test]
    fn child_remove_at_end() {
        let a = AbstractNode::new("ul")
            .with_child(AbstractNode::new("li"))
            .with_child(AbstractNode::new("li"));
        let b = AbstractNode::new("ul").with_child(AbstractNode::new("li"));
        let d = diff(&a, &b, &cfg());
        assert_eq!(d.count_of(DomOpKind::RemoveNode), 1);
    }

    #[test]
    fn text_change_emits_set_text() {
        let a = AbstractNode::new("p").with_text("old");
        let b = AbstractNode::new("p").with_text("new");
        let d = diff(&a, &b, &cfg());
        assert_eq!(d.len(), 1);
        match &d.ops[0] {
            DomOp::SetText { value, .. } => assert_eq!(value, "new"),
            o => panic!("expected SetText, got {o:?}"),
        }
    }

    #[test]
    fn stop_at_depth_truncates() {
        let a = AbstractNode::new("div").with_child(AbstractNode::new("p").with_text("old"));
        let b = AbstractNode::new("div").with_child(AbstractNode::new("p").with_text("new"));
        let c = DiffConfig {
            stop_at_depth: Some(0),
            ..DiffConfig::default()
        };
        // Depth-0 won't descend into the child pair.
        assert!(diff(&a, &b, &c).is_empty());
    }

    #[test]
    fn diff_is_deterministic() {
        let a = AbstractNode::new("div").with_attr("a", "1").with_attr("b", "2");
        let b = AbstractNode::new("div").with_attr("a", "1").with_attr("b", "3");
        let d1 = diff(&a, &b, &cfg());
        let d2 = diff(&a, &b, &cfg());
        assert_eq!(d1, d2);
    }

    #[test]
    fn spec_watches_tag_filters_correctly() {
        let s = DomDiffSpec::watchful_forms();
        assert!(s.watches_tag("form"));
        assert!(s.watches_tag("input"));
        assert!(!s.watches_tag("div"));
    }

    #[test]
    fn spec_emits_ops_filters_correctly() {
        let s = DomDiffSpec::watchful_forms();
        assert!(s.emits(DomOpKind::InsertNode));
        assert!(!s.emits(DomOpKind::ReplaceNode));
    }

    #[test]
    fn spec_filter_ops_applies_emit_whitelist() {
        let s = DomDiffSpec {
            emit_ops: vec![DomOpKind::SetText],
            ..DomDiffSpec::default_profile()
        };
        let d = DomDiff {
            ops: vec![
                DomOp::SetText {
                    path: vec![],
                    value: "x".into(),
                },
                DomOp::InsertNode {
                    path: vec![0],
                    tag: "p".into(),
                },
            ],
        };
        let filtered = s.filter_ops(&d);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].kind(), DomOpKind::SetText);
    }

    #[test]
    fn spec_as_diff_config_projects_correctly() {
        let s = DomDiffSpec {
            ignore_attrs: vec!["data-x".into()],
            ignore_tags: vec!["script".into()],
            stop_at_depth: 5,
            ..DomDiffSpec::default_profile()
        };
        let c = s.as_diff_config();
        assert_eq!(c.ignore_attrs, vec!["data-x".to_owned()]);
        assert_eq!(c.ignore_tags, vec!["script".to_owned()]);
        assert_eq!(c.stop_at_depth, Some(5));
    }

    #[test]
    fn default_profile_is_disabled_for_privacy() {
        let s = DomDiffSpec::default_profile();
        assert!(!s.enabled);
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = DomDiffRegistry::new();
        reg.insert(DomDiffSpec::watchful_forms());
        reg.insert(DomDiffSpec {
            name: "gh".into(),
            host: "*://github.com/*".into(),
            enabled: true,
            ..DomDiffSpec::default_profile()
        });
        assert_eq!(reg.resolve("github.com").unwrap().name, "gh");
        assert_eq!(reg.resolve("example.org").unwrap().name, "watchful-forms");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = DomDiffRegistry::new();
        reg.insert(DomDiffSpec::default_profile());
        assert!(reg.resolve("example.com").is_none());
    }

    #[test]
    fn op_kind_roundtrips_through_serde() {
        for k in [
            DomOpKind::InsertNode,
            DomOpKind::RemoveNode,
            DomOpKind::ReplaceNode,
            DomOpKind::SetAttr,
            DomOpKind::RemoveAttr,
            DomOpKind::SetText,
        ] {
            let s = DomDiffSpec {
                emit_ops: vec![k],
                ..DomDiffSpec::default_profile()
            };
            let j = serde_json::to_string(&s).unwrap();
            let b: DomDiffSpec = serde_json::from_str(&j).unwrap();
            assert_eq!(b.emit_ops, vec![k]);
        }
    }

    #[test]
    fn dom_op_serde_tag_is_kind() {
        let op = DomOp::SetText {
            path: vec![0, 1],
            value: "hello".into(),
        };
        let j = serde_json::to_string(&op).unwrap();
        assert!(j.contains(r#""kind":"set-text""#));
        let b: DomOp = serde_json::from_str(&j).unwrap();
        assert_eq!(b, op);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_dom_diff_form() {
        let src = r#"
            (defdom-diff :name "forms"
                         :host "*"
                         :enabled #t
                         :watch-tags ("form" "input")
                         :emit-ops ("set-attr" "set-text")
                         :max-ops-per-second 60)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert!(s.enabled);
        assert_eq!(s.watch_tags, vec!["form".to_owned(), "input".to_owned()]);
        assert!(s.emits(DomOpKind::SetAttr));
        assert!(!s.emits(DomOpKind::ReplaceNode));
    }
}
