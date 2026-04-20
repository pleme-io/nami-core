//! `(deftext-spacing)` — WCAG 1.4.12 text-spacing override per host.
//!
//! Absorbs stylesheet bookmarklets (Stylebot, Readable, Stylish) and
//! Firefox's "Always underline links + override page fonts" prefs.
//! WCAG 1.4.12 says content must remain readable when:
//!
//!   * line-height ≥ 1.5 × font-size
//!   * spacing after a paragraph ≥ 2 × font-size
//!   * letter-spacing ≥ 0.12em
//!   * word-spacing ≥ 0.16em
//!
//! This DSL lets authors enforce those floors per host (or go
//! further — OpenDyslexic, Atkinson Hyperlegible, max_line_width,
//! underline links, remove italics).
//!
//! ```lisp
//! (deftext-spacing :name         "wcag-strict"
//!                  :host         "*"
//!                  :line-height  1.5
//!                  :paragraph-spacing 2.0
//!                  :letter-spacing 0.12
//!                  :word-spacing 0.16
//!                  :font-override :atkinson-hyperlegible
//!                  :max-line-width-ch 72
//!                  :underline-links #t
//!                  :remove-italics #f)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Curated reading-friendly font override.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FontOverride {
    /// Passthrough — honor the page's font stack.
    #[default]
    None,
    /// OpenDyslexic (open-source dyslexia-friendly typeface).
    OpenDyslexic,
    /// Atkinson Hyperlegible (Braille Institute, low-vision).
    AtkinsonHyperlegible,
    /// Lexend (reading-proficiency research).
    Lexend,
    /// System UI default (whatever OS picks).
    SystemUi,
    /// Monospace (code-centric browsing).
    Monospace,
    /// Serif (print-book mood).
    Serif,
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "deftext-spacing"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextSpacingSpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    /// Minimum line-height as a multiple of font-size. WCAG floor: 1.5.
    #[serde(default = "default_line_height")]
    pub line_height: f32,
    /// Minimum paragraph spacing as a multiple of font-size. WCAG
    /// floor: 2.0.
    #[serde(default = "default_paragraph_spacing")]
    pub paragraph_spacing: f32,
    /// Minimum letter-spacing in em. WCAG floor: 0.12.
    #[serde(default = "default_letter_spacing")]
    pub letter_spacing: f32,
    /// Minimum word-spacing in em. WCAG floor: 0.16.
    #[serde(default = "default_word_spacing")]
    pub word_spacing: f32,
    /// Font-family override (or None = passthrough).
    #[serde(default)]
    pub font_override: FontOverride,
    /// Custom `font-family` list (wins over `font_override` when set).
    #[serde(default)]
    pub font_family: Option<String>,
    /// Minimum font-size in pixels (0 = no floor).
    #[serde(default)]
    pub min_font_px: u32,
    /// Maximum line width in `ch` units (0 = don't constrain). WCAG
    /// says 80 is ceiling for accessibility; 72 is common.
    #[serde(default)]
    pub max_line_width_ch: u32,
    /// Underline every link (good for color-blind users).
    #[serde(default)]
    pub underline_links: bool,
    /// Strip italics (italic is the #1 dyslexia stumbling block).
    #[serde(default)]
    pub remove_italics: bool,
    /// Strip any decorative text-shadow / text-stroke (clean-read mode).
    #[serde(default)]
    pub remove_text_effects: bool,
    /// Whether to apply the `!important` marker on every rule so
    /// page CSS cannot override the floors.
    #[serde(default = "default_enforce")]
    pub enforce: bool,
    /// Hosts exempt from this profile (e.g. code-editor UIs where
    /// monospace letter-spacing would break layout).
    #[serde(default)]
    pub exempt_hosts: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_line_height() -> f32 {
    1.5
}
fn default_paragraph_spacing() -> f32 {
    2.0
}
fn default_letter_spacing() -> f32 {
    0.12
}
fn default_word_spacing() -> f32 {
    0.16
}
fn default_enforce() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

