//! `(deftime-travel)` — DOM-state time-travel capture.
//!
//! **Novel** — mainstream browsers offer devtools-only features
//! that are vaguely adjacent (Firefox's "action recorder", Chrome's
//! Recorder panel, Replay.io as an external tool), but none give
//! authors a first-class substrate-level declarative capture that
//! records DOM state at intervals and exposes a pure
//! `rewind(n_steps)` API.
//!
//! Each profile declares:
//!   * a host glob the trail fires on
//!   * sample cadence (every N ms, or on named events)
//!   * what to capture (DOM sexp, scroll, form state, console log)
//!   * ring-buffer depth + total byte cap
//!   * whether to integrate with (defaudit-trail) for BLAKE3-chained
//!     integrity
//!
//! ```lisp
//! (deftime-travel :name         "default"
//!                 :host         "*"
//!                 :sample-ms    1000
//!                 :trigger      (interval navigate scroll-end dom-mutation)
//!                 :capture      (dom scroll form-state console url)
//!                 :max-snapshots 60
//!                 :max-bytes    8388608
//!                 :chained      #t)
//! ```

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Event that causes a new snapshot.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Trigger {
    /// Fire every `sample_ms` ms.
    Interval,
    /// Fire on navigation (push-state / popstate / nav).
    Navigate,
    /// Fire after a scroll-end idle (~100 ms after last scroll).
    ScrollEnd,
    /// Fire after a DOM-mutation batch lands.
    DomMutation,
    /// Fire on form-input change.
    FormChange,
    /// Fire on explicit `navigator.scheduling.mark('time-travel')` call.
    Explicit,
    /// Fire before the tab closes (last-chance snapshot).
    BeforeUnload,
}

/// Which surfaces get captured.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureField {
    /// Full DOM as an s-expression (substrate-canonical).
    Dom,
    /// (x, y) scroll position.
    Scroll,
    /// All form input values + selection ranges.
    FormState,
    /// URL + path + query.
    Url,
    /// Tail of the console output.
    Console,
    /// Name of the active (defidentity) persona.
    Identity,
    /// Current local-storage snapshot.
    LocalStorage,
    /// Session-storage snapshot.
    SessionStorage,
    /// Visible viewport rect.
    Viewport,
    /// Active focus-element CSS selector.
    FocusSelector,
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "deftime-travel"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimeTravelSpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    /// Interval between `Trigger::Interval` samples.
    #[serde(default = "default_sample_ms")]
    pub sample_ms: u32,
    #[serde(default = "default_trigger")]
    pub trigger: Vec<Trigger>,
    #[serde(default = "default_capture")]
    pub capture: Vec<CaptureField>,
    /// Max snapshots retained in the ring buffer (0 = unlimited).
    #[serde(default = "default_max_snapshots")]
    pub max_snapshots: u32,
    /// Max total payload bytes across all retained snapshots (0 =
    /// no byte cap).
    #[serde(default = "default_max_bytes")]
    pub max_bytes: u64,
    /// Compress snapshots before storing.
    #[serde(default = "default_compressed")]
    pub compressed: bool,
    /// Chain each snapshot's hash to the previous (same BLAKE3
    /// convention as (defaudit-trail)). Lets the UI prove the
    /// replay sequence wasn't reordered.
    #[serde(default = "default_chained")]
    pub chained: bool,
    /// Minimum DOM-byte delta from previous snapshot for the new
    /// sample to actually be recorded (0 = record every sample).
    #[serde(default)]
    pub min_delta_bytes: u64,
    /// Redact form values (passwords, credit cards, 2FA secrets)
    /// before storing. Privacy-first default.
    #[serde(default = "default_redact")]
    pub redact_secrets: bool,
    /// Hosts exempt from capture entirely (banking, health, …).
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
fn default_sample_ms() -> u32 {
    1_000
}
fn default_trigger() -> Vec<Trigger> {
    vec![
        Trigger::Interval,
        Trigger::Navigate,
        Trigger::DomMutation,
        Trigger::BeforeUnload,
    ]
}
fn default_capture() -> Vec<CaptureField> {
    vec![
        CaptureField::Dom,
        CaptureField::Scroll,
        CaptureField::FormState,
        CaptureField::Url,
    ]
}
fn default_max_snapshots() -> u32 {
    60
}
fn default_max_bytes() -> u64 {
    8 * 1024 * 1024
}
fn default_compressed() -> bool {
    true
}
fn default_chained() -> bool {
    true
}
fn default_redact() -> bool {
    true
}
fn default_enabled() -> bool {
    false // privacy-first — opt-in only.
}

