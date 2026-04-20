//! `(defsplit)` — declarative split-view layouts.
//!
//! Absorbs Arc Split View, Vivaldi tile-tabs, Safari's two-pane
//! landscape mode. A split is a named N-pane layout; each pane
//! references a URL. The chrome tiles them along the declared axis,
//! with proportional sizes; focus moves between panes on command.
//!
//! ```lisp
//! (defsplit :name    "docs-side-by-side"
//!           :layout  :horizontal
//!           :panes   ((pane :url "https://docs-a" :weight 1)
//!                     (pane :url "https://docs-b" :weight 1))
//!           :focus   0)
//!
//! (defsplit :name    "three-col"
//!           :layout  :horizontal
//!           :panes   ((pane :url "https://a") (pane :url "https://b") (pane :url "https://c")))
//! ```
//!
//! `weight` is the pane's share of the axis (like flexbox `flex`).
//! Omitted weights default to 1, so three unweighted panes tile
//! equally.

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Split axis.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SplitLayout {
    /// Panes tile left-to-right, each gets a horizontal slice.
    Horizontal,
    /// Panes tile top-to-bottom.
    Vertical,
    /// 2×2 grid — up to 4 panes. Extra panes fall back to horizontal.
    Grid,
}

impl Default for SplitLayout {
    fn default() -> Self {
        Self::Horizontal
    }
}

/// One pane in a split.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SplitPane {
    /// URL loaded when the split activates.
    pub url: String,
    /// Flexbox-style weight — relative share of the axis. Defaults
    /// to 1. Must be > 0.
    #[serde(default = "default_weight")]
    pub weight: f32,
    /// Optional display title override — some sites have ugly titles.
    #[serde(default)]
    pub title: Option<String>,
}

fn default_weight() -> f32 {
    1.0
}

/// Split layout spec.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsplit"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SplitSpec {
    pub name: String,
    #[serde(default)]
    pub layout: SplitLayout,
    pub panes: Vec<SplitPane>,
    /// 0-based index of the initially focused pane. Clamped at
    /// resolve time.
    #[serde(default)]
    pub focus: usize,
    /// Persist the split across app restarts (associates with the
    /// active space in SubstratePipeline).
    #[serde(default = "default_persist")]
    pub persist: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_persist() -> bool {
    true
}

impl SplitSpec {
    /// Proportional pixel widths (or heights) for `axis_size`. Handles
    /// zero/negative weights by replacing them with 1. Returns an
    /// empty vec when `panes` is empty.
    #[must_use]
    pub fn proportional_sizes(&self, axis_size: f32) -> Vec<f32> {
        if self.panes.is_empty() {
            return Vec::new();
        }
        let cleaned: Vec<f32> = self
            .panes
            .iter()
            .map(|p| if p.weight > 0.0 { p.weight } else { 1.0 })
            .collect();
        let total: f32 = cleaned.iter().sum();
        cleaned
            .iter()
            .map(|w| axis_size * (w / total))
            .collect()
    }

    /// Focus index clamped to a valid pane.
    #[must_use]
    pub fn clamped_focus(&self) -> usize {
        if self.panes.is_empty() {
            0
        } else {
            self.focus.min(self.panes.len() - 1)
        }
    }

    /// Valid splits have at least 2 panes and all non-empty URLs.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.panes.len() >= 2 && self.panes.iter().all(|p| !p.url.is_empty())
    }
}

/// Registry of split layouts.
#[derive(Debug, Clone, Default)]
pub struct SplitRegistry {
    specs: Vec<SplitSpec>,
}

