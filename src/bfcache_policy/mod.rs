//! `(defbfcache-policy)` — Back/Forward Cache tuning.
//!
//! Absorbs Chrome/Firefox bfcache + Safari Page Cache. Per-host
//! declarative control over eligibility — max cache size, TTL, which
//! teardown hooks disqualify, which features (WebSockets, Geolocation
//! watchers) are allowed to keep pages eligible.
//!
//! ```lisp
//! (defbfcache-policy :name          "default"
//!                   :host          "*"
//!                   :eligibility   :automatic
//!                   :ttl-seconds   600
//!                   :max-cached    5
//!                   :disqualify    (beforeunload unload pagehide-with-fetch)
//!                   :allow-with-ws #f
//!                   :allow-workers #t
//!                   :preserve-scroll #t)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// High-level eligibility knob — from off to "always".
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Eligibility {
    /// Never cache.
    Off,
    /// Browser default — honor blockers, cache eligible pages.
    #[default]
    Automatic,
    /// Cache even when the page registers `unload` / `beforeunload`.
    Aggressive,
    /// Cache only when author opts in via `Cache-Control: bfcache`.
    OptIn,
}

/// Reasons a page loses bfcache eligibility. Authors list the
/// conditions that DISQUALIFY a page (Chrome lists ~40 such reasons
/// under chrome://back-forward-cache).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Disqualifier {
    /// `beforeunload` handler.
    Beforeunload,
    /// `unload` handler.
    Unload,
    /// `pagehide` handler with pending fetch.
    PagehideWithFetch,
    /// Active WebSocket connection.
    OpenWebsocket,
    /// Active WebRTC connection.
    OpenWebrtc,
    /// Geolocation / other sensor watcher.
    SensorWatcher,
    /// IndexedDB transaction in flight.
    IndexedDbInFlight,
    /// Broadcast Channel or MessagePort still referenced.
    OpenMessageChannel,
    /// Service Worker controller is different.
    SwControllerChange,
    /// CACHE-CONTROL: no-store response header.
    CacheControlNoStore,
    /// HTTP 4xx/5xx response.
    HttpError,
    /// HTTPS certificate error / mixed content.
    CertError,
    /// Top-level frame had unhandled promise rejection at unload time.
    UnhandledRejection,
    /// `<dialog>` was open at page-hide.
    OpenDialog,
    /// `document.pictureInPicture` was active.
    PictureInPicture,
}

/// Scroll + form preservation. These are orthogonal to caching
/// itself — authors might want scroll restored but form state
/// cleared.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ScrollRestoration {
    /// Browser default.
    #[default]
    Auto,
    /// Always restore scroll on bfcache restore.
    Manual,
    /// Never restore (reset to top).
    Reset,
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defbfcache-policy"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BfcachePolicySpec {
    pub name: String,
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    #[serde(default)]
    pub eligibility: Eligibility,
    /// How long a cached page stays valid (seconds). 0 = browser
    /// default (~10 min for Chrome). Hard-clamped to 3600 to match
    /// Chrome's maximum.
    #[serde(default)]
    pub ttl_seconds: u32,
    /// How many pages to keep in the cache per tab history. 0 =
    /// browser default (~6 for Chrome).
    #[serde(default)]
    pub max_cached: u32,
    /// Conditions that disqualify a page.
    #[serde(default = "default_disqualify")]
    pub disqualify: Vec<Disqualifier>,
    /// Allow caching despite an open WebSocket.
    #[serde(default)]
    pub allow_with_ws: bool,
    /// Allow caching despite an open WebRTC session.
    #[serde(default)]
    pub allow_with_webrtc: bool,
    /// Allow caching pages that own shared + dedicated workers.
    #[serde(default = "default_allow_workers")]
    pub allow_workers: bool,
    /// Preserve scroll position on restore.
    #[serde(default = "default_preserve_scroll")]
    pub preserve_scroll: bool,
    #[serde(default)]
    pub scroll_restoration: ScrollRestoration,
    /// Preserve form field state (uncheck if you want Chrome's
    /// autofill-sensitive behavior).
    #[serde(default = "default_preserve_form")]
    pub preserve_form: bool,
    /// Dispatch `pageshow` event on restore (standard behavior).
    #[serde(default = "default_fire_pageshow")]
    pub fire_pageshow: bool,
    /// Maximum memory per cached page (MB). 0 = no cap.
    #[serde(default)]
    pub max_page_memory_mb: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_disqualify() -> Vec<Disqualifier> {
    vec![
        Disqualifier::Beforeunload,
        Disqualifier::Unload,
        Disqualifier::CacheControlNoStore,
        Disqualifier::HttpError,
        Disqualifier::CertError,
    ]
}
fn default_allow_workers() -> bool {
    true
}
fn default_preserve_scroll() -> bool {
    true
}
fn default_preserve_form() -> bool {
    true
}
fn default_fire_pageshow() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