/// One captured moment. Payload is opaque to this crate (the host
/// serializes; we just hold bytes + hashes).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub seq: u64,
    pub ts_ms: i64,
    pub trigger: Trigger,
    /// Raw payload bytes (host-serialized).
    pub payload: Vec<u8>,
    /// BLAKE3-128 26-char base32 hash of the previous snapshot.
    /// Empty for the genesis snapshot.
    pub prev_hash: String,
    /// BLAKE3-128 26-char base32 hash of THIS snapshot's canonical
    /// form (seq || ts || trigger || payload || prev_hash).
    pub content_hash: String,
}

impl Snapshot {
    pub const GENESIS_PREV: &'static str = "";

    #[must_use]
    pub fn compute_hash(
        seq: u64,
        ts_ms: i64,
        trigger: Trigger,
        payload: &[u8],
        prev_hash: &str,
    ) -> String {
        let mut h = blake3::Hasher::new();
        h.update(&seq.to_be_bytes());
        h.update(&ts_ms.to_be_bytes());
        let t = serde_json::to_string(&trigger).unwrap_or_default();
        h.update(t.as_bytes());
        h.update(b"||");
        h.update(payload);
        h.update(b"||");
        h.update(prev_hash.as_bytes());
        base32_16(&h.finalize().as_bytes()[..16])
    }
}

fn base32_16(bytes: &[u8]) -> String {
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
    out
}

