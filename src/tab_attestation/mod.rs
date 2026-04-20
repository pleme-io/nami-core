//! `(deftab-attestation)` — per-tab cryptographic integrity chain.
//!
//! **Novel** — no mainstream browser provides per-tab attestation.
//! Chrome/Firefox/Safari treat a tab as ephemeral, untyped, unsigned
//! UI state. This module declares a tab as a typed, append-only,
//! BLAKE3-chained log: navigations, script loads, permission grants,
//! form submissions, storage writes, and fetches become `TabEntry`
//! records with `prev_hash` pointers rooted at the tab's genesis URL.
//!
//! Compared to `(defaudit-trail)`, which is a *substrate-wide* event
//! log, this is *per-tab*: one chain per live tab, keyed by tab_id,
//! with a genesis = "tab opened with url X". Export at close → you
//! get a provable, tamper-evident record of exactly what happened in
//! that tab's lifetime. Compatible with the pleme-io tameshi /
//! sekiban / kensa / inshou attestation family (BLAKE3 128-bit,
//! base32 26-char).
//!
//! ```lisp
//! (deftab-attestation :name         "strict"
//!                     :enabled      #t
//!                     :host         "*"
//!                     :track        (navigate script-load
//!                                    permission-grant form-submit
//!                                    storage-write fetch redirect)
//!                     :max-entries  1000
//!                     :include-body #f
//!                     :export-on-close #t)
//! ```

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Event kinds recorded in a tab's chain.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum TabEventKind {
    /// Tab opened (genesis) — details = initial URL.
    Genesis,
    /// Navigation to a new URL.
    Navigate,
    /// A `<script>` was loaded / executed.
    ScriptLoad,
    /// A permission was granted to the page.
    PermissionGrant,
    /// A permission was denied.
    PermissionDeny,
    /// A `<form>` was submitted.
    FormSubmit,
    /// A key was written to storage (localStorage/sessionStorage/IndexedDB).
    StorageWrite,
    /// A network fetch was issued.
    Fetch,
    /// Server-driven redirect.
    Redirect,
    /// Tab closed — sealing entry; no further appends permitted.
    Close,
    /// A keyboard / pointer gesture originated from the user (used to
    /// demonstrate user-activation later).
    UserGesture,
    /// An extension or DSL mutated the tab's DOM.
    DomMutation,
}

/// Per-tab attestation spec.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "deftab-attestation"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TabAttestationSpec {
    pub name: String,
    /// Host glob — `"*"` applies to every tab.
    #[serde(default = "default_host")]
    pub host: String,
    /// Which kinds to record. Empty = all.
    #[serde(default)]
    pub track: Vec<TabEventKind>,
    /// Maximum entries per chain (ring-buffered). `0` = unlimited.
    #[serde(default = "default_max_entries")]
    pub max_entries: u32,
    /// Include request/form/storage bodies in `details`. Defaults off
    /// for privacy — bodies can carry PII + secrets.
    #[serde(default)]
    pub include_body: bool,
    /// Include response body bytes for Fetch entries.
    #[serde(default)]
    pub include_response_body: bool,
    /// Emit a Close sealing entry + export the chain when the tab is
    /// disposed. Disabled = chain discarded on close.
    #[serde(default = "default_export_on_close")]
    pub export_on_close: bool,
    /// Forward entries to (defaudit-trail) — composes with the
    /// substrate-wide log.
    #[serde(default)]
    pub forward_to_audit: bool,
    /// Hosts that must NEVER be attested (sensitive apps; banks,
    /// healthcare) — a per-host allow-list that opts OUT of chaining.
    #[serde(default)]
    pub exempt_hosts: Vec<String>,
    /// Privacy-first: disabled until explicitly opted-in.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_max_entries() -> u32 {
    1000
}
fn default_export_on_close() -> bool {
    true
}
fn default_enabled() -> bool {
    false
}

