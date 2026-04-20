//! `(defidentity)` — multi-account persona switcher.
//!
//! Absorbs Chrome Profiles, Firefox Containers, Arc Spaces
//! identities, Safari Profiles (macOS 14+), Microsoft Edge Work
//! Profiles. A persona bundles a display name, avatar, vault binding,
//! cookie jar, default identity credentials, and host auto-apply
//! rules so the same browser can seamlessly act as two different
//! users on GitHub/Google/Notion.
//!
//! Complements [`crate::passwords`] + [`crate::autofill`] — those
//! describe what credentials exist; identity says which persona is
//! active and how the browser should switch.
//!
//! ```lisp
//! (defidentity :name         "work"
//!              :display-name "Jane Doe (Work)"
//!              :avatar-url   "https://pleme.io/avatars/jane-work.png"
//!              :color        "#1a73e8"
//!              :vault        "work-vault"
//!              :cookie-jar   "work-jar"
//!              :default-email "jane@work.example.com"
//!              :auto-apply-hosts ("*://*.github.com/*" "*://*.notion.so/*")
//!              :isolation    :per-profile
//!              :default      #t)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// How strongly identities are isolated from each other.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityIsolation {
    /// Shared cookies/storage with every other identity (just a label).
    None,
    /// Separate cookie jar + storage — Firefox Containers style.
    #[default]
    PerProfile,
    /// Fully ephemeral — cookies cleared on identity switch.
    Ephemeral,
    /// OS-level isolation (Chrome Profile process) — separate
    /// renderer, separate disk.
    OsProcess,
}

/// Identity profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defidentity"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySpec {
    pub name: String,
    /// UI-shown name ("Jane Work"). Empty → falls back to `name`.
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
    /// Accent color (hex). Empty = UI picks from palette.
    #[serde(default)]
    pub color: Option<String>,
    /// Name of a `(defpasswords)` vault to pull credentials from.
    #[serde(default)]
    pub vault: Option<String>,
    /// Named cookie jar — crosses with `isolation` to partition
    /// session state.
    #[serde(default)]
    pub cookie_jar: Option<String>,
    /// Default email identity (for form autofill, git-config
    /// suggestions, …).
    #[serde(default)]
    pub default_email: Option<String>,
    /// Default display name for the persona — used on forms that
    /// ask for "your name".
    #[serde(default)]
    pub default_full_name: Option<String>,
    /// Host-glob list; navigating to a matching host auto-switches
    /// to this identity.
    #[serde(default)]
    pub auto_apply_hosts: Vec<String>,
    #[serde(default)]
    pub isolation: IdentityIsolation,
    /// The active-at-startup identity when multiple are declared.
    #[serde(default)]
    pub default: bool,
    /// TOTP profile names this identity is associated with.
    #[serde(default)]
    pub totp_profiles: Vec<String>,
    /// Priority — higher wins when two identities match the same host.
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_enabled() -> bool {
    true
}

