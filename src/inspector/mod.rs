//! `(definspector)` — declarative inspector panel.
//!
//! The namimado inspector SPA (`/ui`) hosts panels that visualize
//! the live substrate. Chrome DevTools, Firefox DevTools, and Safari
//! Web Inspector all extend via `devtools_page` / `chrome.devtools.*`
//! APIs. Nami absorbs the idea: every panel is a Lisp form that
//! declares its name, data source, refresh strategy, and layout.
//! Composes with everything — panels can pull from (defstorage),
//! MCP tools, HTTP endpoints, state cells, queries.
//!
//! ```lisp
//! (definspector :name     "storage"
//!               :title    "Storage Inspector"
//!               :icon     "database"
//!               :source   (:kind :http :url "/storage")
//!               :view     :table
//!               :refresh  :manual
//!               :columns  ("name" "entry_count"))
//!
//! (definspector :name     "llm-log"
//!               :title    "LLM Log"
//!               :icon     "activity"
//!               :source   (:kind :storage :name "llm-trace")
//!               :view     :timeline
//!               :refresh  :tail
//!               :position :right)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Where the panel's data comes from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PanelSource {
    /// Read-only poll of an HTTP endpoint on namimado itself
    /// (typescape endpoints like `/state`, `/rules`, `/storage`).
    Http {
        #[serde(default)]
        url: String,
    },
    /// MCP tool — panel invokes it and displays the JSON result.
    Mcp {
        #[serde(default)]
        tool: String,
        /// Optional JSON object — static args for the tool call.
        #[serde(default)]
        args: Option<serde_json::Value>,
    },
    /// `(defstorage)` snapshot — panel tails the event log.
    Storage {
        #[serde(default)]
        name: String,
    },
    /// `(defstate)` cell — panel mirrors a live state cell.
    State {
        #[serde(default)]
        name: String,
    },
    /// `(defquery)` — panel runs the query on refresh.
    Query {
        #[serde(default)]
        name: String,
    },
    /// Static JSON — panel displays a fixed blob (debugging, demos).
    Static {
        #[serde(default)]
        value: serde_json::Value,
    },
}

/// How the panel renders.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PanelView {
    /// Rows × columns grid.
    Table,
    /// Nested tree — expand/collapse by key.
    Tree,
    /// Time-ordered event stream.
    Timeline,
    /// Pretty-printed JSON.
    Json,
    /// Raw text / markdown.
    Text,
    /// Metric dial — best for single-number observables.
    Metric,
    /// Log-style tail — appending new rows as data arrives.
    Tail,
}

impl Default for PanelView {
    fn default() -> Self {
        Self::Json
    }
}

/// Refresh strategy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshStrategy {
    /// Fetch only when the user clicks Refresh.
    Manual,
    /// Fetch every N seconds (see `refresh_seconds`).
    Interval,
    /// Tail a log source — fetch incrementally on each change event.
    Tail,
    /// Fetch once on panel open and never again.
    Once,
}

impl Default for RefreshStrategy {
    fn default() -> Self {
        Self::Manual
    }
}

/// Default dock position in the inspector chrome.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PanelPosition {
    Left,
    Right,
    Bottom,
    /// Floating window — user-positioned.
    Floating,
}

impl Default for PanelPosition {
    fn default() -> Self {
        Self::Bottom
    }
}

/// Inspector-panel declaration.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "definspector"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InspectorSpec {
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub source: PanelSource,
    #[serde(default)]
    pub view: PanelView,
    #[serde(default)]
    pub refresh: RefreshStrategy,
    /// Seconds between auto-refresh when `refresh == Interval`.
    /// Clamped to `[1, 3600]` at apply time.
    #[serde(default = "default_refresh_seconds")]
    pub refresh_seconds: u32,
    /// Column names for `Table` view. Ignored by other views.
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub position: PanelPosition,
    /// Whether the panel is visible by default.
    #[serde(default = "default_visible")]
    pub visible: bool,
    /// Minimum height in px (0 = let the layout decide).
    #[serde(default)]
    pub min_height: u32,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_refresh_seconds() -> u32 {
    5
}
fn default_visible() -> bool {
    true
}

