//! `(defscript-policy)` — per-origin JS capability restriction.
//!
//! Absorbs NoScript, JShelter (JavaScript Restrictor), uMatrix, and
//! Brave's JS-off-by-default mode into the substrate. Complements
//! [`crate::spoof`] (which noises specific APIs) and
//! [`crate::security_policy`] (which sets CSP headers) — this is
//! the *fine-grained runtime* layer: which origins may load JS at
//! all, and which Web APIs they may call.
//!
//! ```lisp
//! (defscript-policy :name            "strict"
//!                   :host            "*"
//!                   :mode            :block-all)
//!
//! (defscript-policy :name            "banking"
//!                   :host            "*://*.bank.com/*"
//!                   :mode            :allow-list
//!                   :allowed-origins ("bank.com" "cdn.bank.com")
//!                   :restricted-apis (web-rtc geolocation beacon
//!                                     battery sensors clipboard-read))
//!
//! (defscript-policy :name            "default"
//!                   :host            "*"
//!                   :mode            :allow-all
//!                   :restricted-apis (web-rtc battery sensors))
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Top-level execution mode for JS inside matching origins.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ScriptMode {
    /// Let all scripts run (Web as usual).
    AllowAll,
    /// Refuse all scripts regardless of origin.
    BlockAll,
    /// Only scripts from origins in `allowed_origins` run.
    AllowList,
    /// All scripts run except those from `blocked_origins`.
    BlockList,
}

impl Default for ScriptMode {
    fn default() -> Self {
        Self::AllowAll
    }
}

/// Web API categories that can be selectively denied. Mirrors the
/// JShelter API-level taxonomy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ApiCategory {
    WebRtc,
    Geolocation,
    Beacon,
    Canvas,
    WebGl,
    Camera,
    Microphone,
    Clipboard,
    ClipboardRead,
    ClipboardWrite,
    Notifications,
    Sensors,
    Battery,
    Bluetooth,
    Usb,
    Serial,
    Hid,
    Nfc,
    Midi,
    SharedWorker,
    WebSocket,
    PaymentRequest,
    /// `fetch` + `XMLHttpRequest`. Very coarse; prefer
    /// `FetchAllowedHosts` on a (defjs-runtime).
    Network,
    /// `Performance` high-resolution timers.
    Timing,
    /// `requestIdleCallback`, `setTimeout<5ms`, etc. — fingerprint
    /// vectors via timing attacks.
    FineTimers,
    /// Service Workers.
    ServiceWorker,
    /// WebAuthn / Credential Management.
    Credentials,
}

/// One scoped script-execution policy.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defscript-policy"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScriptPolicySpec {
    pub name: String,
    /// Host glob; `"*"` = everywhere.
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub mode: ScriptMode,
    /// Origins allowed under `AllowList` mode. Plain hostnames;
    /// matched with the host glob rules.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// Origins blocked under `BlockList` mode.
    #[serde(default)]
    pub blocked_origins: Vec<String>,
    /// API categories denied regardless of mode. A script that
    /// would otherwise be allowed to run must still fail calls into
    /// these surfaces.
    #[serde(default)]
    pub restricted_apis: Vec<ApiCategory>,
    /// Inline/event-handler scripts (script-src 'unsafe-inline')?
    #[serde(default)]
    pub allow_inline: bool,
    /// Evaluated `eval`/`new Function`?
    #[serde(default)]
    pub allow_eval: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}

impl ScriptPolicySpec {
    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    /// Would a script loaded from `origin` be allowed to run under
    /// this policy? Doesn't consult API restrictions — that's a
    /// separate check at call time.
    #[must_use]
    pub fn allows_script(&self, origin: &str) -> bool {
        match self.mode {
            ScriptMode::AllowAll => true,
            ScriptMode::BlockAll => false,
            ScriptMode::AllowList => self
                .allowed_origins
                .iter()
                .any(|g| crate::extension::glob_match_host(g, origin)),
            ScriptMode::BlockList => !self
                .blocked_origins
                .iter()
                .any(|g| crate::extension::glob_match_host(g, origin)),
        }
    }

    #[must_use]
    pub fn api_restricted(&self, api: ApiCategory) -> bool {
        self.restricted_apis.contains(&api)
    }

    /// Sensible "deny all known-fingerprinting APIs" set — useful
    /// for quick profile authoring.
    #[must_use]
    pub fn hardening_api_set() -> Vec<ApiCategory> {
        vec![
            ApiCategory::WebRtc,
            ApiCategory::Battery,
            ApiCategory::Sensors,
            ApiCategory::Bluetooth,
            ApiCategory::Usb,
            ApiCategory::Serial,
            ApiCategory::Hid,
            ApiCategory::Nfc,
            ApiCategory::Midi,
            ApiCategory::FineTimers,
        ]
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct ScriptPolicyRegistry {
    specs: Vec<ScriptPolicySpec>,
}

impl ScriptPolicyRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: ScriptPolicySpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ScriptPolicySpec>) {
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
    pub fn specs(&self) -> &[ScriptPolicySpec] {
        &self.specs
    }

