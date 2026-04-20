//! `(defsync)` — declarative cross-device replication.
//!
//! Absorbs Chrome/Firefox/Safari/Brave/Edge native "Sync" (bookmarks,
//! history, tabs, passwords) plus Arc Spaces sync, 1Password/Bitwarden
//! vault sync, and Firefox Sync v5. Each profile is one replication
//! channel: a signal kind + a CRDT flavor + a transport topic + a
//! direction.
//!
//! Complements [`crate::crdt_room`] — where CRDT rooms are per-page
//! live edit, (defsync) is cross-device state. Same CRDT theory, very
//! different scope.
//!
//! ```lisp
//! (defsync :name              "my-bookmarks"
//!          :signal            :bookmarks
//!          :direction         :bidirectional
//!          :crdt              :lww-element-set
//!          :transport         :nats
//!          :topic             "sync.bookmarks.{device}"
//!          :isolation-token   "per-profile"
//!          :conflict          :last-writer-wins
//!          :encrypted         #t
//!          :throttle-ms       500
//!          :peer-devices      ("macbook-air" "linux-desktop"))
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Signal kind being synced.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SyncSignal {
    #[default]
    Bookmarks,
    History,
    Tabs,
    OpenWindows,
    Passwords,
    Passkeys,
    Sessions,
    Extensions,
    Settings,
    ReadingList,
    Annotations,
    Downloads,
    /// Custom signal — user names it via `topic`.
    Custom,
}

/// Which direction data flows.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SyncDirection {
    /// This device sends, never receives.
    Push,
    /// This device receives, never sends.
    Pull,
    /// Full two-way sync.
    Bidirectional,
}

impl Default for SyncDirection {
    fn default() -> Self {
        Self::Bidirectional
    }
}

/// Transport the replication runs over.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SyncTransport {
    Nats,
    Websocket,
    DirectP2p,
    /// Local-only (mirror for dev/preview).
    Local,
}

impl Default for SyncTransport {
    fn default() -> Self {
        Self::Nats
    }
}

/// Which CRDT flavor shapes the payload. Mirrors
/// [`crate::crdt_room::CrdtKind`] but reexported here so (defsync)
/// forms don't have to import the collab namespace.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SyncCrdt {
    YCrdt,
    Automerge,
    LwwElementSet,
    OpLog,
}

impl Default for SyncCrdt {
    fn default() -> Self {
        Self::LwwElementSet
    }
}

/// How to resolve conflicts when the same record is edited on two
/// devices.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictPolicy {
    /// Most-recent mtime wins. Default for Chrome Sync.
    LastWriterWins,
    /// Keep both — append to a "conflicted" bucket.
    KeepBoth,
    /// Named device always wins (`preferred_device`).
    PreferDevice,
    /// Let the CRDT natively merge (Automerge / Y-CRDT auto).
    CrdtNative,
}

impl Default for ConflictPolicy {
    fn default() -> Self {
        Self::LastWriterWins
    }
}

/// `(defsync)` profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsync"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncSpec {
    pub name: String,
    #[serde(default = "default_signal")]
    pub signal: SyncSignal,
    #[serde(default)]
    pub direction: SyncDirection,
    #[serde(default)]
    pub crdt: SyncCrdt,
    #[serde(default)]
    pub transport: SyncTransport,
    /// Topic template — `{device}` / `{profile}` / `{signal}` tokens
    /// substituted at bind time.
    #[serde(default = "default_topic")]
    pub topic: String,
    /// Isolation scope — "per-profile" | "per-device" | "global".
    #[serde(default = "default_isolation_token")]
    pub isolation_token: String,
    #[serde(default)]
    pub conflict: ConflictPolicy,
    /// Device name that wins when `conflict = prefer-device`. Empty =
    /// no override.
    #[serde(default)]
    pub preferred_device: Option<String>,
    #[serde(default = "default_encrypted")]
    pub encrypted: bool,
    /// Throttle outbound deltas (ms) so rapid edits coalesce.
    #[serde(default = "default_throttle_ms")]
    pub throttle_ms: u32,
    /// Maximum deltas kept in memory before flushing to the transport.
    #[serde(default = "default_buffer_max")]
    pub buffer_max: u32,
    /// Peer devices this profile syncs with. Empty = every device on
    /// the topic.
    #[serde(default)]
    pub peer_devices: Vec<String>,
    /// Retention window in days. 0 = forever.
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    /// Periodic full-sync interval (seconds, 0 = never full-sync;
    /// only deltas).
    #[serde(default)]
    pub full_sync_interval_seconds: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_signal() -> SyncSignal {
    SyncSignal::Bookmarks
}
fn default_topic() -> String {
    "sync.{signal}.{device}".into()
}
fn default_isolation_token() -> String {
    "per-profile".into()
}
fn default_encrypted() -> bool {
    true
}
fn default_throttle_ms() -> u32 {
    500
}
fn default_buffer_max() -> u32 {
    1024
}
fn default_retention_days() -> u32 {
    90
}
fn default_enabled() -> bool {
    true
}