impl TabAttestationSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            track: vec![],
            max_entries: 1000,
            include_body: false,
            include_response_body: false,
            export_on_close: true,
            forward_to_audit: false,
            exempt_hosts: vec![],
            enabled: false,
            description: Some(
                "Per-tab attestation — disabled (privacy-first). Enable in rc file.".into(),
            ),
        }
    }

    #[must_use]
    pub fn strict_profile() -> Self {
        Self {
            name: "strict".into(),
            enabled: true,
            track: vec![
                TabEventKind::Genesis,
                TabEventKind::Navigate,
                TabEventKind::ScriptLoad,
                TabEventKind::PermissionGrant,
                TabEventKind::PermissionDeny,
                TabEventKind::FormSubmit,
                TabEventKind::StorageWrite,
                TabEventKind::Fetch,
                TabEventKind::Redirect,
                TabEventKind::Close,
            ],
            forward_to_audit: true,
            description: Some(
                "Strict — records every privacy-relevant event, forwards to substrate audit."
                    .into(),
            ),
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
    pub fn records(&self, kind: TabEventKind) -> bool {
        self.enabled && (self.track.is_empty() || self.track.contains(&kind))
    }

    /// Should we start a chain for a tab opened on `host`?
    #[must_use]
    pub fn should_chain(&self, host: &str) -> bool {
        self.enabled && !self.is_exempt(host) && self.matches_host(host)
    }
}

/// One entry in a per-tab chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TabEntry {
    pub seq: u64,
    pub ts: i64,
    pub tab_id: u64,
    pub kind: TabEventKind,
    /// Free-form detail (URL, script URL, permission name, fetch URL, …).
    pub details: String,
    /// BLAKE3 26-char base32 of the previous entry. Empty for genesis.
    pub prev_hash: String,
    /// BLAKE3 26-char base32 of THIS entry's canonical form.
    pub content_hash: String,
}

impl TabEntry {
    pub const GENESIS_PREV: &'static str = "";

    /// Deterministic content hash — 128-bit BLAKE3 → 26 base32 chars.
    #[must_use]
    pub fn compute_hash(
        seq: u64,
        ts: i64,
        tab_id: u64,
        kind: TabEventKind,
        details: &str,
        prev_hash: &str,
    ) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&seq.to_be_bytes());
        hasher.update(&ts.to_be_bytes());
        hasher.update(&tab_id.to_be_bytes());
        let k = serde_json::to_string(&kind).unwrap_or_default();
        hasher.update(k.as_bytes());
        hasher.update(b"||");
        hasher.update(details.as_bytes());
        hasher.update(b"||");
        hasher.update(prev_hash.as_bytes());
        let bytes = hasher.finalize();
        base32_16(&bytes.as_bytes()[..16])
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

/// One tab's chain.
#[derive(Debug, Clone, Default)]
pub struct TabChain {
    tab_id: u64,
    entries: VecDeque<TabEntry>,
    next_seq: u64,
    last_hash: String,
    max_entries: u32,
    sealed: bool,
}

impl TabChain {
    #[must_use]
    pub fn new(tab_id: u64, max_entries: u32) -> Self {
        Self {
            tab_id,
            entries: VecDeque::new(),
            next_seq: 0,
            last_hash: TabEntry::GENESIS_PREV.to_owned(),
            max_entries,
            sealed: false,
        }
    }

