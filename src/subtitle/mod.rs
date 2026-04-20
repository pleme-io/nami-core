//! `(defsubtitle)` — declarative caption / subtitle handling.
//!
//! Absorbs HTML5 `<track>` auto-load, Netflix/YouTube subtitle UI,
//! Safari/Edge live captions (platform TTS-derived), and addon-
//! flavored auto-translation. Each profile binds a host to
//! language preferences, allowed formats, styling, and optional
//! auto-translate via the AI pack's LlmProvider trait.
//!
//! ```lisp
//! (defsubtitle :name               "default"
//!              :host               "*"
//!              :auto-load          #t
//!              :formats            (vtt srt ssa dfxp)
//!              :language-preferences ("en" "ja")
//!              :font-size-pct      120
//!              :position           :bottom
//!              :auto-translate     #f)
//!
//! (defsubtitle :name               "research"
//!              :host               "*://*.arxiv.org/*"
//!              :auto-translate     #t
//!              :auto-translate-to  "en"
//!              :auto-translate-via "claude")
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Subtitle file format.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum SubtitleFormat {
    /// WebVTT (`.vtt`) — HTML5 native format.
    Vtt,
    /// SubRip (`.srt`).
    Srt,
    /// SSA / ASS (`.ssa`, `.ass`) — advanced styling.
    Ssa,
    /// TTML / DFXP (`.ttml`, `.dfxp`) — Netflix / broadcast standard.
    Dfxp,
    /// Platform live captions — host OS speech-to-text stream, no file.
    PlatformLive,
}

/// Screen position.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SubtitlePosition {
    Bottom,
    Top,
    /// Aligned with the video's own internal caption-region metadata
    /// (VTT regions, ASS layout). Falls back to Bottom on failure.
    Native,
}

impl Default for SubtitlePosition {
    fn default() -> Self {
        Self::Bottom
    }
}

/// Subtitle profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsubtitle"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleSpec {
    pub name: String,
    /// Host glob. `"*"` = everywhere.
    #[serde(default = "default_host")]
    pub host: String,
    /// Auto-enable tracks when the video has them.
    #[serde(default = "default_auto_load")]
    pub auto_load: bool,
    /// Acceptable formats. Empty = every format.
    #[serde(default = "default_formats")]
    pub formats: Vec<SubtitleFormat>,
    /// BCP-47 language tags in preference order. First match wins.
    #[serde(default)]
    pub language_preferences: Vec<String>,
    /// Font-size as a percentage of video height. Clamped [50, 400].
    #[serde(default = "default_font_size_pct")]
    pub font_size_pct: u32,
    #[serde(default)]
    pub position: SubtitlePosition,
    /// Force a specific background color (CSS hex). None = browser default.
    #[serde(default)]
    pub background_color: Option<String>,
    /// Auto-translate captions when the requested language isn't
    /// available upstream. Routes through a `(defllm-provider)` when
    /// `auto_translate_via` is set.
    #[serde(default)]
    pub auto_translate: bool,
    /// Target language for auto-translate.
    #[serde(default)]
    pub auto_translate_to: Option<String>,
    /// `(defllm-provider)` name that handles the translation.
    #[serde(default)]
    pub auto_translate_via: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_auto_load() -> bool {
    true
}
fn default_formats() -> Vec<SubtitleFormat> {
    vec![
        SubtitleFormat::Vtt,
        SubtitleFormat::Srt,
        SubtitleFormat::Ssa,
        SubtitleFormat::Dfxp,
    ]
}
fn default_font_size_pct() -> u32 {
    100
}

const MIN_FONT: u32 = 50;
const MAX_FONT: u32 = 400;

impl SubtitleSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            auto_load: true,
            formats: default_formats(),
            language_preferences: vec![],
            font_size_pct: 100,
            position: SubtitlePosition::Bottom,
            background_color: None,
            auto_translate: false,
            auto_translate_to: None,
            auto_translate_via: None,
            description: Some(
                "Default subtitles — auto-load, VTT/SRT/SSA/DFXP, 100% font.".into(),
            ),
        }
    }

    #[must_use]
    pub fn clamped_font_size(&self) -> u32 {
        self.font_size_pct.clamp(MIN_FONT, MAX_FONT)
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    #[must_use]
    pub fn accepts_format(&self, f: SubtitleFormat) -> bool {
        self.formats.is_empty() || self.formats.contains(&f)
    }

    /// Given a list of `available` track languages, return the one
    /// that best matches the profile's preference order. Exact match
    /// beats prefix (`en-US` resolves when the profile prefers `en`).
    /// Returns None when no preference matches and preferences list
    /// is non-empty; returns the first available track when the list
    /// is empty.
    #[must_use]
    pub fn pick_language<'a>(&self, available: &'a [String]) -> Option<&'a String> {
        if self.language_preferences.is_empty() {
            return available.first();
        }
        for pref in &self.language_preferences {
            // Exact match.
            if let Some(a) = available.iter().find(|a| a.as_str() == pref.as_str()) {
                return Some(a);
            }
            // Prefix match — "en" matches "en-US", "en-GB".
            let prefix_dot = format!("{pref}-");
            if let Some(a) = available.iter().find(|a| a.starts_with(&prefix_dot)) {
                return Some(a);
            }
        }
        None
    }

    /// Whether auto-translate should fire given `available` tracks
    /// don't contain any preference.
    #[must_use]
    pub fn should_auto_translate(&self, available: &[String]) -> bool {
        self.auto_translate && self.pick_language(available).is_none()
    }
}

/// Registry — host-specific wins over wildcard.
#[derive(Debug, Clone, Default)]
pub struct SubtitleRegistry {
    specs: Vec<SubtitleSpec>,
}

