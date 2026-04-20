//! `(defservice-worker)` — persistent-worker + fetch-interceptor DSL.
//!
//! Builds on [`crate::js_runtime`] (foundation lift J1): once
//! authored, the worker runs in a long-lived JsRuntime with a
//! declared lifecycle (install / activate / fetch / message) and
//! declared caching strategy per route.
//!
//! Absorbs Chrome/Firefox/Safari Service Worker API + Workbox route
//! patterns. Authored as one Lisp form instead of a JS file + a
//! `navigator.serviceWorker.register()` call + a manifest scope.
//!
//! ```lisp
//! (defservice-worker :name       "docs-offline"
//!                    :host       "*://*.docs.example.com/*"
//!                    :scope      "/"
//!                    :runtime    "sandbox"
//!                    :lifecycle  (install activate fetch message sync)
//!                    :skip-waiting #t
//!                    :client-claim #t
//!                    :routes ((("/api/*")       :network-first    :timeout 4)
//!                             (("/static/*")    :cache-first      :max-age 86400)
//!                             (("/docs/*")      :stale-while-revalidate)))
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Lifecycle event the worker should subscribe to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum LifecycleEvent {
    Install,
    Activate,
    Fetch,
    Message,
    Push,
    Sync,
    PeriodicSync,
    NotificationClick,
}

/// Caching strategy per route (Workbox-style).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CacheStrategy {
    /// Serve from cache; fall back to network only on cache miss.
    CacheFirst,
    /// Hit network first; fall back to cache on network failure.
    NetworkFirst,
    /// Serve from cache immediately; revalidate in the background.
    StaleWhileRevalidate,
    /// Bypass cache, always hit network.
    NetworkOnly,
    /// Bypass network, always serve from cache.
    CacheOnly,
}

impl Default for CacheStrategy {
    fn default() -> Self {
        Self::StaleWhileRevalidate
    }
}

/// One route entry — pattern globs + strategy + limits.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkerRoute {
    /// Path-pattern globs (Workbox syntax `/api/*`, `/static/**`).
    pub patterns: Vec<String>,
    #[serde(default)]
    pub strategy: CacheStrategy,
    /// Network timeout in seconds before falling back to cache
    /// (only applies to `NetworkFirst`).
    #[serde(default)]
    pub timeout_seconds: u32,
    /// Max age in seconds a cached entry is valid (0 = forever).
    #[serde(default)]
    pub max_age_seconds: u64,
    /// Max cache entries for this route. 0 = unlimited.
    #[serde(default)]
    pub max_entries: u32,
    /// Custom cache name — defaults to the worker name if empty.
    #[serde(default)]
    pub cache_name: Option<String>,
}

impl WorkerRoute {
    /// True when `path` matches any of the configured patterns.
    #[must_use]
    pub fn matches(&self, path: &str) -> bool {
        self.patterns.iter().any(|p| glob_match(p, path))
    }
}

/// Match a Workbox-style path glob. `*` matches any path segment,
/// `**` matches across segments, literal otherwise.
fn glob_match(pattern: &str, path: &str) -> bool {
    if pattern == "**" || pattern == "/*" || pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return path.starts_with(prefix);
    }
    if let Some(prefix) = pattern.strip_suffix("/*") {
        let rest = path.strip_prefix(prefix).map(|r| r.trim_start_matches('/'));
        return rest.is_some_and(|r| !r.contains('/'));
    }
    if let Some(suffix) = pattern.strip_prefix("*") {
        return path.ends_with(suffix);
    }
    pattern == path
}