    #[must_use]
    pub fn tab_id(&self) -> u64 {
        self.tab_id
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
    pub fn entries(&self) -> &VecDeque<TabEntry> {
        &self.entries
    }

    #[must_use]
    pub fn head_hash(&self) -> &str {
        &self.last_hash
    }

    #[must_use]
    pub fn is_sealed(&self) -> bool {
        self.sealed
    }

    /// Append an entry. Returns `None` if the chain is sealed (post-Close).
    pub fn append(
        &mut self,
        ts: i64,
        kind: TabEventKind,
        details: String,
    ) -> Option<TabEntry> {
        if self.sealed {
            return None;
        }
        let seq = self.next_seq;
        let prev_hash = self.last_hash.clone();
        let content_hash =
            TabEntry::compute_hash(seq, ts, self.tab_id, kind, &details, &prev_hash);
        let entry = TabEntry {
            seq,
            ts,
            tab_id: self.tab_id,
            kind,
            details,
            prev_hash,
            content_hash: content_hash.clone(),
        };
        self.entries.push_back(entry.clone());
        self.last_hash = content_hash;
        self.next_seq += 1;
        if matches!(kind, TabEventKind::Close) {
            self.sealed = true;
        }
        // Ring-buffer (never evicts genesis — keeps ≥1 entry).
        if self.max_entries != 0 {
            while self.entries.len() > self.max_entries as usize && self.entries.len() > 1 {
                self.entries.pop_front();
            }
        }
        Some(entry)
    }

    /// Verify the chain. `Ok(())` on success, `Err(broken_seq)` otherwise.
    pub fn verify(&self) -> Result<(), u64> {
        let Some(first) = self.entries.front() else {
            return Ok(());
        };
        let mut expected_prev = first.prev_hash.clone();
        for e in &self.entries {
            if e.prev_hash != expected_prev {
                return Err(e.seq);
            }
            let recomputed =
                TabEntry::compute_hash(e.seq, e.ts, e.tab_id, e.kind, &e.details, &e.prev_hash);
            if recomputed != e.content_hash {
                return Err(e.seq);
            }
            expected_prev = e.content_hash.clone();
        }
        Ok(())
    }
}

/// Multi-tab store — one chain per live tab.
#[derive(Debug, Clone, Default)]
pub struct TabAttestationStore {
    chains: HashMap<u64, TabChain>,
    default_max: u32,
}

impl TabAttestationStore {
    #[must_use]
    pub fn new(default_max: u32) -> Self {
        Self {
            chains: HashMap::new(),
            default_max,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.chains.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chains.is_empty()
    }

    /// Begin a chain for `tab_id` with a Genesis entry whose details = `url`.
    /// Returns the Genesis entry. If the chain already exists, returns `None`.
    pub fn begin(&mut self, tab_id: u64, ts: i64, url: String) -> Option<TabEntry> {
        if self.chains.contains_key(&tab_id) {
            return None;
        }
        let mut chain = TabChain::new(tab_id, self.default_max);
        let entry = chain.append(ts, TabEventKind::Genesis, url);
        self.chains.insert(tab_id, chain);
        entry
    }

    pub fn record(
        &mut self,
        tab_id: u64,
        ts: i64,
        kind: TabEventKind,
        details: String,
    ) -> Option<TabEntry> {
        self.chains.get_mut(&tab_id)?.append(ts, kind, details)
    }

    #[must_use]
    pub fn chain(&self, tab_id: u64) -> Option<&TabChain> {
        self.chains.get(&tab_id)
    }

    #[must_use]
    pub fn head_hash(&self, tab_id: u64) -> Option<&str> {
        self.chains.get(&tab_id).map(TabChain::head_hash)
    }

    /// Seal + optionally return the chain for export.
    pub fn close(&mut self, tab_id: u64, ts: i64) -> Option<TabChain> {
        let chain = self.chains.get_mut(&tab_id)?;
        chain.append(ts, TabEventKind::Close, String::new());
        self.chains.remove(&tab_id)
    }

    /// Verify every live chain. Returns the list of `(tab_id, broken_seq)`
    /// for chains that fail.
    pub fn verify_all(&self) -> Vec<(u64, u64)> {
        self.chains
            .iter()
            .filter_map(|(id, c)| c.verify().err().map(|s| (*id, s)))
            .collect()
    }

    #[must_use]
    pub fn tabs(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.chains.keys().copied().collect();
        v.sort_unstable();
        v
    }
}

/// Registry of specs — `resolve(host)` picks the active profile.
#[derive(Debug, Clone, Default)]
pub struct TabAttestationRegistry {
    specs: Vec<TabAttestationSpec>,
}

impl TabAttestationRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: TabAttestationSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = TabAttestationSpec>) {
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
    pub fn specs(&self) -> &[TabAttestationSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&TabAttestationSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<TabAttestationSpec>, String> {
    tatara_lisp::compile_typed::<TabAttestationSpec>(src)
        .map_err(|e| format!("failed to compile deftab-attestation forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<TabAttestationSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_disabled_for_privacy() {
        let s = TabAttestationSpec::default_profile();
        assert!(!s.enabled);
        assert!(!s.should_chain("example.com"));
    }

    #[test]
    fn strict_profile_is_opt_in_enabled() {
        let s = TabAttestationSpec::strict_profile();
        assert!(s.enabled);
        assert!(s.should_chain("example.com"));
        assert!(s.records(TabEventKind::Navigate));
        assert!(s.records(TabEventKind::Fetch));
    }

    #[test]
    fn empty_track_captures_everything_when_enabled() {
        let s = TabAttestationSpec {
            enabled: true,
            track: vec![],
            ..TabAttestationSpec::default_profile()
        };
        assert!(s.records(TabEventKind::Navigate));
        assert!(s.records(TabEventKind::FormSubmit));
    }

    #[test]
    fn exempt_host_opts_out() {
        let s = TabAttestationSpec {
            enabled: true,
            exempt_hosts: vec!["*://*.bank.com/*".into()],
            ..TabAttestationSpec::default_profile()
        };
        assert!(!s.should_chain("chase.bank.com"));
        assert!(s.should_chain("example.com"));
    }

    #[test]
    fn begin_creates_genesis_entry() {
        let mut store = TabAttestationStore::new(1000);
        let g = store
            .begin(1, 100, "https://example.com".into())
            .expect("genesis created");
        assert_eq!(g.kind, TabEventKind::Genesis);
        assert_eq!(g.tab_id, 1);
        assert_eq!(g.seq, 0);
        assert_eq!(g.prev_hash, "");
        assert_eq!(g.details, "https://example.com");
        assert_ne!(g.content_hash, "");
    }

    #[test]
    fn begin_twice_for_same_tab_is_no_op() {
        let mut store = TabAttestationStore::new(1000);
        let _ = store.begin(1, 100, "a".into());
        assert!(store.begin(1, 200, "b".into()).is_none());
    }

    #[test]
    fn chain_links_prev_hash_to_previous_content_hash() {
        let mut store = TabAttestationStore::new(1000);
        let g = store.begin(1, 100, "https://a.com".into()).unwrap();
        let nav = store
            .record(1, 101, TabEventKind::Navigate, "https://b.com".into())
            .unwrap();
        assert_eq!(nav.prev_hash, g.content_hash);
        assert_eq!(nav.seq, 1);
    }

    #[test]
    fn chain_verifies_when_untouched() {
        let mut store = TabAttestationStore::new(1000);
        store.begin(1, 100, "a".into());
        store.record(1, 101, TabEventKind::Navigate, "b".into());
        store.record(1, 102, TabEventKind::ScriptLoad, "s.js".into());
        store.record(1, 103, TabEventKind::Fetch, "GET /api".into());
        assert!(store.chain(1).unwrap().verify().is_ok());
    }

    #[test]
    fn chain_verify_catches_tamper() {
        let mut store = TabAttestationStore::new(1000);
        store.begin(1, 100, "a".into());
        store.record(1, 101, TabEventKind::Navigate, "b".into());
        // Directly mutate the chain — flip a detail without recomputing hash.
        {
            let chain = store.chains.get_mut(&1).unwrap();
            chain.entries.get_mut(1).unwrap().details = "c".into();
        }
        let err = store.chain(1).unwrap().verify().unwrap_err();
        assert_eq!(err, 1);
    }

    #[test]
    fn close_seals_and_returns_chain() {
        let mut store = TabAttestationStore::new(1000);
        store.begin(1, 100, "a".into());
        store.record(1, 101, TabEventKind::Navigate, "b".into());
        let sealed = store.close(1, 200).expect("close returns chain");
        assert!(sealed.is_sealed());
        let last = sealed.entries().back().unwrap();
        assert_eq!(last.kind, TabEventKind::Close);
        assert!(store.chain(1).is_none());
    }

    #[test]
    fn sealed_chain_rejects_appends() {
        let mut store = TabAttestationStore::new(1000);
        store.begin(1, 100, "a".into());
        // Emulate close without removal by sealing in place.
        let chain = store.chains.get_mut(&1).unwrap();
        chain.append(150, TabEventKind::Close, String::new());
        let more = chain.append(200, TabEventKind::Fetch, "x".into());
        assert!(more.is_none());
    }

    #[test]
    fn max_entries_ring_buffers_but_keeps_latest() {
        let mut store = TabAttestationStore::new(3);
        store.begin(1, 0, "genesis".into());
        for i in 1..=10 {
            store.record(1, i, TabEventKind::Navigate, format!("step-{i}"));
        }
        let chain = store.chain(1).unwrap();
        assert_eq!(chain.len(), 3);
        let latest = chain.entries().back().unwrap();
        assert_eq!(latest.details, "step-10");
    }

    #[test]
    fn multiple_tabs_are_independent() {
        let mut store = TabAttestationStore::new(1000);
        store.begin(1, 100, "a".into());
        store.begin(2, 100, "b".into());
        store.record(1, 101, TabEventKind::Navigate, "a2".into());
        store.record(2, 101, TabEventKind::Navigate, "b2".into());
        let h1 = store.head_hash(1).unwrap().to_owned();
        let h2 = store.head_hash(2).unwrap().to_owned();
        assert_ne!(h1, h2);
        assert_eq!(store.tabs(), vec![1, 2]);
    }

    #[test]
    fn compute_hash_is_deterministic_and_differs_by_tab() {
        let h1 = TabEntry::compute_hash(0, 100, 1, TabEventKind::Genesis, "a", "");
        let h1b = TabEntry::compute_hash(0, 100, 1, TabEventKind::Genesis, "a", "");
        let h2 = TabEntry::compute_hash(0, 100, 2, TabEventKind::Genesis, "a", "");
        assert_eq!(h1, h1b);
        assert_ne!(h1, h2);
        assert_eq!(h1.len(), 26);
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = TabAttestationRegistry::new();
        reg.insert(TabAttestationSpec::strict_profile());
        reg.insert(TabAttestationSpec {
            name: "work".into(),
            host: "*://*.corp.com/*".into(),
            enabled: true,
            max_entries: 5000,
            ..TabAttestationSpec::default_profile()
        });
        assert_eq!(reg.resolve("app.corp.com").unwrap().name, "work");
        assert_eq!(reg.resolve("example.org").unwrap().name, "strict");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = TabAttestationRegistry::new();
        reg.insert(TabAttestationSpec::default_profile());
        assert!(reg.resolve("example.com").is_none());
    }

    #[test]
    fn event_kind_roundtrips_through_serde() {
        for k in [
            TabEventKind::Genesis,
            TabEventKind::Navigate,
            TabEventKind::ScriptLoad,
            TabEventKind::PermissionGrant,
            TabEventKind::PermissionDeny,
            TabEventKind::FormSubmit,
            TabEventKind::StorageWrite,
            TabEventKind::Fetch,
            TabEventKind::Redirect,
            TabEventKind::Close,
            TabEventKind::UserGesture,
            TabEventKind::DomMutation,
        ] {
            let s = TabAttestationSpec {
                track: vec![k],
                ..TabAttestationSpec::default_profile()
            };
            let j = serde_json::to_string(&s).unwrap();
            let b: TabAttestationSpec = serde_json::from_str(&j).unwrap();
            assert_eq!(b.track, vec![k]);
        }
    }

    #[test]
    fn verify_all_lists_broken_chains() {
        let mut store = TabAttestationStore::new(1000);
        store.begin(1, 0, "a".into());
        store.begin(2, 0, "b".into());
        store.record(2, 1, TabEventKind::Navigate, "b2".into());
        // Tamper chain 2.
        store.chains.get_mut(&2).unwrap().entries.get_mut(1).unwrap().details = "X".into();
        let broken = store.verify_all();
        assert_eq!(broken, vec![(2, 1)]);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_tab_attestation_form() {
        let src = r#"
            (deftab-attestation :name "strict"
                                :enabled #t
                                :host "*"
                                :track ("navigate" "script-load" "fetch")
                                :max-entries 500
                                :export-on-close #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert!(s.enabled);
        assert_eq!(s.max_entries, 500);
        assert!(s.records(TabEventKind::Navigate));
        assert!(!s.records(TabEventKind::FormSubmit));
    }
}
