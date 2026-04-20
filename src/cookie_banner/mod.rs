//! `(defcookie-banner)` — auto-dismiss GDPR / cookie-consent banners.
//!
//! Absorbs Brave Shields cookie-notice-blocker, Firefox "Cookie
//! Banner Blocker" (ETP v3), uBlock Origin "Annoyances" filters,
//! Consent-O-Matic, Kagi's built-in dismisser. Each profile declares
//! selectors for the banner, the preferred choice (reject-all /
//! minimal / accept), and a fallback timeout.
//!
//! ```lisp
//! (defcookie-banner :name             "strict"
//!                   :host             "*"
//!                   :preference       :reject-all
//!                   :banner-selectors ("#onetrust-banner-sdk"
//!                                      "[data-testid=cookie-banner]"
//!                                      ".cookieConsent"
//!                                      "#gdpr-consent-tool-wrapper")
//!                   :reject-selectors (".reject-all" "#btn-reject")
//!                   :minimal-selectors ("#btn-essential-only")
//!                   :accept-selectors  (".accept-all" "#btn-accept")
//!                   :timeout-ms       2500
//!                   :hide-on-timeout  #t
//!                   :remember-choice  #t)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// User's preferred outcome for a cookie banner.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Preference {
    /// Click "Reject all" / "Decline all". Privacy-first default.
    #[default]
    RejectAll,
    /// Click "Only essential" / "Required only".
    Minimal,
    /// Click "Accept all" (useful when a site won't load otherwise).
    AcceptAll,
    /// Don't click anything — just hide the banner with CSS.
    HideOnly,
    /// Ignore entirely — let the page render its banner.
    Passthrough,
}

/// Action taken when the selected button can't be found.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FallbackAction {
    /// Hide the banner element with `display: none`.
    #[default]
    Hide,
    /// Leave it alone — user sees the banner.
    LeaveAlone,
    /// Click whatever the author-provided `fallback_selectors`
    /// match.
    ClickFallback,
    /// Reload the page (last resort — rarely correct).
    Reload,
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defcookie-banner"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CookieBannerSpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub preference: Preference,
    /// CSS selectors that identify the banner itself. First match
    /// wins.
    #[serde(default = "default_banner_selectors")]
    pub banner_selectors: Vec<String>,
    /// CSS selectors for the "Reject all" button.
    #[serde(default = "default_reject_selectors")]
    pub reject_selectors: Vec<String>,
    /// CSS selectors for "Only essential" / "Required only".
    #[serde(default)]
    pub minimal_selectors: Vec<String>,
    /// CSS selectors for "Accept all" (opt-in convenience).
    #[serde(default)]
    pub accept_selectors: Vec<String>,
    /// Selectors clicked when `fallback = ClickFallback`.
    #[serde(default)]
    pub fallback_selectors: Vec<String>,
    /// Max ms to wait for the banner DOM node to appear.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u32,
    /// When the preferred button isn't found within `timeout_ms`,
    /// run `fallback`.
    #[serde(default)]
    pub fallback: FallbackAction,
    /// Hide the banner with `display:none` immediately, even before
    /// the click. Prevents a visible flash.
    #[serde(default = "default_hide_flash")]
    pub hide_flash: bool,
    /// Drop a marker (LocalStorage key) so the site doesn't re-prompt
    /// next visit — emulates what a real "Reject" button does.
    #[serde(default = "default_remember_choice")]
    pub remember_choice: bool,
    /// Max times to attempt per session — defends against banners
    /// that keep re-rendering.
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    /// Log every dismissal to (defaudit-trail) if present.
    #[serde(default)]
    pub audit_dismissals: bool,
    /// Hosts exempt from dismissal (fine-tune per-site).
    #[serde(default)]
    pub exempt_hosts: Vec<String>,
    /// Priority tiebreak — higher wins when two profiles both match.
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_banner_selectors() -> Vec<String> {
    vec![
        "#onetrust-banner-sdk".into(),
        "#CybotCookiebotDialog".into(),
        "#didomi-notice".into(),
        "[data-testid=\"cookie-banner\"]".into(),
        "[aria-label*=\"cookie\" i]".into(),
        "[aria-label*=\"consent\" i]".into(),
        ".cookieConsent".into(),
        "#gdpr-consent-tool-wrapper".into(),
        ".gdpr-banner".into(),
    ]
}
fn default_reject_selectors() -> Vec<String> {
    vec![
        "#onetrust-reject-all-handler".into(),
        "#CybotCookiebotDialogBodyButtonDecline".into(),
        ".reject-all".into(),
        "button[aria-label*=\"reject\" i]".into(),
        "button[data-testid=\"reject-all\"]".into(),
        "#btn-reject".into(),
    ]
}
fn default_timeout_ms() -> u32 {
    2_500
}
fn default_hide_flash() -> bool {
    true
}
fn default_remember_choice() -> bool {
    true
}
fn default_max_attempts() -> u32 {
    3
}
fn default_enabled() -> bool {
    true
}

