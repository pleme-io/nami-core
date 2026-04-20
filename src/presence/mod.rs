//! `(defpresence)` — who-is-here tracking.
//!
//! Absorbs Google Docs presence avatars, Figma live viewers, Notion
//! member-viewing-this-page indicators, Slack huddle rosters. Each
//! profile declares how the host publishes presence to a backend —
//! NATS topic, websocket, direct p2p — plus what per-session fields
//! to broadcast (cursor, selection, viewport, typing indicator) and
//! how aggressively to prune stale entries.
//!
//! ```lisp
//! (defpresence :name            "docs-collab"
//!              :host            "*://*.docs.example.com/*"
//!              :transport       :nats
//!              :topic-template  "presence.{origin}.{path}"
//!              :broadcast       (cursor selection viewport typing)
//!              :expires-seconds 30
//!              :display-name    "anonymous")
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Transport for presence broadcasts.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PresenceTransport {
    /// pleme-io NATS broker (denshin/tsunagu adjacent).
    Nats,
    /// WebSocket gateway (denshin).
    Websocket,
    /// Direct peer-to-peer via WebRTC data channel.
    DirectP2p,
    /// No transport — presence is mirrored locally only
    /// (useful for per-space visibility on a single device).
    Local,
}

impl Default for PresenceTransport {
    fn default() -> Self {
        Self::Nats
    }
}

/// Field kind that a session broadcasts.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum BroadcastField {
    /// Cursor position (x, y in CSS px).
    Cursor,
    /// Text-selection range (start + end offsets inside the DOM).
    Selection,
    /// Viewport rectangle — lets collaborators jump to "where Jane is".
    Viewport,
    /// Typing-in-text-field indicator.
    Typing,
    /// Raw attention — page-focused vs blurred.
    Attention,
    /// Audio/video call state (for huddle-style UX).
    VoiceStatus,
}

/// Presence profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defpresence"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PresenceSpec {
    pub name: String,
    /// Host glob the profile fires on. `"*"` = everywhere.
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub transport: PresenceTransport,
    /// Topic/channel template — `{origin}`, `{path}`, `{space}` tokens
    /// get substituted at join time.
    #[serde(default = "default_topic_template")]
    pub topic_template: String,
    /// Which per-session fields to broadcast.
    #[serde(default = "default_broadcast")]
    pub broadcast: Vec<BroadcastField>,
    /// Seconds before an idle session is pruned from the roster.
    #[serde(default = "default_expires_seconds")]
    pub expires_seconds: u64,
    /// Default display name if the user hasn't picked one.
    #[serde(default = "default_display_name")]
    pub display_name: String,
    /// Avatar URL template — `{session}` / `{user}` tokens allowed.
    #[serde(default)]
    pub avatar_template: Option<String>,
    /// Minimum interval between outbound broadcasts (ms). Prevents
    /// cursor floods.
    #[serde(default = "default_throttle_ms")]
    pub throttle_ms: u32,
    /// Upper bound on roster size to render; extra members collapse
    /// into "+N more".
    #[serde(default = "default_max_visible")]
    pub max_visible: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_topic_template() -> String {
    "presence.{origin}.{path}".into()
}
fn default_broadcast() -> Vec<BroadcastField> {
    vec![
        BroadcastField::Cursor,
        BroadcastField::Selection,
        BroadcastField::Attention,
    ]
}
fn default_expires_seconds() -> u64 {
    30
}
fn default_display_name() -> String {
    "anonymous".into()
}
fn default_throttle_ms() -> u32 {
    100
}
fn default_max_visible() -> u32 {
    8
}
fn default_enabled() -> bool {
    true
}

impl PresenceSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            transport: PresenceTransport::Nats,
            topic_template: default_topic_template(),
            broadcast: default_broadcast(),
            expires_seconds: 30,
            display_name: "anonymous".into(),
            avatar_template: None,
            throttle_ms: 100,
            max_visible: 8,
            enabled: true,
            description: Some("Default presence — cursor/selection/attention over NATS.".into()),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    #[must_use]
    pub fn broadcasts(&self, field: BroadcastField) -> bool {
        self.broadcast.contains(&field)
    }

    /// Render the topic for a given origin + path + optional space.
    #[must_use]
    pub fn render_topic(&self, origin: &str, path: &str, space: Option<&str>) -> String {
        self.topic_template
            .replace("{origin}", origin)
            .replace("{path}", path)
            .replace("{space}", space.unwrap_or(""))
    }
}

/// One live roster entry. Mirrors what the fetch pipeline writes
/// back per received presence beacon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PresenceEntry {
    /// Session id — BLAKE3 hash of the user's ephemeral key; 26-char
    /// base32 so it fits the tameshi attestation shape.
    pub session_id: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
    /// Unix-seconds the latest beacon arrived.
    pub last_seen: i64,
    /// Current cursor (x, y) in CSS px when `Cursor` is broadcast.
    #[serde(default)]
    pub cursor: Option<(f32, f32)>,
    /// Selection range — (start, end) DOM offsets when `Selection`
    /// is broadcast.
    #[serde(default)]
    pub selection: Option<(u32, u32)>,
    /// Viewport rectangle — (x, y, w, h) in CSS px.
    #[serde(default)]
    pub viewport: Option<(f32, f32, f32, f32)>,
    /// Typing-in-a-text-field indicator.
    #[serde(default)]
    pub typing: bool,
    /// Page-focused (true) vs blurred / hidden (false).
    #[serde(default = "default_focused")]
    pub focused: bool,
    /// Stable color the UI can use consistently per session.
    /// Empty = UI picks from a palette.
    #[serde(default)]
    pub color: Option<String>,
}

