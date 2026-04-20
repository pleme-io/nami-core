//! `(defautoplay)` — per-host media autoplay policy.
//!
//! Absorbs Chrome autoplay policy (user-gesture-required /
//! document-user-activation-required / no-user-gesture-required),
//! Safari "Auto-Play" per-site toggle (Allow All Auto-Play / Stop
//! Media with Sound / Never Auto-Play), Firefox media.autoplay.default
//! (Allow/Block Audio/Block Audio and Video) + media.autoplay.blocking_policy.
//! Nobody ships a host-glob–driven, declarative authoring surface.
//!
//! ```lisp
//! (defautoplay :name           "default"
//!              :host           "*"
//!              :policy         :block-audio
//!              :muted-video-ok #t
//!              :allow-after-interaction #t)
//!
//! (defautoplay :name           "youtube-allow"
//!              :host           "*://*.youtube.com/*"
//!              :policy         :allow-all)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// What autoplay the host grants.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AutoplayPolicy {
    /// No restrictions — sites can autoplay anything.
    AllowAll,
    /// Block audio playback until user interaction; muted video is fine.
    #[default]
    BlockAudio,
    /// Block all autoplay (muted included) until user interaction.
    BlockAll,
    /// Never autoplay — even post-interaction requires an explicit gesture.
    Never,
}

/// Track kinds the policy applies to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum TrackKind {
    AudioElement,
    VideoElement,
    WebRtc,
    MediaSession,
}

/// Autoplay policy spec.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defautoplay"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AutoplaySpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub policy: AutoplayPolicy,
    /// Allow autoplay of muted `<video>` regardless of policy (Chrome default).
    #[serde(default = "default_muted_video_ok")]
    pub muted_video_ok: bool,
    /// Lift the block once the user has interacted with the document
    /// (click / keydown / touch). Matches Chrome's
    /// "document-user-activation-required".
    #[serde(default = "default_allow_after_interaction")]
    pub allow_after_interaction: bool,
    /// Lift the block for sites with high Media Engagement Index (MEI) —
    /// Chrome's "frequently used for media" heuristic.
    #[serde(default)]
    pub allow_on_high_mei: bool,
    /// Which track kinds the block applies to. Empty = all.
    #[serde(default)]
    pub applies_to: Vec<TrackKind>,
    /// Pause already-playing autoplay-started audio when the tab is
    /// backgrounded. Absorbs Firefox `media.block-autoplay-until-in-foreground`.
    #[serde(default = "default_pause_in_background")]
    pub pause_in_background: bool,
    /// When blocking, also suppress the tab-strip play-indicator to
    /// reduce UI clutter.
    #[serde(default)]
    pub suppress_play_indicator: bool,
    /// Exempt hosts (glob) — always AllowAll regardless of other fields.
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
fn default_muted_video_ok() -> bool {
    true
}
fn default_allow_after_interaction() -> bool {
    true
}
fn default_pause_in_background() -> bool {
    false
}
fn default_enabled() -> bool {
    true
}

/// Runtime context for the autoplay decision.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlaybackContext {
    pub muted: bool,
    pub user_has_interacted: bool,
    pub high_mei: bool,
    pub tab_backgrounded: bool,
    pub kind: Option<TrackKind>,
}

