//! `(defsidebar)` — persistent sidebar webviews.
//!
//! A *sidebar app* is a webview that sits alongside the main tab
//! with its own lifecycle, persists across navigates, and may be
//! per-space. Absorbs Arc's sidebar apps (Essentials), Opera's
//! sidebar messengers, Vivaldi's web panels.
//!
//! ```lisp
//! (defsidebar :name      "chat"
//!             :url       "https://chat.example.com"
//!             :position  :left
//!             :width     360
//!             :icon      "message-circle"
//!             :pinned    #t
//!             :spaces    ("work"))
//!
//! (defsidebar :name      "notes"
//!             :url       "https://notes.example.com"
//!             :position  :right
//!             :width     420
//!             :host-gated  ("*://docs.example.com/*"))
//! ```
//!
//! Gating semantics:
//! - `spaces` empty → visible in every space
//! - `host-gated` empty → visible regardless of the active tab's host
//! - both together are AND'd

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Side of the browser chrome to anchor the sidebar on.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SidebarPosition {
    Left,
    Right,
}

impl Default for SidebarPosition {
    fn default() -> Self {
        Self::Left
    }
}

/// Sidebar-app declaration.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsidebar"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SidebarSpec {
    pub name: String,
    /// URL loaded in the sidebar webview.
    pub url: String,
    #[serde(default)]
    pub position: SidebarPosition,
    /// Width in px. Clamped to `[120, 800]` at apply time.
    #[serde(default = "default_width")]
    pub width: u32,
    /// Optional icon glyph name (chrome-defined vocabulary).
    #[serde(default)]
    pub icon: Option<String>,
    /// Always visible on the toolbar — if false, the sidebar hides
    /// until the user opens it.
    #[serde(default = "default_pinned")]
    pub pinned: bool,
    /// Space names this sidebar is visible in. Empty = all spaces.
    #[serde(default)]
    pub spaces: Vec<String>,
    /// Host globs — sidebar only renders when the active tab's host
    /// matches one of these. Empty = always.
    #[serde(default)]
    pub host_gated: Vec<String>,
    /// Seconds of inactivity before the sidebar webview hibernates
    /// to save memory. `0` = never.
    #[serde(default = "default_hibernate")]
    pub hibernate_seconds: u64,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_width() -> u32 {
    360
}
fn default_pinned() -> bool {
    true
}
fn default_hibernate() -> u64 {
    600
}

const MIN_WIDTH: u32 = 120;
const MAX_WIDTH: u32 = 800;

impl SidebarSpec {
    #[must_use]
    pub fn clamped_width(&self) -> u32 {
        self.width.clamp(MIN_WIDTH, MAX_WIDTH)
    }

    /// Does this sidebar render under (active_space, active_host)?
    #[must_use]
    pub fn visible_under(&self, active_space: Option<&str>, active_host: &str) -> bool {
        // Space gate.
        if !self.spaces.is_empty() {
            let Some(active) = active_space else {
                return false;
            };
            if !self.spaces.iter().any(|s| s == active) {
                return false;
            }
        }
        // Host gate.
        if !self.host_gated.is_empty() {
            if !self
                .host_gated
                .iter()
                .any(|g| crate::extension::glob_match_host(g, active_host))
            {
                return false;
            }
        }
        true
    }
}

/// Registry of sidebar apps.
#[derive(Debug, Clone, Default)]
pub struct SidebarRegistry {
    specs: Vec<SidebarSpec>,
}