impl SplitRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SplitSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SplitSpec>) {
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
    pub fn specs(&self) -> &[SplitSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&SplitSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SplitSpec>, String> {
    tatara_lisp::compile_typed::<SplitSpec>(src)
        .map_err(|e| format!("failed to compile defsplit forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SplitSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(url: &str) -> SplitPane {
        SplitPane {
            url: url.into(),
            weight: 1.0,
            title: None,
        }
    }

    fn sample(name: &str, urls: &[&str]) -> SplitSpec {
        SplitSpec {
            name: name.into(),
            layout: SplitLayout::Horizontal,
            panes: urls.iter().map(|u| p(u)).collect(),
            focus: 0,
            persist: true,
            description: None,
        }
    }

    #[test]
    fn proportional_sizes_equal_for_equal_weights() {
        let s = sample("x", &["https://a", "https://b", "https://c"]);
        let sizes = s.proportional_sizes(600.0);
        assert_eq!(sizes.len(), 3);
        for size in &sizes {
            assert!((size - 200.0).abs() < 0.01);
        }
    }

    #[test]
    fn proportional_sizes_weighted() {
        let s = SplitSpec {
            panes: vec![
                SplitPane { url: "https://a".into(), weight: 2.0, title: None },
                SplitPane { url: "https://b".into(), weight: 1.0, title: None },
            ],
            ..sample("x", &[])
        };
        let sizes = s.proportional_sizes(300.0);
        assert!((sizes[0] - 200.0).abs() < 0.01);
        assert!((sizes[1] - 100.0).abs() < 0.01);
    }

    #[test]
    fn proportional_sizes_treat_zero_weight_as_one() {
        let s = SplitSpec {
            panes: vec![
                SplitPane { url: "https://a".into(), weight: 0.0, title: None },
                SplitPane { url: "https://b".into(), weight: 0.0, title: None },
            ],
            ..sample("x", &[])
        };
        let sizes = s.proportional_sizes(200.0);
        assert!((sizes[0] - 100.0).abs() < 0.01);
        assert!((sizes[1] - 100.0).abs() < 0.01);
    }

    #[test]
    fn proportional_sizes_empty_for_no_panes() {
        let s = SplitSpec {
            panes: vec![],
            ..sample("x", &[])
        };
        assert!(s.proportional_sizes(600.0).is_empty());
    }

    #[test]
    fn clamped_focus_within_pane_count() {
        let s = SplitSpec {
            focus: 10,
            ..sample("x", &["https://a", "https://b", "https://c"])
        };
        assert_eq!(s.clamped_focus(), 2);
    }

    #[test]
    fn clamped_focus_handles_empty() {
        let s = SplitSpec {
            panes: vec![],
            focus: 5,
            ..sample("x", &[])
        };
        assert_eq!(s.clamped_focus(), 0);
    }

    #[test]
    fn is_valid_requires_two_panes_with_urls() {
        let ok = sample("x", &["https://a", "https://b"]);
        assert!(ok.is_valid());
        let single = sample("x", &["https://a"]);
        assert!(!single.is_valid());
        let empty_url = SplitSpec {
            panes: vec![
                SplitPane { url: String::new(), weight: 1.0, title: None },
                SplitPane { url: "https://b".into(), weight: 1.0, title: None },
            ],
            ..sample("x", &[])
        };
        assert!(!empty_url.is_valid());
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = SplitRegistry::new();
        reg.insert(sample("a", &["https://a", "https://b"]));
        reg.insert(SplitSpec {
            layout: SplitLayout::Vertical,
            ..sample("a", &["https://c", "https://d"])
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("a").unwrap().layout, SplitLayout::Vertical);
    }

    #[test]
    fn default_layout_is_horizontal() {
        assert_eq!(SplitLayout::default(), SplitLayout::Horizontal);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_split_form() {
        // tatara-lisp accepts panes as a vector of SplitPane shapes.
        // Serde's default camelCase rename reuses the struct fields.
        let src = r#"
            (defsplit :name   "docs"
                      :layout "horizontal"
                      :panes  (
                        (:url "https://a" :weight 2)
                        (:url "https://b" :weight 1))
                      :focus  0)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "docs");
        assert_eq!(s.layout, SplitLayout::Horizontal);
        assert_eq!(s.panes.len(), 2);
        assert!((s.panes[0].weight - 2.0).abs() < 0.01);
    }
}
