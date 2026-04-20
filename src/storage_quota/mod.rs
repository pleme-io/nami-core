//! `(defstorage-quota)` — per-origin storage caps + eviction.
//!
//! Absorbs Chrome Storage API quota (navigator.storage.estimate,
//! navigator.storage.persist), Firefox storage-pressure eviction,
//! Safari ITP 7-day storage expiry. One Lisp form declares how much
//! each storage surface gets per origin, how eviction runs, and
//! whether persistent-storage grants are permitted.
//!
//! ```lisp
//! (defstorage-quota :name          "strict"
//!                   :host          "*"
//!                   :total-bytes   536870912
//!                   :indexeddb     268435456
//!                   :cache-storage 134217728
//!                   :local-storage 5242880
//!                   :opfs          67108864
//!                   :eviction      :lru
//!                   :persistent    :prompt
//!                   :allow-persist-hosts ("*://*.docs.example.com/*"))
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// How the browser reclaims space when over-budget.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EvictionStrategy {
    /// Least-recently-used origin first.
    #[default]
    Lru,
    /// Largest-origin first.
    Largest,
    /// Oldest-data first (by created-at).
    Age,
    /// Random — useful for testing.
    Random,
    /// Never evict — prefer failing new writes with QuotaExceededError.
    None,
}

/// Persistent-storage grant policy (navigator.storage.persist()).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PersistencePolicy {
    /// Always deny.
    Deny,
    /// Always grant (demo/dev).
    Allow,
    /// Prompt the user once per origin.
    #[default]
    Prompt,
    /// Grant automatically to origins the user has interacted with
    /// more than N times (Chrome's current heuristic).
    HeuristicEngagement,
    /// Honor an explicit allow-list (see `allow_persist_hosts`).
    AllowList,
}

/// Per-surface byte cap. 0 = unlimited (subject only to `total_bytes`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceCaps {
    pub indexeddb: u64,
    pub cache_storage: u64,
    pub local_storage: u64,
    pub session_storage: u64,
    pub opfs: u64,
    pub file_system: u64,
    pub cookies: u64,
}

impl Default for SurfaceCaps {
    fn default() -> Self {
        Self {
            indexeddb: 256 * 1024 * 1024,
            cache_storage: 128 * 1024 * 1024,
            local_storage: 5 * 1024 * 1024,
            session_storage: 5 * 1024 * 1024,
            opfs: 64 * 1024 * 1024,
            file_system: 0,
            cookies: 128 * 1024,
        }
    }
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defstorage-quota"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StorageQuotaSpec {
    pub name: String,
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    /// Top-level bytes per origin. 0 = unlimited (only surface caps
    /// apply). The caller enforces `min(surface_cap, remaining_total)`.
    #[serde(default = "default_total_bytes")]
    pub total_bytes: u64,
    /// Per-surface caps — individual IndexedDB/CacheStorage/etc.
    #[serde(default)]
    pub indexeddb: Option<u64>,
    #[serde(default)]
    pub cache_storage: Option<u64>,
    #[serde(default)]
    pub local_storage: Option<u64>,
    #[serde(default)]
    pub session_storage: Option<u64>,
    #[serde(default)]
    pub opfs: Option<u64>,
    #[serde(default)]
    pub file_system: Option<u64>,
    #[serde(default)]
    pub cookies: Option<u64>,
    #[serde(default)]
    pub eviction: EvictionStrategy,
    /// Global system-level high-water mark (MB). When system
    /// storage goes over, eviction kicks in regardless of
    /// per-origin caps.
    #[serde(default)]
    pub system_high_water_mb: u32,
    #[serde(default)]
    pub persistent: PersistencePolicy,
    /// Hosts that always get persist() grant (used when
    /// `persistent = allow-list`).
    #[serde(default)]
    pub allow_persist_hosts: Vec<String>,
    /// Hosts that never get persist() grant regardless of policy.
    #[serde(default)]
    pub deny_persist_hosts: Vec<String>,
    /// Expose the actual backend-reported quota/usage in the
    /// Storage API (`estimate()`). Turning this off makes the
    /// estimate fuzzier — helps anti-fingerprint.
    #[serde(default = "default_honest_estimate")]
    pub honest_estimate: bool,
    /// Drop unused origins after N days with no user visit.
    #[serde(default = "default_expiry_days")]
    pub unused_origin_expiry_days: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_total_bytes() -> u64 {
    1024 * 1024 * 1024
}
fn default_honest_estimate() -> bool {
    true
}
fn default_expiry_days() -> u32 {
    30
}
fn default_enabled() -> bool {
    true
}

/// Per-surface kind.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum StorageSurface {
    IndexedDb,
    CacheStorage,
    LocalStorage,
    SessionStorage,
    Opfs,
    FileSystem,
    Cookies,
}