impl SyncSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default-bookmarks".into(),
            signal: SyncSignal::Bookmarks,
            direction: SyncDirection::Bidirectional,
            crdt: SyncCrdt::LwwElementSet,
            transport: SyncTransport::Nats,
            topic: default_topic(),
            isolation_token: default_isolation_token(),
            conflict: ConflictPolicy::LastWriterWins,
            preferred_device: None,
            encrypted: true,
            throttle_ms: 500,
            buffer_max: 1024,
            peer_devices: vec![],
            retention_days: 90,
            full_sync_interval_seconds: 0,
            enabled: true,
            description: Some(
                "Default sync — bookmarks over NATS with LWW + encryption.".into(),
            ),
        }
    }

    /// Render the topic for a given device + profile + signal name.
    #[must_use]
    pub fn render_topic(&self, device: &str, profile: &str) -> String {
        let signal = signal_kebab(self.signal);
        self.topic
            .replace("{device}", device)
            .replace("{profile}", profile)
            .replace("{signal}", signal)
    }

    /// Does the profile accept traffic from `device`?
    #[must_use]
    pub fn accepts_peer(&self, device: &str) -> bool {
        self.peer_devices.is_empty() || self.peer_devices.iter().any(|d| d == device)
    }

    /// True when changes on this device should be broadcast.
    #[must_use]
    pub fn sends(&self) -> bool {
        matches!(self.direction, SyncDirection::Push | SyncDirection::Bidirectional)
    }

    /// True when this device should apply inbound changes.
    #[must_use]
    pub fn receives(&self) -> bool {
        matches!(self.direction, SyncDirection::Pull | SyncDirection::Bidirectional)
    }

    /// Resolve who wins for a tie-break on identical mtime. Returns
    /// None when the policy can't decide on its own (KeepBoth,
    /// CrdtNative) — caller handles those paths.
    #[must_use]
    pub fn tiebreak<'a>(&'a self, local: &'a str, remote: &'a str) -> Option<&'a str> {
        match self.conflict {
            ConflictPolicy::LastWriterWins => Some(local),
            ConflictPolicy::PreferDevice => {
                self.preferred_device.as_deref().map(|pref| {
                    if pref == local {
                        local
                    } else if pref == remote {
                        remote
                    } else {
                        local
                    }
                })
            }
            ConflictPolicy::KeepBoth | ConflictPolicy::CrdtNative => None,
        }
    }
}

fn signal_kebab(s: SyncSignal) -> &'static str {
    use SyncSignal::*;
    match s {
        Bookmarks => "bookmarks",
        History => "history",
        Tabs => "tabs",
        OpenWindows => "open-windows",
        Passwords => "passwords",
        Passkeys => "passkeys",
        Sessions => "sessions",
        Extensions => "extensions",
        Settings => "settings",
        ReadingList => "reading-list",
        Annotations => "annotations",
        Downloads => "downloads",
        Custom => "custom",
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct SyncRegistry {
    specs: Vec<SyncSpec>,
}

impl SyncRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SyncSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SyncSpec>) {
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
    pub fn specs(&self) -> &[SyncSpec] {
        &self.specs
    }

