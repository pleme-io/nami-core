//! `(defcookie-jar)` — cookie storage partitioning + clearing policy.
//!
//! Absorbs Firefox Total Cookie Protection (Dynamic First-Party
//! Isolation), Safari Intelligent Tracking Prevention, Brave Shields
//! cookie handling, Chrome Tracking Protection, Arc cookie per-space
//! isolation. A jar has an isolation scope + a lifetime + optional
//! allow / block lists.
//!
//! ```lisp
//! (defcookie-jar :name          "strict"
//!                :host          "*"
//!                :partition     :per-site
//:                :third-party   :block
//!                :lifetime      :session
//!                :max-cookies   600
//!                :clear-on-close (third-party all-if-idle-days-gt 14))
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// How cookies are partitioned. Matches Firefox TCP / Chrome CHIPS
/// terminology.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Partition {
    /// Traditional shared cookie jar (anti-pattern nowadays).
    None,
    /// Per-eTLD+1 partition — Firefox TCP / Chrome CHIPS default.
    #[default]
    PerSite,
    /// Per-tab — max isolation, breaks single-sign-on.
    PerTab,
    /// Per-identity (bind to (defidentity) profiles).
    PerIdentity,
    /// Fully ephemeral — every navigation gets a fresh jar.
    Ephemeral,
}

/// Third-party cookie policy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ThirdPartyPolicy {
    /// Allow everything (old Chrome default).
    Allow,
    /// Block all third-party cookies — Safari/Brave/Firefox default.
    #[default]
    Block,
    /// Block only known-tracker third-parties (honor filter lists).
    BlockTrackers,
    /// Allow if the user has interacted with the third-party
    /// top-level — Firefox "storage access" grant model.
    RequireInteraction,
    /// Prompt the user.
    Prompt,
}

/// How long a cookie lasts regardless of server `Max-Age`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CookieLifetime {
    /// Keep until server-specified expiry.
    #[default]
    Server,
    /// Drop at browser close.
    Session,
    /// Hard-clamp to 24 h regardless of server request (Safari ITP).
    Clamp24h,
    /// Hard-clamp to 7 days (Firefox ETP).
    Clamp7days,
}

/// Trigger for cookie clearing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ClearTrigger {
    /// Clear all cookies on browser close.
    All,
    /// Clear third-party cookies on close.
    ThirdParty,
    /// Clear when a cookie becomes idle (no site visit) for
    /// `idle_days` days.
    AllIfIdleDaysGt,
    /// Clear when the user switches identity.
    IdentitySwitch,
    /// Clear on tab close (paired with `Partition::PerTab`).
    TabClose,
}

/// Cookie-jar profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defcookie-jar"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CookieJarSpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub partition: Partition,
    #[serde(default)]
    pub third_party: ThirdPartyPolicy,
    #[serde(default)]
    pub lifetime: CookieLifetime,
    /// Per-partition cookie count cap. 0 = unlimited.
    #[serde(default = "default_max_cookies")]
    pub max_cookies: u32,
    /// Per-cookie byte cap (sum of name + value + attributes).
    /// 0 = unlimited.
    #[serde(default = "default_max_cookie_bytes")]
    pub max_cookie_bytes: u32,
    /// Idle threshold when `AllIfIdleDaysGt` is in `clear_on_close`.
    #[serde(default = "default_idle_days")]
    pub idle_days: u32,
    /// When to clear.
    #[serde(default)]
    pub clear_on_close: Vec<ClearTrigger>,
    /// Hosts for which this jar is bypassed (cookies flow freely).
    #[serde(default)]
    pub allow_hosts: Vec<String>,
    /// Hosts whose cookies are always blocked regardless of other
    /// rules.
    #[serde(default)]
    pub block_hosts: Vec<String>,
    /// Suppress the `SameSite=None` default-upgrade (Chrome M100+).
    #[serde(default)]
    pub suppress_samesite_upgrade: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_max_cookies() -> u32 {
    600
}
fn default_max_cookie_bytes() -> u32 {
    4096
}
fn default_idle_days() -> u32 {
    30
}
fn default_enabled() -> bool {
    true
}

