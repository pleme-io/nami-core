//! `(defreader-aloud)` — TTS (text-to-speech) page reading.
//!
//! Absorbs Safari Speak Screen, Edge Read Aloud, Chrome Select-to-
//! Speak, NVDA / JAWS / VoiceOver web plugins. Each profile declares
//! voice, rate, pitch, volume, source scope (reader text / selection /
//! whole page), and whether to highlight the currently-spoken
//! sentence.
//!
//! Composes with (defreader) for source extraction and oto (pleme-io
//! audio) or a platform speech API for synthesis.
//!
//! ```lisp
//! (defreader-aloud :name          "default"
//!                  :voice         "en-US-Neural"
//!                  :rate          1.0
//!                  :pitch         1.0
//!                  :volume        1.0
//!                  :scope         :reader-text
//!                  :highlight     #t
//!                  :stop-on-navigate #t)
//!
//! (defreader-aloud :name          "fast-skim"
//!                  :voice         "en-US-Neural-HQ"
//!                  :rate          1.8
//!                  :scope         :selection
//!                  :highlight     #t)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// What content to read.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReadScope {
    /// Full `<body>.text_content()`.
    WholePage,
    /// `(defreader)` simplified output — best signal.
    ReaderText,
    /// User selection only.
    Selection,
    /// A single element by CSS selector.
    Selector,
}

impl Default for ReadScope {
    fn default() -> Self {
        Self::ReaderText
    }
}

/// Speech source — how synthesis happens.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SpeechSource {
    /// Host platform Web Speech API (fastest, free, lower quality).
    Platform,
    /// pleme-io `oto` + a local neural model (best trade-off).
    Oto,
    /// Cloud TTS via an HTTP endpoint.
    Http,
    /// Route through an `(defllm-provider)` that exposes audio-out.
    Llm,
}

impl Default for SpeechSource {
    fn default() -> Self {
        Self::Platform
    }
}

/// Reader-aloud profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defreader-aloud"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReaderAloudSpec {
    pub name: String,
    /// Voice identifier. Platform-specific — BCP-47 tag or a neural-
    /// model name. Empty = platform default.
    #[serde(default)]
    pub voice: String,
    /// Playback rate (1.0 = normal). Clamped [0.25, 4.0].
    #[serde(default = "default_rate")]
    pub rate: f32,
    /// Pitch shift (1.0 = normal). Clamped [0.25, 2.0].
    #[serde(default = "default_pitch")]
    pub pitch: f32,
    /// Volume (1.0 = full). Clamped [0.0, 1.0].
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub scope: ReadScope,
    /// CSS selector when `scope = Selector`.
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub source: SpeechSource,
    /// Highlight the currently-spoken sentence in the page.
    #[serde(default = "default_highlight")]
    pub highlight: bool,
    /// Highlight color (CSS hex). Default yellow when `highlight = true`.
    #[serde(default = "default_highlight_color")]
    pub highlight_color: String,
    /// Stop playback on any navigate event.
    #[serde(default = "default_stop_on_navigate")]
    pub stop_on_navigate: bool,
    /// Auto-start playback when the page finishes loading.
    #[serde(default)]
    pub auto_start: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_rate() -> f32 {
    1.0
}
fn default_pitch() -> f32 {
    1.0
}
fn default_volume() -> f32 {
    1.0
}
fn default_highlight() -> bool {
    true
}
fn default_highlight_color() -> String {
    "#ffff00".into()
}
fn default_stop_on_navigate() -> bool {
    true
}

impl ReaderAloudSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            voice: String::new(),
            rate: 1.0,
            pitch: 1.0,
            volume: 1.0,
            scope: ReadScope::ReaderText,
            selector: None,
            source: SpeechSource::Platform,
            highlight: true,
            highlight_color: "#ffff00".into(),
            stop_on_navigate: true,
            auto_start: false,
            description: Some(
                "Default read-aloud — platform voice, reader-text scope.".into(),
            ),
        }
    }

    #[must_use]
    pub fn clamped_rate(&self) -> f32 {
        self.rate.clamp(0.25, 4.0)
    }

    #[must_use]
    pub fn clamped_pitch(&self) -> f32 {
        self.pitch.clamp(0.25, 2.0)
    }

    #[must_use]
    pub fn clamped_volume(&self) -> f32 {
        self.volume.clamp(0.0, 1.0)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("reader-aloud name is empty".into());
        }
        if matches!(self.scope, ReadScope::Selector) && self.selector.is_none() {
            return Err(format!(
                "reader-aloud '{}' :scope :selector needs :selector field",
                self.name
            ));
        }
        Ok(())
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct ReaderAloudRegistry {
    specs: Vec<ReaderAloudSpec>,
}

