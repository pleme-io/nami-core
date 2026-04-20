//! `(deffingerprint-randomize)` — canvas/WebGL/audio/font fingerprint
//! randomization policy.
//!
//! Absorbs Brave Shields farbling, Tor Browser letterboxing + resist-
//! fingerprinting, LibreWolf, Mullvad Browser, uBlock Origin
//! anti-fingerprint filters. Farbling = tiny per-session noise so
//! the same user looks different each session, and different from
//! every other user.
//!
//! ```lisp
//! (deffingerprint-randomize :name        "strict"
//!                           :host        "*"
//!                           :canvas      :noise
//!                           :webgl       :noise
//!                           :audio       :noise
//!                           :fonts       :randomize-metrics
//!                           :client-rects :noise
//!                           :user-agent   :generic
//!                           :session-scope :per-host)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// How a fingerprint surface is handled.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FingerprintMode {
    /// Pass the API through untouched.
    #[default]
    Allow,
    /// Add per-session noise (Brave "farbling" — tiny, deterministic
    /// per session-key).
    Noise,
    /// Return a canonical shape that every user sees (Tor approach).
    Generic,
    /// Block the API entirely — null/empty returns.
    Block,
    /// Mark the call and prompt the user.
    Prompt,
}

/// Font-fingerprint handling.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FontMode {
    #[default]
    Allow,
    /// Return only system-core fonts.
    SystemOnly,
    /// Randomize metric queries (Font Detection via character width).
    RandomizeMetrics,
    /// Block font enumeration entirely.
    Block,
}

/// User-agent shape.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UserAgentMode {
    #[default]
    Real,
    /// Latest-stable Firefox on Linux x86_64 (Tor convention).
    Generic,
    /// Per-session randomized — new UA each shell session.
    Randomize,
    /// Site-specific allow-list — passes real UA to trusted hosts.
    AllowList,
}

/// Scope at which the noise value is cached.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SessionScope {
    /// Same noise for the whole shell session — lowest friction.
    PerSession,
    /// Per-host + per-session — trackers can't cross-correlate hosts.
    #[default]
    PerHost,
    /// Per-tab + per-session — hardest, but breaks some sites.
    PerTab,
    /// Fresh noise on every single API call — near-useless for
    /// trackers but also breaks canvas rendering.
    PerCall,
}

/// Fingerprint-randomization profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "deffingerprint-randomize"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FingerprintRandomizeSpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub canvas: FingerprintMode,
    #[serde(default)]
    pub webgl: FingerprintMode,
    #[serde(default)]
    pub audio: FingerprintMode,
    #[serde(default)]
    pub client_rects: FingerprintMode,
    /// Pointer/hover media-query values — mobile detection surface.
    #[serde(default)]
    pub pointer_hover: FingerprintMode,
    /// prefers-color-scheme + prefers-reduced-motion reveal light
    /// style + accessibility toggles.
    #[serde(default)]
    pub prefers_media: FingerprintMode,
    /// Intl.DateTimeFormat().resolvedOptions() → locale fingerprint.
    #[serde(default)]
    pub locale: FingerprintMode,
    /// navigator.hardwareConcurrency / deviceMemory / platform.
    #[serde(default)]
    pub navigator_info: FingerprintMode,
    #[serde(default)]
    pub fonts: FontMode,
    #[serde(default)]
    pub user_agent: UserAgentMode,
    #[serde(default)]
    pub session_scope: SessionScope,
    /// Noise intensity in [0.0, 1.0] — higher breaks more sites.
    #[serde(default = "default_intensity")]
    pub intensity: f32,
    /// Hosts that are exempt (e.g. your bank, online game).
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
fn default_intensity() -> f32 {
    0.25
}
fn default_enabled() -> bool {
    true
}