    /// Most-specific host match wins.
    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&ScriptPolicySpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ScriptPolicySpec>, String> {
    tatara_lisp::compile_typed::<ScriptPolicySpec>(src)
        .map_err(|e| format!("failed to compile defscript-policy forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ScriptPolicySpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, mode: ScriptMode) -> ScriptPolicySpec {
        ScriptPolicySpec {
            name: name.into(),
            host: "*".into(),
            mode,
            allowed_origins: vec![],
            blocked_origins: vec![],
            restricted_apis: vec![],
            allow_inline: false,
            allow_eval: false,
            description: None,
        }
    }

    #[test]
    fn allow_all_mode_allows_everything() {
        let s = sample("x", ScriptMode::AllowAll);
        assert!(s.allows_script("cdn.example.com"));
        assert!(s.allows_script("tracker.net"));
    }

    #[test]
    fn block_all_mode_refuses_everything() {
        let s = sample("x", ScriptMode::BlockAll);
        assert!(!s.allows_script("cdn.example.com"));
        assert!(!s.allows_script("self.com"));
    }

    #[test]
    fn allow_list_mode_only_listed_origins() {
        let s = ScriptPolicySpec {
            mode: ScriptMode::AllowList,
            allowed_origins: vec!["*.example.com".into(), "cdn.trusted.com".into()],
            ..sample("x", ScriptMode::AllowList)
        };
        assert!(s.allows_script("blog.example.com"));
        assert!(s.allows_script("cdn.trusted.com"));
        assert!(!s.allows_script("evil.net"));
    }

    #[test]
    fn block_list_mode_refuses_listed_origins() {
        let s = ScriptPolicySpec {
            mode: ScriptMode::BlockList,
            blocked_origins: vec!["*.tracker.net".into(), "ads.example.com".into()],
            ..sample("x", ScriptMode::BlockList)
        };
        assert!(!s.allows_script("pixel.tracker.net"));
        assert!(!s.allows_script("ads.example.com"));
        assert!(s.allows_script("self.com"));
    }

    #[test]
    fn api_restricted_checks_membership() {
        let s = ScriptPolicySpec {
            restricted_apis: vec![
                ApiCategory::WebRtc,
                ApiCategory::Battery,
                ApiCategory::FineTimers,
            ],
            ..sample("x", ScriptMode::AllowAll)
        };
        assert!(s.api_restricted(ApiCategory::WebRtc));
        assert!(s.api_restricted(ApiCategory::Battery));
        assert!(!s.api_restricted(ApiCategory::Canvas));
    }

    #[test]
    fn hardening_set_covers_known_fingerprint_vectors() {
        let hardened = ScriptPolicySpec::hardening_api_set();
        assert!(hardened.contains(&ApiCategory::WebRtc));
        assert!(hardened.contains(&ApiCategory::Battery));
        assert!(hardened.contains(&ApiCategory::FineTimers));
    }

    #[test]
    fn host_glob_matches_subdomains() {
        let s = ScriptPolicySpec {
            host: "*://*.bank.com/*".into(),
            ..sample("x", ScriptMode::AllowAll)
        };
        assert!(s.matches_host("online.bank.com"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn registry_dedupes_and_resolves_specific_over_wildcard() {
        let mut reg = ScriptPolicyRegistry::new();
        reg.insert(sample("default", ScriptMode::AllowAll));
        reg.insert(ScriptPolicySpec {
            name: "bank".into(),
            host: "*://*.bank.com/*".into(),
            mode: ScriptMode::AllowList,
            allowed_origins: vec!["bank.com".into()],
            ..sample("bank", ScriptMode::AllowList)
        });

        let bank = reg.resolve("online.bank.com").unwrap();
        assert_eq!(bank.name, "bank");
        let default = reg.resolve("example.com").unwrap();
        assert_eq!(default.name, "default");
    }

    #[test]
    fn api_category_roundtrips_through_serde() {
        for c in [
            ApiCategory::WebRtc,
            ApiCategory::Geolocation,
            ApiCategory::FineTimers,
            ApiCategory::ServiceWorker,
        ] {
            let json = serde_json::to_string(&c).unwrap();
            let back: ApiCategory = serde_json::from_str(&json).unwrap();
            assert_eq!(c, back);
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_script_policy_form() {
        let src = r#"
            (defscript-policy :name "banking"
                              :host "*://*.bank.com/*"
                              :mode "allow-list"
                              :allowed-origins ("bank.com" "cdn.bank.com")
                              :restricted-apis (web-rtc geolocation beacon)
                              :allow-inline #f
                              :allow-eval   #f)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.mode, ScriptMode::AllowList);
        assert_eq!(s.allowed_origins.len(), 2);
        assert!(s.restricted_apis.contains(&ApiCategory::WebRtc));
        assert!(s.restricted_apis.contains(&ApiCategory::Geolocation));
        assert!(!s.allow_inline);
    }
}