impl AutoplaySpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            policy: AutoplayPolicy::BlockAudio,
            muted_video_ok: true,
            allow_after_interaction: true,
            allow_on_high_mei: true,
            applies_to: vec![],
            pause_in_background: false,
            suppress_play_indicator: false,
            exempt_hosts: vec![],
            enabled: true,
            description: Some("Chrome-style: muted video autoplays, audio blocked until interaction.".into()),
        }
    }

    #[must_use]
    pub fn block_all_profile() -> Self {
        Self {
            name: "block-all".into(),
            policy: AutoplayPolicy::BlockAll,
            muted_video_ok: false,
            allow_after_interaction: true,
            allow_on_high_mei: false,
            description: Some("Strict: nothing autoplays until user interacts.".into()),
            ..Self::default_profile()
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
            .any(|g| crate::extension::host_pattern_matches(g, host))
    }

    #[must_use]
    pub fn applies_to_kind(&self, kind: TrackKind) -> bool {
        self.applies_to.is_empty() || self.applies_to.contains(&kind)
    }

    /// Core decision: may a media element begin playback without a gesture?
    #[must_use]
    pub fn admits_autoplay(&self, host: &str, ctx: PlaybackContext) -> bool {
        if !self.enabled {
            return true;
        }
        if self.is_exempt(host) {
            return true;
        }
        if let Some(k) = ctx.kind {
            if !self.applies_to_kind(k) {
                return true;
            }
        }
        match self.policy {
            AutoplayPolicy::AllowAll => true,
            AutoplayPolicy::BlockAudio => {
                ctx.muted
                    || (self.allow_after_interaction && ctx.user_has_interacted)
                    || (self.allow_on_high_mei && ctx.high_mei)
            }
            AutoplayPolicy::BlockAll => {
                (self.muted_video_ok
                    && ctx.muted
                    && matches!(ctx.kind, Some(TrackKind::VideoElement)))
                    || (self.allow_after_interaction && ctx.user_has_interacted)
                    || (self.allow_on_high_mei && ctx.high_mei)
            }
            AutoplayPolicy::Never => false,
        }
    }

    /// Should an already-playing track be paused because the tab went
    /// to the background?
    #[must_use]
    pub fn should_pause_backgrounded(&self, ctx: PlaybackContext) -> bool {
        self.enabled
            && self.pause_in_background
            && ctx.tab_backgrounded
            && !ctx.muted
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct AutoplayRegistry {
    specs: Vec<AutoplaySpec>,
}

impl AutoplayRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: AutoplaySpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = AutoplaySpec>) {
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
    pub fn specs(&self) -> &[AutoplaySpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&AutoplaySpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<AutoplaySpec>, String> {
    tatara_lisp::compile_typed::<AutoplaySpec>(src)
        .map_err(|e| format!("failed to compile defautoplay forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<AutoplaySpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> PlaybackContext {
        PlaybackContext::default()
    }

    #[test]
    fn default_profile_allows_muted_video() {
        let s = AutoplaySpec::default_profile();
        let c = PlaybackContext {
            muted: true,
            kind: Some(TrackKind::VideoElement),
            ..ctx()
        };
        assert!(s.admits_autoplay("example.com", c));
    }

    #[test]
    fn default_profile_blocks_unmuted_audio() {
        let s = AutoplaySpec::default_profile();
        let c = PlaybackContext {
            muted: false,
            kind: Some(TrackKind::AudioElement),
            ..ctx()
        };
        assert!(!s.admits_autoplay("example.com", c));
    }

    #[test]
    fn allow_after_interaction_unblocks() {
        let s = AutoplaySpec::default_profile();
        let c = PlaybackContext {
            muted: false,
            user_has_interacted: true,
            kind: Some(TrackKind::AudioElement),
            ..ctx()
        };
        assert!(s.admits_autoplay("example.com", c));
    }

    #[test]
    fn never_policy_blocks_everything() {
        let s = AutoplaySpec {
            policy: AutoplayPolicy::Never,
            ..AutoplaySpec::default_profile()
        };
        let c = PlaybackContext {
            muted: true,
            user_has_interacted: true,
            high_mei: true,
            kind: Some(TrackKind::VideoElement),
            ..ctx()
        };
        assert!(!s.admits_autoplay("example.com", c));
    }

    #[test]
    fn block_all_allows_muted_video_when_muted_video_ok() {
        let s = AutoplaySpec {
            policy: AutoplayPolicy::BlockAll,
            muted_video_ok: true,
            allow_after_interaction: false,
            allow_on_high_mei: false,
            ..AutoplaySpec::default_profile()
        };
        let c = PlaybackContext {
            muted: true,
            kind: Some(TrackKind::VideoElement),
            ..ctx()
        };
        assert!(s.admits_autoplay("example.com", c));
    }

    #[test]
    fn block_all_rejects_muted_audio_element() {
        let s = AutoplaySpec {
            policy: AutoplayPolicy::BlockAll,
            muted_video_ok: true,
            allow_after_interaction: false,
            allow_on_high_mei: false,
            ..AutoplaySpec::default_profile()
        };
        let c = PlaybackContext {
            muted: true,
            kind: Some(TrackKind::AudioElement),
            ..ctx()
        };
        assert!(!s.admits_autoplay("example.com", c));
    }

    #[test]
    fn high_mei_bypasses_block_audio() {
        let s = AutoplaySpec::default_profile();
        let c = PlaybackContext {
            muted: false,
            high_mei: true,
            kind: Some(TrackKind::AudioElement),
            ..ctx()
        };
        assert!(s.admits_autoplay("example.com", c));
    }

    #[test]
    fn exempt_host_always_allows() {
        let s = AutoplaySpec {
            policy: AutoplayPolicy::Never,
            exempt_hosts: vec!["*://*.youtube.com/*".into()],
            ..AutoplaySpec::default_profile()
        };
        assert!(s.admits_autoplay("www.youtube.com", ctx()));
        assert!(!s.admits_autoplay("example.com", ctx()));
    }

    #[test]
    fn disabled_profile_does_not_block() {
        let s = AutoplaySpec {
            enabled: false,
            policy: AutoplayPolicy::Never,
            ..AutoplaySpec::default_profile()
        };
        assert!(s.admits_autoplay("example.com", ctx()));
    }

    #[test]
    fn applies_to_restricts_enforcement() {
        let s = AutoplaySpec {
            policy: AutoplayPolicy::Never,
            applies_to: vec![TrackKind::AudioElement],
            ..AutoplaySpec::default_profile()
        };
        let vid = PlaybackContext {
            kind: Some(TrackKind::VideoElement),
            ..ctx()
        };
        let aud = PlaybackContext {
            kind: Some(TrackKind::AudioElement),
            ..ctx()
        };
        assert!(s.admits_autoplay("example.com", vid));
        assert!(!s.admits_autoplay("example.com", aud));
    }

    #[test]
    fn pause_in_background_predicate() {
        let s = AutoplaySpec {
            pause_in_background: true,
            ..AutoplaySpec::default_profile()
        };
        let c = PlaybackContext {
            tab_backgrounded: true,
            muted: false,
            ..ctx()
        };
        assert!(s.should_pause_backgrounded(c));
        let muted = PlaybackContext { muted: true, ..c };
        assert!(!s.should_pause_backgrounded(muted));
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = AutoplayRegistry::new();
        reg.insert(AutoplaySpec::default_profile());
        reg.insert(AutoplaySpec {
            name: "yt".into(),
            host: "*://*.youtube.com/*".into(),
            policy: AutoplayPolicy::AllowAll,
            ..AutoplaySpec::default_profile()
        });
        assert_eq!(
            reg.resolve("www.youtube.com").unwrap().policy,
            AutoplayPolicy::AllowAll
        );
        assert_eq!(
            reg.resolve("example.org").unwrap().policy,
            AutoplayPolicy::BlockAudio
        );
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = AutoplayRegistry::new();
        reg.insert(AutoplaySpec {
            enabled: false,
            ..AutoplaySpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[test]
    fn policy_roundtrips_through_serde() {
        for p in [
            AutoplayPolicy::AllowAll,
            AutoplayPolicy::BlockAudio,
            AutoplayPolicy::BlockAll,
            AutoplayPolicy::Never,
        ] {
            let s = AutoplaySpec {
                policy: p,
                ..AutoplaySpec::default_profile()
            };
            let j = serde_json::to_string(&s).unwrap();
            let b: AutoplaySpec = serde_json::from_str(&j).unwrap();
            assert_eq!(b.policy, p);
        }
    }

    #[test]
    fn track_kind_roundtrips_through_serde() {
        for k in [
            TrackKind::AudioElement,
            TrackKind::VideoElement,
            TrackKind::WebRtc,
            TrackKind::MediaSession,
        ] {
            let s = AutoplaySpec {
                applies_to: vec![k],
                ..AutoplaySpec::default_profile()
            };
            let j = serde_json::to_string(&s).unwrap();
            let b: AutoplaySpec = serde_json::from_str(&j).unwrap();
            assert_eq!(b.applies_to, vec![k]);
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_autoplay_form() {
        let src = r#"
            (defautoplay :name "yt"
                         :host "*://*.youtube.com/*"
                         :policy "allow-all"
                         :muted-video-ok #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].policy, AutoplayPolicy::AllowAll);
        assert!(specs[0].matches_host("www.youtube.com"));
    }
}
