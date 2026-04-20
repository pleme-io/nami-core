//! `(deftab-hibernate)` — declarative tab-hibernation policy.
//!
//! Absorbs Chrome Memory Saver, Edge Sleeping Tabs, Vivaldi Tab
//! Hibernation, Firefox Tab Unloading on low memory. Per-host
//! thresholds so audio/document tabs survive aggressive sleep while
//! feed-apps hibernate after 60 seconds.
//!
//! ```lisp
//! (deftab-hibernate :name               "aggressive-feeds"
//!                   :host               "*://*.twitter.com/*"
//!                   :inactive-seconds   60
//!                   :discard-state      :keep-scroll
//!                   :keep-loaded        #f
//!                   :keep-audio         #t
//!                   :keep-pinned        #t
//!                   :memory-pressure    :high)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// What to keep around when a tab hibernates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DiscardState {
    /// Free everything — reload from scratch when clicked.
    #[default]
    DiscardAll,
    /// Keep scroll position.
    KeepScroll,
    /// Keep scroll + form-field values.
    KeepForm,
    /// Keep a screenshot preview of the page.
    KeepScreenshot,
    /// Keep the full serialized DOM (most memory, least jank).
    KeepDom,
}

/// Minimum system memory pressure before a tab is eligible for
/// hibernation. Matches OS memory-pressure levels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryPressure {
    /// Only hibernate when pressure is at least "high".
    High,
    /// Hibernate at moderate pressure or above.
    #[default]
    Moderate,
    /// Hibernate regardless of pressure (purely time-based).
    Any,
    /// Never hibernate unless the time threshold is met AND the
    /// system is actually thrashing (critical pressure).
    Critical,
}

/// Hibernation profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "deftab-hibernate"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TabHibernateSpec {
    pub name: String,
    /// Host glob the profile fires on. `"*"` = everywhere.
    #[serde(default = "default_host")]
    pub host: String,
    /// Seconds of user inactivity on the tab before it's a candidate
    /// to hibernate. 0 = disabled.
    #[serde(default = "default_inactive_seconds")]
    pub inactive_seconds: u32,
    #[serde(default)]
    pub discard_state: DiscardState,
    /// Exempt list — hosts that should never hibernate.
    #[serde(default)]
    pub keep_loaded_hosts: Vec<String>,
    /// Never hibernate tabs currently emitting audio/video.
    #[serde(default = "default_keep_audio")]
    pub keep_audio: bool,
    /// Never hibernate pinned tabs.
    #[serde(default = "default_keep_pinned")]
    pub keep_pinned: bool,
    /// Never hibernate tabs with unsaved form input.
    #[serde(default = "default_keep_form_dirty")]
    pub keep_form_dirty: bool,
    /// Never hibernate tabs with an active upload/download.
    #[serde(default = "default_keep_transfer")]
    pub keep_active_transfer: bool,
    #[serde(default)]
    pub memory_pressure: MemoryPressure,
    /// Max bytes in-memory before eligibility (0 = any size).
    #[serde(default)]
    pub max_resident_bytes: u64,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_inactive_seconds() -> u32 {
    900 // 15 min — matches Chrome Memory Saver default.
}
fn default_keep_audio() -> bool {
    true
}
fn default_keep_pinned() -> bool {
    true
}
fn default_keep_form_dirty() -> bool {
    true
}
fn default_keep_transfer() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

/// Runtime snapshot of a tab that the hibernator needs to make its
/// decision. Callers fill this in from the real tab at policy-check
/// time.
#[derive(Debug, Clone, Default)]
pub struct TabSnapshot<'a> {
    pub host: &'a str,
    pub inactive_seconds: u32,
    pub pinned: bool,
    pub playing_audio: bool,
    pub form_dirty: bool,
    pub active_transfer: bool,
    pub resident_bytes: u64,
    pub pressure: MemoryPressure,
}

