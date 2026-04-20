//! `(defcrdt-room)` — CRDT sync-room profile.
//!
//! Absorbs Figma multiplayer, Linear live-edit, Notion realtime, Google
//! Docs collaboration, tldraw whiteboard sync, Excalidraw rooms. Each
//! profile declares the transport (NATS / websocket / direct p2p), the
//! CRDT flavor (Y.js, Automerge, LWW-element-set), the topic template
//! tokens, and the isolation token for per-tab room scoping.
//!
//! ```lisp
//! (defcrdt-room :name          "notion-page"
//!               :host          "*://*.notion.so/*"
//!               :transport     :nats
//!               :crdt          :automerge
//!               :topic-template "crdt.{origin}.{path}"
//!               :awareness     #t
//!               :persistence   :indexeddb
//!               :isolation-token "per-tab"
//!               :max-peers     32)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Transport for CRDT sync traffic.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RoomTransport {
    /// pleme-io NATS broker.
    Nats,
    /// WebSocket gateway (denshin).
    Websocket,
    /// WebRTC data channel between peers.
    DirectP2p,
    /// Local only — one-tab rooms for preview / undo.
    Local,
}

impl Default for RoomTransport {
    fn default() -> Self {
        Self::Nats
    }
}

/// CRDT flavor.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CrdtKind {
    /// Y.js / Y-CRDT (text + arrays + maps, RGA-split).
    YCrdt,
    /// Automerge.
    Automerge,
    /// LWW element set (smallest useful CRDT).
    LwwElementSet,
    /// Pure operation log (no conflict resolution — app handles).
    OpLog,
}

impl Default for CrdtKind {
    fn default() -> Self {
        Self::YCrdt
    }
}

/// Persistence backend.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Persistence {
    /// No persistence — room evaporates when the tab closes.
    None,
    /// Browser IndexedDB.
    IndexedDb,
    /// LocalStorage (small rooms only).
    LocalStorage,
    /// Daemon-side sqlite (survives browser restart, cross-device).
    Daemon,
}

impl Default for Persistence {
    fn default() -> Self {
        Self::IndexedDb
    }
}

/// CRDT room profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defcrdt-room"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CrdtRoomSpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub transport: RoomTransport,
    #[serde(default)]
    pub crdt: CrdtKind,
    /// Topic template — `{origin}`, `{path}`, `{space}`, `{room}` tokens
    /// substituted at join time.
    #[serde(default = "default_topic_template")]
    pub topic_template: String,
    /// Whether to publish awareness (cursor, selection) alongside doc
    /// updates — Y.js-style.
    #[serde(default = "default_awareness")]
    pub awareness: bool,
    #[serde(default)]
    pub persistence: Persistence,
    /// Isolation token — `"per-tab"`, `"per-profile"`, `"global"`.
    #[serde(default = "default_isolation_token")]
    pub isolation_token: String,
    /// Upper bound on peer count per room (0 = unlimited).
    #[serde(default = "default_max_peers")]
    pub max_peers: u32,
    /// Snapshot cadence in seconds (0 = never snapshot, let log grow).
    #[serde(default = "default_snapshot_interval")]
    pub snapshot_interval_seconds: u32,
    /// Minimum sync-frame interval in ms (throttle outbound updates).
    #[serde(default = "default_throttle_ms")]
    pub throttle_ms: u32,
    /// End-to-end encryption toggle (key derived per-room via isolation token).
    #[serde(default = "default_encrypted")]
    pub encrypted: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_topic_template() -> String {
    "crdt.{origin}.{path}".into()
}
fn default_awareness() -> bool {
    true
}
fn default_isolation_token() -> String {
    "per-profile".into()
}
fn default_max_peers() -> u32 {
    32
}
fn default_snapshot_interval() -> u32 {
    60
}
fn default_throttle_ms() -> u32 {
    32
}
fn default_encrypted() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

