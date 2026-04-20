//! `(defhigh-contrast)` — declarative WCAG-grade contrast enforcement.
//!
//! Absorbs Windows High Contrast mode, macOS Increase Contrast,
//! Chrome forced-colors, Firefox override page colors, browser
//! dark-mode toggles, and the contrast-fixer extensions (Dark
//! Reader, High Contrast). Each profile declares minimum contrast
//! ratio, color-scheme forcing, link emphasis, focus-ring beefing,
//! and per-host activation.
//!
//! ```lisp
//! (defhigh-contrast :name         "aa-default"
//!                   :host         "*"
//!                   :min-ratio    4.5
//!                   :force-scheme :auto
//!                   :link-boost   #t
//!                   :focus-ring-px 3)
//!
//! (defhigh-contrast :name         "aaa-night"
//!                   :host         "*"
//!                   :min-ratio    7.0
//!                   :force-scheme :dark
//!                   :foreground   "#f8f8f2"
//!                   :background   "#1e1e2e"
//!                   :link         "#89b4fa")
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// WCAG conformance level.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WcagLevel {
    /// 4.5:1 for normal text, 3:1 for large — baseline.
    Aa,
    /// 7.0:1 for normal text, 4.5:1 for large — extra margin.
    Aaa,
}

/// Force a color-scheme on the page.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SchemeOverride {
    /// Respect the site's color scheme.
    Auto,
    /// Force light.
    Light,
    /// Force dark.
    Dark,
    /// Invert — "reader view" style; flips all colors.
    Invert,
    /// Apply user foreground + background regardless of site styles.
    Custom,
}

impl Default for SchemeOverride {
    fn default() -> Self {
        Self::Auto
    }
}

/// High-contrast profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defhigh-contrast"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HighContrastSpec {
    pub name: String,
    /// Host glob. `"*"` = everywhere.
    #[serde(default = "default_host")]
    pub host: String,
    /// Minimum contrast ratio to enforce. The engine rewrites
    /// foreground colors toward the target when a computed style
    /// falls below. Clamped [1.0, 21.0] (black-on-white = 21.0).
    #[serde(default = "default_min_ratio")]
    pub min_ratio: f32,
    #[serde(default)]
    pub force_scheme: SchemeOverride,
    /// Override foreground color for Custom/Dark/Light schemes.
    /// Empty = scheme default.
    #[serde(default)]
    pub foreground: Option<String>,
    /// Override background.
    #[serde(default)]
    pub background: Option<String>,
    /// Override link color.
    #[serde(default)]
    pub link: Option<String>,
    /// Override visited-link color.
    #[serde(default)]
    pub link_visited: Option<String>,
    /// Boost link weight + underline so they stand out.
    #[serde(default = "default_link_boost")]
    pub link_boost: bool,
    /// Focus-ring thickness in CSS px. Clamped [0, 10].
    #[serde(default = "default_focus_ring_px")]
    pub focus_ring_px: u32,
    /// Focus-ring color. Empty = foreground.
    #[serde(default)]
    pub focus_ring_color: Option<String>,
    /// Runtime toggle.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_min_ratio() -> f32 {
    4.5
}
fn default_link_boost() -> bool {
    true
}
fn default_focus_ring_px() -> u32 {
    3
}
fn default_enabled() -> bool {
    true
}

const MIN_CONTRAST: f32 = 1.0;
const MAX_CONTRAST: f32 = 21.0;
const MAX_FOCUS_RING: u32 = 10;

impl HighContrastSpec {
    #[must_use]
    pub fn default_aa() -> Self {
        Self {
            name: "aa".into(),
            host: "*".into(),
            min_ratio: 4.5,
            force_scheme: SchemeOverride::Auto,
            foreground: None,
            background: None,
            link: None,
            link_visited: None,
            link_boost: true,
            focus_ring_px: 3,
            focus_ring_color: None,
            enabled: true,
            description: Some("Default WCAG AA contrast enforcement.".into()),
        }
    }

