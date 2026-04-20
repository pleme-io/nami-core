//! `(defaudit-trail)` — tamper-evident audit log for substrate actions.
//!
//! **Novel surface** — no mainstream browser records a
//! cryptographically-chained audit of policy/config changes. This
//! module declares one: every DSL mutation (rc file reload,
//! extension install, permission grant, identity switch, sync
//! change, …) becomes an `AuditEntry` with BLAKE3 content hash and
//! a `prev_hash` pointer to the previous entry — a Merkle chain
//! that any later moment can verify was not rewritten.
//!
//! Matches the tameshi / sekiban / kensa / inshou attestation
//! convention (BLAKE3 128-bit, base32, 26 chars). A single rogue
//! tamper turns every subsequent hash into a forgery — detectable.
//!
//! ```lisp
//! (defaudit-trail :name            "default"
//!                 :sinks           (memory disk syslog)
//!                 :events          (rc-reload extension-install
//!                                   permission-grant permission-deny
//!                                   identity-switch sync-change
//!                                   storage-clear)
//!                 :actor-fields    (user-id device-id session-id)
//!                 :max-entries     10000
//!                 :redact-secrets  #t
//!                 :disk-path       "~/.cache/nami/audit.jsonl"
//!                 :syslog-facility "user")
//! ```

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Kinds of events recorded.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    /// rc file (extensions.lisp / transforms.lisp) reloaded.
    RcReload,
    ExtensionInstall,
    ExtensionUninstall,
    ExtensionToggle,
    PermissionGrant,
    PermissionDeny,
    IdentitySwitch,
    /// (defsync) emitted or consumed a delta.
    SyncChange,
    /// Storage or cookies cleared.
    StorageClear,
    /// (defbridge) or (defrouting) rule changed.
    RoutingChange,
    /// Omnibox navigated to a new host.
    Navigation,
    /// TOTP code read (the secret, not the code — see `redact_secrets`).
    TotpRead,
    /// DSL parse failed.
    DslFailure,
    /// A blocked permission was requested.
    PermissionBlocked,
    /// Extension requested a capability.
    CapabilityRequest,
    /// User cleared all audit entries.
    AuditClear,
    Custom,
}

/// Where audit entries are written. Multiple sinks = fan-out.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Sink {
    /// In-process ring buffer only.
    Memory,
    /// Append-only JSONL file on disk.
    Disk,
    /// System syslog (OSLog on macOS, journald on Linux).
    Syslog,
    /// Push to a (defsync) "audit" signal — distributes to other
    /// devices.
    Sync,
    /// Ship to a remote endpoint (if (defhttp) / external API).
    RemoteHttp,
    /// NATS topic (pleme-io Sui-adjacent).
    Nats,
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defaudit-trail"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuditTrailSpec {
    pub name: String,
    #[serde(default = "default_sinks")]
    pub sinks: Vec<Sink>,
    /// Event kinds the trail captures. Empty = capture everything.
    #[serde(default)]
    pub events: Vec<EventKind>,
    /// Metadata fields to capture per event (free-form; the host
    /// decides what populates each label — user-id / device-id /
    /// session-id / ip / …).
    #[serde(default = "default_actor_fields")]
    pub actor_fields: Vec<String>,
    /// Ring buffer size for the Memory sink.
    #[serde(default = "default_max_entries")]
    pub max_entries: u32,
    /// Redact values that look like secrets (passwords, TOTP
    /// secrets, API keys).
    #[serde(default = "default_redact_secrets")]
    pub redact_secrets: bool,
    /// Disk path template for the `Disk` sink (`{profile}` token
    /// substituted at open).
    #[serde(default)]
    pub disk_path: Option<String>,
    /// Syslog facility name when `Syslog` sink is active.
    #[serde(default)]
    pub syslog_facility: Option<String>,
    /// Rotate the on-disk log after N bytes (0 = never).
    #[serde(default = "default_rotate_bytes")]
    pub rotate_bytes: u64,
    /// (defsync) signal name when `Sync` sink is active. Empty = "audit".
    #[serde(default)]
    pub sync_signal: Option<String>,
    /// Include the DSL source text in `details` (verbose — off by
    /// default for privacy).
    #[serde(default)]
    pub record_source_text: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_sinks() -> Vec<Sink> {
    vec![Sink::Memory]
}
fn default_actor_fields() -> Vec<String> {
    vec![
        "device_id".into(),
        "session_id".into(),
        "identity_name".into(),
    ]
}
fn default_max_entries() -> u32 {
    10_000
}
fn default_redact_secrets() -> bool {
    true
}
fn default_rotate_bytes() -> u64 {
    16 * 1024 * 1024
}
fn default_enabled() -> bool {
    false // privacy-first: opt-in.
}