impl CookieJarSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            partition: Partition::PerSite,
            third_party: ThirdPartyPolicy::Block,
            lifetime: CookieLifetime::Server,
            max_cookies: 600,
            max_cookie_bytes: 4096,
            idle_days: 30,
            clear_on_close: vec![
                ClearTrigger::ThirdParty,
                ClearTrigger::AllIfIdleDaysGt,
            ],
            allow_hosts: vec![],
            block_hosts: vec![],
            suppress_samesite_upgrade: false,
            enabled: true,
            description: Some(
                "Default jar — per-site partition, third-party blocked, server lifetime with 30-day idle sweep.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    #[must_use]
    pub fn is_allowed(&self, host: &str) -> bool {
        self.allow_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    #[must_use]
    pub fn is_blocked(&self, host: &str) -> bool {
        self.block_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    /// Should a cookie from `third_party_host` be admitted when the
    /// top-level context is the user's actual visit?
    /// `had_user_interaction` models Firefox's storage-access grant.
    #[must_use]
    pub fn admits_third_party(&self, third_party_host: &str, had_user_interaction: bool) -> bool {
        if self.is_blocked(third_party_host) {
            return false;
        }
        if self.is_allowed(third_party_host) {
            return true;
        }
        match self.third_party {
            ThirdPartyPolicy::Allow => true,
            ThirdPartyPolicy::Block => false,
            ThirdPartyPolicy::BlockTrackers => true, // filter list is caller's problem
            ThirdPartyPolicy::RequireInteraction => had_user_interaction,
            ThirdPartyPolicy::Prompt => false, // default until the prompt resolves
        }
    }

    #[must_use]
    pub fn accepts_cookie_count(&self, current: u32) -> bool {
        self.max_cookies == 0 || current < self.max_cookies
    }

    #[must_use]
    pub fn accepts_cookie_size(&self, bytes: u32) -> bool {
        self.max_cookie_bytes == 0 || bytes <= self.max_cookie_bytes
    }

    #[must_use]
    pub fn clears(&self, trigger: ClearTrigger) -> bool {
        self.clear_on_close.contains(&trigger)
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct CookieJarRegistry {
    specs: Vec<CookieJarSpec>,
}

impl CookieJarRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: CookieJarSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = CookieJarSpec>) {
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
    pub fn specs(&self) -> &[CookieJarSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&CookieJarSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<CookieJarSpec>, String> {
    tatara_lisp::compile_typed::<CookieJarSpec>(src)
        .map_err(|e| format!("failed to compile defcookie-jar forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<CookieJarSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_strict_and_partitioned() {
        let s = CookieJarSpec::default_profile();
        assert_eq!(s.partition, Partition::PerSite);
        assert_eq!(s.third_party, ThirdPartyPolicy::Block);
        assert!(s.clears(ClearTrigger::ThirdParty));
        assert!(s.clears(ClearTrigger::AllIfIdleDaysGt));
        assert!(!s.clears(ClearTrigger::All));
    }

    #[test]
    fn admits_third_party_respects_block() {
        let s = CookieJarSpec::default_profile();
        assert!(!s.admits_third_party("tracker.com", false));
    }

    #[test]
    fn admits_third_party_allow_bypasses_policy() {
        let s = CookieJarSpec {
            third_party: ThirdPartyPolicy::Block,
            allow_hosts: vec!["*://*.trusted.com/*".into()],
            ..CookieJarSpec::default_profile()
        };
        assert!(s.admits_third_party("api.trusted.com", false));
    }

    #[test]
    fn admits_third_party_block_hosts_override_even_allow_policy() {
        let s = CookieJarSpec {
            third_party: ThirdPartyPolicy::Allow,
            block_hosts: vec!["*://*.tracker.com/*".into()],
            ..CookieJarSpec::default_profile()
        };
        assert!(!s.admits_third_party("api.tracker.com", true));
    }

    #[test]
    fn admits_third_party_require_interaction_honors_grant() {
        let s = CookieJarSpec {
            third_party: ThirdPartyPolicy::RequireInteraction,
            ..CookieJarSpec::default_profile()
        };
        assert!(!s.admits_third_party("other.com", false));
        assert!(s.admits_third_party("other.com", true));
    }

    #[test]
    fn accepts_cookie_count_respects_cap() {
        let capped = CookieJarSpec {
            max_cookies: 3,
            ..CookieJarSpec::default_profile()
        };
        assert!(capped.accepts_cookie_count(0));
        assert!(capped.accepts_cookie_count(2));
        assert!(!capped.accepts_cookie_count(3));

        let unlimited = CookieJarSpec {
            max_cookies: 0,
            ..CookieJarSpec::default_profile()
        };
        assert!(unlimited.accepts_cookie_count(999_999));
    }

    #[test]
    fn accepts_cookie_size_respects_max_bytes() {
        let s = CookieJarSpec {
            max_cookie_bytes: 100,
            ..CookieJarSpec::default_profile()
        };
        assert!(s.accepts_cookie_size(50));
        assert!(s.accepts_cookie_size(100));
        assert!(!s.accepts_cookie_size(101));
    }

    #[test]
    fn partition_roundtrips_through_serde() {
        for p in [
            Partition::None,
            Partition::PerSite,
            Partition::PerTab,
            Partition::PerIdentity,
            Partition::Ephemeral,
        ] {
            let s = CookieJarSpec {
                partition: p,
                ..CookieJarSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: CookieJarSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.partition, p);
        }
    }

    #[test]
    fn third_party_policy_roundtrips_through_serde() {
        for p in [
            ThirdPartyPolicy::Allow,
            ThirdPartyPolicy::Block,
            ThirdPartyPolicy::BlockTrackers,
            ThirdPartyPolicy::RequireInteraction,
            ThirdPartyPolicy::Prompt,
        ] {
            let s = CookieJarSpec {
                third_party: p,
                ..CookieJarSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: CookieJarSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.third_party, p);
        }
    }

    #[test]
    fn clear_trigger_roundtrips_through_serde() {
        let s = CookieJarSpec {
            clear_on_close: vec![
                ClearTrigger::All,
                ClearTrigger::ThirdParty,
                ClearTrigger::AllIfIdleDaysGt,
                ClearTrigger::IdentitySwitch,
                ClearTrigger::TabClose,
            ],
            ..CookieJarSpec::default_profile()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: CookieJarSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.clear_on_close.len(), 5);
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = CookieJarRegistry::new();
        reg.insert(CookieJarSpec::default_profile());
        reg.insert(CookieJarSpec {
            name: "ephemeral-news".into(),
            host: "*://*.nytimes.com/*".into(),
            partition: Partition::Ephemeral,
            ..CookieJarSpec::default_profile()
        });
        let ny = reg.resolve("www.nytimes.com").unwrap();
        assert_eq!(ny.partition, Partition::Ephemeral);
        let other = reg.resolve("example.org").unwrap();
        assert_eq!(other.name, "default");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = CookieJarRegistry::new();
        reg.insert(CookieJarSpec {
            enabled: false,
            ..CookieJarSpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_cookie_jar_form() {
        let src = r#"
            (defcookie-jar :name "strict"
                           :host "*"
                           :partition "per-site"
                           :third-party "block"
                           :lifetime "clamp7days"
                           :max-cookies 600
                           :max-cookie-bytes 4096
                           :idle-days 14
                           :clear-on-close ("third-party" "all-if-idle-days-gt")
                           :allow-hosts ("*://*.trusted.com/*")
                           :suppress-samesite-upgrade #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.partition, Partition::PerSite);
        assert_eq!(s.third_party, ThirdPartyPolicy::Block);
        assert_eq!(s.lifetime, CookieLifetime::Clamp7days);
        assert_eq!(s.idle_days, 14);
        assert!(s.clears(ClearTrigger::ThirdParty));
        assert!(s.clears(ClearTrigger::AllIfIdleDaysGt));
        assert!(s.suppress_samesite_upgrade);
    }
}