impl SidebarRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SidebarSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SidebarSpec>) {
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
    pub fn specs(&self) -> &[SidebarSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&SidebarSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// Every sidebar visible under `(active_space, active_host)`.
    #[must_use]
    pub fn visible(&self, active_space: Option<&str>, active_host: &str) -> Vec<&SidebarSpec> {
        self.specs
            .iter()
            .filter(|s| s.visible_under(active_space, active_host))
            .collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SidebarSpec>, String> {
    tatara_lisp::compile_typed::<SidebarSpec>(src)
        .map_err(|e| format!("failed to compile defsidebar forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SidebarSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, url: &str) -> SidebarSpec {
        SidebarSpec {
            name: name.into(),
            url: url.into(),
            position: SidebarPosition::Left,
            width: 360,
            icon: None,
            pinned: true,
            spaces: vec![],
            host_gated: vec![],
            hibernate_seconds: 600,
            description: None,
        }
    }

    #[test]
    fn width_clamps_to_valid_range() {
        let a = SidebarSpec {
            width: 50,
            ..sample("a", "https://x/")
        };
        assert_eq!(a.clamped_width(), MIN_WIDTH);
        let b = SidebarSpec {
            width: 9999,
            ..sample("a", "https://x/")
        };
        assert_eq!(b.clamped_width(), MAX_WIDTH);
        let c = SidebarSpec {
            width: 400,
            ..sample("a", "https://x/")
        };
        assert_eq!(c.clamped_width(), 400);
    }

    #[test]
    fn visible_everywhere_when_no_gates() {
        let s = sample("x", "https://x/");
        assert!(s.visible_under(Some("work"), "example.com"));
        assert!(s.visible_under(None, ""));
    }

    #[test]
    fn space_gate_filters_wrong_space() {
        let s = SidebarSpec {
            spaces: vec!["work".into()],
            ..sample("x", "https://x/")
        };
        assert!(s.visible_under(Some("work"), "a.com"));
        assert!(!s.visible_under(Some("home"), "a.com"));
        // No active space → space-gated sidebar hides.
        assert!(!s.visible_under(None, "a.com"));
    }

    #[test]
    fn host_gate_filters_wrong_host() {
        let s = SidebarSpec {
            host_gated: vec!["*://docs.example.com/*".into()],
            ..sample("x", "https://x/")
        };
        assert!(s.visible_under(None, "docs.example.com"));
        assert!(!s.visible_under(None, "evil.com"));
    }

    #[test]
    fn both_gates_are_anded() {
        let s = SidebarSpec {
            spaces: vec!["work".into()],
            host_gated: vec!["*://docs.example.com/*".into()],
            ..sample("x", "https://x/")
        };
        assert!(s.visible_under(Some("work"), "docs.example.com"));
        assert!(!s.visible_under(Some("work"), "other.com"));
        assert!(!s.visible_under(Some("home"), "docs.example.com"));
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = SidebarRegistry::new();
        reg.insert(sample("chat", "https://a/"));
        reg.insert(sample("chat", "https://b/"));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("chat").unwrap().url, "https://b/");
    }

    #[test]
    fn visible_list_filters_by_gates() {
        let mut reg = SidebarRegistry::new();
        reg.insert(SidebarSpec {
            spaces: vec!["work".into()],
            ..sample("work-chat", "https://chat/")
        });
        reg.insert(sample("always-notes", "https://notes/"));
        let visible: Vec<&str> = reg
            .visible(Some("home"), "example.com")
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(visible, vec!["always-notes"]);
        let w: Vec<&str> = reg
            .visible(Some("work"), "example.com")
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn default_position_is_left() {
        let s = sample("x", "https://x/");
        assert_eq!(s.position, SidebarPosition::Left);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_sidebar_form() {
        let src = r#"
            (defsidebar :name     "chat"
                        :url      "https://chat.example.com"
                        :position "left"
                        :width    360
                        :pinned   #t
                        :spaces   ("work")
                        :host-gated ("*://docs.example.com/*")
                        :hibernate-seconds 300)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "chat");
        assert_eq!(s.url, "https://chat.example.com");
        assert_eq!(s.position, SidebarPosition::Left);
        assert_eq!(s.width, 360);
        assert!(s.pinned);
        assert_eq!(s.spaces, vec!["work"]);
        assert_eq!(s.hibernate_seconds, 300);
    }
}