    #[must_use]
    pub fn default_aaa_night() -> Self {
        Self {
            name: "aaa-night".into(),
            host: "*".into(),
            min_ratio: 7.0,
            force_scheme: SchemeOverride::Dark,
            foreground: Some("#f8f8f2".into()),
            background: Some("#1e1e2e".into()),
            link: Some("#89b4fa".into()),
            link_visited: Some("#cba6f7".into()),
            link_boost: true,
            focus_ring_px: 4,
            focus_ring_color: Some("#f9e2af".into()),
            enabled: true,
            description: Some("AAA night-mode — catppuccin-flavored dark.".into()),
        }
    }

    #[must_use]
    pub fn wcag_level(&self) -> WcagLevel {
        if self.min_ratio >= 7.0 {
            WcagLevel::Aaa
        } else {
            WcagLevel::Aa
        }
    }

    #[must_use]
    pub fn clamped_ratio(&self) -> f32 {
        self.min_ratio.clamp(MIN_CONTRAST, MAX_CONTRAST)
    }

    #[must_use]
    pub fn clamped_focus_ring(&self) -> u32 {
        self.focus_ring_px.clamp(0, MAX_FOCUS_RING)
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }
}

/// Compute WCAG relative-luminance contrast ratio between two
/// `[r, g, b]` colors (each 0..=255). Returns `[1.0, 21.0]`.
#[must_use]
pub fn contrast_ratio(fg: [u8; 3], bg: [u8; 3]) -> f32 {
    let lf = relative_luminance(fg);
    let lb = relative_luminance(bg);
    let (lighter, darker) = if lf > lb { (lf, lb) } else { (lb, lf) };
    (lighter + 0.05) / (darker + 0.05)
}

