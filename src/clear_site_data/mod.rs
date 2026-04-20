//! `(defclear-site-data)` — Clear-Site-Data header + triggered clears.
//!
//! Absorbs the `Clear-Site-Data` HTTP response header (cache,
//! cookies, storage, executionContexts, clientHints, prefetchCache)
//! + Chrome/Firefox/Safari "clear site data" UI + Brave "Clear
//! Browsing Data" + Safari ITP auto-clear. Each profile declares
//! which surfaces to clear, when (on-close / periodic /
//! trigger-event), and for which hosts.
//!
//! ```lisp
//! (defclear-site-data :name         "periodic-trim"
//!                     :host         "*"
//!                     :surfaces     (cache cookies storage execution-contexts)
//!                     :trigger      :periodic
//!                     :interval-hours 24
//!                     :on-navigate-away #f
//!                     :on-idle-minutes 60
//!                     :exempt-hosts ("*://*.auth.example.com/*"))
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Clearable surfaces — union of Clear-Site-Data directives + some
/// Arc/Safari additions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Surface {
    /// HTTP cache + image cache.
    Cache,
    /// Cookies (including partitioned + SameSite).
    Cookies,
    /// All origin storage: IndexedDB, CacheStorage, LocalStorage,
    /// SessionStorage, OPFS.
    Storage,
    /// Tear down document + worker execution contexts.
    ExecutionContexts,
    /// Client hints opt-out / reset.
    ClientHints,
    /// Prefetch cache (Speculation Rules).
    PrefetchCache,
    /// Service worker registration + waiting workers.
    ServiceWorkers,
    /// BroadcastChannel + MessageChannel state.
    Channels,
    /// Clear permissions grants (camera, mic, …).
    Permissions,
    /// Clear origin-scoped (defsync) replicas.
    SyncData,
    /// Autofill / passwords / passkeys (scoped: only the local
    /// origin-level cache, not vault).
    AutofillLocal,
    /// The wildcard directive — future-proof; covers everything.
    All,
}

/// When to run the clear.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Trigger {
    /// Only when the server sends `Clear-Site-Data`.
    #[default]
    HeaderDriven,
    /// On browser close.
    OnClose,
    /// On tab close.
    OnTabClose,
    /// On navigate-away to a different origin.
    OnNavigateAway,
    /// On declared `on_idle_minutes` idle threshold.
    OnIdle,
    /// On a fixed interval (`interval_hours`).
    Periodic,
    /// Only when the user explicitly requests.
    Manual,
    /// When (defidentity) persona switches.
    OnIdentitySwitch,
}

/// Scope of the clear.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// Only the origin that triggered the clear.
    #[default]
    ThisOrigin,
    /// All origins on the same eTLD+1 (registrable domain).
    RegistrableDomain,
    /// All origins.
    AllOrigins,
    /// Origin + all its third-party partitioned jars.
    ThisOriginAndPartitions,
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defclear-site-data"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClearSiteDataSpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_surfaces")]
    pub surfaces: Vec<Surface>,
    #[serde(default)]
    pub trigger: Trigger,
    /// For `Trigger::Periodic` — hours between runs. 0 = never.
    #[serde(default)]
    pub interval_hours: u32,
    /// For `Trigger::OnIdle` — idle minutes threshold. 0 = never.
    #[serde(default)]
    pub on_idle_minutes: u32,
    #[serde(default)]
    pub scope: Scope,
    /// Hosts that are exempt from this profile entirely.
    #[serde(default)]
    pub exempt_hosts: Vec<String>,
    /// Surfaces to NEVER clear for this host profile (e.g. always
    /// keep cookies even if triggered).
    #[serde(default)]
    pub always_preserve: Vec<Surface>,
    /// Force the trigger's clear to include executionContexts — ends
    /// up being the only way to honor Clear-Site-Data "*" strictly.
    #[serde(default)]
    pub force_execution_contexts: bool,
    /// Grace period in seconds between trigger firing and actual
    /// clear (useful for showing a warning toast).
    #[serde(default)]
    pub grace_period_seconds: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_surfaces() -> Vec<Surface> {
    vec![Surface::Cache, Surface::Cookies, Surface::Storage]
}
fn default_enabled() -> bool {
    true
}

