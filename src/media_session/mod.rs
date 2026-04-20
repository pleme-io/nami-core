//! `(defmedia-session)` — lock-screen / OS-level media controls.
//!
//! Absorbs the W3C Media Session API (Chrome/Edge/Firefox) + iOS
//! lock-screen controls + macOS Now Playing + Android media
//! notification into one substrate DSL. Each profile scopes to a
//! host, enables a subset of transport actions, and declares the
//! selector pattern for metadata extraction (title / artist /
//! album / artwork).
//!
//! ```lisp
//! (defmedia-session :name      "default"
//!                   :host      "*"
//!                   :actions   (play pause seek-forward seek-backward
//!                               previous-track next-track)
//!                   :metadata  (:title    "[data-media-title]"
//!                               :artist   "[data-media-artist]"
//!                               :album    "[data-media-album]"
//!                               :artwork  "[data-media-artwork]"))
//!
//! (defmedia-session :name "youtube"
//!                   :host "*://*.youtube.com/*"
//!                   :actions (play pause next-track previous-track
//!                             seek-backward seek-forward)
//!                   :metadata (:title   ".html5-video-title"
//!                              :artist  "#channel-name"
//!                              :artwork "video"))
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Transport action the media session exposes to the OS.
/// Mirrors the W3C `MediaSessionAction` enum.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum MediaAction {
    Play,
    Pause,
    Stop,
    SeekForward,
    SeekBackward,
    /// Absolute `seekTo(time)`.
    SeekTo,
    PreviousTrack,
    NextTrack,
    /// Skip ad / unwanted content.
    SkipAd,
    /// Enter picture-in-picture from the lock screen.
    EnterPictureInPicture,
    /// Toggle microphone (video-call sites).
    ToggleMicrophone,
    /// Toggle camera (video-call sites).
    ToggleCamera,
    /// Hang up (video-call sites).
    HangUp,
}

/// CSS selectors the engine walks to extract metadata the OS shows
/// in the lock-screen widget.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MetadataSelectors {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    /// Selector that returns either an `<img>` / `<source>` with a URL
    /// or a `<video>` whose `poster` attribute we lift.
    #[serde(default)]
    pub artwork: Option<String>,
}

impl MetadataSelectors {
    #[must_use]
    pub fn html5_defaults() -> Self {
        Self {
            title: Some("title".into()),
            artist: Some("meta[name=author]".into()),
            album: None,
            artwork: Some("meta[property='og:image']".into()),
        }
    }
}

/// Media-session profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defmedia-session"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaSessionSpec {
    pub name: String,
    /// Host glob. `"*"` = everywhere.
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    /// Actions exposed to the OS. Empty = every action.
    #[serde(default = "default_actions")]
    pub actions: Vec<MediaAction>,
    /// Selectors used to pull metadata for the OS widget.
    #[serde(default)]
    pub metadata: MetadataSelectors,
    /// Default seek increment in seconds (used when the OS fires
    /// SeekForward/SeekBackward without a specific value).
    #[serde(default = "default_seek_seconds")]
    pub seek_seconds: u32,
    /// Activate the session automatically on first play, vs wait
    /// for the site's own `navigator.mediaSession` activation.
    #[serde(default = "default_auto_activate")]
    pub auto_activate: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_actions() -> Vec<MediaAction> {
    vec![
        MediaAction::Play,
        MediaAction::Pause,
        MediaAction::Stop,
        MediaAction::SeekForward,
        MediaAction::SeekBackward,
        MediaAction::PreviousTrack,
        MediaAction::NextTrack,
    ]
}
fn default_seek_seconds() -> u32 {
    10
}
fn default_auto_activate() -> bool {
    true
}

impl MediaSessionSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            actions: default_actions(),
            metadata: MetadataSelectors::html5_defaults(),
            seek_seconds: 10,
            auto_activate: true,
            description: Some(
                "Default media session — play/pause/seek/prev/next, HTML5 metadata."
                    .into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    #[must_use]
    pub fn exposes(&self, action: MediaAction) -> bool {
        self.actions.is_empty() || self.actions.contains(&action)
    }
}