fn relative_luminance(c: [u8; 3]) -> f32 {
    let rgb = c.map(channel_luminance);
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

fn channel_luminance(v: u8) -> f32 {
    let s = f32::from(v) / 255.0;
    if s <= 0.03928 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// Parse a `#rrggbb` (or `#rrggbbaa`) hex string. Accepts `rgb`/`rgba`
/// shorthand too (expands each nibble). Returns `[r, g, b]`.
#[must_use]
pub fn parse_hex(input: &str) -> Option<[u8; 3]> {
    let s = input.trim().trim_start_matches('#');
    match s.len() {
        3 | 4 => {
            let chars: Vec<char> = s.chars().collect();
            let r = u8::from_str_radix(&format!("{0}{0}", chars[0]), 16).ok()?;
            let g = u8::from_str_radix(&format!("{0}{0}", chars[1]), 16).ok()?;
            let b = u8::from_str_radix(&format!("{0}{0}", chars[2]), 16).ok()?;
            Some([r, g, b])
        }
        6 | 8 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            Some([r, g, b])
        }
        _ => None,
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct HighContrastRegistry {
    specs: Vec<HighContrastSpec>,
}

impl HighContrastRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: HighContrastSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = HighContrastSpec>) {
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
    pub fn specs(&self) -> &[HighContrastSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&HighContrastSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<HighContrastSpec>, String> {
    tatara_lisp::compile_typed::<HighContrastSpec>(src)
        .map_err(|e| format!("failed to compile defhigh-contrast forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<HighContrastSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_ratio_extremes() {
        // Black on white → 21.0.
        let r = contrast_ratio([0, 0, 0], [255, 255, 255]);
        assert!((r - 21.0).abs() < 0.01);
        // Same color → 1.0.
        let r = contrast_ratio([128, 128, 128], [128, 128, 128]);
        assert!((r - 1.0).abs() < 0.01);
    }

    #[test]
    fn contrast_ratio_aa_threshold() {
        // #595959 on white crosses AA threshold (4.5:1).
        let r = contrast_ratio([0x59, 0x59, 0x59], [255, 255, 255]);
        assert!(r >= 4.5, "expected ≥4.5, got {r}");
    }

    #[test]
    fn contrast_ratio_is_symmetric() {
        let a = contrast_ratio([100, 150, 200], [30, 30, 30]);
        let b = contrast_ratio([30, 30, 30], [100, 150, 200]);
        assert!((a - b).abs() < 0.001);
    }

    #[test]
    fn parse_hex_six_char() {
        assert_eq!(parse_hex("#ffff00"), Some([255, 255, 0]));
        assert_eq!(parse_hex("ff00ff"), Some([255, 0, 255]));
    }

    #[test]
    fn parse_hex_three_char_expands() {
        assert_eq!(parse_hex("#f0a"), Some([0xff, 0x00, 0xaa]));
    }

    #[test]
    fn parse_hex_eight_char_strips_alpha() {
        assert_eq!(parse_hex("#ff0000ff"), Some([255, 0, 0]));
    }

    #[test]
    fn parse_hex_rejects_garbage() {
        assert!(parse_hex("not-hex").is_none());
        assert!(parse_hex("#xyz").is_none());
        assert!(parse_hex("").is_none());
    }

    #[test]
    fn wcag_level_maps_from_ratio() {
        assert_eq!(HighContrastSpec::default_aa().wcag_level(), WcagLevel::Aa);
        assert_eq!(
            HighContrastSpec::default_aaa_night().wcag_level(),
            WcagLevel::Aaa
        );
    }

    #[test]
    fn clamped_ratio_respects_bounds() {
        let lo = HighContrastSpec {
            min_ratio: 0.1,
            ..HighContrastSpec::default_aa()
        };
        assert_eq!(lo.clamped_ratio(), MIN_CONTRAST);
        let hi = HighContrastSpec {
            min_ratio: 999.0,
            ..HighContrastSpec::default_aa()
        };
        assert_eq!(hi.clamped_ratio(), MAX_CONTRAST);
    }

    #[test]
    fn focus_ring_clamps() {
        let hi = HighContrastSpec {
            focus_ring_px: 9999,
            ..HighContrastSpec::default_aa()
        };
        assert_eq!(hi.clamped_focus_ring(), MAX_FOCUS_RING);
    }

    #[test]
    fn registry_dedupes_and_resolves_specific() {
        let mut reg = HighContrastRegistry::new();
        reg.insert(HighContrastSpec::default_aa());
        reg.insert(HighContrastSpec {
            name: "gov".into(),
            host: "*://*.gov/*".into(),
            ..HighContrastSpec::default_aaa_night()
        });
        let gov = reg.resolve("www.whitehouse.gov").unwrap();
        assert_eq!(gov.name, "gov");
        let other = reg.resolve("example.com").unwrap();
        assert_eq!(other.name, "aa");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = HighContrastRegistry::new();
        reg.insert(HighContrastSpec {
            enabled: false,
            ..HighContrastSpec::default_aa()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[test]
    fn scheme_roundtrips_through_serde() {
        for scheme in [
            SchemeOverride::Auto,
            SchemeOverride::Light,
            SchemeOverride::Dark,
            SchemeOverride::Invert,
            SchemeOverride::Custom,
        ] {
            let s = HighContrastSpec {
                force_scheme: scheme,
                ..HighContrastSpec::default_aa()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: HighContrastSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.force_scheme, scheme);
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_high_contrast_form() {
        let src = r##"
            (defhigh-contrast :name "night"
                              :host "*"
                              :min-ratio 7.0
                              :force-scheme "dark"
                              :foreground "#f8f8f2"
                              :background "#1e1e2e"
                              :link "#89b4fa"
                              :link-boost #t
                              :focus-ring-px 4)
        "##;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.wcag_level(), WcagLevel::Aaa);
        assert_eq!(s.force_scheme, SchemeOverride::Dark);
        assert_eq!(s.foreground.as_deref(), Some("#f8f8f2"));
    }
}