/// Runtime snapshot a caller feeds `is_eligible`.
#[derive(Debug, Clone, Default)]
pub struct PageSignal {
    pub has_beforeunload: bool,
    pub has_unload: bool,
    pub has_pending_fetch_on_pagehide: bool,
    pub has_open_websocket: bool,
    pub has_open_webrtc: bool,
    pub has_sensor_watcher: bool,
    pub has_indexed_db_in_flight: bool,
    pub has_open_message_channel: bool,
    pub sw_controller_changed: bool,
    pub cache_control_no_store: bool,
    pub http_error: bool,
    pub cert_error: bool,
    pub unhandled_rejection: bool,
    pub open_dialog: bool,
    pub picture_in_picture: bool,
    pub page_memory_mb: u32,
}

impl BfcachePolicySpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            eligibility: Eligibility::Automatic,
            ttl_seconds: 600,
            max_cached: 6,
            disqualify: default_disqualify(),
            allow_with_ws: false,
            allow_with_webrtc: false,
            allow_workers: true,
            preserve_scroll: true,
            scroll_restoration: ScrollRestoration::Auto,
            preserve_form: true,
            fire_pageshow: true,
            max_page_memory_mb: 0,
            enabled: true,
            description: Some(
                "Default bfcache — 10 min TTL, keep 6 pages, standard disqualifier set.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    /// Clamped TTL — browsers in the wild cap at ~1 hour.
    #[must_use]
    pub fn clamped_ttl_seconds(&self) -> u32 {
        self.ttl_seconds.min(3_600)
    }

    /// Is a page with these signals eligible for bfcache?
    #[must_use]
    pub fn is_eligible(&self, signal: &PageSignal) -> bool {
        if !self.enabled {
            return false;
        }
        match self.eligibility {
            Eligibility::Off => return false,
            Eligibility::Aggressive => {
                // Aggressive still rejects certificate errors and
                // no-store responses — those are security issues.
                if signal.cert_error || signal.cache_control_no_store || signal.http_error {
                    return false;
                }
                return true;
            }
            Eligibility::OptIn | Eligibility::Automatic => {}
        }

        if self.max_page_memory_mb != 0 && signal.page_memory_mb > self.max_page_memory_mb {
            return false;
        }

        for d in &self.disqualify {
            if triggered(*d, signal) {
                let allowed = matches!(
                    d,
                    Disqualifier::OpenWebsocket if self.allow_with_ws
                ) || matches!(
                    d,
                    Disqualifier::OpenWebrtc if self.allow_with_webrtc
                );
                if !allowed {
                    return false;
                }
            }
        }
        true
    }
}