impl AuditTrailSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            sinks: default_sinks(),
            events: vec![],
            actor_fields: default_actor_fields(),
            max_entries: 10_000,
            redact_secrets: true,
            disk_path: None,
            syslog_facility: None,
            rotate_bytes: default_rotate_bytes(),
            sync_signal: None,
            record_source_text: false,
            enabled: false,
            description: Some(
                "Default audit — disabled (privacy-first). Enable explicitly in rc file.".into(),
            ),
        }
    }

    #[must_use]
    pub fn captures(&self, kind: EventKind) -> bool {
        self.enabled && (self.events.is_empty() || self.events.contains(&kind))
    }

    #[must_use]
    pub fn sinks_to(&self, s: Sink) -> bool {
        self.sinks.contains(&s)
    }
}

/// One audit entry. Content-hashed; chained to the previous entry
/// via `prev_hash` to form a tamper-evident Merkle list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    /// Sequence number (0-based). Gaps are a tamper signal.
    pub seq: u64,
    /// Unix-seconds the event was recorded.
    pub ts: i64,
    pub kind: EventKind,
    /// Free-form actor metadata — keys are whatever the spec declared
    /// in `actor_fields`.
    pub actor: Vec<(String, String)>,
    /// Payload detail — JSON-serialized string; redacted when
    /// `redact_secrets` is on.
    pub details: String,
    /// BLAKE3 26-char base32 hash of the PREVIOUS entry. Empty for
    /// the genesis entry.
    pub prev_hash: String,
    /// BLAKE3 26-char base32 hash of THIS entry's canonical form.
    pub content_hash: String,
}

impl AuditEntry {
    /// Genesis hash = BLAKE3("audit-genesis") as the chain root.
    pub const GENESIS_PREV: &'static str = "";

    /// Compute the canonical content hash for `(seq, ts, kind, actor,
    /// details, prev_hash)`. Deterministic across platforms.
    #[must_use]
    pub fn compute_hash(
        seq: u64,
        ts: i64,
        kind: EventKind,
        actor: &[(String, String)],
        details: &str,
        prev_hash: &str,
    ) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&seq.to_be_bytes());
        hasher.update(&ts.to_be_bytes());
        let k = serde_json::to_string(&kind).unwrap_or_default();
        hasher.update(k.as_bytes());
        for (k, v) in actor {
            hasher.update(b"|");
            hasher.update(k.as_bytes());
            hasher.update(b"=");
            hasher.update(v.as_bytes());
        }
        hasher.update(b"||");
        hasher.update(details.as_bytes());
        hasher.update(b"||");
        hasher.update(prev_hash.as_bytes());
        let bytes = hasher.finalize();
        // 16 bytes (128-bit) → 26 chars base32 without padding.
        base32_16(&bytes.as_bytes()[..16])
    }
}

fn base32_16(bytes: &[u8]) -> String {
    // RFC 4648 alphabet, no padding.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity(26);
    let mut buf: u64 = 0;
    let mut bits: u32 = 0;
    for b in bytes {
        buf = (buf << 8) | u64::from(*b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buf >> bits) & 0x1f) as usize;
            out.push(ALPHABET[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buf << (5 - bits)) & 0x1f) as usize;
        out.push(ALPHABET[idx] as char);
    }
    // 128 bits → 26 characters (25 full + 1 remainder).
    out
}

/// Append-only audit store. Memory-sink implementation; disk/syslog/
/// sync/nats/http are caller responsibility (they read `AuditEntry`
/// from the public surface and forward).
#[derive(Debug, Clone, Default)]
pub struct AuditStore {
    entries: VecDeque<AuditEntry>,
    next_seq: u64,
    last_hash: String,
    max_entries: u32,
}