impl SubtitleRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SubtitleSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SubtitleSpec>) {
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
    pub fn specs(&self) -> &[SubtitleSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&SubtitleSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SubtitleSpec>, String> {
    tatara_lisp::compile_typed::<SubtitleSpec>(src)
        .map_err(|e| format!("failed to compile defsubtitle forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SubtitleSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn langs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn clamped_font_size_respects_bounds() {
        let low = SubtitleSpec {
            font_size_pct: 10,
            ..SubtitleSpec::default_profile()
        };
        assert_eq!(low.clamped_font_size(), MIN_FONT);
        let high = SubtitleSpec {
            font_size_pct: 9999,
            ..SubtitleSpec::default_profile()
        };
        assert_eq!(high.clamped_font_size(), MAX_FONT);
        let ok = SubtitleSpec {
            font_size_pct: 120,
            ..SubtitleSpec::default_profile()
        };
        assert_eq!(ok.clamped_font_size(), 120);
    }

    #[test]
    fn accepts_format_default_list_covers_common() {
        let s = SubtitleSpec::default_profile();
        assert!(s.accepts_format(SubtitleFormat::Vtt));
        assert!(s.accepts_format(SubtitleFormat::Srt));
        assert!(s.accepts_format(SubtitleFormat::Ssa));
        assert!(s.accepts_format(SubtitleFormat::Dfxp));
        // Platform-live isn't in the default list.
        assert!(!s.accepts_format(SubtitleFormat::PlatformLive));
    }

    #[test]
    fn empty_format_list_accepts_everything() {
        let s = SubtitleSpec {
            formats: vec![],
            ..SubtitleSpec::default_profile()
        };
        assert!(s.accepts_format(SubtitleFormat::PlatformLive));
    }

    #[test]
    fn pick_language_exact_match() {
        let s = SubtitleSpec {
            language_preferences: langs(&["en", "ja"]),
            ..SubtitleSpec::default_profile()
        };
        let avail = langs(&["fr", "ja", "en"]);
        assert_eq!(s.pick_language(&avail).map(String::as_str), Some("en"));
    }

    #[test]
    fn pick_language_prefix_match() {
        let s = SubtitleSpec {
            language_preferences: langs(&["en"]),
            ..SubtitleSpec::default_profile()
        };
        let avail = langs(&["en-US", "fr"]);
        assert_eq!(s.pick_language(&avail).map(String::as_str), Some("en-US"));
    }

    #[test]
    fn pick_language_preference_order_wins() {
        let s = SubtitleSpec {
            language_preferences: langs(&["ja", "en"]),
            ..SubtitleSpec::default_profile()
        };
        let avail = langs(&["en-US", "ja"]);
        assert_eq!(s.pick_language(&avail).map(String::as_str), Some("ja"));
    }

    #[test]
    fn pick_language_no_prefs_returns_first_available() {
        let s = SubtitleSpec::default_profile();
        let avail = langs(&["de", "it"]);
        assert_eq!(s.pick_language(&avail).map(String::as_str), Some("de"));
    }

    #[test]
    fn pick_language_no_match_is_none() {
        let s = SubtitleSpec {
            language_preferences: langs(&["en"]),
            ..SubtitleSpec::default_profile()
        };
        let avail = langs(&["de", "it"]);
        assert!(s.pick_language(&avail).is_none());
    }

    #[test]
    fn should_auto_translate_requires_both_flag_and_miss() {
        let enabled = SubtitleSpec {
            auto_translate: true,
            language_preferences: langs(&["en"]),
            ..SubtitleSpec::default_profile()
        };
        assert!(enabled.should_auto_translate(&langs(&["de"])));
        assert!(!enabled.should_auto_translate(&langs(&["en-US"])));

        let disabled = SubtitleSpec {
            auto_translate: false,
            language_preferences: langs(&["en"]),
            ..SubtitleSpec::default_profile()
        };
        assert!(!disabled.should_auto_translate(&langs(&["de"])));
    }

    #[test]
    fn registry_dedupes_and_resolves_specific() {
        let mut reg = SubtitleRegistry::new();
        reg.insert(SubtitleSpec::default_profile());
        reg.insert(SubtitleSpec {
            name: "arxiv".into(),
            host: "*://*.arxiv.org/*".into(),
            auto_translate: true,
            auto_translate_to: Some("en".into()),
            auto_translate_via: Some("claude".into()),
            ..SubtitleSpec::default_profile()
        });
        let on_arxiv = reg.resolve("www.arxiv.org").unwrap();
        assert_eq!(on_arxiv.name, "arxiv");
        assert!(on_arxiv.auto_translate);
        let default = reg.resolve("example.com").unwrap();
        assert_eq!(default.name, "default");
    }

    #[test]
    fn format_and_position_roundtrip_through_serde() {
        for f in [
            SubtitleFormat::Vtt,
            SubtitleFormat::Srt,
            SubtitleFormat::Ssa,
            SubtitleFormat::Dfxp,
            SubtitleFormat::PlatformLive,
        ] {
            let s = SubtitleSpec {
                formats: vec![f],
                ..SubtitleSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: SubtitleSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.formats, vec![f]);
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_subtitle_form() {
        let src = r#"
            (defsubtitle :name "research"
                         :host "*://*.arxiv.org/*"
                         :auto-load #t
                         :formats ("vtt" "srt")
                         :language-preferences ("en" "ja")
                         :font-size-pct 120
                         :position "bottom"
                         :auto-translate #t
                         :auto-translate-to "en"
                         :auto-translate-via "claude")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert!(s.auto_translate);
        assert_eq!(s.auto_translate_to.as_deref(), Some("en"));
        assert_eq!(s.auto_translate_via.as_deref(), Some("claude"));
        assert_eq!(s.font_size_pct, 120);
    }
}