impl TabHibernateSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            inactive_seconds: default_inactive_seconds(),
            discard_state: DiscardState::KeepScroll,
            keep_loaded_hosts: vec![],
            keep_audio: true,
            keep_pinned: true,
            keep_form_dirty: true,
            keep_active_transfer: true,
            memory_pressure: MemoryPressure::Moderate,
            max_resident_bytes: 0,
            enabled: true,
            description: Some(
                "Default hibernation — 15 min inactivity, keeps scroll, respects audio/pinned/form.".into(),
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

    /// Is `host` explicitly exempt?
    #[must_use]
    pub fn is_keep_loaded(&self, host: &str) -> bool {
        self.keep_loaded_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    /// Policy check — should this tab be hibernated RIGHT NOW?
    #[must_use]
    pub fn should_hibernate(&self, tab: &TabSnapshot<'_>) -> bool {
        if !self.enabled {
            return false;
        }
        if self.inactive_seconds == 0 || tab.inactive_seconds < self.inactive_seconds {
            return false;
        }
        if self.is_keep_loaded(tab.host) {
            return false;
        }
        if self.keep_pinned && tab.pinned {
            return false;
        }
        if self.keep_audio && tab.playing_audio {
            return false;
        }
        if self.keep_form_dirty && tab.form_dirty {
            return false;
        }
        if self.keep_active_transfer && tab.active_transfer {
            return false;
        }
        if self.max_resident_bytes != 0 && tab.resident_bytes < self.max_resident_bytes {
            return false;
        }
        pressure_meets(self.memory_pressure, tab.pressure)
    }
}

fn pressure_meets(policy: MemoryPressure, actual: MemoryPressure) -> bool {
    fn rank(p: MemoryPressure) -> u8 {
        match p {
            MemoryPressure::Any => 0,
            MemoryPressure::Moderate => 1,
            MemoryPressure::High => 2,
            MemoryPressure::Critical => 3,
        }
    }
    rank(actual) >= rank(policy)
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct TabHibernateRegistry {
    specs: Vec<TabHibernateSpec>,
}

impl TabHibernateRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: TabHibernateSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = TabHibernateSpec>) {
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
    pub fn specs(&self) -> &[TabHibernateSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&TabHibernateSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<TabHibernateSpec>, String> {
    tatara_lisp::compile_typed::<TabHibernateSpec>(src)
        .map_err(|e| format!("failed to compile deftab-hibernate forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<TabHibernateSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab<'a>(host: &'a str, secs: u32) -> TabSnapshot<'a> {
        TabSnapshot {
            host,
            inactive_seconds: secs,
            pinned: false,
            playing_audio: false,
            form_dirty: false,
            active_transfer: false,
            resident_bytes: 0,
            pressure: MemoryPressure::Moderate,
        }
    }

    #[test]
    fn default_profile_is_15min_with_safety_rails() {
        let s = TabHibernateSpec::default_profile();
        assert_eq!(s.inactive_seconds, 900);
        assert!(s.keep_audio);
        assert!(s.keep_pinned);
        assert!(s.keep_form_dirty);
    }

    #[test]
    fn should_hibernate_after_inactive_threshold() {
        let s = TabHibernateSpec::default_profile();
        let t = tab("example.com", 1000);
        assert!(s.should_hibernate(&t));

        let short = tab("example.com", 100);
        assert!(!s.should_hibernate(&short));
    }

    #[test]
    fn audio_tab_is_never_hibernated_when_policy_opts_in() {
        let s = TabHibernateSpec::default_profile();
        let mut t = tab("example.com", 1000);
        t.playing_audio = true;
        assert!(!s.should_hibernate(&t));
    }

    #[test]
    fn pinned_tab_is_exempt() {
        let s = TabHibernateSpec::default_profile();
        let mut t = tab("example.com", 1000);
        t.pinned = true;
        assert!(!s.should_hibernate(&t));
    }

    #[test]
    fn form_dirty_tab_is_exempt() {
        let s = TabHibernateSpec::default_profile();
        let mut t = tab("example.com", 1000);
        t.form_dirty = true;
        assert!(!s.should_hibernate(&t));
    }

    #[test]
    fn keep_loaded_hosts_exempt_matching_hosts() {
        let s = TabHibernateSpec {
            keep_loaded_hosts: vec!["*://*.docs.com/*".into()],
            ..TabHibernateSpec::default_profile()
        };
        let t = tab("www.docs.com", 1000);
        assert!(!s.should_hibernate(&t));
    }

    #[test]
    fn memory_pressure_gating() {
        let high = TabHibernateSpec {
            memory_pressure: MemoryPressure::High,
            ..TabHibernateSpec::default_profile()
        };
        let mut t = tab("example.com", 1000);
        t.pressure = MemoryPressure::Moderate;
        assert!(!high.should_hibernate(&t));
        t.pressure = MemoryPressure::High;
        assert!(high.should_hibernate(&t));
    }

    #[test]
    fn max_resident_bytes_gating() {
        let s = TabHibernateSpec {
            max_resident_bytes: 1024 * 1024,
            ..TabHibernateSpec::default_profile()
        };
        let mut t = tab("example.com", 1000);
        t.resident_bytes = 500_000;
        // Below threshold — not eligible yet.
        assert!(!s.should_hibernate(&t));
        t.resident_bytes = 2_000_000;
        assert!(s.should_hibernate(&t));
    }

    #[test]
    fn disabled_profile_never_hibernates() {
        let s = TabHibernateSpec {
            enabled: false,
            ..TabHibernateSpec::default_profile()
        };
        let t = tab("example.com", 1_000_000);
        assert!(!s.should_hibernate(&t));
    }

    #[test]
    fn inactive_seconds_zero_disables() {
        let s = TabHibernateSpec {
            inactive_seconds: 0,
            ..TabHibernateSpec::default_profile()
        };
        let t = tab("example.com", 1_000_000);
        assert!(!s.should_hibernate(&t));
    }

    #[test]
    fn discard_state_roundtrips_through_serde() {
        for st in [
            DiscardState::DiscardAll,
            DiscardState::KeepScroll,
            DiscardState::KeepForm,
            DiscardState::KeepScreenshot,
            DiscardState::KeepDom,
        ] {
            let s = TabHibernateSpec {
                discard_state: st,
                ..TabHibernateSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: TabHibernateSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.discard_state, st);
        }
    }

    #[test]
    fn memory_pressure_roundtrips_through_serde() {
        for p in [
            MemoryPressure::Any,
            MemoryPressure::Moderate,
            MemoryPressure::High,
            MemoryPressure::Critical,
        ] {
            let s = TabHibernateSpec {
                memory_pressure: p,
                ..TabHibernateSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: TabHibernateSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.memory_pressure, p);
        }
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = TabHibernateRegistry::new();
        reg.insert(TabHibernateSpec::default_profile());
        reg.insert(TabHibernateSpec {
            name: "twitter".into(),
            host: "*://*.twitter.com/*".into(),
            inactive_seconds: 60,
            ..TabHibernateSpec::default_profile()
        });
        let tw = reg.resolve("www.twitter.com").unwrap();
        assert_eq!(tw.inactive_seconds, 60);
        let other = reg.resolve("example.org").unwrap();
        assert_eq!(other.name, "default");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_hibernate_form() {
        let src = r#"
            (deftab-hibernate :name "aggressive-feeds"
                              :host "*://*.twitter.com/*"
                              :inactive-seconds 60
                              :discard-state "keep-scroll"
                              :keep-audio #t
                              :keep-pinned #t
                              :memory-pressure "high")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "aggressive-feeds");
        assert_eq!(s.inactive_seconds, 60);
        assert_eq!(s.memory_pressure, MemoryPressure::High);
        assert_eq!(s.discard_state, DiscardState::KeepScroll);
    }
}