impl StorageQuotaSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            total_bytes: 1024 * 1024 * 1024,
            indexeddb: None,
            cache_storage: None,
            local_storage: None,
            session_storage: None,
            opfs: None,
            file_system: None,
            cookies: None,
            eviction: EvictionStrategy::Lru,
            system_high_water_mb: 0,
            persistent: PersistencePolicy::Prompt,
            allow_persist_hosts: vec![],
            deny_persist_hosts: vec![],
            honest_estimate: true,
            unused_origin_expiry_days: 30,
            enabled: true,
            description: Some(
                "Default quota — 1 GB total per origin, LRU eviction, prompt on persist().".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    #[must_use]
    pub fn surface_cap(&self, s: StorageSurface) -> u64 {
        let default = SurfaceCaps::default();
        match s {
            StorageSurface::IndexedDb => self.indexeddb.unwrap_or(default.indexeddb),
            StorageSurface::CacheStorage => self.cache_storage.unwrap_or(default.cache_storage),
            StorageSurface::LocalStorage => self.local_storage.unwrap_or(default.local_storage),
            StorageSurface::SessionStorage => {
                self.session_storage.unwrap_or(default.session_storage)
            }
            StorageSurface::Opfs => self.opfs.unwrap_or(default.opfs),
            StorageSurface::FileSystem => self.file_system.unwrap_or(default.file_system),
            StorageSurface::Cookies => self.cookies.unwrap_or(default.cookies),
        }
    }

    /// Does a write of `bytes` to `surface` fit?
    /// `currently_used` is the origin's current total across surfaces.
    #[must_use]
    pub fn admits_write(
        &self,
        surface: StorageSurface,
        currently_used_on_surface: u64,
        currently_used_total: u64,
        bytes: u64,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        let surface_cap = self.surface_cap(surface);
        if surface_cap != 0 && currently_used_on_surface.saturating_add(bytes) > surface_cap {
            return false;
        }
        if self.total_bytes != 0 && currently_used_total.saturating_add(bytes) > self.total_bytes {
            return false;
        }
        true
    }

    /// Should `host` receive a persistent-storage grant?
    /// `interaction_count`: for heuristic-engagement policy.
    #[must_use]
    pub fn admits_persist(&self, host: &str, interaction_count: u32) -> bool {
        if self.deny_persist_hosts.iter().any(|pat| crate::extension::glob_match_host(pat, host)) {
            return false;
        }
        match self.persistent {
            PersistencePolicy::Deny => false,
            PersistencePolicy::Allow => true,
            PersistencePolicy::Prompt => false, // requires user action; caller honors prompt
            PersistencePolicy::HeuristicEngagement => interaction_count >= 3,
            PersistencePolicy::AllowList => self
                .allow_persist_hosts
                .iter()
                .any(|pat| crate::extension::glob_match_host(pat, host)),
        }
    }

    /// Is an origin stale (has had no visit for longer than
    /// unused_origin_expiry_days)?
    #[must_use]
    pub fn is_stale(&self, last_visit_days_ago: u32) -> bool {
        self.unused_origin_expiry_days != 0
            && last_visit_days_ago > self.unused_origin_expiry_days
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct StorageQuotaRegistry {
    specs: Vec<StorageQuotaSpec>,
}

impl StorageQuotaRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: StorageQuotaSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = StorageQuotaSpec>) {
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
    pub fn specs(&self) -> &[StorageQuotaSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&StorageQuotaSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<StorageQuotaSpec>, String> {
    tatara_lisp::compile_typed::<StorageQuotaSpec>(src)
        .map_err(|e| format!("failed to compile defstorage-quota forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<StorageQuotaSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_1gb_lru_prompt() {
        let s = StorageQuotaSpec::default_profile();
        assert_eq!(s.total_bytes, 1024 * 1024 * 1024);
        assert_eq!(s.eviction, EvictionStrategy::Lru);
        assert_eq!(s.persistent, PersistencePolicy::Prompt);
    }

    #[test]
    fn surface_cap_falls_back_to_defaults() {
        let s = StorageQuotaSpec::default_profile();
        let defaults = SurfaceCaps::default();
        assert_eq!(s.surface_cap(StorageSurface::IndexedDb), defaults.indexeddb);
        assert_eq!(s.surface_cap(StorageSurface::LocalStorage), defaults.local_storage);
    }

    #[test]
    fn surface_cap_honors_per_field_overrides() {
        let s = StorageQuotaSpec {
            indexeddb: Some(99),
            ..StorageQuotaSpec::default_profile()
        };
        assert_eq!(s.surface_cap(StorageSurface::IndexedDb), 99);
    }

    #[test]
    fn admits_write_respects_surface_and_total_caps() {
        let s = StorageQuotaSpec {
            total_bytes: 1024,
            indexeddb: Some(500),
            ..StorageQuotaSpec::default_profile()
        };
        assert!(s.admits_write(StorageSurface::IndexedDb, 0, 0, 200));
        // Exceeds surface cap.
        assert!(!s.admits_write(StorageSurface::IndexedDb, 400, 400, 200));
        // Exceeds total cap.
        assert!(!s.admits_write(StorageSurface::IndexedDb, 0, 900, 200));
    }

    #[test]
    fn admits_write_zero_caps_mean_unlimited() {
        let s = StorageQuotaSpec {
            total_bytes: 0,
            indexeddb: Some(0),
            ..StorageQuotaSpec::default_profile()
        };
        assert!(s.admits_write(
            StorageSurface::IndexedDb,
            u64::MAX / 2,
            u64::MAX / 2,
            u64::MAX / 4,
        ));
    }

    #[test]
    fn admits_write_disabled_profile_always_rejects() {
        let s = StorageQuotaSpec {
            enabled: false,
            ..StorageQuotaSpec::default_profile()
        };
        assert!(!s.admits_write(StorageSurface::LocalStorage, 0, 0, 1));
    }

    #[test]
    fn admits_persist_deny_rejects_all() {
        let s = StorageQuotaSpec {
            persistent: PersistencePolicy::Deny,
            ..StorageQuotaSpec::default_profile()
        };
        assert!(!s.admits_persist("example.com", 99));
    }

    #[test]
    fn admits_persist_allow_grants_all() {
        let s = StorageQuotaSpec {
            persistent: PersistencePolicy::Allow,
            ..StorageQuotaSpec::default_profile()
        };
        assert!(s.admits_persist("example.com", 0));
    }

    #[test]
    fn admits_persist_prompt_returns_false_pending_user() {
        let s = StorageQuotaSpec::default_profile();
        assert!(!s.admits_persist("example.com", 99));
    }

    #[test]
    fn admits_persist_heuristic_engagement_gates_on_count() {
        let s = StorageQuotaSpec {
            persistent: PersistencePolicy::HeuristicEngagement,
            ..StorageQuotaSpec::default_profile()
        };
        assert!(!s.admits_persist("ex.com", 2));
        assert!(s.admits_persist("ex.com", 3));
        assert!(s.admits_persist("ex.com", 10));
    }

    #[test]
    fn admits_persist_allow_list_matches_glob() {
        let s = StorageQuotaSpec {
            persistent: PersistencePolicy::AllowList,
            allow_persist_hosts: vec!["*://*.docs.com/*".into()],
            ..StorageQuotaSpec::default_profile()
        };
        assert!(s.admits_persist("www.docs.com", 0));
        assert!(!s.admits_persist("ads.com", 0));
    }

    #[test]
    fn admits_persist_deny_list_overrides_allow_policy() {
        let s = StorageQuotaSpec {
            persistent: PersistencePolicy::Allow,
            deny_persist_hosts: vec!["*://*.bank.com/*".into()],
            ..StorageQuotaSpec::default_profile()
        };
        assert!(!s.admits_persist("my.bank.com", 100));
        assert!(s.admits_persist("ex.com", 100));
    }

    #[test]
    fn is_stale_threshold() {
        let s = StorageQuotaSpec {
            unused_origin_expiry_days: 30,
            ..StorageQuotaSpec::default_profile()
        };
        assert!(!s.is_stale(30));
        assert!(s.is_stale(31));

        let none = StorageQuotaSpec {
            unused_origin_expiry_days: 0,
            ..StorageQuotaSpec::default_profile()
        };
        assert!(!none.is_stale(10_000));
    }

    #[test]
    fn eviction_roundtrips_through_serde() {
        for e in [
            EvictionStrategy::Lru,
            EvictionStrategy::Largest,
            EvictionStrategy::Age,
            EvictionStrategy::Random,
            EvictionStrategy::None,
        ] {
            let s = StorageQuotaSpec {
                eviction: e,
                ..StorageQuotaSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: StorageQuotaSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.eviction, e);
        }
    }

    #[test]
    fn persistence_policy_roundtrips_through_serde() {
        for p in [
            PersistencePolicy::Deny,
            PersistencePolicy::Allow,
            PersistencePolicy::Prompt,
            PersistencePolicy::HeuristicEngagement,
            PersistencePolicy::AllowList,
        ] {
            let s = StorageQuotaSpec {
                persistent: p,
                ..StorageQuotaSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: StorageQuotaSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.persistent, p);
        }
    }

    #[test]
    fn surface_enum_roundtrips_through_serde() {
        for k in [
            StorageSurface::IndexedDb,
            StorageSurface::CacheStorage,
            StorageSurface::LocalStorage,
            StorageSurface::SessionStorage,
            StorageSurface::Opfs,
            StorageSurface::FileSystem,
            StorageSurface::Cookies,
        ] {
            let json = serde_json::to_string(&k).unwrap();
            let back: StorageSurface = serde_json::from_str(&json).unwrap();
            assert_eq!(back, k);
        }
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = StorageQuotaRegistry::new();
        reg.insert(StorageQuotaSpec::default_profile());
        reg.insert(StorageQuotaSpec {
            name: "docs".into(),
            host: "*://*.docs.com/*".into(),
            total_bytes: 2 * 1024 * 1024 * 1024,
            ..StorageQuotaSpec::default_profile()
        });
        assert_eq!(
            reg.resolve("www.docs.com").unwrap().total_bytes,
            2 * 1024 * 1024 * 1024
        );
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_storage_quota_form() {
        let src = r#"
            (defstorage-quota :name "strict"
                              :host "*"
                              :total-bytes 536870912
                              :indexeddb 268435456
                              :cache-storage 134217728
                              :local-storage 5242880
                              :eviction "lru"
                              :persistent "allow-list"
                              :allow-persist-hosts ("*://*.docs.example.com/*")
                              :honest-estimate #t
                              :unused-origin-expiry-days 30)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.total_bytes, 536_870_912);
        assert_eq!(s.persistent, PersistencePolicy::AllowList);
        assert_eq!(s.allow_persist_hosts.len(), 1);
    }
}
