//! `(defsimplify)` — cognitive-load reduction.
//!
//! Absorbs dyslexia-friendly reading plugins, "reader-flow" research
//! tools, and the broader ADHD/autism-spectrum UX mitigations the
//! web notoriously lacks. Each profile strips animations, slows
//! scroll momentum, enforces line-height minimums, optionally
//! highlights the current reading line, and can substitute fonts.
//!
//! Composes with (defreader) and (defhigh-contrast) — this layer
//! doesn't fight them, it cleans up *after* they've simplified the
//! page.
//!
//! ```lisp
//! (defsimplify :name           "focus-mode"
//!              :host           "*"
//!              :strip-animations   #t
//!              :strip-autoplay     #t
//!              :slow-scroll        :gentle
//!              :line-height-min    1.6
//!              :font-override      "Atkinson Hyperlegible"
//!              :reading-guide      #t
//!              :hide-sidebars      #t
//!              :reduce-motion      #t
//!              :spacing-boost-pct  120)
//!
//! (defsimplify :name      "dyslexia"
//!              :host      "*"
//!              :font-override "OpenDyslexic"
//!              :line-height-min 2.0
//!              :spacing-boost-pct 150
//!              :reading-guide #t)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Scroll-momentum damping.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ScrollDamping {
    /// Platform default.
    Native,
    /// Mild damping — ~70% of native velocity.
    Gentle,
    /// Strong damping — ~40% of native.
    Slow,
    /// Eliminate momentum — step-scroll only.
    StepOnly,
}

impl Default for ScrollDamping {
    fn default() -> Self {
        Self::Native
    }
}

impl ScrollDamping {
    /// Effective velocity multiplier to apply to wheel/touch events.
    #[must_use]
    pub fn velocity_scalar(self) -> f32 {
        match self {
            Self::Native => 1.0,
            Self::Gentle => 0.7,
            Self::Slow => 0.4,
            Self::StepOnly => 0.0,
        }
    }
}

/// Simplify profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsimplify"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SimplifySpec {
    pub name: String,
    /// Host glob. `"*"` = everywhere.
    #[serde(default = "default_host")]
    pub host: String,
    /// Strip CSS animations + transitions entirely.
    #[serde(default)]
    pub strip_animations: bool,
    /// Pause autoplaying video/audio on navigate.
    #[serde(default)]
    pub strip_autoplay: bool,
    /// Halt parallax / sticky-scroll-animation patterns.
    #[serde(default = "default_reduce_motion")]
    pub reduce_motion: bool,
    /// Scroll-momentum damping.
    #[serde(default)]
    pub slow_scroll: ScrollDamping,
    /// Minimum `line-height` multiplier. Clamped [1.0, 4.0].
    #[serde(default = "default_line_height_min")]
    pub line_height_min: f32,
    /// Font family forced on body text. Empty = keep site font.
    #[serde(default)]
    pub font_override: Option<String>,
    /// Overlay a horizontal reading guide under the current line
    /// (mouse or caret position).
    #[serde(default)]
    pub reading_guide: bool,
    /// Hide elements matching common sidebar selectors.
    #[serde(default)]
    pub hide_sidebars: bool,
    /// Letter-spacing + word-spacing boost as a percentage of default.
    /// Clamped [100, 400].
    #[serde(default = "default_spacing_boost_pct")]
    pub spacing_boost_pct: u32,
    /// Optional paragraph-spacing multiplier.
    #[serde(default = "default_paragraph_spacing")]
    pub paragraph_spacing_mult: f32,
    /// Extra selectors the user wants stripped on this profile.
    /// Append to the built-in sidebar list.
    #[serde(default)]
    pub extra_strip_selectors: Vec<String>,
    /// Runtime toggle.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_reduce_motion() -> bool {
    true
}
fn default_line_height_min() -> f32 {
    1.5
}
fn default_spacing_boost_pct() -> u32 {
    100
}
fn default_paragraph_spacing() -> f32 {
    1.3
}
fn default_enabled() -> bool {
    true
}

const MIN_LINE_HEIGHT: f32 = 1.0;
const MAX_LINE_HEIGHT: f32 = 4.0;
const MIN_SPACING: u32 = 100;
const MAX_SPACING: u32 = 400;

/// Built-in sidebar selectors hidden when `hide_sidebars = true`.
pub const DEFAULT_SIDEBAR_SELECTORS: &[&str] = &[
    "aside",
    "[role=complementary]",
    ".sidebar",
    "#sidebar",
    ".advertisement",
    ".promo",
    ".related",
    "[data-related]",
    ".share",
    ".social-share",
    ".comments",
    "#comments",
    "[aria-label*=advertisement i]",
];