/// Service-worker profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defservice-worker"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceWorkerSpec {
    pub name: String,
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    /// Scope path (SW-API `scope`) — worker only intercepts requests
    /// under this path.
    #[serde(default = "default_scope")]
    pub scope: String,
    /// Name of a `(defjs-runtime)` spec — picks up fuel/memory/
    /// capabilities from there. Empty = use system default.
    #[serde(default)]
    pub runtime: String,
    /// Lifecycle events the worker handles.
    #[serde(default = "default_lifecycle")]
    pub lifecycle: Vec<LifecycleEvent>,
    /// Skip the `waiting` phase on update (`self.skipWaiting()`).
    #[serde(default)]
    pub skip_waiting: bool,
    /// Claim existing clients immediately on activate.
    #[serde(default)]
    pub client_claim: bool,
    /// Declared routes.
    #[serde(default)]
    pub routes: Vec<WorkerRoute>,
    /// Optional JS source that the worker runs at install. Empty
    /// means the declared lifecycle + routes are all that's needed.
    #[serde(default)]
    pub source: Option<String>,
    /// Maximum total cache storage in MB (0 = unlimited).
    #[serde(default = "default_max_cache_mb")]
    pub max_cache_mb: u32,
    /// Fallback HTML page served on offline + no-match (e.g. "/offline.html").
    #[serde(default)]
    pub offline_fallback: Option<String>,
    /// Seconds between periodic-sync wake-ups (0 = disabled).
    #[serde(default)]
    pub periodic_sync_seconds: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_scope() -> String {
    "/".into()
}
fn default_lifecycle() -> Vec<LifecycleEvent> {
    vec![
        LifecycleEvent::Install,
        LifecycleEvent::Activate,
        LifecycleEvent::Fetch,
    ]
}
fn default_max_cache_mb() -> u32 {
    64
}
fn default_enabled() -> bool {
    true
}