fn default_focused() -> bool {
    true
}

impl PresenceEntry {
    /// True when the entry hasn't beaconed in `expires_seconds` seconds.
    #[must_use]
    pub fn is_stale(&self, now: i64, expires_seconds: u64) -> bool {
        if expires_seconds == 0 {
            return false;
        }
        now.saturating_sub(self.last_seen) >= expires_seconds as i64
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct PresenceRegistry {
    specs: Vec<PresenceSpec>,
}

impl PresenceRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: PresenceSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = PresenceSpec>) {
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
    pub fn specs(&self) -> &[PresenceSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&PresenceSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<PresenceSpec>, String> {
    tatara_lisp::compile_typed::<PresenceSpec>(src)
        .map_err(|e| format!("failed to compile defpresence forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<PresenceSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_covers_cursor_selection_attention() {
        let s = PresenceSpec::default_profile();
        assert!(s.broadcasts(BroadcastField::Cursor));
        assert!(s.broadcasts(BroadcastField::Selection));
        assert!(s.broadcasts(BroadcastField::Attention));
        assert!(!s.broadcasts(BroadcastField::VoiceStatus));
    }

    #[test]
    fn matches_host_glob() {
        let s = PresenceSpec {
            host: "*://*.docs.example.com/*".into(),
            ..PresenceSpec::default_profile()
        };
        assert!(s.matches_host("edit.docs.example.com"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn topic_template_substitutes_tokens() {
        let s = PresenceSpec {
            topic_template: "room/{origin}/{path}/{space}".into(),
            ..PresenceSpec::default_profile()
        };
        assert_eq!(
            s.render_topic("docs.example.com", "/doc/42", Some("work")),
            "room/docs.example.com//doc/42/work"
        );
    }

    #[test]
    fn topic_template_handles_missing_space() {
        let s = PresenceSpec::default_profile();
        assert_eq!(
            s.render_topic("ex.com", "/x", None),
            "presence.ex.com./x"
        );
    }

    #[test]
    fn presence_entry_stale_check() {
        let e = PresenceEntry {
            session_id: "sid".into(),
            display_name: "Jane".into(),
            avatar_url: None,
            last_seen: 1_000,
            cursor: None,
            selection: None,
            viewport: None,
            typing: false,
            focused: true,
            color: None,
        };
        assert!(!e.is_stale(1_005, 30));
        assert!(e.is_stale(1_100, 30));
        // expires = 0 → never stale.
        assert!(!e.is_stale(i64::MAX, 0));
    }

    #[test]
    fn presence_entry_roundtrips_through_json() {
        let e = PresenceEntry {
            session_id: "abc".into(),
            display_name: "Bob".into(),
            avatar_url: Some("https://example.com/a.png".into()),
            last_seen: 42,
            cursor: Some((10.0, 20.0)),
            selection: Some((5, 12)),
            viewport: Some((0.0, 0.0, 800.0, 600.0)),
            typing: true,
            focused: false,
            color: Some("#ff0000".into()),
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: PresenceEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn transport_roundtrips_through_serde() {
        for t in [
            PresenceTransport::Nats,
            PresenceTransport::Websocket,
            PresenceTransport::DirectP2p,
            PresenceTransport::Local,
        ] {
            let s = PresenceSpec {
                transport: t,
                ..PresenceSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: PresenceSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.transport, t);
        }
    }

    #[test]
    fn registry_dedupes_and_resolves_specific() {
        let mut reg = PresenceRegistry::new();
        reg.insert(PresenceSpec::default_profile());
        reg.insert(PresenceSpec {
            name: "docs".into(),
            host: "*://*.docs.example.com/*".into(),
            ..PresenceSpec::default_profile()
        });
        let docs = reg.resolve("edit.docs.example.com").unwrap();
        assert_eq!(docs.name, "docs");
        let other = reg.resolve("example.org").unwrap();
        assert_eq!(other.name, "default");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = PresenceRegistry::new();
        reg.insert(PresenceSpec {
            enabled: false,
            ..PresenceSpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_presence_form() {
        let src = r#"
            (defpresence :name "docs-collab"
                         :host "*://*.docs.example.com/*"
                         :transport "nats"
                         :topic-template "presence.{origin}.{path}"
                         :broadcast ("cursor" "selection" "viewport")
                         :expires-seconds 45
                         :throttle-ms 50
                         :max-visible 12)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "docs-collab");
        assert_eq!(s.transport, PresenceTransport::Nats);
        assert_eq!(s.expires_seconds, 45);
        assert!(s.broadcasts(BroadcastField::Viewport));
    }
}
