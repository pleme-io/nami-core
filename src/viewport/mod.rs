//! `(defviewport)` — mobile viewport policy.
//!
//! Absorbs `<meta name="viewport">` parsing, CSS
//! `env(safe-area-inset-*)`, Screen Orientation API lock, and
//! Android accessibility's "force-enable zoom" override. One Lisp
//! form declares how a host's mobile rendering is sized, padded,
//! locked, and whether user zoom is respected regardless of the
//! page's wishes.
//!
//! ```lisp
//! (defviewport :name           "readable"
//!              :host           "*"
//!              :width          :device-width
//!              :initial-scale  1.0
//!              :min-scale      0.5
//!              :max-scale      5.0
//!              :user-scalable  :force-yes
//!              :safe-area-inset (top right bottom left)
//!              :orientation    :any
//!              :viewport-fit   :cover)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Viewport width directive.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ViewportWidth {
    /// `width=device-width` — standard responsive.
    #[default]
    DeviceWidth,
    /// Fixed pixel width (stored in `width_px`).
    FixedPx,
    /// Fixed em count — nicer for readable apps.
    FixedEm,
    /// Honor whatever the page declared.
    Passthrough,
}

/// User zoom policy. "force-yes" ignores `user-scalable=no` — matches
/// Android's accessibility override.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UserScalable {
    /// Honor whatever the page requested (default browser behaviour).
    #[default]
    Passthrough,
    /// Always yes — override `user-scalable=no` so pinch-zoom works
    /// even on hostile pages. **Accessibility win.**
    ForceYes,
    /// Always no — lock zoom for kiosk / game mode.
    ForceNo,
}

/// iOS-style viewport-fit (controls how content fills notched screens).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ViewportFit {
    /// Default — content stops at the safe area.
    #[default]
    Auto,
    /// Contain — fit within safe area.
    Contain,
    /// Cover — fill the whole screen including under the notch.
    Cover,
}

/// Which sides expose `env(safe-area-inset-*)` — lets a DSL author
/// selectively block a side from content (e.g. never let ads render
/// under the bottom home-indicator bar).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Inset {
    Top,
    Right,
    Bottom,
    Left,
}

/// Orientation lock.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Orientation {
    /// No lock — any orientation works.
    #[default]
    Any,
    Portrait,
    Landscape,
    PortraitPrimary,
    PortraitSecondary,
    LandscapePrimary,
    LandscapeSecondary,
    /// Whichever the device is currently in; don't allow rotation.
    Natural,
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defviewport"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ViewportSpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub width: ViewportWidth,
    /// Used only when `width = FixedPx`.
    #[serde(default)]
    pub width_px: u32,
    /// Used only when `width = FixedEm`.
    #[serde(default)]
    pub width_em: u32,
    #[serde(default = "default_initial_scale")]
    pub initial_scale: f32,
    #[serde(default = "default_min_scale")]
    pub min_scale: f32,
    #[serde(default = "default_max_scale")]
    pub max_scale: f32,
    #[serde(default)]
    pub user_scalable: UserScalable,
    /// Which sides expose `env(safe-area-inset-*)`. Empty = expose
    /// all four (browser default).
    #[serde(default)]
    pub safe_area_inset: Vec<Inset>,
    /// Minimum safe-area inset in CSS pixels per side when the host
    /// wants a floor (useful to avoid the home-indicator overlap).
    #[serde(default)]
    pub min_inset_top: u32,
    #[serde(default)]
    pub min_inset_right: u32,
    #[serde(default)]
    pub min_inset_bottom: u32,
    #[serde(default)]
    pub min_inset_left: u32,
    #[serde(default)]
    pub orientation: Orientation,
    #[serde(default)]
    pub viewport_fit: ViewportFit,
    /// Override the page's `<meta name=theme-color>` (useful for
    /// matching user identity color or ambient-light-sensor mode).
    #[serde(default)]
    pub theme_color: Option<String>,
    /// Also apply `color-scheme` CSS — `"light"`, `"dark"`,
    /// `"light dark"`, or empty for page default.
    #[serde(default)]
    pub color_scheme: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_initial_scale() -> f32 {
    1.0
}
fn default_min_scale() -> f32 {
    1.0
}
fn default_max_scale() -> f32 {
    5.0
}
fn default_enabled() -> bool {
    true
}