impl InspectorSpec {
    #[must_use]
    pub fn clamped_refresh(&self) -> u32 {
        self.refresh_seconds.clamp(1, 3600)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("inspector panel name is empty".into());
        }
        if self.title.trim().is_empty() {
            return Err(format!(
                "inspector panel '{}' has empty :title",
                self.name
            ));
        }
        match &self.source {
            PanelSource::Http { url } if url.is_empty() => {
                Err(format!("inspector '{}' :http source needs :url", self.name))
            }
            PanelSource::Mcp { tool, .. } if tool.is_empty() => {
                Err(format!("inspector '{}' :mcp source needs :tool", self.name))
            }
            PanelSource::Storage { name } if name.is_empty() => {
                Err(format!(
                    "inspector '{}' :storage source needs :name",
                    self.name
                ))
            }
            PanelSource::State { name } if name.is_empty() => {
                Err(format!(
                    "inspector '{}' :state source needs :name",
                    self.name
                ))
            }
            PanelSource::Query { name } if name.is_empty() => {
                Err(format!(
                    "inspector '{}' :query source needs :name",
                    self.name
                ))
            }
            _ => Ok(()),
        }
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct InspectorRegistry {
    specs: Vec<InspectorSpec>,
}

impl InspectorRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: InspectorSpec) -> Result<(), String> {
        spec.validate()?;
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
        Ok(())
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = InspectorSpec>) {
        for s in specs {
            if let Err(e) = self.insert(s.clone()) {
                tracing::warn!("definspector '{}' rejected: {}", s.name, e);
            }
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
    pub fn specs(&self) -> &[InspectorSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&InspectorSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// Every visible panel, in insertion order.
    #[must_use]
    pub fn visible(&self) -> Vec<&InspectorSpec> {
        self.specs.iter().filter(|s| s.visible).collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<InspectorSpec>, String> {
    tatara_lisp::compile_typed::<InspectorSpec>(src)
        .map_err(|e| format!("failed to compile definspector forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<InspectorSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_http() -> InspectorSpec {
        InspectorSpec {
            name: "storage".into(),
            title: "Storage Inspector".into(),
            icon: Some("database".into()),
            source: PanelSource::Http { url: "/storage".into() },
            view: PanelView::Table,
            refresh: RefreshStrategy::Manual,
            refresh_seconds: 5,
            columns: vec!["name".into(), "entry_count".into()],
            position: PanelPosition::Bottom,
            visible: true,
            min_height: 0,
            description: None,
        }
    }

    #[test]
    fn validate_requires_name_and_title() {
        let mut s = sample_http();
        s.name = String::new();
        assert!(s.validate().is_err());
        let mut s = sample_http();
        s.title = String::new();
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_source_missing_key_field() {
        let specs = [
            InspectorSpec {
                source: PanelSource::Http { url: String::new() },
                ..sample_http()
            },
            InspectorSpec {
                source: PanelSource::Mcp {
                    tool: String::new(),
                    args: None,
                },
                ..sample_http()
            },
            InspectorSpec {
                source: PanelSource::Storage { name: String::new() },
                ..sample_http()
            },
            InspectorSpec {
                source: PanelSource::State { name: String::new() },
                ..sample_http()
            },
            InspectorSpec {
                source: PanelSource::Query { name: String::new() },
                ..sample_http()
            },
        ];
        for s in specs {
            assert!(
                s.validate().is_err(),
                "expected validation error for {s:?}"
            );
        }
    }

    #[test]
    fn validate_accepts_static_source_with_no_fields() {
        let s = InspectorSpec {
            source: PanelSource::Static {
                value: serde_json::json!({"note": "demo"}),
            },
            ..sample_http()
        };
        assert!(s.validate().is_ok());
    }

    #[test]
    fn clamped_refresh_respects_bounds() {
        let low = InspectorSpec {
            refresh_seconds: 0,
            ..sample_http()
        };
        assert_eq!(low.clamped_refresh(), 1);
        let high = InspectorSpec {
            refresh_seconds: 99_999,
            ..sample_http()
        };
        assert_eq!(high.clamped_refresh(), 3600);
    }

    #[test]
    fn registry_insert_validates_and_dedupes() {
        let mut reg = InspectorRegistry::new();
        assert!(reg
            .insert(InspectorSpec {
                title: String::new(),
                ..sample_http()
            })
            .is_err());
        reg.insert(sample_http()).unwrap();
        reg.insert(InspectorSpec {
            view: PanelView::Tree,
            ..sample_http()
        })
        .unwrap();
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].view, PanelView::Tree);
    }

    #[test]
    fn visible_filters_hidden_panels() {
        let mut reg = InspectorRegistry::new();
        reg.insert(sample_http()).unwrap();
        reg.insert(InspectorSpec {
            name: "hidden".into(),
            visible: false,
            ..sample_http()
        })
        .unwrap();
        let vis = reg.visible();
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].name, "storage");
    }

    #[test]
    fn source_kinds_roundtrip_through_serde() {
        let cases: Vec<PanelSource> = vec![
            PanelSource::Http { url: "/x".into() },
            PanelSource::Mcp {
                tool: "t".into(),
                args: Some(serde_json::json!({"a": 1})),
            },
            PanelSource::Storage { name: "n".into() },
            PanelSource::State { name: "n".into() },
            PanelSource::Query { name: "n".into() },
            PanelSource::Static {
                value: serde_json::json!([1, 2, 3]),
            },
        ];
        for source in cases {
            let s = InspectorSpec {
                source: source.clone(),
                ..sample_http()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: InspectorSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.source, source);
        }
    }

    #[test]
    fn view_default_is_json() {
        assert_eq!(PanelView::default(), PanelView::Json);
    }

    #[test]
    fn position_default_is_bottom() {
        assert_eq!(PanelPosition::default(), PanelPosition::Bottom);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_inspector_form() {
        // tatara-lisp flattens the `source` tagged enum — callers
        // supply :kind + co-fields on the top form.
        let src = r#"
            (definspector :name "storage"
                          :title "Storage Inspector"
                          :icon "database"
                          :source (:kind "http" :url "/storage")
                          :view "table"
                          :refresh "manual"
                          :columns ("name" "entry_count"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "storage");
        assert_eq!(s.view, PanelView::Table);
        assert!(matches!(&s.source, PanelSource::Http { url } if url == "/storage"));
    }
}