impl CookieBannerSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            preference: Preference::RejectAll,
            banner_selectors: default_banner_selectors(),
            reject_selectors: default_reject_selectors(),
            minimal_selectors: vec![
                "#onetrust-pc-btn-handler".into(),
                ".essential-only".into(),
                "button[data-testid=\"essential-only\"]".into(),
            ],
            accept_selectors: vec![
                "#onetrust-accept-btn-handler".into(),
                ".accept-all".into(),
                "button[aria-label*=\"accept\" i]".into(),
            ],
            fallback_selectors: vec![],
            timeout_ms: 2_500,
            fallback: FallbackAction::Hide,
            hide_flash: true,
            remember_choice: true,
            max_attempts: 3,
            audit_dismissals: false,
            exempt_hosts: vec![],
            priority: 0,
            enabled: true,
            description: Some(
                "Default — reject-all with hide fallback; covers OneTrust/Cookiebot/Didomi + common aria-label patterns.".into(),
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

    /// The primary selector list the engine should try to click,
    /// given the user's preference. `HideOnly` + `Passthrough`
    /// return empty.
    #[must_use]
    pub fn preferred_selectors(&self) -> &[String] {
        match self.preference {
            Preference::RejectAll => &self.reject_selectors,
            Preference::Minimal => {
                if self.minimal_selectors.is_empty() {
                    &self.reject_selectors
                } else {
                    &self.minimal_selectors
                }
            }
            Preference::AcceptAll => &self.accept_selectors,
            Preference::HideOnly | Preference::Passthrough => &[],
        }
    }

    /// Does this profile ever try to click anything?
    #[must_use]
    pub fn clicks_any(&self) -> bool {
        !matches!(
            self.preference,
            Preference::HideOnly | Preference::Passthrough
        )
    }

    /// Should this profile immediately hide the banner with CSS?
    /// Passthrough always returns false.
    #[must_use]
    pub fn should_hide_flash(&self) -> bool {
        self.hide_flash && !matches!(self.preference, Preference::Passthrough)
    }

    /// CSS rule text that hides every banner selector. Returns
    /// `None` when no hiding is desired.
    #[must_use]
    pub fn render_hide_css(&self) -> Option<String> {
        if !self.should_hide_flash() {
            return None;
        }
        if self.banner_selectors.is_empty() {
            return None;
        }
        let joined = self.banner_selectors.join(", ");
        Some(format!(
            "{joined} {{ display: none !important; visibility: hidden !important; }}"
        ))
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct CookieBannerRegistry {
    specs: Vec<CookieBannerSpec>,
}

impl CookieBannerRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: CookieBannerSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = CookieBannerSpec>) {
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
    pub fn specs(&self) -> &[CookieBannerSpec] {
        &self.specs
    }

    /// Resolve the active profile for `host`. Preference order:
    /// host-specific → wildcard; priority tiebreak within each tier.
    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&CookieBannerSpec> {
        let mut candidates: Vec<&CookieBannerSpec> = self
            .specs
            .iter()
            .filter(|s| s.enabled && s.matches_host(host) && !s.is_exempt(host))
            .collect();
        candidates.sort_by_key(|s| {
            let host_specific = !(s.host.is_empty() || s.host == "*");
            (u32::from(!host_specific), -s.priority)
        });
        candidates.first().copied()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<CookieBannerSpec>, String> {
    tatara_lisp::compile_typed::<CookieBannerSpec>(src)
        .map_err(|e| format!("failed to compile defcookie-banner forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<CookieBannerSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_reject_all_with_hide_fallback() {
        let s = CookieBannerSpec::default_profile();
        assert_eq!(s.preference, Preference::RejectAll);
        assert_eq!(s.fallback, FallbackAction::Hide);
        assert!(s.hide_flash);
        assert!(s.remember_choice);
        assert!(!s.reject_selectors.is_empty());
    }

    #[test]
    fn preferred_selectors_dispatches_on_preference() {
        let s = CookieBannerSpec::default_profile();
        assert!(!s.preferred_selectors().is_empty()); // RejectAll
        let accept = CookieBannerSpec {
            preference: Preference::AcceptAll,
            ..CookieBannerSpec::default_profile()
        };
        assert_eq!(accept.preferred_selectors(), accept.accept_selectors);
        let hide = CookieBannerSpec {
            preference: Preference::HideOnly,
            ..CookieBannerSpec::default_profile()
        };
        assert!(hide.preferred_selectors().is_empty());
        let pass = CookieBannerSpec {
            preference: Preference::Passthrough,
            ..CookieBannerSpec::default_profile()
        };
        assert!(pass.preferred_selectors().is_empty());
    }

    #[test]
    fn minimal_preference_falls_back_to_reject_when_empty() {
        let s = CookieBannerSpec {
            preference: Preference::Minimal,
            minimal_selectors: vec![],
            ..CookieBannerSpec::default_profile()
        };
        assert_eq!(s.preferred_selectors(), s.reject_selectors);
    }

    #[test]
    fn clicks_any_predicate() {
        assert!(CookieBannerSpec::default_profile().clicks_any());
        for p in [Preference::Minimal, Preference::AcceptAll, Preference::RejectAll] {
            let s = CookieBannerSpec {
                preference: p,
                ..CookieBannerSpec::default_profile()
            };
            assert!(s.clicks_any());
        }
        for p in [Preference::HideOnly, Preference::Passthrough] {
            let s = CookieBannerSpec {
                preference: p,
                ..CookieBannerSpec::default_profile()
            };
            assert!(!s.clicks_any());
        }
    }

    #[test]
    fn should_hide_flash_honors_passthrough_and_flag() {
        let s = CookieBannerSpec::default_profile();
        assert!(s.should_hide_flash());
        let off = CookieBannerSpec {
            hide_flash: false,
            ..CookieBannerSpec::default_profile()
        };
        assert!(!off.should_hide_flash());
        let pass = CookieBannerSpec {
            preference: Preference::Passthrough,
            ..CookieBannerSpec::default_profile()
        };
        assert!(!pass.should_hide_flash());
    }

    #[test]
    fn render_hide_css_joins_selectors() {
        let s = CookieBannerSpec {
            banner_selectors: vec![".a".into(), "#b".into()],
            ..CookieBannerSpec::default_profile()
        };
        let css = s.render_hide_css().unwrap();
        assert!(css.starts_with(".a, #b {"));
        assert!(css.contains("display: none !important"));
        assert!(css.contains("visibility: hidden !important"));
    }

    #[test]
    fn render_hide_css_none_when_passthrough_or_empty() {
        let pass = CookieBannerSpec {
            preference: Preference::Passthrough,
            ..CookieBannerSpec::default_profile()
        };
        assert!(pass.render_hide_css().is_none());

        let empty = CookieBannerSpec {
            banner_selectors: vec![],
            ..CookieBannerSpec::default_profile()
        };
        assert!(empty.render_hide_css().is_none());
    }

    #[test]
    fn matches_host_glob() {
        let s = CookieBannerSpec {
            host: "*://*.nytimes.com/*".into(),
            ..CookieBannerSpec::default_profile()
        };
        assert!(s.matches_host("www.nytimes.com"));
        assert!(!s.matches_host("example.com"));
    }

    #[test]
    fn is_exempt_matches_glob() {
        let s = CookieBannerSpec {
            exempt_hosts: vec!["*://*.bank.com/*".into()],
            ..CookieBannerSpec::default_profile()
        };
        assert!(s.is_exempt("my.bank.com"));
        assert!(!s.is_exempt("example.com"));
    }

    #[test]
    fn preference_roundtrips_through_serde() {
        for p in [
            Preference::RejectAll,
            Preference::Minimal,
            Preference::AcceptAll,
            Preference::HideOnly,
            Preference::Passthrough,
        ] {
            let s = CookieBannerSpec {
                preference: p,
                ..CookieBannerSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: CookieBannerSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.preference, p);
        }
    }

    #[test]
    fn fallback_roundtrips_through_serde() {
        for f in [
            FallbackAction::Hide,
            FallbackAction::LeaveAlone,
            FallbackAction::ClickFallback,
            FallbackAction::Reload,
        ] {
            let s = CookieBannerSpec {
                fallback: f,
                ..CookieBannerSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: CookieBannerSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.fallback, f);
        }
    }

    #[test]
    fn registry_prefers_specific_host_over_wildcard() {
        let mut reg = CookieBannerRegistry::new();
        reg.insert(CookieBannerSpec::default_profile());
        reg.insert(CookieBannerSpec {
            name: "ny".into(),
            host: "*://*.nytimes.com/*".into(),
            preference: Preference::AcceptAll,
            ..CookieBannerSpec::default_profile()
        });
        let ny = reg.resolve("www.nytimes.com").unwrap();
        assert_eq!(ny.name, "ny");
        let other = reg.resolve("example.org").unwrap();
        assert_eq!(other.name, "default");
    }

    #[test]
    fn registry_priority_tiebreaks_within_tier() {
        let mut reg = CookieBannerRegistry::new();
        reg.insert(CookieBannerSpec {
            name: "lo".into(),
            priority: 0,
            ..CookieBannerSpec::default_profile()
        });
        reg.insert(CookieBannerSpec {
            name: "hi".into(),
            priority: 99,
            ..CookieBannerSpec::default_profile()
        });
        assert_eq!(reg.resolve("example.com").unwrap().name, "hi");
    }

    #[test]
    fn registry_exempt_host_hides_profile() {
        let mut reg = CookieBannerRegistry::new();
        reg.insert(CookieBannerSpec {
            name: "off".into(),
            exempt_hosts: vec!["*://*.bank.com/*".into()],
            ..CookieBannerSpec::default_profile()
        });
        assert!(reg.resolve("my.bank.com").is_none());
        assert_eq!(reg.resolve("example.com").unwrap().name, "off");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_cookie_banner_form() {
        let src = r##"
            (defcookie-banner :name "strict"
                              :host "*"
                              :preference "reject-all"
                              :banner-selectors ("#cookie")
                              :reject-selectors (".reject-all")
                              :minimal-selectors ("#essential")
                              :accept-selectors (".accept-all")
                              :timeout-ms 3000
                              :fallback "hide"
                              :hide-flash #t
                              :remember-choice #t
                              :max-attempts 5
                              :audit-dismissals #t
                              :priority 10)
        "##;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.preference, Preference::RejectAll);
        assert_eq!(s.fallback, FallbackAction::Hide);
        assert_eq!(s.timeout_ms, 3_000);
        assert_eq!(s.max_attempts, 5);
        assert!(s.audit_dismissals);
        assert_eq!(s.priority, 10);
    }
}