impl SimplifySpec {
    #[must_use]
    pub fn focus_mode() -> Self {
        Self {
            name: "focus-mode".into(),
            host: "*".into(),
            strip_animations: true,
            strip_autoplay: true,
            reduce_motion: true,
            slow_scroll: ScrollDamping::Gentle,
            line_height_min: 1.6,
            font_override: Some("Atkinson Hyperlegible".into()),
            reading_guide: true,
            hide_sidebars: true,
            spacing_boost_pct: 120,
            paragraph_spacing_mult: 1.5,
            extra_strip_selectors: vec![],
            enabled: true,
            description: Some("Focus mode — strip animations, guide on, sidebars off.".into()),
        }
    }

    #[must_use]
    pub fn dyslexia_mode() -> Self {
        Self {
            name: "dyslexia".into(),
            host: "*".into(),
            strip_animations: true,
            strip_autoplay: false,
            reduce_motion: true,
            slow_scroll: ScrollDamping::Native,
            line_height_min: 2.0,
            font_override: Some("OpenDyslexic".into()),
            reading_guide: true,
            hide_sidebars: false,
            spacing_boost_pct: 150,
            paragraph_spacing_mult: 2.0,
            extra_strip_selectors: vec![],
            enabled: true,
            description: Some(
                "Dyslexia-friendly — OpenDyslexic font, wide spacing, reading guide.".into(),
            ),
        }
    }

    #[must_use]
    pub fn clamped_line_height(&self) -> f32 {
        self.line_height_min.clamp(MIN_LINE_HEIGHT, MAX_LINE_HEIGHT)
    }

