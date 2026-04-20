//! `(defmultiplayer-cursor)` — live cursor visualization profile.
//!
//! Absorbs Figma cursor chat, Excalidraw live cursors, tldraw
//! multiplayer pointers, Arc Easels. Separate from `(defpresence)` so
//! the visualization layer (shape, color palette, name tag, fade
//! timing, click-echo) is declarative independently from transport.
//!
//! ```lisp
//! (defmultiplayer-cursor :name       "figma-style"
//!                        :host       "*://*.figma.com/*"
//!                        :style      :pointer
//!                        :palette    ("#ff4d4f" "#1890ff" "#52c41a" "#faad14")
//!                        :name-tag   #t
//!                        :fade-after-seconds 2
//!                        :click-echo #t
//!                        :scope      :per-tab)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Shape used to draw remote cursors.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CursorStyle {
    /// Default OS pointer arrow.
    Pointer,
    /// I-beam for text-heavy canvases.
    Caret,
    /// Crosshair (whiteboards, CAD).
    Crosshair,
    /// Small circle dot.
    Dot,
    /// Hand (like tldraw).
    Hand,
    /// Custom SVG provided at runtime.
    CustomSvg,
}

impl Default for CursorStyle {
    fn default() -> Self {
        Self::Pointer
    }
}

/// How cursors are scoped — a single tab, every tab in a profile,
/// every tab across all profiles.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CursorScope {
    PerTab,
    PerProfile,
    Global,
}

impl Default for CursorScope {
    fn default() -> Self {
        Self::PerTab
    }
}

/// Multiplayer cursor profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defmultiplayer-cursor"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MultiplayerCursorSpec {
    pub name: String,
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    #[serde(default)]
    pub style: CursorStyle,
    /// Color palette — cursors round-robin when no per-session color
    /// arrives on the wire.
    #[serde(default = "default_palette")]
    pub palette: Vec<String>,
    /// Render a floating name tag next to each cursor.
    #[serde(default = "default_name_tag")]
    pub name_tag: bool,
    /// Seconds of inactivity before the cursor fades.
    #[serde(default = "default_fade_after_seconds")]
    pub fade_after_seconds: u32,
    /// Draw a ripple animation when a remote user clicks.
    #[serde(default = "default_click_echo")]
    pub click_echo: bool,
    /// Follow mode — if true the camera jumps to a member you "follow".
    #[serde(default)]
    pub follow_mode: bool,
    #[serde(default)]
    pub scope: CursorScope,
    /// Hide cursors after this many members to reduce clutter
    /// (0 = never hide).
    #[serde(default = "default_crowd_threshold")]
    pub crowd_threshold: u32,
    /// Smoothing coefficient for cursor interpolation
    /// (0.0 = snap, 1.0 = very smooth).
    #[serde(default = "default_smoothing")]
    pub smoothing: f32,
    /// Accessibility — show cursors even when user prefers reduced motion.
    #[serde(default = "default_respect_reduced_motion")]
    pub respect_reduced_motion: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_palette() -> Vec<String> {
    vec![
        "#ff4d4f".into(),
        "#1890ff".into(),
        "#52c41a".into(),
        "#faad14".into(),
        "#722ed1".into(),
        "#eb2f96".into(),
    ]
}
fn default_name_tag() -> bool {
    true
}
fn default_fade_after_seconds() -> u32 {
    3
}
fn default_click_echo() -> bool {
    true
}
fn default_crowd_threshold() -> u32 {
    16
}
fn default_smoothing() -> f32 {
    0.35
}
fn default_respect_reduced_motion() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