impl FingerprintRandomizeSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            canvas: FingerprintMode::Noise,
            webgl: FingerprintMode::Noise,
            audio: FingerprintMode::Noise,
            client_rects: FingerprintMode::Allow,
            pointer_hover: FingerprintMode::Allow,
            prefers_media: FingerprintMode::Allow,
            locale: FingerprintMode::Allow,
            navigator_info: FingerprintMode::Generic,
            fonts: FontMode::RandomizeMetrics,
            user_agent: UserAgentMode::Real,
            session_scope: SessionScope::PerHost,
            intensity: 0.25,
            exempt_hosts: vec![],
            enabled: true,
            description: Some(
                "Default farbling — Brave-style noise on canvas/webgl/audio, metric font randomization, per-host session scope.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        if self.host.is_empty() || self.host == "*" {
            return true;
        }
        crate::extension::glob_match_host(&self.host, host)
    }

    /// Is `host` exempt (e.g. real banks, SSO)?
    #[must_use]
    pub fn is_exempt(&self, host: &str) -> bool {
        self.exempt_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    /// Clamped intensity in [0.0, 1.0].
    #[must_use]
    pub fn clamped_intensity(&self) -> f32 {
        self.intensity.clamp(0.0, 1.0)
    }

    /// Effective canvas mode at runtime — `Allow` on exempt hosts,
    /// else the declared mode.
    #[must_use]
    pub fn canvas_for(&self, host: &str) -> FingerprintMode {
        if self.is_exempt(host) {
            FingerprintMode::Allow
        } else {
            self.canvas
        }
    }

    #[must_use]
    pub fn webgl_for(&self, host: &str) -> FingerprintMode {
        if self.is_exempt(host) {
            FingerprintMode::Allow
        } else {
            self.webgl
        }
    }

    #[must_use]
    pub fn audio_for(&self, host: &str) -> FingerprintMode {
        if self.is_exempt(host) {
            FingerprintMode::Allow
        } else {
            self.audio
        }
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct FingerprintRandomizeRegistry {
    specs: Vec<FingerprintRandomizeSpec>,
}

impl FingerprintRandomizeRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: FingerprintRandomizeSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = FingerprintRandomizeSpec>) {
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
    pub fn specs(&self) -> &[FingerprintRandomizeSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&FingerprintRandomizeSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<FingerprintRandomizeSpec>, String> {
    tatara_lisp::compile_typed::<FingerprintRandomizeSpec>(src)
        .map_err(|e| format!("failed to compile deffingerprint-randomize forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<FingerprintRandomizeSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_noises_canvas_webgl_audio() {
        let s = FingerprintRandomizeSpec::default_profile();
        assert_eq!(s.canvas, FingerprintMode::Noise);
        assert_eq!(s.webgl, FingerprintMode::Noise);
        assert_eq!(s.audio, FingerprintMode::Noise);
        assert_eq!(s.navigator_info, FingerprintMode::Generic);
        assert_eq!(s.fonts, FontMode::RandomizeMetrics);
        assert_eq!(s.session_scope, SessionScope::PerHost);
    }

    #[test]
    fn clamped_intensity_stays_in_range() {
        let s = FingerprintRandomizeSpec {
            intensity: 2.5,
            ..FingerprintRandomizeSpec::default_profile()
        };
        assert!((s.clamped_intensity() - 1.0).abs() < 1e-5);
        let neg = FingerprintRandomizeSpec {
            intensity: -0.5,
            ..FingerprintRandomizeSpec::default_profile()
        };
        assert!(neg.clamped_intensity().abs() < 1e-5);
    }

    #[test]
    fn is_exempt_matches_glob() {
        let s = FingerprintRandomizeSpec {
            exempt_hosts: vec!["*://*.bank.com/*".into()],
            ..FingerprintRandomizeSpec::default_profile()
        };
        assert!(s.is_exempt("my.bank.com"));
        assert!(!s.is_exempt("trackers.com"));
    }

    #[test]
    fn canvas_for_flips_to_allow_on_exempt_host() {
        let s = FingerprintRandomizeSpec {
            exempt_hosts: vec!["*://*.bank.com/*".into()],
            ..FingerprintRandomizeSpec::default_profile()
        };
        assert_eq!(s.canvas_for("my.bank.com"), FingerprintMode::Allow);
        assert_eq!(s.canvas_for("trackers.com"), FingerprintMode::Noise);
    }

    #[test]
    fn webgl_for_and_audio_for_follow_same_exempt_rule() {
        let s = FingerprintRandomizeSpec {
            exempt_hosts: vec!["*://*.game.com/*".into()],
            ..FingerprintRandomizeSpec::default_profile()
        };
        assert_eq!(s.webgl_for("play.game.com"), FingerprintMode::Allow);
        assert_eq!(s.audio_for("play.game.com"), FingerprintMode::Allow);
        assert_eq!(s.webgl_for("other.com"), FingerprintMode::Noise);
    }

    #[test]
    fn fingerprint_mode_roundtrips_through_serde() {
        for m in [
            FingerprintMode::Allow,
            FingerprintMode::Noise,
            FingerprintMode::Generic,
            FingerprintMode::Block,
            FingerprintMode::Prompt,
        ] {
            let s = FingerprintRandomizeSpec {
                canvas: m,
                ..FingerprintRandomizeSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: FingerprintRandomizeSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.canvas, m);
        }
    }

    #[test]
    fn font_mode_roundtrips_through_serde() {
        for m in [
            FontMode::Allow,
            FontMode::SystemOnly,
            FontMode::RandomizeMetrics,
            FontMode::Block,
        ] {
            let s = FingerprintRandomizeSpec {
                fonts: m,
                ..FingerprintRandomizeSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: FingerprintRandomizeSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.fonts, m);
        }
    }

    #[test]
    fn session_scope_roundtrips_through_serde() {
        for s in [
            SessionScope::PerSession,
            SessionScope::PerHost,
            SessionScope::PerTab,
            SessionScope::PerCall,
        ] {
            let spec = FingerprintRandomizeSpec {
                session_scope: s,
                ..FingerprintRandomizeSpec::default_profile()
            };
            let json = serde_json::to_string(&spec).unwrap();
            let back: FingerprintRandomizeSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.session_scope, s);
        }
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = FingerprintRandomizeRegistry::new();
        reg.insert(FingerprintRandomizeSpec::default_profile());
        reg.insert(FingerprintRandomizeSpec {
            name: "strict-news".into(),
            host: "*://*.nytimes.com/*".into(),
            canvas: FingerprintMode::Block,
            ..FingerprintRandomizeSpec::default_profile()
        });
        let ny = reg.resolve("www.nytimes.com").unwrap();
        assert_eq!(ny.canvas, FingerprintMode::Block);
        let other = reg.resolve("example.org").unwrap();
        assert_eq!(other.name, "default");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = FingerprintRandomizeRegistry::new();
        reg.insert(FingerprintRandomizeSpec {
            enabled: false,
            ..FingerprintRandomizeSpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_fingerprint_form() {
        let src = r#"
            (deffingerprint-randomize :name "strict"
                                      :host "*"
                                      :canvas "noise"
                                      :webgl "generic"
                                      :audio "block"
                                      :fonts "system-only"
                                      :user-agent "generic"
                                      :session-scope "per-tab"
                                      :intensity 0.5)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.canvas, FingerprintMode::Noise);
        assert_eq!(s.webgl, FingerprintMode::Generic);
        assert_eq!(s.audio, FingerprintMode::Block);
        assert_eq!(s.fonts, FontMode::SystemOnly);
        assert_eq!(s.user_agent, UserAgentMode::Generic);
        assert_eq!(s.session_scope, SessionScope::PerTab);
        assert!((s.intensity - 0.5).abs() < 1e-5);
    }
}