    #[must_use]
    pub fn clamped_spacing(&self) -> u32 {
        self.spacing_boost_pct.clamp(MIN_SPACING, MAX_SPACING)
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    /// Every selector the host should hide on matched pages — the
    /// built-in sidebar list + whatever the spec appended.
    #[must_use]
    pub fn strip_selectors(&self) -> Vec<String> {
        let mut out: Vec<String> = if self.hide_sidebars {
            DEFAULT_SIDEBAR_SELECTORS.iter().map(|s| (*s).into()).collect()
        } else {
            Vec::new()
        };
        for s in &self.extra_strip_selectors {
            if !out.contains(s) {
                out.push(s.clone());
            }
        }
        out
    }

    /// Emit the `<style>` CSS block the host should inject to apply
    /// typography + motion overrides. Caller handles
    /// `display: none`-style selectors separately (via
    /// [`strip_selectors`]).
    #[must_use]
    pub fn inject_css(&self) -> String {
        let mut css = String::new();
        if self.reduce_motion {
            css.push_str("@media (prefers-reduced-motion: no-preference) { *, *::before, *::after { ");
            css.push_str("animation-duration: 0.001ms !important;");
            css.push_str("transition-duration: 0.001ms !important;");
            css.push_str(" } }\n");
        } else if self.strip_animations {
            css.push_str("*, *::before, *::after { ");
            css.push_str("animation: none !important;");
            css.push_str("transition: none !important;");
            css.push_str(" }\n");
        }
        let lh = self.clamped_line_height();
        let spacing = self.clamped_spacing();
        let para = self.paragraph_spacing_mult.max(0.5);
        css.push_str(&format!(
            "body, p, li, blockquote {{ line-height: {lh} !important; letter-spacing: {}em; word-spacing: {}em; }}\n",
            (spacing as f32 - 100.0) / 1000.0,
            (spacing as f32 - 100.0) / 500.0,
        ));
        css.push_str(&format!("p + p {{ margin-top: {}em !important; }}\n", para));
        if let Some(font) = &self.font_override {
            if !font.is_empty() {
                css.push_str(&format!(
                    "body, p, li, blockquote, h1, h2, h3, h4, h5, h6 {{ font-family: \"{font}\", system-ui, sans-serif !important; }}\n"
                ));
            }
        }
        css
    }
}

/// Registry — host-specific wins over wildcard.
#[derive(Debug, Clone, Default)]
pub struct SimplifyRegistry {
    specs: Vec<SimplifySpec>,
}

impl SimplifyRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SimplifySpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SimplifySpec>) {
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
    pub fn specs(&self) -> &[SimplifySpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&SimplifySpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SimplifySpec>, String> {
    tatara_lisp::compile_typed::<SimplifySpec>(src)
        .map_err(|e| format!("failed to compile defsimplify forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SimplifySpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_mode_sane_defaults() {
        let s = SimplifySpec::focus_mode();
        assert!(s.strip_animations);
        assert!(s.strip_autoplay);
        assert!(s.reduce_motion);
        assert!(s.hide_sidebars);
        assert!(s.reading_guide);
        assert_eq!(s.slow_scroll, ScrollDamping::Gentle);
    }

    #[test]
    fn dyslexia_mode_bumps_spacing_and_font() {
        let s = SimplifySpec::dyslexia_mode();
        assert_eq!(s.spacing_boost_pct, 150);
        assert_eq!(s.font_override.as_deref(), Some("OpenDyslexic"));
        assert!(s.line_height_min >= 2.0);
    }

    #[test]
    fn scroll_damping_velocity_scalars() {
        assert_eq!(ScrollDamping::Native.velocity_scalar(), 1.0);
        assert!((ScrollDamping::Gentle.velocity_scalar() - 0.7).abs() < 0.01);
        assert!((ScrollDamping::Slow.velocity_scalar() - 0.4).abs() < 0.01);
        assert_eq!(ScrollDamping::StepOnly.velocity_scalar(), 0.0);
    }

    #[test]
    fn clamped_line_height_respects_bounds() {
        let lo = SimplifySpec {
            line_height_min: 0.1,
            ..SimplifySpec::focus_mode()
        };
        assert_eq!(lo.clamped_line_height(), MIN_LINE_HEIGHT);
        let hi = SimplifySpec {
            line_height_min: 99.0,
            ..SimplifySpec::focus_mode()
        };
        assert_eq!(hi.clamped_line_height(), MAX_LINE_HEIGHT);
    }

    #[test]
    fn clamped_spacing_respects_bounds() {
        let lo = SimplifySpec {
            spacing_boost_pct: 50,
            ..SimplifySpec::focus_mode()
        };
        assert_eq!(lo.clamped_spacing(), MIN_SPACING);
        let hi = SimplifySpec {
            spacing_boost_pct: 9999,
            ..SimplifySpec::focus_mode()
        };
        assert_eq!(hi.clamped_spacing(), MAX_SPACING);
    }

    #[test]
    fn strip_selectors_includes_builtins_when_hide_sidebars() {
        let s = SimplifySpec::focus_mode();
        let sel = s.strip_selectors();
        assert!(sel.iter().any(|x| x == "aside"));
        assert!(sel.iter().any(|x| x == "[role=complementary]"));
    }

    #[test]
    fn strip_selectors_empty_when_hide_sidebars_off() {
        let s = SimplifySpec {
            hide_sidebars: false,
            ..SimplifySpec::focus_mode()
        };
        assert!(s.strip_selectors().is_empty());
    }

    #[test]
    fn strip_selectors_appends_user_list_without_dup() {
        let s = SimplifySpec {
            extra_strip_selectors: vec!["aside".into(), ".cookie-banner".into()],
            ..SimplifySpec::focus_mode()
        };
        let sel = s.strip_selectors();
        let count_aside = sel.iter().filter(|x| *x == "aside").count();
        assert_eq!(count_aside, 1);
        assert!(sel.iter().any(|x| x == ".cookie-banner"));
    }

    #[test]
    fn inject_css_mentions_line_height_and_font() {
        let s = SimplifySpec::dyslexia_mode();
        let css = s.inject_css();
        assert!(css.contains("line-height"));
        assert!(css.contains("OpenDyslexic"));
    }

    #[test]
    fn inject_css_handles_reduce_motion_strip_animations() {
        let s = SimplifySpec {
            reduce_motion: false,
            strip_animations: true,
            ..SimplifySpec::focus_mode()
        };
        let css = s.inject_css();
        assert!(css.contains("animation: none"));
    }

    #[test]
    fn registry_dedupes_and_resolves_specific_over_wildcard() {
        let mut reg = SimplifyRegistry::new();
        reg.insert(SimplifySpec::focus_mode());
        reg.insert(SimplifySpec {
            name: "news".into(),
            host: "*://*.ycombinator.com/*".into(),
            ..SimplifySpec::dyslexia_mode()
        });
        let news = reg.resolve("news.ycombinator.com").unwrap();
        assert_eq!(news.name, "news");
        let other = reg.resolve("example.com").unwrap();
        assert_eq!(other.name, "focus-mode");
    }

    #[test]
    fn disabled_profile_skipped_by_resolve() {
        let mut reg = SimplifyRegistry::new();
        reg.insert(SimplifySpec {
            enabled: false,
            ..SimplifySpec::focus_mode()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_simplify_form() {
        let src = r#"
            (defsimplify :name "focus-mode"
                         :host "*"
                         :strip-animations #t
                         :reduce-motion #t
                         :slow-scroll "gentle"
                         :line-height-min 1.6
                         :font-override "Atkinson Hyperlegible"
                         :reading-guide #t
                         :hide-sidebars #t
                         :spacing-boost-pct 120)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert!(s.strip_animations);
        assert_eq!(s.slow_scroll, ScrollDamping::Gentle);
        assert_eq!(s.spacing_boost_pct, 120);
    }
}