fn triggered(d: Disqualifier, s: &PageSignal) -> bool {
    use Disqualifier::*;
    match d {
        Beforeunload => s.has_beforeunload,
        Unload => s.has_unload,
        PagehideWithFetch => s.has_pending_fetch_on_pagehide,
        OpenWebsocket => s.has_open_websocket,
        OpenWebrtc => s.has_open_webrtc,
        SensorWatcher => s.has_sensor_watcher,
        IndexedDbInFlight => s.has_indexed_db_in_flight,
        OpenMessageChannel => s.has_open_message_channel,
        SwControllerChange => s.sw_controller_changed,
        CacheControlNoStore => s.cache_control_no_store,
        HttpError => s.http_error,
        CertError => s.cert_error,
        UnhandledRejection => s.unhandled_rejection,
        OpenDialog => s.open_dialog,
        PictureInPicture => s.picture_in_picture,
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct BfcachePolicyRegistry {
    specs: Vec<BfcachePolicySpec>,
}

impl BfcachePolicyRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: BfcachePolicySpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = BfcachePolicySpec>) {
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
    pub fn specs(&self) -> &[BfcachePolicySpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&BfcachePolicySpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<BfcachePolicySpec>, String> {
    tatara_lisp::compile_typed::<BfcachePolicySpec>(src)
        .map_err(|e| format!("failed to compile defbfcache-policy forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<BfcachePolicySpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal_clean() -> PageSignal {
        PageSignal::default()
    }

    #[test]
    fn default_profile_is_automatic_with_standard_disqualifiers() {
        let s = BfcachePolicySpec::default_profile();
        assert_eq!(s.eligibility, Eligibility::Automatic);
        assert_eq!(s.ttl_seconds, 600);
        assert!(s.disqualify.contains(&Disqualifier::Beforeunload));
        assert!(s.disqualify.contains(&Disqualifier::CertError));
    }

    #[test]
    fn clean_page_is_eligible() {
        let s = BfcachePolicySpec::default_profile();
        assert!(s.is_eligible(&signal_clean()));
    }

    #[test]
    fn beforeunload_disqualifies() {
        let s = BfcachePolicySpec::default_profile();
        let sig = PageSignal {
            has_beforeunload: true,
            ..signal_clean()
        };
        assert!(!s.is_eligible(&sig));
    }

    #[test]
    fn off_eligibility_rejects_clean_pages() {
        let s = BfcachePolicySpec {
            eligibility: Eligibility::Off,
            ..BfcachePolicySpec::default_profile()
        };
        assert!(!s.is_eligible(&signal_clean()));
    }

    #[test]
    fn aggressive_eligibility_ignores_beforeunload() {
        let s = BfcachePolicySpec {
            eligibility: Eligibility::Aggressive,
            ..BfcachePolicySpec::default_profile()
        };
        let sig = PageSignal {
            has_beforeunload: true,
            ..signal_clean()
        };
        assert!(s.is_eligible(&sig));
    }

    #[test]
    fn aggressive_eligibility_still_rejects_cert_error() {
        let s = BfcachePolicySpec {
            eligibility: Eligibility::Aggressive,
            ..BfcachePolicySpec::default_profile()
        };
        let sig = PageSignal {
            cert_error: true,
            ..signal_clean()
        };
        assert!(!s.is_eligible(&sig));
    }

    #[test]
    fn allow_with_ws_opts_back_in() {
        let s = BfcachePolicySpec {
            disqualify: vec![Disqualifier::OpenWebsocket],
            allow_with_ws: true,
            ..BfcachePolicySpec::default_profile()
        };
        let sig = PageSignal {
            has_open_websocket: true,
            ..signal_clean()
        };
        assert!(s.is_eligible(&sig));
    }

    #[test]
    fn memory_cap_disqualifies_over_budget_pages() {
        let s = BfcachePolicySpec {
            max_page_memory_mb: 100,
            ..BfcachePolicySpec::default_profile()
        };
        let sig = PageSignal {
            page_memory_mb: 250,
            ..signal_clean()
        };
        assert!(!s.is_eligible(&sig));
    }

    #[test]
    fn memory_cap_zero_means_no_limit() {
        let s = BfcachePolicySpec {
            max_page_memory_mb: 0,
            ..BfcachePolicySpec::default_profile()
        };
        let sig = PageSignal {
            page_memory_mb: u32::MAX,
            ..signal_clean()
        };
        assert!(s.is_eligible(&sig));
    }

    #[test]
    fn clamped_ttl_seconds_capped_at_hour() {
        let s = BfcachePolicySpec {
            ttl_seconds: 10_000,
            ..BfcachePolicySpec::default_profile()
        };
        assert_eq!(s.clamped_ttl_seconds(), 3_600);
    }

    #[test]
    fn disabled_profile_is_never_eligible() {
        let s = BfcachePolicySpec {
            enabled: false,
            ..BfcachePolicySpec::default_profile()
        };
        assert!(!s.is_eligible(&signal_clean()));
    }

    #[test]
    fn eligibility_roundtrips_through_serde() {
        for e in [
            Eligibility::Off,
            Eligibility::Automatic,
            Eligibility::Aggressive,
            Eligibility::OptIn,
        ] {
            let s = BfcachePolicySpec {
                eligibility: e,
                ..BfcachePolicySpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: BfcachePolicySpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.eligibility, e);
        }
    }

    #[test]
    fn disqualifier_set_roundtrips_through_serde() {
        let s = BfcachePolicySpec {
            disqualify: vec![
                Disqualifier::Beforeunload,
                Disqualifier::Unload,
                Disqualifier::PagehideWithFetch,
                Disqualifier::OpenWebsocket,
                Disqualifier::OpenWebrtc,
                Disqualifier::SensorWatcher,
                Disqualifier::IndexedDbInFlight,
                Disqualifier::OpenMessageChannel,
                Disqualifier::SwControllerChange,
                Disqualifier::CacheControlNoStore,
                Disqualifier::HttpError,
                Disqualifier::CertError,
                Disqualifier::UnhandledRejection,
                Disqualifier::OpenDialog,
                Disqualifier::PictureInPicture,
            ],
            ..BfcachePolicySpec::default_profile()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: BfcachePolicySpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.disqualify.len(), 15);
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = BfcachePolicyRegistry::new();
        reg.insert(BfcachePolicySpec::default_profile());
        reg.insert(BfcachePolicySpec {
            name: "aggressive-news".into(),
            host: "*://*.nytimes.com/*".into(),
            eligibility: Eligibility::Aggressive,
            ..BfcachePolicySpec::default_profile()
        });
        let ny = reg.resolve("www.nytimes.com").unwrap();
        assert_eq!(ny.eligibility, Eligibility::Aggressive);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_bfcache_policy_form() {
        let src = r#"
            (defbfcache-policy :name "custom"
                               :host "*"
                               :eligibility "automatic"
                               :ttl-seconds 600
                               :max-cached 5
                               :allow-with-ws #f
                               :allow-workers #t
                               :preserve-scroll #t
                               :max-page-memory-mb 200
                               :disqualify ("beforeunload" "unload" "cache-control-no-store"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.eligibility, Eligibility::Automatic);
        assert_eq!(s.disqualify.len(), 3);
        assert_eq!(s.max_page_memory_mb, 200);
    }
}