/// Runtime snapshot that the trigger evaluator consumes.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClearTriggerInput {
    pub is_browser_close: bool,
    pub is_tab_close: bool,
    pub navigate_away_to_different_origin: bool,
    pub idle_minutes: u32,
    pub periodic_tick: bool,
    pub manual_request: bool,
    pub identity_switch: bool,
    pub received_clear_site_data_header: bool,
}

impl ClearSiteDataSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            surfaces: default_surfaces(),
            trigger: Trigger::HeaderDriven,
            interval_hours: 0,
            on_idle_minutes: 0,
            scope: Scope::ThisOrigin,
            exempt_hosts: vec![],
            always_preserve: vec![],
            force_execution_contexts: false,
            grace_period_seconds: 0,
            enabled: true,
            description: Some(
                "Default clear — header-driven, this-origin scope, standard Cache+Cookies+Storage surfaces.".into(),
            ),
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
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    #[must_use]
    pub fn preserves(&self, s: Surface) -> bool {
        self.always_preserve.contains(&s)
    }

    /// Does the trigger fire given this input?
    #[must_use]
    pub fn should_fire(&self, input: &ClearTriggerInput) -> bool {
        if !self.enabled {
            return false;
        }
        match self.trigger {
            Trigger::HeaderDriven => input.received_clear_site_data_header,
            Trigger::OnClose => input.is_browser_close,
            Trigger::OnTabClose => input.is_tab_close,
            Trigger::OnNavigateAway => input.navigate_away_to_different_origin,
            Trigger::OnIdle => {
                self.on_idle_minutes != 0 && input.idle_minutes >= self.on_idle_minutes
            }
            Trigger::Periodic => self.interval_hours != 0 && input.periodic_tick,
            Trigger::Manual => input.manual_request,
            Trigger::OnIdentitySwitch => input.identity_switch,
        }
    }

    /// Effective surface list for a clear — honors `always_preserve`
    /// filter and injects `ExecutionContexts` when forced.
    #[must_use]
    pub fn effective_surfaces(&self) -> Vec<Surface> {
        let mut out: Vec<Surface> = self
            .surfaces
            .iter()
            .copied()
            .filter(|s| !self.preserves(*s))
            .collect();
        if self.force_execution_contexts && !out.contains(&Surface::ExecutionContexts) {
            out.push(Surface::ExecutionContexts);
        }
        out
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct ClearSiteDataRegistry {
    specs: Vec<ClearSiteDataSpec>,
}

impl ClearSiteDataRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: ClearSiteDataSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ClearSiteDataSpec>) {
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
    pub fn specs(&self) -> &[ClearSiteDataSpec] {
        &self.specs
    }