impl CrdtRoomSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            transport: RoomTransport::Nats,
            crdt: CrdtKind::YCrdt,
            topic_template: default_topic_template(),
            awareness: true,
            persistence: Persistence::IndexedDb,
            isolation_token: "per-profile".into(),
            max_peers: 32,
            snapshot_interval_seconds: 60,
            throttle_ms: 32,
            encrypted: true,
            enabled: true,
            description: Some(
                "Default CRDT room — Y-CRDT over NATS with awareness and IndexedDB.".into(),
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

    #[must_use]
    pub fn render_topic(
        &self,
        origin: &str,
        path: &str,
        space: Option<&str>,
        room: Option<&str>,
    ) -> String {
        self.topic_template
            .replace("{origin}", origin)
            .replace("{path}", path)
            .replace("{space}", space.unwrap_or(""))
            .replace("{room}", room.unwrap_or(""))
    }

    #[must_use]
    pub fn can_accept_peer(&self, current_peers: u32) -> bool {
        self.max_peers == 0 || current_peers < self.max_peers
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct CrdtRoomRegistry {
    specs: Vec<CrdtRoomSpec>,
}

impl CrdtRoomRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: CrdtRoomSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = CrdtRoomSpec>) {
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
    pub fn specs(&self) -> &[CrdtRoomSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&CrdtRoomSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<CrdtRoomSpec>, String> {
    tatara_lisp::compile_typed::<CrdtRoomSpec>(src)
        .map_err(|e| format!("failed to compile defcrdt-room forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<CrdtRoomSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_y_crdt_over_nats_with_awareness() {
        let s = CrdtRoomSpec::default_profile();
        assert_eq!(s.crdt, CrdtKind::YCrdt);
        assert_eq!(s.transport, RoomTransport::Nats);
        assert!(s.awareness);
        assert_eq!(s.persistence, Persistence::IndexedDb);
        assert!(s.encrypted);
    }

    #[test]
    fn topic_template_substitutes_all_tokens() {
        let s = CrdtRoomSpec {
            topic_template: "r/{origin}/{path}/{space}/{room}".into(),
            ..CrdtRoomSpec::default_profile()
        };
        assert_eq!(
            s.render_topic("fig.example.com", "/file/1", Some("team"), Some("main")),
            "r/fig.example.com//file/1/team/main"
        );
    }

    #[test]
    fn topic_template_handles_missing_optional_tokens() {
        let s = CrdtRoomSpec::default_profile();
        assert_eq!(
            s.render_topic("ex.com", "/x", None, None),
            "crdt.ex.com./x"
        );
    }

    #[test]
    fn can_accept_peer_respects_cap() {
        let capped = CrdtRoomSpec {
            max_peers: 4,
            ..CrdtRoomSpec::default_profile()
        };
        assert!(capped.can_accept_peer(0));
        assert!(capped.can_accept_peer(3));
        assert!(!capped.can_accept_peer(4));

        let unlimited = CrdtRoomSpec {
            max_peers: 0,
            ..CrdtRoomSpec::default_profile()
        };
        assert!(unlimited.can_accept_peer(9_999));
    }

    #[test]
    fn matches_host_glob() {
        let s = CrdtRoomSpec {
            host: "*://*.notion.so/*".into(),
            ..CrdtRoomSpec::default_profile()
        };
        assert!(s.matches_host("www.notion.so"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn transport_roundtrips_through_serde() {
        for t in [
            RoomTransport::Nats,
            RoomTransport::Websocket,
            RoomTransport::DirectP2p,
            RoomTransport::Local,
        ] {
            let s = CrdtRoomSpec {
                transport: t,
                ..CrdtRoomSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: CrdtRoomSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.transport, t);
        }
    }

    #[test]
    fn crdt_kind_roundtrips_through_serde() {
        for k in [
            CrdtKind::YCrdt,
            CrdtKind::Automerge,
            CrdtKind::LwwElementSet,
            CrdtKind::OpLog,
        ] {
            let s = CrdtRoomSpec {
                crdt: k,
                ..CrdtRoomSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: CrdtRoomSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.crdt, k);
        }
    }

    #[test]
    fn persistence_roundtrips_through_serde() {
        for p in [
            Persistence::None,
            Persistence::IndexedDb,
            Persistence::LocalStorage,
            Persistence::Daemon,
        ] {
            let s = CrdtRoomSpec {
                persistence: p,
                ..CrdtRoomSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: CrdtRoomSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.persistence, p);
        }
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = CrdtRoomRegistry::new();
        reg.insert(CrdtRoomSpec::default_profile());
        reg.insert(CrdtRoomSpec {
            name: "notion".into(),
            host: "*://*.notion.so/*".into(),
            crdt: CrdtKind::Automerge,
            ..CrdtRoomSpec::default_profile()
        });
        let notion = reg.resolve("www.notion.so").unwrap();
        assert_eq!(notion.crdt, CrdtKind::Automerge);
        let other = reg.resolve("example.org").unwrap();
        assert_eq!(other.name, "default");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = CrdtRoomRegistry::new();
        reg.insert(CrdtRoomSpec {
            enabled: false,
            ..CrdtRoomSpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_crdt_room_form() {
        let src = r#"
            (defcrdt-room :name "notion-page"
                          :host "*://*.notion.so/*"
                          :transport "nats"
                          :crdt "automerge"
                          :topic-template "crdt.{origin}.{path}"
                          :awareness #t
                          :persistence "indexed-db"
                          :isolation-token "per-tab"
                          :max-peers 64
                          :snapshot-interval-seconds 30
                          :throttle-ms 16
                          :encrypted #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "notion-page");
        assert_eq!(s.crdt, CrdtKind::Automerge);
        assert_eq!(s.max_peers, 64);
        assert_eq!(s.isolation_token, "per-tab");
    }
}
