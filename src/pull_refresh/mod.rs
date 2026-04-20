//! `(defpull-to-refresh)` — declarative pull-to-refresh gesture.
//!
//! Absorbs mobile Chrome / Safari / Firefox pull-to-refresh. A rule
//! scopes to a host, declares the pull-distance threshold, how the
//! indicator animates, and which command fires on release.
//!
//! ```lisp
//! (defpull-to-refresh :name      "default"
//!                     :host      "*"
//!                     :threshold 80
//!                     :command   "reload"
//!                     :animation-ms 250)
//!
//! (defpull-to-refresh :name      "news-sites"
//!                     :host      "*://*.ycombinator.com/*"
//!                     :threshold 120
//!                     :command   "history:back"
//!                     :animation-ms 150)
//! ```
//!
//! Disabled rules still count as an explicit "no PTR here" for the
//! host. Useful on sites with custom scroll handlers that shouldn't
//! be hijacked.

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Pull-to-refresh rule.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defpull-to-refresh"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRefreshSpec {
    pub name: String,
    /// Host glob. `"*"` = everywhere.
    #[serde(default = "default_host")]
    pub host: String,
    /// Pull distance in CSS pixels before firing. Clamped to `[40, 300]`.
    #[serde(default = "default_threshold")]
    pub threshold: u32,
    /// Command name (from `(defcommand)`) invoked on release.
    /// Typical values: `reload`, `history:back`, `history:forward`,
    /// `navigate:<url>`, a user-defined command.
    #[serde(default = "default_command")]
    pub command: String,
    /// Indicator animation duration in milliseconds. `0` = no animation.
    #[serde(default = "default_animation_ms")]
    pub animation_ms: u32,
    /// Runtime toggle.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_threshold() -> u32 {
    80
}
fn default_command() -> String {
    "reload".into()
}
fn default_animation_ms() -> u32 {
    250
}
fn default_enabled() -> bool {
    true
}

const MIN_THRESHOLD: u32 = 40;
const MAX_THRESHOLD: u32 = 300;

impl PullRefreshSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            threshold: 80,
            command: "reload".into(),
            animation_ms: 250,
            enabled: true,
            description: Some("Default pull-to-refresh — 80px, reload.".into()),
        }
    }

    #[must_use]
    pub fn clamped_threshold(&self) -> u32 {
        self.threshold.clamp(MIN_THRESHOLD, MAX_THRESHOLD)
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }
}

/// Registry. Host-specific wins over wildcard.
#[derive(Debug, Clone, Default)]
pub struct PullRefreshRegistry {
    specs: Vec<PullRefreshSpec>,
}

impl PullRefreshRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: PullRefreshSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = PullRefreshSpec>) {
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
    pub fn specs(&self) -> &[PullRefreshSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&PullRefreshSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.matches_host(host)))
    }

    /// Enabled + matching command for a host. Returns `None` if the
    /// rule is disabled or no rule matches.
    #[must_use]
    pub fn command_for(&self, host: &str) -> Option<String> {
        self.resolve(host)
            .filter(|s| s.enabled)
            .map(|s| s.command.clone())
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<PullRefreshSpec>, String> {
    tatara_lisp::compile_typed::<PullRefreshSpec>(src)
        .map_err(|e| format!("failed to compile defpull-to-refresh forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<PullRefreshSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamped_threshold_respects_bounds() {
        let too_low = PullRefreshSpec {
            threshold: 10,
            ..PullRefreshSpec::default_profile()
        };
        assert_eq!(too_low.clamped_threshold(), MIN_THRESHOLD);
        let too_high = PullRefreshSpec {
            threshold: 9999,
            ..PullRefreshSpec::default_profile()
        };
        assert_eq!(too_high.clamped_threshold(), MAX_THRESHOLD);
        let ok = PullRefreshSpec {
            threshold: 100,
            ..PullRefreshSpec::default_profile()
        };
        assert_eq!(ok.clamped_threshold(), 100);
    }

    #[test]
    fn wildcard_matches_everything() {
        assert!(PullRefreshSpec::default_profile().matches_host("anywhere.com"));
    }

    #[test]
    fn glob_matches_subdomain() {
        let s = PullRefreshSpec {
            host: "*://*.ycombinator.com/*".into(),
            ..PullRefreshSpec::default_profile()
        };
        assert!(s.matches_host("news.ycombinator.com"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn registry_dedupes_and_resolves_specific_over_wildcard() {
        let mut reg = PullRefreshRegistry::new();
        reg.insert(PullRefreshSpec::default_profile());
        reg.insert(PullRefreshSpec {
            name: "yc".into(),
            host: "*://*.ycombinator.com/*".into(),
            command: "history:back".into(),
            ..PullRefreshSpec::default_profile()
        });
        let yc = reg.resolve("news.ycombinator.com").unwrap();
        assert_eq!(yc.name, "yc");
        let default = reg.resolve("example.com").unwrap();
        assert_eq!(default.name, "default");
    }

    #[test]
    fn command_for_returns_none_when_disabled() {
        let mut reg = PullRefreshRegistry::new();
        reg.insert(PullRefreshSpec {
            enabled: false,
            ..PullRefreshSpec::default_profile()
        });
        assert!(reg.command_for("anywhere.com").is_none());
    }

    #[test]
    fn command_for_returns_some_when_enabled() {
        let mut reg = PullRefreshRegistry::new();
        reg.insert(PullRefreshSpec::default_profile());
        assert_eq!(reg.command_for("example.com").as_deref(), Some("reload"));
    }

    #[test]
    fn default_command_is_reload() {
        assert_eq!(PullRefreshSpec::default_profile().command, "reload");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_ptr_form() {
        let src = r#"
            (defpull-to-refresh :name      "yc"
                                :host      "*://*.ycombinator.com/*"
                                :threshold 120
                                :command   "history:back"
                                :animation-ms 150)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "yc");
        assert_eq!(s.threshold, 120);
        assert_eq!(s.command, "history:back");
        assert_eq!(s.animation_ms, 150);
    }
}