    /// All enabled profiles applicable to `host` (not exempt).
    #[must_use]
    pub fn applicable_to<'a>(&'a self, host: &str) -> Vec<&'a ClearSiteDataSpec> {
        self.specs
            .iter()
            .filter(|s| s.enabled && s.matches_host(host) && !s.is_exempt(host))
            .collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ClearSiteDataSpec>, String> {
    tatara_lisp::compile_typed::<ClearSiteDataSpec>(src)
        .map_err(|e| format!("failed to compile defclear-site-data forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ClearSiteDataSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_header_driven_standard_surfaces() {
        let s = ClearSiteDataSpec::default_profile();
        assert_eq!(s.trigger, Trigger::HeaderDriven);
        assert_eq!(s.scope, Scope::ThisOrigin);
        assert_eq!(s.surfaces.len(), 3);
        assert!(s.surfaces.contains(&Surface::Cache));
        assert!(s.surfaces.contains(&Surface::Cookies));
        assert!(s.surfaces.contains(&Surface::Storage));
    }

    #[test]
    fn should_fire_header_driven_only_on_header() {
        let s = ClearSiteDataSpec::default_profile();
        let input = ClearTriggerInput {
            received_clear_site_data_header: true,
            ..ClearTriggerInput::default()
        };
        assert!(s.should_fire(&input));
        assert!(!s.should_fire(&ClearTriggerInput::default()));
    }

    #[test]
    fn should_fire_on_close() {
        let s = ClearSiteDataSpec {
            trigger: Trigger::OnClose,
            ..ClearSiteDataSpec::default_profile()
        };
        let input = ClearTriggerInput {
            is_browser_close: true,
            ..ClearTriggerInput::default()
        };
        assert!(s.should_fire(&input));
        assert!(!s.should_fire(&ClearTriggerInput {
            is_tab_close: true,
            ..ClearTriggerInput::default()
        }));
    }

    #[test]
    fn should_fire_on_idle_gates_on_threshold() {
        let s = ClearSiteDataSpec {
            trigger: Trigger::OnIdle,
            on_idle_minutes: 60,
            ..ClearSiteDataSpec::default_profile()
        };
        let input = ClearTriggerInput {
            idle_minutes: 30,
            ..ClearTriggerInput::default()
        };
        assert!(!s.should_fire(&input));
        let input = ClearTriggerInput {
            idle_minutes: 60,
            ..ClearTriggerInput::default()
        };
        assert!(s.should_fire(&input));
    }

    #[test]
    fn should_fire_on_idle_threshold_zero_never_fires() {
        let s = ClearSiteDataSpec {
            trigger: Trigger::OnIdle,
            on_idle_minutes: 0,
            ..ClearSiteDataSpec::default_profile()
        };
        let input = ClearTriggerInput {
            idle_minutes: 9_999,
            ..ClearTriggerInput::default()
        };
        assert!(!s.should_fire(&input));
    }

    #[test]
    fn should_fire_periodic_requires_both_interval_and_tick() {
        let s = ClearSiteDataSpec {
            trigger: Trigger::Periodic,
            interval_hours: 24,
            ..ClearSiteDataSpec::default_profile()
        };
        let input = ClearTriggerInput {
            periodic_tick: true,
            ..ClearTriggerInput::default()
        };
        assert!(s.should_fire(&input));
        let input = ClearTriggerInput {
            periodic_tick: false,
            ..ClearTriggerInput::default()
        };
        assert!(!s.should_fire(&input));

        let zero = ClearSiteDataSpec {
            trigger: Trigger::Periodic,
            interval_hours: 0,
            ..ClearSiteDataSpec::default_profile()
        };
        assert!(!zero.should_fire(&ClearTriggerInput {
            periodic_tick: true,
            ..ClearTriggerInput::default()
        }));
    }

    #[test]
    fn should_fire_disabled_never_fires() {
        let s = ClearSiteDataSpec {
            enabled: false,
            trigger: Trigger::OnClose,
            ..ClearSiteDataSpec::default_profile()
        };
        let input = ClearTriggerInput {
            is_browser_close: true,
            ..ClearTriggerInput::default()
        };
        assert!(!s.should_fire(&input));
    }

    #[test]
    fn should_fire_on_identity_switch() {
        let s = ClearSiteDataSpec {
            trigger: Trigger::OnIdentitySwitch,
            ..ClearSiteDataSpec::default_profile()
        };
        let input = ClearTriggerInput {
            identity_switch: true,
            ..ClearTriggerInput::default()
        };
        assert!(s.should_fire(&input));
    }

    #[test]
    fn effective_surfaces_filters_preserve_list() {
        let s = ClearSiteDataSpec {
            surfaces: vec![Surface::Cache, Surface::Cookies, Surface::Storage],
            always_preserve: vec![Surface::Cookies],
            ..ClearSiteDataSpec::default_profile()
        };
        let out = s.effective_surfaces();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&Surface::Cache));
        assert!(out.contains(&Surface::Storage));
        assert!(!out.contains(&Surface::Cookies));
    }

    #[test]
    fn effective_surfaces_injects_execution_contexts_when_forced() {
        let s = ClearSiteDataSpec {
            surfaces: vec![Surface::Cache],
            force_execution_contexts: true,
            ..ClearSiteDataSpec::default_profile()
        };
        let out = s.effective_surfaces();
        assert!(out.contains(&Surface::ExecutionContexts));
    }

    #[test]
    fn effective_surfaces_does_not_duplicate_execution_contexts() {
        let s = ClearSiteDataSpec {
            surfaces: vec![Surface::ExecutionContexts],
            force_execution_contexts: true,
            ..ClearSiteDataSpec::default_profile()
        };
        let out = s.effective_surfaces();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn is_exempt_matches_glob() {
        let s = ClearSiteDataSpec {
            exempt_hosts: vec!["*://*.auth.com/*".into()],
            ..ClearSiteDataSpec::default_profile()
        };
        assert!(s.is_exempt("sso.auth.com"));
        assert!(!s.is_exempt("ex.com"));
    }

    #[test]
    fn surface_roundtrips_through_serde() {
        for k in [
            Surface::Cache,
            Surface::Cookies,
            Surface::Storage,
            Surface::ExecutionContexts,
            Surface::ClientHints,
            Surface::PrefetchCache,
            Surface::ServiceWorkers,
            Surface::Channels,
            Surface::Permissions,
            Surface::SyncData,
            Surface::AutofillLocal,
            Surface::All,
        ] {
            let json = serde_json::to_string(&k).unwrap();
            let back: Surface = serde_json::from_str(&json).unwrap();
            assert_eq!(back, k);
        }
    }

    #[test]
    fn trigger_roundtrips_through_serde() {
        for t in [
            Trigger::HeaderDriven,
            Trigger::OnClose,
            Trigger::OnTabClose,
            Trigger::OnNavigateAway,
            Trigger::OnIdle,
            Trigger::Periodic,
            Trigger::Manual,
            Trigger::OnIdentitySwitch,
        ] {
            let s = ClearSiteDataSpec {
                trigger: t,
                ..ClearSiteDataSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: ClearSiteDataSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.trigger, t);
        }
    }

    #[test]
    fn scope_roundtrips_through_serde() {
        for sc in [
            Scope::ThisOrigin,
            Scope::RegistrableDomain,
            Scope::AllOrigins,
            Scope::ThisOriginAndPartitions,
        ] {
            let s = ClearSiteDataSpec {
                scope: sc,
                ..ClearSiteDataSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: ClearSiteDataSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.scope, sc);
        }
    }

    #[test]
    fn registry_applicable_filters_host_and_exempt() {
        let mut reg = ClearSiteDataRegistry::new();
        reg.insert(ClearSiteDataSpec::default_profile());
        reg.insert(ClearSiteDataSpec {
            name: "gh-exempt".into(),
            host: "*".into(),
            exempt_hosts: vec!["*://*.github.com/*".into()],
            ..ClearSiteDataSpec::default_profile()
        });
        // For github.com, only the profile whose exempt list does
        // NOT include github.com applies — so just the default one.
        let list = reg.applicable_to("www.github.com");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "default");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_clear_site_data_form() {
        let src = r#"
            (defclear-site-data :name "periodic-trim"
                                :host "*"
                                :surfaces ("cache" "cookies" "storage" "execution-contexts")
                                :trigger "periodic"
                                :interval-hours 24
                                :scope "this-origin"
                                :exempt-hosts ("*://*.auth.example.com/*")
                                :always-preserve ("cookies")
                                :force-execution-contexts #t
                                :grace-period-seconds 15)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.trigger, Trigger::Periodic);
        assert_eq!(s.interval_hours, 24);
        assert_eq!(s.exempt_hosts.len(), 1);
        assert!(s.always_preserve.contains(&Surface::Cookies));
        assert!(s.force_execution_contexts);
    }
}