impl ServiceWorkerSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            scope: "/".into(),
            runtime: String::new(),
            lifecycle: default_lifecycle(),
            skip_waiting: false,
            client_claim: false,
            routes: vec![WorkerRoute {
                patterns: vec!["/*".into()],
                strategy: CacheStrategy::StaleWhileRevalidate,
                timeout_seconds: 0,
                max_age_seconds: 0,
                max_entries: 0,
                cache_name: None,
            }],
            source: None,
            max_cache_mb: 64,
            offline_fallback: None,
            periodic_sync_seconds: 0,
            enabled: true,
            description: Some(
                "Default service worker — install/activate/fetch with SWR on /*.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    #[must_use]
    pub fn handles(&self, event: LifecycleEvent) -> bool {
        self.lifecycle.contains(&event)
    }

    /// Does the worker's scope cover `path`?
    #[must_use]
    pub fn in_scope(&self, path: &str) -> bool {
        self.scope == "/" || path.starts_with(&self.scope)
    }

    /// Resolve the first matching route for `path`. Route order wins
    /// — authors can express priority by ordering the `:routes` list.
    #[must_use]
    pub fn route_for(&self, path: &str) -> Option<&WorkerRoute> {
        self.routes.iter().find(|r| r.matches(path))
    }

    #[must_use]
    pub fn cache_name(&self, route: &WorkerRoute) -> String {
        route
            .cache_name
            .clone()
            .unwrap_or_else(|| self.name.clone())
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct ServiceWorkerRegistry {
    specs: Vec<ServiceWorkerSpec>,
}

impl ServiceWorkerRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: ServiceWorkerSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ServiceWorkerSpec>) {
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
    pub fn specs(&self) -> &[ServiceWorkerSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&ServiceWorkerSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ServiceWorkerSpec>, String> {
    tatara_lisp::compile_typed::<ServiceWorkerSpec>(src)
        .map_err(|e| format!("failed to compile defservice-worker forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ServiceWorkerSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_handles_install_activate_fetch() {
        let s = ServiceWorkerSpec::default_profile();
        assert!(s.handles(LifecycleEvent::Install));
        assert!(s.handles(LifecycleEvent::Activate));
        assert!(s.handles(LifecycleEvent::Fetch));
        assert!(!s.handles(LifecycleEvent::Push));
    }

    #[test]
    fn glob_matches_segment_star() {
        assert!(glob_match("/api/*", "/api/foo"));
        assert!(!glob_match("/api/*", "/api/foo/bar"));
        assert!(glob_match("/api/**", "/api/foo/bar/baz"));
    }

    #[test]
    fn glob_matches_wildcard() {
        assert!(glob_match("**", "/anything/at/all"));
        assert!(glob_match("/*", "/x"));
    }

    #[test]
    fn glob_matches_extension_suffix() {
        assert!(glob_match("*.js", "app.js"));
        assert!(!glob_match("*.js", "app.css"));
    }

    #[test]
    fn route_matches_first_pattern() {
        let r = WorkerRoute {
            patterns: vec!["/api/*".into(), "/auth/**".into()],
            strategy: CacheStrategy::NetworkFirst,
            timeout_seconds: 0,
            max_age_seconds: 0,
            max_entries: 0,
            cache_name: None,
        };
        assert!(r.matches("/api/users"));
        assert!(r.matches("/auth/login/twofactor"));
        assert!(!r.matches("/static/main.css"));
    }

    #[test]
    fn route_for_respects_author_order() {
        let s = ServiceWorkerSpec {
            routes: vec![
                WorkerRoute {
                    patterns: vec!["/api/*".into()],
                    strategy: CacheStrategy::NetworkFirst,
                    timeout_seconds: 5,
                    max_age_seconds: 0,
                    max_entries: 0,
                    cache_name: None,
                },
                WorkerRoute {
                    patterns: vec!["/api/users".into()],
                    strategy: CacheStrategy::CacheFirst,
                    timeout_seconds: 0,
                    max_age_seconds: 60,
                    max_entries: 0,
                    cache_name: None,
                },
            ],
            ..ServiceWorkerSpec::default_profile()
        };
        let r = s.route_for("/api/users").unwrap();
        // First matching wins — NetworkFirst, NOT the more-specific CacheFirst.
        assert_eq!(r.strategy, CacheStrategy::NetworkFirst);
    }

    #[test]
    fn in_scope_honors_path_prefix() {
        let s = ServiceWorkerSpec {
            scope: "/docs/".into(),
            ..ServiceWorkerSpec::default_profile()
        };
        assert!(s.in_scope("/docs/intro"));
        assert!(!s.in_scope("/blog/post"));
    }

    #[test]
    fn cache_name_falls_back_to_worker_name() {
        let s = ServiceWorkerSpec::default_profile();
        let r = &s.routes[0];
        assert_eq!(s.cache_name(r), "default");
        let named = WorkerRoute {
            cache_name: Some("shared-runtime".into()),
            ..r.clone()
        };
        assert_eq!(s.cache_name(&named), "shared-runtime");
    }

    #[test]
    fn matches_host_glob() {
        let s = ServiceWorkerSpec {
            host: "*://*.example.com/*".into(),
            ..ServiceWorkerSpec::default_profile()
        };
        assert!(s.matches_host("www.example.com"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn strategy_roundtrips_through_serde() {
        for st in [
            CacheStrategy::CacheFirst,
            CacheStrategy::NetworkFirst,
            CacheStrategy::StaleWhileRevalidate,
            CacheStrategy::NetworkOnly,
            CacheStrategy::CacheOnly,
        ] {
            let r = WorkerRoute {
                patterns: vec!["/*".into()],
                strategy: st,
                timeout_seconds: 0,
                max_age_seconds: 0,
                max_entries: 0,
                cache_name: None,
            };
            let json = serde_json::to_string(&r).unwrap();
            let back: WorkerRoute = serde_json::from_str(&json).unwrap();
            assert_eq!(back.strategy, st);
        }
    }

    #[test]
    fn lifecycle_roundtrips_through_serde() {
        for ev in [
            LifecycleEvent::Install,
            LifecycleEvent::Activate,
            LifecycleEvent::Fetch,
            LifecycleEvent::Message,
            LifecycleEvent::Push,
            LifecycleEvent::Sync,
            LifecycleEvent::PeriodicSync,
            LifecycleEvent::NotificationClick,
        ] {
            let s = ServiceWorkerSpec {
                lifecycle: vec![ev],
                ..ServiceWorkerSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: ServiceWorkerSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.lifecycle, vec![ev]);
        }
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = ServiceWorkerRegistry::new();
        reg.insert(ServiceWorkerSpec::default_profile());
        reg.insert(ServiceWorkerSpec {
            name: "docs".into(),
            host: "*://*.docs.example.com/*".into(),
            skip_waiting: true,
            ..ServiceWorkerSpec::default_profile()
        });
        let docs = reg.resolve("www.docs.example.com").unwrap();
        assert_eq!(docs.name, "docs");
        assert!(docs.skip_waiting);
        let other = reg.resolve("example.org").unwrap();
        assert_eq!(other.name, "default");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = ServiceWorkerRegistry::new();
        reg.insert(ServiceWorkerSpec {
            enabled: false,
            ..ServiceWorkerSpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_service_worker_form() {
        let src = r#"
            (defservice-worker :name "docs-offline"
                               :host "*://*.docs.example.com/*"
                               :scope "/"
                               :runtime "sandbox"
                               :skip-waiting #t
                               :client-claim #t
                               :max-cache-mb 128
                               :offline-fallback "/offline.html")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "docs-offline");
        assert!(s.skip_waiting);
        assert!(s.client_claim);
        assert_eq!(s.max_cache_mb, 128);
        assert_eq!(s.offline_fallback.as_deref(), Some("/offline.html"));
    }
}