/// Live metadata extracted at playback time — what the OS renders.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaMetadata {
    pub title: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    /// Artwork URL(s) — first one is the preferred source.
    #[serde(default)]
    pub artwork_urls: Vec<String>,
}

/// Registry — host-specific wins over wildcard.
#[derive(Debug, Clone, Default)]
pub struct MediaSessionRegistry {
    specs: Vec<MediaSessionSpec>,
}

impl MediaSessionRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: MediaSessionSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = MediaSessionSpec>) {
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
    pub fn specs(&self) -> &[MediaSessionSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&MediaSessionSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<MediaSessionSpec>, String> {
    tatara_lisp::compile_typed::<MediaSessionSpec>(src)
        .map_err(|e| format!("failed to compile defmedia-session forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<MediaSessionSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_exposes_transport_actions() {
        let s = MediaSessionSpec::default_profile();
        assert!(s.exposes(MediaAction::Play));
        assert!(s.exposes(MediaAction::Pause));
        assert!(s.exposes(MediaAction::NextTrack));
        assert!(!s.exposes(MediaAction::HangUp));
    }

    #[test]
    fn empty_actions_list_exposes_everything() {
        let s = MediaSessionSpec {
            actions: vec![],
            ..MediaSessionSpec::default_profile()
        };
        for action in [
            MediaAction::Play,
            MediaAction::HangUp,
            MediaAction::ToggleCamera,
            MediaAction::EnterPictureInPicture,
        ] {
            assert!(s.exposes(action));
        }
    }

    #[test]
    fn matches_host_glob() {
        let s = MediaSessionSpec {
            host: "*://*.youtube.com/*".into(),
            ..MediaSessionSpec::default_profile()
        };
        assert!(s.matches_host("www.youtube.com"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn wildcard_matches_everything() {
        assert!(MediaSessionSpec::default_profile().matches_host("anywhere.com"));
    }

    #[test]
    fn registry_dedupes_and_resolves_specific() {
        let mut reg = MediaSessionRegistry::new();
        reg.insert(MediaSessionSpec::default_profile());
        reg.insert(MediaSessionSpec {
            name: "yt".into(),
            host: "*://*.youtube.com/*".into(),
            ..MediaSessionSpec::default_profile()
        });
        let yt = reg.resolve("www.youtube.com").unwrap();
        assert_eq!(yt.name, "yt");
        let other = reg.resolve("example.com").unwrap();
        assert_eq!(other.name, "default");
    }

    #[test]
    fn metadata_defaults_cover_html5_meta() {
        let m = MetadataSelectors::html5_defaults();
        assert_eq!(m.title.as_deref(), Some("title"));
        assert!(m.artwork.as_deref().unwrap().contains("og:image"));
    }

    #[test]
    fn seek_seconds_has_reasonable_default() {
        assert_eq!(MediaSessionSpec::default_profile().seek_seconds, 10);
    }

    #[test]
    fn actions_roundtrip_through_serde() {
        for a in [
            MediaAction::Play,
            MediaAction::Pause,
            MediaAction::ToggleMicrophone,
            MediaAction::HangUp,
            MediaAction::SkipAd,
        ] {
            let s = MediaSessionSpec {
                actions: vec![a],
                ..MediaSessionSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: MediaSessionSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.actions, vec![a]);
        }
    }

    #[test]
    fn metadata_roundtrip() {
        let m = MediaMetadata {
            title: "Title".into(),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            artwork_urls: vec!["https://example.com/art.png".into()],
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: MediaMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_media_session_form() {
        let src = r##"
            (defmedia-session :name "yt"
                              :host "*://*.youtube.com/*"
                              :actions ("play" "pause" "seek-forward"
                                        "seek-backward" "previous-track" "next-track")
                              :metadata (:title ".html5-video-title"
                                         :artist "#channel-name"
                                         :artwork "video")
                              :seek-seconds 15
                              :auto-activate #t)
        "##;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "yt");
        assert_eq!(s.seek_seconds, 15);
        assert!(s.auto_activate);
        assert!(s.exposes(MediaAction::SeekForward));
    }
}