impl TextSpacingSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "wcag".into(),
            host: "*".into(),
            line_height: 1.5,
            paragraph_spacing: 2.0,
            letter_spacing: 0.12,
            word_spacing: 0.16,
            font_override: FontOverride::None,
            font_family: None,
            min_font_px: 0,
            max_line_width_ch: 0,
            underline_links: false,
            remove_italics: false,
            remove_text_effects: false,
            enforce: true,
            exempt_hosts: vec![],
            enabled: true,
            description: Some(
                "Default — WCAG 1.4.12 floors (1.5 line / 2.0 paragraph / 0.12em letter / 0.16em word), !important enforced.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    #[must_use]
    pub fn is_exempt(&self, host: &str) -> bool {
        self.exempt_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    /// Does this profile meet the WCAG 1.4.12 floors?
    #[must_use]
    pub fn is_wcag_compliant(&self) -> bool {
        self.line_height >= 1.5
            && self.paragraph_spacing >= 2.0
            && self.letter_spacing >= 0.12
            && self.word_spacing >= 0.16
    }

    /// Resolved `font-family` string — `font_family` if set, else
    /// derived from `font_override`.
    #[must_use]
    pub fn resolved_font(&self) -> Option<String> {
        if let Some(f) = &self.font_family {
            return Some(f.clone());
        }
        match self.font_override {
            FontOverride::None => None,
            FontOverride::OpenDyslexic => Some("'OpenDyslexic', sans-serif".into()),
            FontOverride::AtkinsonHyperlegible => {
                Some("'Atkinson Hyperlegible', sans-serif".into())
            }
            FontOverride::Lexend => Some("'Lexend', sans-serif".into()),
            FontOverride::SystemUi => Some("system-ui, sans-serif".into()),
            FontOverride::Monospace => {
                Some("'SF Mono', 'JetBrains Mono', monospace".into())
            }
            FontOverride::Serif => Some("Georgia, 'Times New Roman', serif".into()),
        }
    }

    /// Render the complete CSS stylesheet for this profile.
    /// `!important` is appended on every rule when `enforce = true`.
    #[must_use]
    pub fn render_css(&self) -> String {
        let imp = if self.enforce { " !important" } else { "" };
        let mut out = String::new();

        // body-wide rules
        out.push_str("body, html {\n");
        out.push_str(&format!(
            "  line-height: {}{imp};\n",
            self.line_height
        ));
        out.push_str(&format!(
            "  letter-spacing: {}em{imp};\n",
            self.letter_spacing
        ));
        out.push_str(&format!(
            "  word-spacing: {}em{imp};\n",
            self.word_spacing
        ));
        if let Some(font) = self.resolved_font() {
            out.push_str(&format!("  font-family: {font}{imp};\n"));
        }
        if self.min_font_px != 0 {
            out.push_str(&format!(
                "  font-size: max({}px, 1em){imp};\n",
                self.min_font_px
            ));
        }
        if self.max_line_width_ch != 0 {
            out.push_str(&format!(
                "  max-width: {}ch{imp};\n",
                self.max_line_width_ch
            ));
        }
        out.push_str("}\n");

        // paragraph spacing
        out.push_str("p, li, dl, dt, dd, blockquote {\n");
        out.push_str(&format!(
            "  margin-bottom: {}em{imp};\n",
            self.paragraph_spacing
        ));
        out.push_str("}\n");

        if self.underline_links {
            out.push_str(&format!(
                "a {{ text-decoration: underline{imp}; }}\n"
            ));
        }
        if self.remove_italics {
            out.push_str(&format!(
                "em, i, cite, dfn, var {{ font-style: normal{imp}; }}\n"
            ));
        }
        if self.remove_text_effects {
            out.push_str(&format!(
                "* {{ text-shadow: none{imp}; -webkit-text-stroke: 0{imp}; }}\n"
            ));
        }

        out
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct TextSpacingRegistry {
    specs: Vec<TextSpacingSpec>,
}

impl TextSpacingRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: TextSpacingSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = TextSpacingSpec>) {
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
    pub fn specs(&self) -> &[TextSpacingSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&TextSpacingSpec> {
        let specific = self.specs.iter().find(|s| {
            s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host) && !s.is_exempt(host)
        });
        specific.or_else(|| {
            self.specs
                .iter()
                .find(|s| s.enabled && s.matches_host(host) && !s.is_exempt(host))
        })
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<TextSpacingSpec>, String> {
    tatara_lisp::compile_typed::<TextSpacingSpec>(src)
        .map_err(|e| format!("failed to compile deftext-spacing forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<TextSpacingSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_meets_wcag_14_12() {
        let s = TextSpacingSpec::default_profile();
        assert!(s.is_wcag_compliant());
        assert!(s.enforce);
    }

    #[test]
    fn is_wcag_compliant_catches_under_floor() {
        for bad in [
            TextSpacingSpec {
                line_height: 1.4,
                ..TextSpacingSpec::default_profile()
            },
            TextSpacingSpec {
                paragraph_spacing: 1.9,
                ..TextSpacingSpec::default_profile()
            },
            TextSpacingSpec {
                letter_spacing: 0.1,
                ..TextSpacingSpec::default_profile()
            },
            TextSpacingSpec {
                word_spacing: 0.15,
                ..TextSpacingSpec::default_profile()
            },
        ] {
            assert!(!bad.is_wcag_compliant());
        }
    }

    #[test]
    fn resolved_font_passes_through_custom_family() {
        let s = TextSpacingSpec {
            font_family: Some("'My Font', serif".into()),
            font_override: FontOverride::OpenDyslexic,
            ..TextSpacingSpec::default_profile()
        };
        assert_eq!(s.resolved_font().unwrap(), "'My Font', serif");
    }

    #[test]
    fn resolved_font_maps_override_to_stack() {
        for (o, needle) in [
            (FontOverride::OpenDyslexic, "OpenDyslexic"),
            (FontOverride::AtkinsonHyperlegible, "Atkinson Hyperlegible"),
            (FontOverride::Lexend, "Lexend"),
            (FontOverride::SystemUi, "system-ui"),
            (FontOverride::Monospace, "monospace"),
            (FontOverride::Serif, "serif"),
        ] {
            let s = TextSpacingSpec {
                font_override: o,
                font_family: None,
                ..TextSpacingSpec::default_profile()
            };
            let f = s.resolved_font().unwrap();
            assert!(f.contains(needle), "{o:?} → {f}");
        }
    }

    #[test]
    fn resolved_font_none_for_passthrough_and_no_family() {
        let s = TextSpacingSpec {
            font_override: FontOverride::None,
            font_family: None,
            ..TextSpacingSpec::default_profile()
        };
        assert!(s.resolved_font().is_none());
    }

    #[test]
    fn render_css_default_contains_wcag_values() {
        let css = TextSpacingSpec::default_profile().render_css();
        assert!(css.contains("line-height: 1.5"));
        assert!(css.contains("letter-spacing: 0.12em"));
        assert!(css.contains("word-spacing: 0.16em"));
        assert!(css.contains("margin-bottom: 2em"));
        assert!(css.contains("!important"));
    }

    #[test]
    fn render_css_omits_important_when_not_enforced() {
        let s = TextSpacingSpec {
            enforce: false,
            ..TextSpacingSpec::default_profile()
        };
        let css = s.render_css();
        assert!(!css.contains("!important"));
    }

    #[test]
    fn render_css_includes_font_and_limits_when_set() {
        let s = TextSpacingSpec {
            font_override: FontOverride::AtkinsonHyperlegible,
            min_font_px: 16,
            max_line_width_ch: 72,
            underline_links: true,
            remove_italics: true,
            remove_text_effects: true,
            ..TextSpacingSpec::default_profile()
        };
        let css = s.render_css();
        assert!(css.contains("Atkinson Hyperlegible"));
        assert!(css.contains("font-size: max(16px, 1em)"));
        assert!(css.contains("max-width: 72ch"));
        assert!(css.contains("text-decoration: underline"));
        assert!(css.contains("font-style: normal"));
        assert!(css.contains("text-shadow: none"));
    }

    #[test]
    fn render_css_skips_optional_blocks_by_default() {
        let css = TextSpacingSpec::default_profile().render_css();
        assert!(!css.contains("text-decoration: underline"));
        assert!(!css.contains("font-style: normal"));
        assert!(!css.contains("text-shadow: none"));
        assert!(!css.contains("max-width"));
        assert!(!css.contains("font-size: max"));
    }

    #[test]
    fn matches_host_glob() {
        let s = TextSpacingSpec {
            host: "*://*.dyslexia-friendly.com/*".into(),
            ..TextSpacingSpec::default_profile()
        };
        assert!(s.matches_host("www.dyslexia-friendly.com"));
        assert!(!s.matches_host("example.com"));
    }

    #[test]
    fn is_exempt_matches_glob() {
        let s = TextSpacingSpec {
            exempt_hosts: vec!["*://*.code.com/*".into()],
            ..TextSpacingSpec::default_profile()
        };
        assert!(s.is_exempt("www.code.com"));
        assert!(!s.is_exempt("example.com"));
    }

    #[test]
    fn font_override_roundtrips_through_serde() {
        for f in [
            FontOverride::None,
            FontOverride::OpenDyslexic,
            FontOverride::AtkinsonHyperlegible,
            FontOverride::Lexend,
            FontOverride::SystemUi,
            FontOverride::Monospace,
            FontOverride::Serif,
        ] {
            let s = TextSpacingSpec {
                font_override: f,
                ..TextSpacingSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: TextSpacingSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.font_override, f);
        }
    }

    #[test]
    fn registry_prefers_specific_host_and_respects_exempt() {
        let mut reg = TextSpacingRegistry::new();
        reg.insert(TextSpacingSpec::default_profile());
        reg.insert(TextSpacingSpec {
            name: "aggressive".into(),
            host: "*://*.reader.com/*".into(),
            line_height: 2.0,
            paragraph_spacing: 3.0,
            ..TextSpacingSpec::default_profile()
        });
        let reader = reg.resolve("www.reader.com").unwrap();
        assert_eq!(reader.name, "aggressive");
        let other = reg.resolve("example.org").unwrap();
        assert_eq!(other.name, "wcag");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = TextSpacingRegistry::new();
        reg.insert(TextSpacingSpec {
            enabled: false,
            ..TextSpacingSpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_text_spacing_form() {
        let src = r#"
            (deftext-spacing :name "dyslexia"
                             :host "*"
                             :line-height 1.75
                             :paragraph-spacing 2.5
                             :letter-spacing 0.16
                             :word-spacing 0.2
                             :font-override "open-dyslexic"
                             :min-font-px 18
                             :max-line-width-ch 72
                             :underline-links #t
                             :remove-italics #t
                             :remove-text-effects #t
                             :enforce #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.font_override, FontOverride::OpenDyslexic);
        assert!(s.remove_italics);
        assert_eq!(s.max_line_width_ch, 72);
        assert!(s.is_wcag_compliant());
    }
}
