//! `(deftab-preview)` — hover tab preview.
//!
//! Absorbs Chrome hover-card, Edge vertical-tab hover preview, Vivaldi
//! Tab Preview, Safari tab previews. Declarative shape (screenshot,
//! domain info, open tabs), hover delay, preview size.
//!
//! ```lisp
//! (deftab-preview :name        "rich"
//!                 :host        "*"
//!                 :shape       :rich
//!                 :content     (screenshot title url favicon subtitle)
//!                 :delay-ms    350
//!                 :width-px    320
//!                 :height-px   180
//!                 :follow-cursor #t
//!                 :live-update   #f)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Visual shape of the preview.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewShape {
    /// Title only — classic tab tooltip.
    Tooltip,
    /// Title + URL + favicon, no image.
    Compact,
    /// Screenshot thumbnail + title + URL.
    #[default]
    Rich,
    /// Full tab reproduction (heavy; SFU-level).
    Full,
    /// No preview.
    None,
}

/// Fields included in the rendered preview.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewField {
    Screenshot,
    Title,
    Url,
    Favicon,
    /// Short subtitle (e.g. meta description, "N unread messages").
    Subtitle,
    /// Whether the tab is currently playing audio.
    AudioState,
    /// Whether the page is in loading state.
    LoadingState,
    /// Certificate info (secure / insecure / invalid).
    Security,
}

/// Preview profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "deftab-preview"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TabPreviewSpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub shape: PreviewShape,
    /// Fields to render.
    #[serde(default = "default_content")]
    pub content: Vec<PreviewField>,
    /// Hover delay in ms before the preview is shown.
    #[serde(default = "default_delay_ms")]
    pub delay_ms: u32,
    /// Preview width in CSS pixels.
    #[serde(default = "default_width_px")]
    pub width_px: u32,
    /// Preview height in CSS pixels.
    #[serde(default = "default_height_px")]
    pub height_px: u32,
    /// Anchor the preview to the cursor (true) or to the tab strip (false).
    #[serde(default)]
    pub follow_cursor: bool,
    /// Refresh the screenshot every few seconds while hovering.
    #[serde(default)]
    pub live_update: bool,
    /// Live-update interval in ms (ignored when live_update=false).
    #[serde(default = "default_live_update_ms")]
    pub live_update_ms: u32,
    /// Respect OS "reduced motion" preference (skip fade-in).
    #[serde(default = "default_respect_reduced_motion")]
    pub respect_reduced_motion: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_content() -> Vec<PreviewField> {
    vec![
        PreviewField::Screenshot,
        PreviewField::Title,
        PreviewField::Url,
        PreviewField::Favicon,
    ]
}
fn default_delay_ms() -> u32 {
    400
}
fn default_width_px() -> u32 {
    320
}
fn default_height_px() -> u32 {
    180
}
fn default_live_update_ms() -> u32 {
    1000
}
fn default_respect_reduced_motion() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

impl TabPreviewSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            shape: PreviewShape::Rich,
            content: default_content(),
            delay_ms: 400,
            width_px: 320,
            height_px: 180,
            follow_cursor: false,
            live_update: false,
            live_update_ms: 1000,
            respect_reduced_motion: true,
            enabled: true,
            description: Some("Default preview — rich shape, 320×180, 400 ms delay.".into()),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        if self.host.is_empty() || self.host == "*" {
            return true;
        }
        crate::extension::glob_match_host(&self.host, host)
    }

    #[must_use]
    pub fn shows(&self, field: PreviewField) -> bool {
        self.content.contains(&field)
    }

    /// Aspect ratio — width/height. Returns 1.0 when `height_px == 0`.
    #[must_use]
    pub fn aspect_ratio(&self) -> f32 {
        if self.height_px == 0 {
            1.0
        } else {
            self.width_px as f32 / self.height_px as f32
        }
    }

    /// Is this profile disabled or explicitly PreviewShape::None?
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.enabled && self.shape != PreviewShape::None
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct TabPreviewRegistry {
    specs: Vec<TabPreviewSpec>,
}