impl TimeTravelSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            sample_ms: 1_000,
            trigger: default_trigger(),
            capture: default_capture(),
            max_snapshots: 60,
            max_bytes: 8 * 1024 * 1024,
            compressed: true,
            chained: true,
            min_delta_bytes: 0,
            redact_secrets: true,
            exempt_hosts: vec![],
            enabled: false,
            description: Some(
                "Default time-travel — DISABLED (privacy-first). Enable + author in rc file.".into(),
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
    pub fn is_exempt(&self, host: &str) -> bool {
        self.exempt_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    #[must_use]
    pub fn captures(&self, field: CaptureField) -> bool {
        self.capture.contains(&field)
    }

    #[must_use]
    pub fn fires_on(&self, trig: Trigger) -> bool {
        self.trigger.contains(&trig)
    }
}

/// Ring-buffered time-travel recorder. One instance per enabled
/// profile × tab (bookkeeping is caller's).
#[derive(Debug, Clone, Default)]
pub struct TimeTravelStore {
    snapshots: VecDeque<Snapshot>,
    next_seq: u64,
    last_hash: String,
    total_bytes: u64,
    max_snapshots: u32,
    max_bytes: u64,
    chained: bool,
    min_delta_bytes: u64,
    last_payload_len: u64,
}

impl TimeTravelStore {
    #[must_use]
    pub fn new(max_snapshots: u32, max_bytes: u64, chained: bool, min_delta_bytes: u64) -> Self {
        Self {
            snapshots: VecDeque::new(),
            next_seq: 0,
            last_hash: Snapshot::GENESIS_PREV.into(),
            total_bytes: 0,
            max_snapshots,
            max_bytes,
            chained,
            min_delta_bytes,
            last_payload_len: 0,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    #[must_use]
    pub fn snapshots(&self) -> &VecDeque<Snapshot> {
        &self.snapshots
    }

    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    #[must_use]
    pub fn last_hash(&self) -> &str {
        &self.last_hash
    }

    /// Try to record a new snapshot. Returns the accepted snapshot,
    /// or None when the `min_delta_bytes` filter rejects it.
    pub fn record(
        &mut self,
        ts_ms: i64,
        trigger: Trigger,
        payload: Vec<u8>,
    ) -> Option<Snapshot> {
        // Delta gate — compare absolute byte-length delta of payload.
        let new_len = payload.len() as u64;
        if self.min_delta_bytes != 0 && self.next_seq > 0 {
            let delta = new_len.abs_diff(self.last_payload_len);
            if delta < self.min_delta_bytes {
                return None;
            }
        }

        let seq = self.next_seq;
        let prev_hash = if self.chained {
            self.last_hash.clone()
        } else {
            String::new()
        };
        let content_hash =
            Snapshot::compute_hash(seq, ts_ms, trigger, &payload, &prev_hash);
        let snap = Snapshot {
            seq,
            ts_ms,
            trigger,
            payload,
            prev_hash,
            content_hash: content_hash.clone(),
        };
        self.total_bytes += new_len;
        self.snapshots.push_back(snap.clone());
        if self.chained {
            self.last_hash = content_hash;
        }
        self.last_payload_len = new_len;
        self.next_seq += 1;
        self.enforce_caps();
        Some(snap)
    }

    fn enforce_caps(&mut self) {
        if self.max_snapshots != 0 {
            while self.snapshots.len() > self.max_snapshots as usize {
                if let Some(front) = self.snapshots.pop_front() {
                    self.total_bytes = self
                        .total_bytes
                        .saturating_sub(front.payload.len() as u64);
                }
            }
        }
        if self.max_bytes != 0 {
            while self.total_bytes > self.max_bytes && self.snapshots.len() > 1 {
                if let Some(front) = self.snapshots.pop_front() {
                    self.total_bytes = self
                        .total_bytes
                        .saturating_sub(front.payload.len() as u64);
                }
            }
        }
    }

    /// Rewind `n` steps from the newest snapshot. Returns `None` if
    /// the buffer is empty or `n` is out of range.
    #[must_use]
    pub fn rewind(&self, n: usize) -> Option<&Snapshot> {
        if self.snapshots.is_empty() {
            return None;
        }
        let last = self.snapshots.len() - 1;
        last.checked_sub(n).and_then(|idx| self.snapshots.get(idx))
    }

    /// Walk the chain — if any link is broken, returns the `seq` of
    /// the first broken link. Used to prove a replay is faithful.
    pub fn verify(&self) -> Result<(), u64> {
        if !self.chained {
            return Ok(());
        }
        let mut expected_prev = if let Some(first) = self.snapshots.front() {
            first.prev_hash.clone()
        } else {
            return Ok(());
        };
        for s in &self.snapshots {
            if s.prev_hash != expected_prev {
                return Err(s.seq);
            }
            let recomputed = Snapshot::compute_hash(
                s.seq,
                s.ts_ms,
                s.trigger,
                &s.payload,
                &s.prev_hash,
            );
            if recomputed != s.content_hash {
                return Err(s.seq);
            }
            expected_prev = s.content_hash.clone();
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.snapshots.clear();
        self.total_bytes = 0;
        self.next_seq = 0;
        self.last_hash = Snapshot::GENESIS_PREV.into();
        self.last_payload_len = 0;
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct TimeTravelRegistry {
    specs: Vec<TimeTravelSpec>,
}

impl TimeTravelRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: TimeTravelSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = TimeTravelSpec>) {
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
    pub fn specs(&self) -> &[TimeTravelSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&TimeTravelSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// All enabled profiles applicable to `host` (not exempt).
    #[must_use]
    pub fn applicable_to(&self, host: &str) -> Vec<&TimeTravelSpec> {
        self.specs
            .iter()
            .filter(|s| s.enabled && s.matches_host(host) && !s.is_exempt(host))
            .collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<TimeTravelSpec>, String> {
    tatara_lisp::compile_typed::<TimeTravelSpec>(src)
        .map_err(|e| format!("failed to compile deftime-travel forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<TimeTravelSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_disabled_privacy_first() {
        let s = TimeTravelSpec::default_profile();
        assert!(!s.enabled);
        assert!(s.redact_secrets);
        assert!(s.chained);
    }

    #[test]
    fn fires_on_trigger_set() {
        let s = TimeTravelSpec::default_profile();
        assert!(s.fires_on(Trigger::Interval));
        assert!(s.fires_on(Trigger::Navigate));
        assert!(s.fires_on(Trigger::BeforeUnload));
        assert!(!s.fires_on(Trigger::FormChange));
    }

    #[test]
    fn captures_default_fields() {
        let s = TimeTravelSpec::default_profile();
        assert!(s.captures(CaptureField::Dom));
        assert!(s.captures(CaptureField::Scroll));
        assert!(s.captures(CaptureField::Url));
        assert!(!s.captures(CaptureField::LocalStorage));
    }

    #[test]
    fn record_appends_genesis() {
        let mut store = TimeTravelStore::new(10, 0, true, 0);
        let s = store.record(1, Trigger::Interval, vec![0u8; 32]).unwrap();
        assert_eq!(s.seq, 0);
        assert_eq!(s.prev_hash, Snapshot::GENESIS_PREV);
        assert_eq!(s.content_hash.len(), 26);
    }

    #[test]
    fn record_chains_hashes() {
        let mut store = TimeTravelStore::new(10, 0, true, 0);
        let a = store.record(1, Trigger::Interval, vec![0u8; 16]).unwrap();
        let b = store.record(2, Trigger::Navigate, vec![1u8; 16]).unwrap();
        assert_eq!(b.prev_hash, a.content_hash);
        assert_ne!(a.content_hash, b.content_hash);
    }

    #[test]
    fn record_unchained_store_leaves_prev_empty() {
        let mut store = TimeTravelStore::new(10, 0, false, 0);
        let a = store.record(1, Trigger::Interval, vec![0u8; 16]).unwrap();
        let b = store.record(2, Trigger::Interval, vec![0u8; 16]).unwrap();
        assert_eq!(a.prev_hash, "");
        assert_eq!(b.prev_hash, "");
    }

    #[test]
    fn min_delta_bytes_filters_small_samples() {
        let mut store = TimeTravelStore::new(10, 0, true, 10);
        // First one always goes in (no prior payload to compare).
        assert!(store.record(1, Trigger::Interval, vec![0u8; 32]).is_some());
        // Payload same length → delta = 0 → rejected.
        assert!(store.record(2, Trigger::Interval, vec![1u8; 32]).is_none());
        // Much larger delta → accepted.
        assert!(store.record(3, Trigger::Interval, vec![2u8; 100]).is_some());
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn ring_buffer_evicts_oldest_when_over_snapshots_cap() {
        let mut store = TimeTravelStore::new(3, 0, true, 0);
        for i in 0..5 {
            store.record(i, Trigger::Interval, vec![0u8; 8]).unwrap();
        }
        assert_eq!(store.len(), 3);
        assert_eq!(store.snapshots.front().unwrap().seq, 2);
        assert_eq!(store.snapshots.back().unwrap().seq, 4);
    }

    #[test]
    fn byte_cap_evicts_when_over_size() {
        let mut store = TimeTravelStore::new(0, 100, true, 0);
        store.record(1, Trigger::Interval, vec![0u8; 60]).unwrap();
        store.record(2, Trigger::Interval, vec![0u8; 60]).unwrap();
        // Together they're 120 > 100 cap. Oldest (60 bytes) evicted.
        assert_eq!(store.len(), 1);
        assert_eq!(store.total_bytes(), 60);
    }

    #[test]
    fn byte_cap_never_fully_empties_active_window() {
        let mut store = TimeTravelStore::new(0, 1, true, 0);
        store.record(1, Trigger::Interval, vec![0u8; 100]).unwrap();
        // New payload alone is 100× the cap — but we keep at least 1.
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn rewind_walks_backwards() {
        let mut store = TimeTravelStore::new(10, 0, true, 0);
        for i in 0..5 {
            store.record(i, Trigger::Interval, vec![i as u8; 4]).unwrap();
        }
        assert_eq!(store.rewind(0).unwrap().seq, 4);
        assert_eq!(store.rewind(2).unwrap().seq, 2);
        assert_eq!(store.rewind(4).unwrap().seq, 0);
        assert!(store.rewind(10).is_none());
    }

    #[test]
    fn rewind_empty_is_none() {
        let store = TimeTravelStore::default();
        assert!(store.rewind(0).is_none());
    }

    #[test]
    fn verify_chain_untouched_log() {
        let mut store = TimeTravelStore::new(10, 0, true, 0);
        for i in 0..10 {
            store.record(i, Trigger::Interval, vec![i as u8; 8]).unwrap();
        }
        assert!(store.verify().is_ok());
    }

    #[test]
    fn verify_detects_tampered_payload() {
        let mut store = TimeTravelStore::new(10, 0, true, 0);
        store.record(1, Trigger::Interval, vec![0u8; 8]).unwrap();
        store.record(2, Trigger::Navigate, vec![1u8; 8]).unwrap();
        store.record(3, Trigger::Interval, vec![2u8; 8]).unwrap();
        if let Some(mid) = store.snapshots.get_mut(1) {
            mid.payload[0] = 0xff;
        }
        assert_eq!(store.verify(), Err(1));
    }

    #[test]
    fn verify_detects_tampered_prev_hash() {
        let mut store = TimeTravelStore::new(10, 0, true, 0);
        store.record(1, Trigger::Interval, vec![0u8; 8]).unwrap();
        store.record(2, Trigger::Interval, vec![1u8; 8]).unwrap();
        if let Some(second) = store.snapshots.get_mut(1) {
            second.prev_hash = "FAKEHASH00000000000000000A".into();
        }
        assert!(store.verify().is_err());
    }

    #[test]
    fn verify_unchained_store_is_always_ok() {
        let mut store = TimeTravelStore::new(10, 0, false, 0);
        store.record(1, Trigger::Interval, vec![0u8; 8]).unwrap();
        if let Some(s) = store.snapshots.get_mut(0) {
            s.payload[0] = 0xff;
        }
        assert!(store.verify().is_ok());
    }

    #[test]
    fn clear_resets_everything() {
        let mut store = TimeTravelStore::new(10, 0, true, 0);
        store.record(1, Trigger::Interval, vec![0u8; 8]).unwrap();
        store.record(2, Trigger::Interval, vec![0u8; 8]).unwrap();
        store.clear();
        assert!(store.is_empty());
        assert_eq!(store.total_bytes(), 0);
        assert_eq!(store.last_hash(), Snapshot::GENESIS_PREV);
    }

    #[test]
    fn trigger_roundtrips_through_serde() {
        for t in [
            Trigger::Interval,
            Trigger::Navigate,
            Trigger::ScrollEnd,
            Trigger::DomMutation,
            Trigger::FormChange,
            Trigger::Explicit,
            Trigger::BeforeUnload,
        ] {
            let json = serde_json::to_string(&t).unwrap();
            let back: Trigger = serde_json::from_str(&json).unwrap();
            assert_eq!(back, t);
        }
    }

    #[test]
    fn capture_field_roundtrips_through_serde() {
        let all = vec![
            CaptureField::Dom,
            CaptureField::Scroll,
            CaptureField::FormState,
            CaptureField::Url,
            CaptureField::Console,
            CaptureField::Identity,
            CaptureField::LocalStorage,
            CaptureField::SessionStorage,
            CaptureField::Viewport,
            CaptureField::FocusSelector,
        ];
        let s = TimeTravelSpec {
            capture: all.clone(),
            ..TimeTravelSpec::default_profile()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: TimeTravelSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.capture, all);
    }

    #[test]
    fn applicable_to_filters_by_host_and_enabled() {
        let mut reg = TimeTravelRegistry::new();
        reg.insert(TimeTravelSpec::default_profile()); // disabled
        reg.insert(TimeTravelSpec {
            name: "docs".into(),
            host: "*://*.docs.com/*".into(),
            enabled: true,
            ..TimeTravelSpec::default_profile()
        });
        assert!(reg.applicable_to("example.org").is_empty());
        let docs = reg.applicable_to("www.docs.com");
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].name, "docs");
    }

    #[test]
    fn applicable_to_respects_exempt_hosts() {
        let mut reg = TimeTravelRegistry::new();
        reg.insert(TimeTravelSpec {
            name: "everywhere".into(),
            enabled: true,
            exempt_hosts: vec!["*://*.bank.com/*".into()],
            ..TimeTravelSpec::default_profile()
        });
        assert!(reg.applicable_to("my.bank.com").is_empty());
        assert_eq!(reg.applicable_to("example.com").len(), 1);
    }

    #[test]
    fn compute_hash_is_deterministic_and_sensitive() {
        let a = Snapshot::compute_hash(1, 100, Trigger::Interval, b"abc", "prev");
        let b = Snapshot::compute_hash(1, 100, Trigger::Interval, b"abc", "prev");
        assert_eq!(a, b);
        assert_eq!(a.len(), 26);
        let c = Snapshot::compute_hash(1, 100, Trigger::Interval, b"abd", "prev");
        assert_ne!(a, c);
        let d = Snapshot::compute_hash(2, 100, Trigger::Interval, b"abc", "prev");
        assert_ne!(a, d);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_time_travel_form() {
        let src = r#"
            (deftime-travel :name "default"
                            :host "*"
                            :sample-ms 500
                            :trigger ("interval" "navigate" "dom-mutation")
                            :capture ("dom" "scroll" "form-state" "url" "console")
                            :max-snapshots 120
                            :max-bytes 16777216
                            :compressed #t
                            :chained #t
                            :redact-secrets #t
                            :enabled #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.sample_ms, 500);
        assert_eq!(s.max_snapshots, 120);
        assert!(s.enabled);
        assert!(s.chained);
        assert_eq!(s.trigger.len(), 3);
    }
}