impl IdentitySpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "personal".into(),
            display_name: "Personal".into(),
            avatar_url: None,
            color: Some("#6366f1".into()),
            vault: Some("primary".into()),
            cookie_jar: None,
            default_email: None,
            default_full_name: None,
            auto_apply_hosts: vec![],
            isolation: IdentityIsolation::PerProfile,
            default: true,
            totp_profiles: vec![],
            priority: 0,
            enabled: true,
            description: Some("Default personal identity — per-profile isolation.".into()),
        }
    }

    /// UI label — `display_name` when set, else `name`.
    #[must_use]
    pub fn label(&self) -> &str {
        if self.display_name.is_empty() {
            &self.name
        } else {
            &self.display_name
        }
    }

    /// Does `host` fall under this identity's auto-apply list?
    #[must_use]
    pub fn applies_to(&self, host: &str) -> bool {
        self.auto_apply_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct IdentityRegistry {
    specs: Vec<IdentitySpec>,
}

impl IdentityRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: IdentitySpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = IdentitySpec>) {
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
    pub fn specs(&self) -> &[IdentitySpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&IdentitySpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// Active identity for `host` — highest-priority enabled identity
    /// whose auto-apply-hosts contain it, else the default, else the
    /// first enabled.
    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&IdentitySpec> {
        let matched = self
            .specs
            .iter()
            .filter(|s| s.enabled && s.applies_to(host))
            .max_by_key(|s| s.priority);
        matched
            .or_else(|| {
                self.specs
                    .iter()
                    .filter(|s| s.enabled && s.default)
                    .max_by_key(|s| s.priority)
            })
            .or_else(|| self.specs.iter().find(|s| s.enabled))
    }

    /// All identities the named vault feeds.
    #[must_use]
    pub fn for_vault(&self, vault: &str) -> Vec<&IdentitySpec> {
        self.specs
            .iter()
            .filter(|s| s.enabled && s.vault.as_deref() == Some(vault))
            .collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<IdentitySpec>, String> {
    tatara_lisp::compile_typed::<IdentitySpec>(src)
        .map_err(|e| format!("failed to compile defidentity forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<IdentitySpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_personal_with_vault() {
        let s = IdentitySpec::default_profile();
        assert_eq!(s.name, "personal");
        assert_eq!(s.vault.as_deref(), Some("primary"));
        assert!(s.default);
        assert_eq!(s.isolation, IdentityIsolation::PerProfile);
    }

    #[test]
    fn label_falls_back_to_name_when_display_empty() {
        let mut s = IdentitySpec::default_profile();
        s.display_name = String::new();
        assert_eq!(s.label(), s.name);
    }

    #[test]
    fn label_uses_display_name_when_set() {
        let s = IdentitySpec {
            display_name: "Jane Work".into(),
            ..IdentitySpec::default_profile()
        };
        assert_eq!(s.label(), "Jane Work");
    }

    #[test]
    fn applies_to_host_glob() {
        let s = IdentitySpec {
            auto_apply_hosts: vec!["*://*.github.com/*".into()],
            ..IdentitySpec::default_profile()
        };
        assert!(s.applies_to("www.github.com"));
        assert!(!s.applies_to("evil.com"));
    }

    #[test]
    fn resolve_prefers_auto_apply_match_by_priority() {
        let mut reg = IdentityRegistry::new();
        reg.insert(IdentitySpec {
            name: "personal".into(),
            default: true,
            ..IdentitySpec::default_profile()
        });
        reg.insert(IdentitySpec {
            name: "work".into(),
            default: false,
            auto_apply_hosts: vec!["*://*.github.com/*".into()],
            priority: 10,
            ..IdentitySpec::default_profile()
        });
        assert_eq!(reg.resolve("www.github.com").unwrap().name, "work");
    }

    #[test]
    fn resolve_falls_back_to_default_when_no_match() {
        let mut reg = IdentityRegistry::new();
        reg.insert(IdentitySpec {
            name: "personal".into(),
            default: true,
            ..IdentitySpec::default_profile()
        });
        reg.insert(IdentitySpec {
            name: "work".into(),
            default: false,
            auto_apply_hosts: vec!["*://*.github.com/*".into()],
            ..IdentitySpec::default_profile()
        });
        assert_eq!(reg.resolve("example.org").unwrap().name, "personal");
    }

    #[test]
    fn resolve_picks_default_by_priority_tiebreak() {
        let mut reg = IdentityRegistry::new();
        reg.insert(IdentitySpec {
            name: "a".into(),
            default: true,
            priority: 0,
            ..IdentitySpec::default_profile()
        });
        reg.insert(IdentitySpec {
            name: "b".into(),
            default: true,
            priority: 20,
            ..IdentitySpec::default_profile()
        });
        assert_eq!(reg.resolve("example.org").unwrap().name, "b");
    }

    #[test]
    fn resolve_falls_back_to_first_enabled_when_no_default() {
        let mut reg = IdentityRegistry::new();
        reg.insert(IdentitySpec {
            name: "a".into(),
            default: false,
            ..IdentitySpec::default_profile()
        });
        assert_eq!(reg.resolve("example.org").unwrap().name, "a");
    }

    #[test]
    fn for_vault_filters_by_vault_binding() {
        let mut reg = IdentityRegistry::new();
        reg.insert(IdentitySpec {
            name: "personal".into(),
            vault: Some("v1".into()),
            ..IdentitySpec::default_profile()
        });
        reg.insert(IdentitySpec {
            name: "work".into(),
            vault: Some("v2".into()),
            ..IdentitySpec::default_profile()
        });
        assert_eq!(reg.for_vault("v1").len(), 1);
        assert_eq!(reg.for_vault("v1")[0].name, "personal");
    }

    #[test]
    fn isolation_roundtrips_through_serde() {
        for iso in [
            IdentityIsolation::None,
            IdentityIsolation::PerProfile,
            IdentityIsolation::Ephemeral,
            IdentityIsolation::OsProcess,
        ] {
            let s = IdentitySpec {
                isolation: iso,
                ..IdentitySpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: IdentitySpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.isolation, iso);
        }
    }

    #[test]
    fn disabled_identity_never_resolves() {
        let mut reg = IdentityRegistry::new();
        reg.insert(IdentitySpec {
            enabled: false,
            default: true,
            ..IdentitySpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_identity_form() {
        let src = r##"
            (defidentity :name "work"
                         :display-name "Jane Work"
                         :color "#1a73e8"
                         :vault "work-vault"
                         :cookie-jar "work-jar"
                         :default-email "jane@work.example.com"
                         :auto-apply-hosts ("*://*.github.com/*" "*://*.notion.so/*")
                         :isolation "per-profile"
                         :default #t
                         :priority 10
                         :totp-profiles ("github-2fa" "notion-2fa"))
        "##;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "work");
        assert_eq!(s.display_name, "Jane Work");
        assert_eq!(s.cookie_jar.as_deref(), Some("work-jar"));
        assert_eq!(s.auto_apply_hosts.len(), 2);
        assert_eq!(s.totp_profiles.len(), 2);
        assert_eq!(s.priority, 10);
    }
}