    /// Lookup by name — most natural index for `(defsync)`, which
    /// isn't host-scoped.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&SyncSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// All profiles syncing a given signal.
    #[must_use]
    pub fn for_signal(&self, signal: SyncSignal) -> Vec<&SyncSpec> {
        self.specs.iter().filter(|s| s.enabled && s.signal == signal).collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SyncSpec>, String> {
    tatara_lisp::compile_typed::<SyncSpec>(src)
        .map_err(|e| format!("failed to compile defsync forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SyncSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_bookmarks_lww_nats() {
        let s = SyncSpec::default_profile();
        assert_eq!(s.signal, SyncSignal::Bookmarks);
        assert_eq!(s.crdt, SyncCrdt::LwwElementSet);
        assert_eq!(s.transport, SyncTransport::Nats);
        assert!(s.encrypted);
    }

    #[test]
    fn direction_gating() {
        let push = SyncSpec {
            direction: SyncDirection::Push,
            ..SyncSpec::default_profile()
        };
        assert!(push.sends());
        assert!(!push.receives());

        let pull = SyncSpec {
            direction: SyncDirection::Pull,
            ..SyncSpec::default_profile()
        };
        assert!(!pull.sends());
        assert!(pull.receives());

        let bi = SyncSpec::default_profile();
        assert!(bi.sends());
        assert!(bi.receives());
    }

    #[test]
    fn render_topic_substitutes_device_profile_and_signal() {
        let s = SyncSpec {
            topic: "{profile}.{signal}.{device}".into(),
            signal: SyncSignal::History,
            ..SyncSpec::default_profile()
        };
        assert_eq!(s.render_topic("macbook", "work"), "work.history.macbook");
    }

    #[test]
    fn render_topic_uses_kebab_signal_name() {
        let s = SyncSpec {
            topic: "{signal}".into(),
            signal: SyncSignal::ReadingList,
            ..SyncSpec::default_profile()
        };
        assert_eq!(s.render_topic("dev", "prof"), "reading-list");
    }

    #[test]
    fn accepts_peer_defaults_to_any() {
        let s = SyncSpec::default_profile();
        assert!(s.accepts_peer("laptop"));
        assert!(s.accepts_peer("desktop"));
    }

    #[test]
    fn accepts_peer_enforces_allow_list() {
        let s = SyncSpec {
            peer_devices: vec!["laptop".into(), "phone".into()],
            ..SyncSpec::default_profile()
        };
        assert!(s.accepts_peer("laptop"));
        assert!(s.accepts_peer("phone"));
        assert!(!s.accepts_peer("tablet"));
    }

    #[test]
    fn tiebreak_lww_picks_local() {
        let s = SyncSpec::default_profile();
        assert_eq!(s.tiebreak("a", "b"), Some("a"));
    }

    #[test]
    fn tiebreak_prefer_device_picks_preferred() {
        let s = SyncSpec {
            conflict: ConflictPolicy::PreferDevice,
            preferred_device: Some("remote".into()),
            ..SyncSpec::default_profile()
        };
        assert_eq!(s.tiebreak("local", "remote"), Some("remote"));

        let s2 = SyncSpec {
            conflict: ConflictPolicy::PreferDevice,
            preferred_device: Some("local".into()),
            ..SyncSpec::default_profile()
        };
        assert_eq!(s2.tiebreak("local", "remote"), Some("local"));
    }

    #[test]
    fn tiebreak_keep_both_and_crdt_native_return_none() {
        let s = SyncSpec {
            conflict: ConflictPolicy::KeepBoth,
            ..SyncSpec::default_profile()
        };
        assert!(s.tiebreak("a", "b").is_none());

        let s2 = SyncSpec {
            conflict: ConflictPolicy::CrdtNative,
            ..SyncSpec::default_profile()
        };
        assert!(s2.tiebreak("a", "b").is_none());
    }

    #[test]
    fn signal_roundtrips_through_serde() {
        for sig in [
            SyncSignal::Bookmarks,
            SyncSignal::History,
            SyncSignal::Tabs,
            SyncSignal::OpenWindows,
            SyncSignal::Passwords,
            SyncSignal::Passkeys,
            SyncSignal::Sessions,
            SyncSignal::Extensions,
            SyncSignal::Settings,
            SyncSignal::ReadingList,
            SyncSignal::Annotations,
            SyncSignal::Downloads,
            SyncSignal::Custom,
        ] {
            let s = SyncSpec {
                signal: sig,
                ..SyncSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: SyncSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.signal, sig);
        }
    }

    #[test]
    fn conflict_policy_roundtrips_through_serde() {
        for p in [
            ConflictPolicy::LastWriterWins,
            ConflictPolicy::KeepBoth,
            ConflictPolicy::PreferDevice,
            ConflictPolicy::CrdtNative,
        ] {
            let s = SyncSpec {
                conflict: p,
                ..SyncSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: SyncSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.conflict, p);
        }
    }

    #[test]
    fn registry_for_signal_filters_by_kind() {
        let mut reg = SyncRegistry::new();
        reg.insert(SyncSpec::default_profile()); // bookmarks
        reg.insert(SyncSpec {
            name: "my-history".into(),
            signal: SyncSignal::History,
            ..SyncSpec::default_profile()
        });
        reg.insert(SyncSpec {
            name: "paused-tabs".into(),
            signal: SyncSignal::Tabs,
            enabled: false,
            ..SyncSpec::default_profile()
        });
        let hist = reg.for_signal(SyncSignal::History);
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].name, "my-history");
        // Disabled profile doesn't show up.
        assert!(reg.for_signal(SyncSignal::Tabs).is_empty());
    }

    #[test]
    fn registry_get_by_name_dedupes_on_insert() {
        let mut reg = SyncRegistry::new();
        reg.insert(SyncSpec::default_profile());
        let replaced = SyncSpec {
            throttle_ms: 99,
            ..SyncSpec::default_profile()
        };
        reg.insert(replaced);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("default-bookmarks").unwrap().throttle_ms, 99);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_sync_form() {
        let src = r#"
            (defsync :name "my-bookmarks"
                     :signal "bookmarks"
                     :direction "bidirectional"
                     :crdt "lww-element-set"
                     :transport "nats"
                     :topic "sync.bookmarks.{device}"
                     :isolation-token "per-profile"
                     :conflict "last-writer-wins"
                     :encrypted #t
                     :throttle-ms 750
                     :buffer-max 2048
                     :retention-days 120
                     :peer-devices ("macbook" "linux-desktop"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "my-bookmarks");
        assert_eq!(s.signal, SyncSignal::Bookmarks);
        assert_eq!(s.throttle_ms, 750);
        assert_eq!(s.retention_days, 120);
        assert_eq!(s.peer_devices.len(), 2);
    }
}