impl TabPreviewRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: TabPreviewSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = TabPreviewSpec>) {
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
    pub fn specs(&self) -> &[TabPreviewSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&TabPreviewSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<TabPreviewSpec>, String> {
    tatara_lisp::compile_typed::<TabPreviewSpec>(src)
        .map_err(|e| format!("failed to compile deftab-preview forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<TabPreviewSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_rich_with_basic_content() {
        let s = TabPreviewSpec::default_profile();
        assert_eq!(s.shape, PreviewShape::Rich);
        assert!(s.shows(PreviewField::Screenshot));
        assert!(s.shows(PreviewField::Title));
        assert!(!s.shows(PreviewField::AudioState));
    }

    #[test]
    fn aspect_ratio_is_computed_correctly() {
        let s = TabPreviewSpec::default_profile();
        let expected = 320.0 / 180.0;
        assert!((s.aspect_ratio() - expected).abs() < 1e-5);

        let zero_h = TabPreviewSpec {
            height_px: 0,
            ..TabPreviewSpec::default_profile()
        };
        assert!((zero_h.aspect_ratio() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn is_active_respects_enabled_and_shape_none() {
        let off = TabPreviewSpec {
            enabled: false,
            ..TabPreviewSpec::default_profile()
        };
        assert!(!off.is_active());

        let none = TabPreviewSpec {
            shape: PreviewShape::None,
            ..TabPreviewSpec::default_profile()
        };
        assert!(!none.is_active());

        assert!(TabPreviewSpec::default_profile().is_active());
    }

    #[test]
    fn matches_host_glob() {
        let s = TabPreviewSpec {
            host: "*://*.youtube.com/*".into(),
            ..TabPreviewSpec::default_profile()
        };
        assert!(s.matches_host("www.youtube.com"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn shape_roundtrips_through_serde() {
        for sh in [
            PreviewShape::Tooltip,
            PreviewShape::Compact,
            PreviewShape::Rich,
            PreviewShape::Full,
            PreviewShape::None,
        ] {
            let s = TabPreviewSpec {
                shape: sh,
                ..TabPreviewSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: TabPreviewSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.shape, sh);
        }
    }

    #[test]
    fn content_fields_roundtrip_through_serde() {
        let fields = vec![
            PreviewField::Screenshot,
            PreviewField::Title,
            PreviewField::Url,
            PreviewField::Favicon,
            PreviewField::Subtitle,
            PreviewField::AudioState,
            PreviewField::LoadingState,
            PreviewField::Security,
        ];
        let s = TabPreviewSpec {
            content: fields.clone(),
            ..TabPreviewSpec::default_profile()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: TabPreviewSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content, fields);
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = TabPreviewRegistry::new();
        reg.insert(TabPreviewSpec::default_profile());
        reg.insert(TabPreviewSpec {
            name: "yt".into(),
            host: "*://*.youtube.com/*".into(),
            shape: PreviewShape::Full,
            ..TabPreviewSpec::default_profile()
        });
        assert_eq!(reg.resolve("www.youtube.com").unwrap().shape, PreviewShape::Full);
        assert_eq!(reg.resolve("example.org").unwrap().name, "default");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = TabPreviewRegistry::new();
        reg.insert(TabPreviewSpec {
            enabled: false,
            ..TabPreviewSpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_tab_preview_form() {
        let src = r#"
            (deftab-preview :name "rich"
                            :host "*"
                            :shape "rich"
                            :content ("screenshot" "title" "url" "favicon" "subtitle")
                            :delay-ms 350
                            :width-px 320
                            :height-px 180
                            :follow-cursor #t
                            :live-update #f)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "rich");
        assert_eq!(s.shape, PreviewShape::Rich);
        assert!(s.shows(PreviewField::Subtitle));
        assert_eq!(s.delay_ms, 350);
        assert!(s.follow_cursor);
    }
}