impl AuditStore {
    #[must_use]
    pub fn new(max_entries: u32) -> Self {
        Self {
            entries: VecDeque::new(),
            next_seq: 0,
            last_hash: AuditEntry::GENESIS_PREV.to_owned(),
            max_entries,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn entries(&self) -> &VecDeque<AuditEntry> {
        &self.entries
    }

    #[must_use]
    pub fn last_hash(&self) -> &str {
        &self.last_hash
    }

    /// Append a new entry. Ring-buffers when over `max_entries`.
    pub fn append(
        &mut self,
        ts: i64,
        kind: EventKind,
        actor: Vec<(String, String)>,
        details: String,
    ) -> AuditEntry {
        let seq = self.next_seq;
        let prev_hash = self.last_hash.clone();
        let content_hash =
            AuditEntry::compute_hash(seq, ts, kind, &actor, &details, &prev_hash);
        let entry = AuditEntry {
            seq,
            ts,
            kind,
            actor,
            details,
            prev_hash,
            content_hash: content_hash.clone(),
        };
        self.entries.push_back(entry.clone());
        self.last_hash = content_hash;
        self.next_seq += 1;
        // Ring buffer eviction — drop oldest.
        if self.max_entries != 0 {
            while self.entries.len() > self.max_entries as usize {
                self.entries.pop_front();
            }
        }
        entry
    }

    /// Verify that the chain is intact: each entry's `content_hash`
    /// matches the recomputed BLAKE3, and each `prev_hash` matches
    /// the previous entry's `content_hash`.
    ///
    /// Returns `Ok(())` when the chain verifies; `Err(seq)` with the
    /// sequence number of the first broken link otherwise.
    pub fn verify(&self) -> Result<(), u64> {
        let mut expected_prev = if let Some(first) = self.entries.front() {
            first.prev_hash.clone()
        } else {
            return Ok(());
        };
        for e in &self.entries {
            if e.prev_hash != expected_prev {
                return Err(e.seq);
            }
            let recomputed = AuditEntry::compute_hash(
                e.seq,
                e.ts,
                e.kind,
                &e.actor,
                &e.details,
                &e.prev_hash,
            );
            if recomputed != e.content_hash {
                return Err(e.seq);
            }
            expected_prev = e.content_hash.clone();
        }
        Ok(())
    }

    /// Clear the log — itself audited by emitting one last
    /// `AuditClear` entry.
    pub fn clear(&mut self, ts: i64, actor: Vec<(String, String)>) {
        self.append(ts, EventKind::AuditClear, actor, String::from("{}"));
        // Keep the AuditClear entry so the chain isn't empty.
        let clear_entry = self.entries.pop_back().expect("just appended");
        self.entries.clear();
        self.next_seq = clear_entry.seq + 1;
        self.last_hash = clear_entry.content_hash.clone();
        self.entries.push_back(clear_entry);
    }
}

/// Registry — usually one profile is active, but multiple are
/// allowed (one per sink strategy).
#[derive(Debug, Clone, Default)]
pub struct AuditTrailRegistry {
    specs: Vec<AuditTrailSpec>,
}

impl AuditTrailRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: AuditTrailSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = AuditTrailSpec>) {
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
    pub fn specs(&self) -> &[AuditTrailSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&AuditTrailSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// All enabled profiles that would record this event kind.
    #[must_use]
    pub fn profiles_for(&self, kind: EventKind) -> Vec<&AuditTrailSpec> {
        self.specs.iter().filter(|s| s.captures(kind)).collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<AuditTrailSpec>, String> {
    tatara_lisp::compile_typed::<AuditTrailSpec>(src)
        .map_err(|e| format!("failed to compile defaudit-trail forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<AuditTrailSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_disabled_privacy_first() {
        let s = AuditTrailSpec::default_profile();
        assert!(!s.enabled);
        assert!(s.redact_secrets);
        assert_eq!(s.sinks, vec![Sink::Memory]);
    }

    #[test]
    fn captures_filters_on_enabled() {
        let s = AuditTrailSpec {
            enabled: false,
            events: vec![],
            ..AuditTrailSpec::default_profile()
        };
        assert!(!s.captures(EventKind::RcReload));
    }

    #[test]
    fn captures_empty_event_list_means_all() {
        let s = AuditTrailSpec {
            enabled: true,
            events: vec![],
            ..AuditTrailSpec::default_profile()
        };
        assert!(s.captures(EventKind::RcReload));
        assert!(s.captures(EventKind::PermissionGrant));
        assert!(s.captures(EventKind::Custom));
    }

    #[test]
    fn captures_named_events_limits_to_set() {
        let s = AuditTrailSpec {
            enabled: true,
            events: vec![EventKind::RcReload, EventKind::IdentitySwitch],
            ..AuditTrailSpec::default_profile()
        };
        assert!(s.captures(EventKind::RcReload));
        assert!(s.captures(EventKind::IdentitySwitch));
        assert!(!s.captures(EventKind::SyncChange));
    }

    #[test]
    fn genesis_chain_starts_with_empty_prev_hash() {
        let mut store = AuditStore::new(100);
        let e = store.append(1, EventKind::RcReload, vec![], "{}".into());
        assert_eq!(e.seq, 0);
        assert_eq!(e.prev_hash, AuditEntry::GENESIS_PREV);
        assert_eq!(e.content_hash.len(), 26);
    }

    #[test]
    fn append_chains_hashes() {
        let mut store = AuditStore::new(100);
        let e1 = store.append(1, EventKind::RcReload, vec![], "{}".into());
        let e2 = store.append(
            2,
            EventKind::ExtensionInstall,
            vec![("device_id".into(), "abc".into())],
            r#"{"name":"foo"}"#.into(),
        );
        assert_eq!(e2.prev_hash, e1.content_hash);
        assert_ne!(e1.content_hash, e2.content_hash);
    }

    #[test]
    fn chain_verifies_untouched_log() {
        let mut store = AuditStore::new(100);
        for i in 0..20 {
            store.append(
                i as i64,
                EventKind::RcReload,
                vec![("seq".into(), i.to_string())],
                format!(r#"{{"i":{i}}}"#),
            );
        }
        assert!(store.verify().is_ok());
    }

    #[test]
    fn chain_detects_tampered_details() {
        let mut store = AuditStore::new(100);
        store.append(1, EventKind::RcReload, vec![], "a".into());
        store.append(2, EventKind::RcReload, vec![], "b".into());
        store.append(3, EventKind::RcReload, vec![], "c".into());
        // Tamper with the middle entry.
        if let Some(mid) = store.entries.get_mut(1) {
            mid.details = "HACKED".into();
        }
        match store.verify() {
            Err(seq) => assert_eq!(seq, 1),
            Ok(()) => panic!("tamper should be detected"),
        }
    }

    #[test]
    fn chain_detects_tampered_prev_hash() {
        let mut store = AuditStore::new(100);
        store.append(1, EventKind::RcReload, vec![], "a".into());
        store.append(2, EventKind::RcReload, vec![], "b".into());
        if let Some(second) = store.entries.get_mut(1) {
            second.prev_hash = "FAKEHASH00000000000000000A".into();
        }
        assert!(store.verify().is_err());
    }

    #[test]
    fn ring_buffer_evicts_oldest() {
        let mut store = AuditStore::new(3);
        for i in 0..5 {
            store.append(i, EventKind::RcReload, vec![], format!("{i}"));
        }
        assert_eq!(store.len(), 3);
        // Oldest kept is seq=2 (0 and 1 got evicted).
        assert_eq!(store.entries.front().unwrap().seq, 2);
        assert_eq!(store.entries.back().unwrap().seq, 4);
    }

    #[test]
    fn ring_buffered_tail_still_verifies() {
        let mut store = AuditStore::new(3);
        for i in 0..5 {
            store.append(i, EventKind::RcReload, vec![], format!("{i}"));
        }
        // Even after eviction, the kept tail must verify against itself
        // — the chain within the remaining window is intact.
        assert!(store.verify().is_ok());
    }

    #[test]
    fn clear_emits_final_audit_clear_entry() {
        let mut store = AuditStore::new(100);
        store.append(1, EventKind::RcReload, vec![], "a".into());
        store.append(2, EventKind::RcReload, vec![], "b".into());
        store.clear(99, vec![("user".into(), "me".into())]);
        assert_eq!(store.len(), 1);
        assert_eq!(store.entries.front().unwrap().kind, EventKind::AuditClear);
        // Clear is itself verifiable (a 1-entry chain).
        assert!(store.verify().is_ok());
    }

    #[test]
    fn compute_hash_is_deterministic() {
        let h1 = AuditEntry::compute_hash(
            7,
            1_700_000_000,
            EventKind::PermissionGrant,
            &[("device".into(), "abc".into())],
            "{}",
            "PREV00000000000000000000AB",
        );
        let h2 = AuditEntry::compute_hash(
            7,
            1_700_000_000,
            EventKind::PermissionGrant,
            &[("device".into(), "abc".into())],
            "{}",
            "PREV00000000000000000000AB",
        );
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 26);
    }

    #[test]
    fn compute_hash_differs_on_any_field_change() {
        let base = AuditEntry::compute_hash(1, 0, EventKind::RcReload, &[], "{}", "");
        let different_seq = AuditEntry::compute_hash(2, 0, EventKind::RcReload, &[], "{}", "");
        let different_ts = AuditEntry::compute_hash(1, 1, EventKind::RcReload, &[], "{}", "");
        let different_kind =
            AuditEntry::compute_hash(1, 0, EventKind::PermissionGrant, &[], "{}", "");
        let different_details =
            AuditEntry::compute_hash(1, 0, EventKind::RcReload, &[], "{\"x\":1}", "");
        let different_prev = AuditEntry::compute_hash(1, 0, EventKind::RcReload, &[], "{}", "x");

        assert_ne!(base, different_seq);
        assert_ne!(base, different_ts);
        assert_ne!(base, different_kind);
        assert_ne!(base, different_details);
        assert_ne!(base, different_prev);
    }

    #[test]
    fn event_kind_roundtrips_through_serde() {
        for k in [
            EventKind::RcReload,
            EventKind::ExtensionInstall,
            EventKind::ExtensionUninstall,
            EventKind::ExtensionToggle,
            EventKind::PermissionGrant,
            EventKind::PermissionDeny,
            EventKind::IdentitySwitch,
            EventKind::SyncChange,
            EventKind::StorageClear,
            EventKind::RoutingChange,
            EventKind::Navigation,
            EventKind::TotpRead,
            EventKind::DslFailure,
            EventKind::PermissionBlocked,
            EventKind::CapabilityRequest,
            EventKind::AuditClear,
            EventKind::Custom,
        ] {
            let json = serde_json::to_string(&k).unwrap();
            let back: EventKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, k);
        }
    }

    #[test]
    fn sink_roundtrips_through_serde() {
        for s in [
            Sink::Memory,
            Sink::Disk,
            Sink::Syslog,
            Sink::Sync,
            Sink::RemoteHttp,
            Sink::Nats,
        ] {
            let spec = AuditTrailSpec {
                sinks: vec![s],
                ..AuditTrailSpec::default_profile()
            };
            let json = serde_json::to_string(&spec).unwrap();
            let back: AuditTrailSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.sinks, vec![s]);
        }
    }

    #[test]
    fn registry_profiles_for_filters_by_event_and_enabled() {
        let mut reg = AuditTrailRegistry::new();
        reg.insert(AuditTrailSpec::default_profile()); // disabled
        reg.insert(AuditTrailSpec {
            name: "on".into(),
            enabled: true,
            events: vec![EventKind::RcReload],
            ..AuditTrailSpec::default_profile()
        });
        let rc = reg.profiles_for(EventKind::RcReload);
        assert_eq!(rc.len(), 1);
        assert_eq!(rc[0].name, "on");
        // Another event kind should not match.
        assert!(reg.profiles_for(EventKind::IdentitySwitch).is_empty());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_audit_trail_form() {
        let src = r#"
            (defaudit-trail :name "security"
                            :sinks ("memory" "disk" "syslog")
                            :events ("rc-reload" "extension-install"
                                     "permission-grant" "permission-deny"
                                     "identity-switch" "storage-clear")
                            :actor-fields ("device-id" "session-id")
                            :max-entries 5000
                            :redact-secrets #t
                            :enabled #t
                            :disk-path "~/.cache/nami/audit.jsonl"
                            :rotate-bytes 8388608)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.sinks.len(), 3);
        assert!(s.enabled);
        assert_eq!(s.max_entries, 5000);
        assert_eq!(s.events.len(), 6);
    }
}