impl ViewportSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            width: ViewportWidth::DeviceWidth,
            width_px: 0,
            width_em: 0,
            initial_scale: 1.0,
            min_scale: 1.0,
            max_scale: 5.0,
            user_scalable: UserScalable::ForceYes, // accessibility-first
            safe_area_inset: vec![],
            min_inset_top: 0,
            min_inset_right: 0,
            min_inset_bottom: 0,
            min_inset_left: 0,
            orientation: Orientation::Any,
            viewport_fit: ViewportFit::Auto,
            theme_color: None,
            color_scheme: None,
            enabled: true,
            description: Some(
                "Default viewport — device-width, ForceYes user-scalable (a11y win), max 5×, auto viewport-fit.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    #[must_use]
    pub fn exposes(&self, side: Inset) -> bool {
        self.safe_area_inset.is_empty() || self.safe_area_inset.contains(&side)
    }

    /// Effective inset for a side — max of the DSL floor and the
    /// platform-reported value.
    #[must_use]
    pub fn effective_inset(&self, side: Inset, platform_value: u32) -> u32 {
        let floor = match side {
            Inset::Top => self.min_inset_top,
            Inset::Right => self.min_inset_right,
            Inset::Bottom => self.min_inset_bottom,
            Inset::Left => self.min_inset_left,
        };
        floor.max(platform_value)
    }

    /// Final scale bounds after clamping to the spec's
    /// `[min_scale, max_scale]` range.
    #[must_use]
    pub fn clamp_scale(&self, requested: f32) -> f32 {
        requested.clamp(self.min_scale, self.max_scale)
    }

    /// Should the browser allow pinch-zoom?
    #[must_use]
    pub fn allows_user_zoom(&self, page_requested_scalable: bool) -> bool {
        match self.user_scalable {
            UserScalable::ForceYes => true,
            UserScalable::ForceNo => false,
            UserScalable::Passthrough => page_requested_scalable,
        }
    }

    /// Render the synthesized `<meta name="viewport">` string. Uses
    /// the same token vocabulary the spec does.
    #[must_use]
    pub fn render_meta(&self) -> String {
        let w = match self.width {
            ViewportWidth::DeviceWidth => "device-width".to_owned(),
            ViewportWidth::FixedPx => self.width_px.to_string(),
            ViewportWidth::FixedEm => format!("{}em", self.width_em),
            ViewportWidth::Passthrough => return String::new(),
        };
        let fmt_scale = |f: f32| {
            if (f - f.round()).abs() < 1e-6 {
                format!("{:.0}", f)
            } else {
                format!("{:.2}", f)
            }
        };
        let mut out = format!(
            "width={w}, initial-scale={}",
            fmt_scale(self.initial_scale)
        );
        out.push_str(&format!(", minimum-scale={}", fmt_scale(self.min_scale)));
        out.push_str(&format!(", maximum-scale={}", fmt_scale(self.max_scale)));
        let scalable = match self.user_scalable {
            UserScalable::ForceYes | UserScalable::Passthrough => "yes",
            UserScalable::ForceNo => "no",
        };
        out.push_str(&format!(", user-scalable={scalable}"));
        let fit = match self.viewport_fit {
            ViewportFit::Auto => None,
            ViewportFit::Contain => Some("contain"),
            ViewportFit::Cover => Some("cover"),
        };
        if let Some(f) = fit {
            out.push_str(&format!(", viewport-fit={f}"));
        }
        out
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct ViewportRegistry {
    specs: Vec<ViewportSpec>,
}

impl ViewportRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: ViewportSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ViewportSpec>) {
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
    pub fn specs(&self) -> &[ViewportSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&ViewportSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ViewportSpec>, String> {
    tatara_lisp::compile_typed::<ViewportSpec>(src)
        .map_err(|e| format!("failed to compile defviewport forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ViewportSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_forces_user_zoom_accessibility_win() {
        let s = ViewportSpec::default_profile();
        assert_eq!(s.user_scalable, UserScalable::ForceYes);
        // A page that said user-scalable=no still gets zoom.
        assert!(s.allows_user_zoom(false));
    }

    #[test]
    fn allows_user_zoom_honors_passthrough() {
        let s = ViewportSpec {
            user_scalable: UserScalable::Passthrough,
            ..ViewportSpec::default_profile()
        };
        assert!(s.allows_user_zoom(true));
        assert!(!s.allows_user_zoom(false));
    }

    #[test]
    fn allows_user_zoom_force_no_locks() {
        let s = ViewportSpec {
            user_scalable: UserScalable::ForceNo,
            ..ViewportSpec::default_profile()
        };
        assert!(!s.allows_user_zoom(true));
    }

    #[test]
    fn clamp_scale_bounds_request() {
        let s = ViewportSpec {
            min_scale: 0.5,
            max_scale: 3.0,
            ..ViewportSpec::default_profile()
        };
        assert!((s.clamp_scale(0.1) - 0.5).abs() < 1e-5);
        assert!((s.clamp_scale(1.5) - 1.5).abs() < 1e-5);
        assert!((s.clamp_scale(10.0) - 3.0).abs() < 1e-5);
    }

    #[test]
    fn exposes_empty_list_means_all_sides() {
        let s = ViewportSpec::default_profile();
        for side in [Inset::Top, Inset::Right, Inset::Bottom, Inset::Left] {
            assert!(s.exposes(side));
        }
    }

    #[test]
    fn exposes_restricts_when_list_populated() {
        let s = ViewportSpec {
            safe_area_inset: vec![Inset::Top, Inset::Bottom],
            ..ViewportSpec::default_profile()
        };
        assert!(s.exposes(Inset::Top));
        assert!(s.exposes(Inset::Bottom));
        assert!(!s.exposes(Inset::Left));
        assert!(!s.exposes(Inset::Right));
    }

    #[test]
    fn effective_inset_is_max_of_floor_and_platform() {
        let s = ViewportSpec {
            min_inset_bottom: 20,
            ..ViewportSpec::default_profile()
        };
        assert_eq!(s.effective_inset(Inset::Bottom, 10), 20);
        assert_eq!(s.effective_inset(Inset::Bottom, 50), 50);
        // No floor on other sides.
        assert_eq!(s.effective_inset(Inset::Top, 5), 5);
    }

    #[test]
    fn render_meta_emits_standard_tokens() {
        let s = ViewportSpec::default_profile();
        let m = s.render_meta();
        assert!(m.contains("width=device-width"));
        assert!(m.contains("initial-scale=1"));
        assert!(m.contains("minimum-scale=1"));
        assert!(m.contains("maximum-scale=5"));
        assert!(m.contains("user-scalable=yes"));
    }

    #[test]
    fn render_meta_fixed_px() {
        let s = ViewportSpec {
            width: ViewportWidth::FixedPx,
            width_px: 1024,
            ..ViewportSpec::default_profile()
        };
        assert!(s.render_meta().starts_with("width=1024,"));
    }

    #[test]
    fn render_meta_fixed_em() {
        let s = ViewportSpec {
            width: ViewportWidth::FixedEm,
            width_em: 40,
            ..ViewportSpec::default_profile()
        };
        assert!(s.render_meta().starts_with("width=40em,"));
    }

    #[test]
    fn render_meta_passthrough_returns_empty() {
        let s = ViewportSpec {
            width: ViewportWidth::Passthrough,
            ..ViewportSpec::default_profile()
        };
        assert_eq!(s.render_meta(), "");
    }

    #[test]
    fn render_meta_includes_viewport_fit_when_set() {
        let s = ViewportSpec {
            viewport_fit: ViewportFit::Cover,
            ..ViewportSpec::default_profile()
        };
        assert!(s.render_meta().contains("viewport-fit=cover"));
    }

    #[test]
    fn render_meta_omits_viewport_fit_when_auto() {
        let s = ViewportSpec::default_profile();
        assert!(!s.render_meta().contains("viewport-fit"));
    }

    #[test]
    fn render_meta_user_scalable_no() {
        let s = ViewportSpec {
            user_scalable: UserScalable::ForceNo,
            ..ViewportSpec::default_profile()
        };
        assert!(s.render_meta().contains("user-scalable=no"));
    }

    #[test]
    fn render_meta_formats_fractional_scale_to_two_decimals() {
        let s = ViewportSpec {
            initial_scale: 1.25,
            min_scale: 0.5,
            max_scale: 2.75,
            ..ViewportSpec::default_profile()
        };
        let m = s.render_meta();
        assert!(m.contains("initial-scale=1.25"));
        assert!(m.contains("minimum-scale=0.50"));
        assert!(m.contains("maximum-scale=2.75"));
    }

    #[test]
    fn width_roundtrips_through_serde() {
        for w in [
            ViewportWidth::DeviceWidth,
            ViewportWidth::FixedPx,
            ViewportWidth::FixedEm,
            ViewportWidth::Passthrough,
        ] {
            let s = ViewportSpec {
                width: w,
                ..ViewportSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: ViewportSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.width, w);
        }
    }

    #[test]
    fn user_scalable_roundtrips_through_serde() {
        for u in [
            UserScalable::Passthrough,
            UserScalable::ForceYes,
            UserScalable::ForceNo,
        ] {
            let s = ViewportSpec {
                user_scalable: u,
                ..ViewportSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: ViewportSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.user_scalable, u);
        }
    }

    #[test]
    fn viewport_fit_roundtrips_through_serde() {
        for f in [ViewportFit::Auto, ViewportFit::Contain, ViewportFit::Cover] {
            let s = ViewportSpec {
                viewport_fit: f,
                ..ViewportSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: ViewportSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.viewport_fit, f);
        }
    }

    #[test]
    fn orientation_roundtrips_through_serde() {
        for o in [
            Orientation::Any,
            Orientation::Portrait,
            Orientation::Landscape,
            Orientation::PortraitPrimary,
            Orientation::PortraitSecondary,
            Orientation::LandscapePrimary,
            Orientation::LandscapeSecondary,
            Orientation::Natural,
        ] {
            let s = ViewportSpec {
                orientation: o,
                ..ViewportSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: ViewportSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.orientation, o);
        }
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = ViewportRegistry::new();
        reg.insert(ViewportSpec::default_profile());
        reg.insert(ViewportSpec {
            name: "reader".into(),
            host: "*://*.medium.com/*".into(),
            width: ViewportWidth::FixedEm,
            width_em: 40,
            ..ViewportSpec::default_profile()
        });
        let med = reg.resolve("medium.com");
        assert!(med.is_none() || med.unwrap().name == "default"); // bare medium.com may not match
        let m = reg.resolve("www.medium.com").unwrap();
        assert_eq!(m.name, "reader");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = ViewportRegistry::new();
        reg.insert(ViewportSpec {
            enabled: false,
            ..ViewportSpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_viewport_form() {
        let src = r#"
            (defviewport :name "readable"
                         :host "*"
                         :width "device-width"
                         :initial-scale 1.0
                         :min-scale 0.5
                         :max-scale 5.0
                         :user-scalable "force-yes"
                         :safe-area-inset ("top" "bottom")
                         :min-inset-bottom 20
                         :orientation "any"
                         :viewport-fit "cover")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.user_scalable, UserScalable::ForceYes);
        assert_eq!(s.viewport_fit, ViewportFit::Cover);
        assert_eq!(s.safe_area_inset.len(), 2);
        assert_eq!(s.min_inset_bottom, 20);
    }
}