impl MultiplayerCursorSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            style: CursorStyle::Pointer,
            palette: default_palette(),
            name_tag: true,
            fade_after_seconds: 3,
            click_echo: true,
            follow_mode: false,
            scope: CursorScope::PerTab,
            crowd_threshold: 16,
            smoothing: 0.35,
            respect_reduced_motion: true,
            enabled: true,
            description: Some(
                "Default multiplayer cursor — pointer style, per-tab scope, 6-color palette."
                    .into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    /// Pick a palette color for a given session index. Empty palette
    /// falls back to a deterministic gray.
    #[must_use]
    pub fn color_for_index(&self, index: usize) -> String {
        if self.palette.is_empty() {
            "#808080".into()
        } else {
            self.palette[index % self.palette.len()].clone()
        }
    }

    /// Clamp smoothing into `[0.0, 1.0]`.
    #[must_use]
    pub fn clamped_smoothing(&self) -> f32 {
        self.smoothing.clamp(0.0, 1.0)
    }

    /// Should cursors be hidden at `member_count` members?
    #[must_use]
    pub fn should_hide_crowd(&self, member_count: u32) -> bool {
        self.crowd_threshold > 0 && member_count > self.crowd_threshold
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct MultiplayerCursorRegistry {
    specs: Vec<MultiplayerCursorSpec>,
}

impl MultiplayerCursorRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: MultiplayerCursorSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = MultiplayerCursorSpec>) {
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
    pub fn specs(&self) -> &[MultiplayerCursorSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&MultiplayerCursorSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<MultiplayerCursorSpec>, String> {
    tatara_lisp::compile_typed::<MultiplayerCursorSpec>(src)
        .map_err(|e| format!("failed to compile defmultiplayer-cursor forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<MultiplayerCursorSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_has_sensible_palette_size() {
        let s = MultiplayerCursorSpec::default_profile();
        assert!(s.palette.len() >= 4);
        assert_eq!(s.scope, CursorScope::PerTab);
        assert!(s.click_echo);
    }

    #[test]
    fn color_for_index_wraps_modulo_palette() {
        let s = MultiplayerCursorSpec::default_profile();
        let n = s.palette.len();
        assert_eq!(s.color_for_index(0), s.palette[0]);
        assert_eq!(s.color_for_index(n), s.palette[0]);
        assert_eq!(s.color_for_index(n + 1), s.palette[1]);
    }

    #[test]
    fn color_for_index_falls_back_when_empty_palette() {
        let s = MultiplayerCursorSpec {
            palette: vec![],
            ..MultiplayerCursorSpec::default_profile()
        };
        assert_eq!(s.color_for_index(42), "#808080");
    }

    #[test]
    fn clamped_smoothing_bounds() {
        let s = MultiplayerCursorSpec {
            smoothing: 2.5,
            ..MultiplayerCursorSpec::default_profile()
        };
        assert!((s.clamped_smoothing() - 1.0).abs() < 1e-6);
        let s2 = MultiplayerCursorSpec {
            smoothing: -1.0,
            ..MultiplayerCursorSpec::default_profile()
        };
        assert!(s2.clamped_smoothing().abs() < 1e-6);
    }

    #[test]
    fn should_hide_crowd_respects_threshold() {
        let s = MultiplayerCursorSpec {
            crowd_threshold: 4,
            ..MultiplayerCursorSpec::default_profile()
        };
        assert!(!s.should_hide_crowd(4));
        assert!(s.should_hide_crowd(5));

        let unlimited = MultiplayerCursorSpec {
            crowd_threshold: 0,
            ..MultiplayerCursorSpec::default_profile()
        };
        assert!(!unlimited.should_hide_crowd(9_999));
    }

    #[test]
    fn matches_host_glob() {
        let s = MultiplayerCursorSpec {
            host: "*://*.figma.com/*".into(),
            ..MultiplayerCursorSpec::default_profile()
        };
        assert!(s.matches_host("www.figma.com"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn style_roundtrips_through_serde() {
        for k in [
            CursorStyle::Pointer,
            CursorStyle::Caret,
            CursorStyle::Crosshair,
            CursorStyle::Dot,
            CursorStyle::Hand,
            CursorStyle::CustomSvg,
        ] {
            let s = MultiplayerCursorSpec {
                style: k,
                ..MultiplayerCursorSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: MultiplayerCursorSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.style, k);
        }
    }

    #[test]
    fn scope_roundtrips_through_serde() {
        for sc in [
            CursorScope::PerTab,
            CursorScope::PerProfile,
            CursorScope::Global,
        ] {
            let s = MultiplayerCursorSpec {
                scope: sc,
                ..MultiplayerCursorSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: MultiplayerCursorSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.scope, sc);
        }
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = MultiplayerCursorRegistry::new();
        reg.insert(MultiplayerCursorSpec::default_profile());
        reg.insert(MultiplayerCursorSpec {
            name: "figma".into(),
            host: "*://*.figma.com/*".into(),
            style: CursorStyle::Crosshair,
            ..MultiplayerCursorSpec::default_profile()
        });
        let figma = reg.resolve("www.figma.com").unwrap();
        assert_eq!(figma.style, CursorStyle::Crosshair);
        let other = reg.resolve("example.org").unwrap();
        assert_eq!(other.name, "default");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = MultiplayerCursorRegistry::new();
        reg.insert(MultiplayerCursorSpec {
            enabled: false,
            ..MultiplayerCursorSpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_multiplayer_cursor_form() {
        let src = r##"
            (defmultiplayer-cursor :name "figma-style"
                                   :host "*://*.figma.com/*"
                                   :style "crosshair"
                                   :palette ("#ff4d4f" "#1890ff" "#52c41a")
                                   :name-tag #t
                                   :fade-after-seconds 2
                                   :click-echo #t
                                   :follow-mode #f
                                   :scope "per-tab"
                                   :crowd-threshold 24
                                   :smoothing 0.5
                                   :respect-reduced-motion #t)
        "##;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "figma-style");
        assert_eq!(s.style, CursorStyle::Crosshair);
        assert_eq!(s.palette.len(), 3);
        assert_eq!(s.crowd_threshold, 24);
    }
}