impl ReaderAloudRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: ReaderAloudSpec) -> Result<(), String> {
        spec.validate()?;
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
        Ok(())
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ReaderAloudSpec>) {
        for s in specs {
            if let Err(e) = self.insert(s.clone()) {
                tracing::warn!("defreader-aloud '{}' rejected: {}", s.name, e);
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
    pub fn specs(&self) -> &[ReaderAloudSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ReaderAloudSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ReaderAloudSpec>, String> {
    tatara_lisp::compile_typed::<ReaderAloudSpec>(src)
        .map_err(|e| format!("failed to compile defreader-aloud forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ReaderAloudSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_platform_reader_text() {
        let s = ReaderAloudSpec::default_profile();
        assert_eq!(s.scope, ReadScope::ReaderText);
        assert_eq!(s.source, SpeechSource::Platform);
        assert!(s.highlight);
        assert!(s.stop_on_navigate);
    }

    #[test]
    fn clamped_rate_respects_bounds() {
        let slow = ReaderAloudSpec {
            rate: 0.1,
            ..ReaderAloudSpec::default_profile()
        };
        assert_eq!(slow.clamped_rate(), 0.25);
        let fast = ReaderAloudSpec {
            rate: 10.0,
            ..ReaderAloudSpec::default_profile()
        };
        assert_eq!(fast.clamped_rate(), 4.0);
    }

    #[test]
    fn clamped_pitch_respects_bounds() {
        assert_eq!(
            ReaderAloudSpec {
                pitch: 10.0,
                ..ReaderAloudSpec::default_profile()
            }
            .clamped_pitch(),
            2.0
        );
        assert_eq!(
            ReaderAloudSpec {
                pitch: -1.0,
                ..ReaderAloudSpec::default_profile()
            }
            .clamped_pitch(),
            0.25
        );
    }

    #[test]
    fn clamped_volume_unit_interval() {
        assert_eq!(
            ReaderAloudSpec {
                volume: 99.0,
                ..ReaderAloudSpec::default_profile()
            }
            .clamped_volume(),
            1.0
        );
        assert_eq!(
            ReaderAloudSpec {
                volume: -0.5,
                ..ReaderAloudSpec::default_profile()
            }
            .clamped_volume(),
            0.0
        );
    }

    #[test]
    fn selector_scope_requires_selector_field() {
        let s = ReaderAloudSpec {
            scope: ReadScope::Selector,
            selector: None,
            ..ReaderAloudSpec::default_profile()
        };
        assert!(s.validate().is_err());
        let s2 = ReaderAloudSpec {
            scope: ReadScope::Selector,
            selector: Some("article".into()),
            ..ReaderAloudSpec::default_profile()
        };
        assert!(s2.validate().is_ok());
    }

    #[test]
    fn scope_roundtrips_through_serde() {
        for scope in [
            ReadScope::WholePage,
            ReadScope::ReaderText,
            ReadScope::Selection,
            ReadScope::Selector,
        ] {
            let s = ReaderAloudSpec {
                scope,
                selector: if matches!(scope, ReadScope::Selector) {
                    Some("article".into())
                } else {
                    None
                },
                ..ReaderAloudSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: ReaderAloudSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.scope, scope);
        }
    }

    #[test]
    fn registry_insert_validates_and_dedupes() {
        let mut reg = ReaderAloudRegistry::new();
        assert!(reg
            .insert(ReaderAloudSpec {
                name: String::new(),
                ..ReaderAloudSpec::default_profile()
            })
            .is_err());
        reg.insert(ReaderAloudSpec::default_profile()).unwrap();
        reg.insert(ReaderAloudSpec {
            rate: 2.0,
            ..ReaderAloudSpec::default_profile()
        })
        .unwrap();
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].rate, 2.0);
    }

    #[test]
    fn default_highlight_color_is_yellow() {
        assert_eq!(
            ReaderAloudSpec::default_profile().highlight_color,
            "#ffff00"
        );
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_reader_aloud_form() {
        let src = r#"
            (defreader-aloud :name "fast-skim"
                             :voice "en-US-Neural-HQ"
                             :rate 1.8
                             :pitch 1.0
                             :volume 0.8
                             :scope "selection"
                             :highlight #t
                             :stop-on-navigate #t
                             :auto-start #f)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "fast-skim");
        assert_eq!(s.scope, ReadScope::Selection);
        assert!((s.rate - 1.8).abs() < 0.01);
    }
}
