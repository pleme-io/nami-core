//! `(defauth-saver)` — capture new logins for the password vault.
//!
//! Absorbs the "save this password?" prompt every browser shows after
//! a successful form submission. Authored as a profile: which vault
//! to save into, which hosts are in-scope, what selectors hint at
//! login vs signup vs change-password, and how aggressively to
//! deduplicate.
//!
//! ```lisp
//! (defauth-saver :name      "primary"
//!                :vault     "primary"
//!                :host      "*"
//!                :prompt    :always
//!                :detection (:username-selectors ("input[type=email]"
//!                                                 "input[name=login]"
//!                                                 "input[autocomplete=username]")
//!                            :password-selectors ("input[type=password]"))
//!                :ignore-hosts ("*://*.bank.com/*"))
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// How aggressively to prompt on a new credential.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PromptPolicy {
    /// Always ask the user before saving.
    Always,
    /// Save silently when the form is on a host in the allow-list;
    /// ask otherwise.
    SilentAllowList,
    /// Never save automatically — only manual adds via the vault UI.
    Never,
}

impl Default for PromptPolicy {
    fn default() -> Self {
        Self::Always
    }
}

/// Detection heuristic — the selectors authors can tune per-site
/// when the defaults don't pick up the login form correctly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DetectionHints {
    #[serde(default = "default_username_selectors")]
    pub username_selectors: Vec<String>,
    #[serde(default = "default_password_selectors")]
    pub password_selectors: Vec<String>,
    /// Hints that the form is signup (create a new account) rather
    /// than login — the saver offers "create with generated password"
    /// instead of "save entered password".
    #[serde(default = "default_signup_hints")]
    pub signup_hints: Vec<String>,
    /// Hints that the form is a change-password flow.
    #[serde(default = "default_change_password_hints")]
    pub change_password_hints: Vec<String>,
}

fn default_username_selectors() -> Vec<String> {
    vec![
        "input[type=email]".into(),
        "input[autocomplete=username]".into(),
        "input[autocomplete=email]".into(),
        "input[name=login]".into(),
        "input[name=username]".into(),
        "input[name=user]".into(),
        "input[id=user]".into(),
        "input[id=email]".into(),
    ]
}

fn default_password_selectors() -> Vec<String> {
    vec![
        "input[type=password]".into(),
        "input[autocomplete=current-password]".into(),
        "input[autocomplete=new-password]".into(),
    ]
}

fn default_signup_hints() -> Vec<String> {
    vec![
        "[action*=signup]".into(),
        "[action*=register]".into(),
        "input[autocomplete=new-password]".into(),
    ]
}

fn default_change_password_hints() -> Vec<String> {
    vec![
        "[action*=password]".into(),
        "input[autocomplete=current-password][autocomplete=new-password]".into(),
    ]
}

impl Default for DetectionHints {
    fn default() -> Self {
        Self {
            username_selectors: default_username_selectors(),
            password_selectors: default_password_selectors(),
            signup_hints: default_signup_hints(),
            change_password_hints: default_change_password_hints(),
        }
    }
}

/// Save-on-submit profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defauth-saver"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthSaverSpec {
    pub name: String,
    /// Name of the `(defpasswords)` vault to save into.
    pub vault: String,
    /// Host glob the profile applies to. `"*"` everywhere.
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    #[serde(default)]
    pub prompt: PromptPolicy,
    #[serde(default)]
    pub detection: DetectionHints,
    /// Hosts to ignore — save prompts never fire here even when a
    /// credential was submitted (prevents banking/medical leaks to
    /// whichever local vault is default).
    #[serde(default)]
    pub ignore_hosts: Vec<String>,
    /// Deduplicate on save — skip when vault already has
    /// (host, username) with the same password.
    #[serde(default = "default_deduplicate")]
    pub deduplicate: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_deduplicate() -> bool {
    true
}

/// A captured login candidate — what the saver feeds to the vault.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapturedCredential {
    pub url: String,
    pub username: String,
    pub password: String,
    /// `login` | `signup` | `change`.
    pub flow: String,
    /// Unix seconds when the submission fired.
    pub captured_at: i64,
}

impl AuthSaverSpec {
    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    /// True when the host is in `ignore_hosts` — save must not fire.
    #[must_use]
    pub fn is_ignored(&self, host: &str) -> bool {
        self.ignore_hosts
            .iter()
            .any(|g| crate::extension::glob_match_host(g, host))
    }

    /// Does this profile fire on a submission from `host`?
    #[must_use]
    pub fn fires_on(&self, host: &str) -> bool {
        self.matches_host(host) && !self.is_ignored(host)
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct AuthSaverRegistry {
    specs: Vec<AuthSaverSpec>,
}

impl AuthSaverRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: AuthSaverSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = AuthSaverSpec>) {
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
    pub fn specs(&self) -> &[AuthSaverSpec] {
        &self.specs
    }

    /// Most-specific host match wins, ignoring host-blocked profiles.
    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&AuthSaverSpec> {
        let specific = self.specs.iter().find(|s| {
            !s.host.is_empty() && s.host != "*" && s.fires_on(host)
        });
        specific.or_else(|| self.specs.iter().find(|s| s.fires_on(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<AuthSaverSpec>, String> {
    tatara_lisp::compile_typed::<AuthSaverSpec>(src)
        .map_err(|e| format!("failed to compile defauth-saver forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<AuthSaverSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AuthSaverSpec {
        AuthSaverSpec {
            name: "primary".into(),
            vault: "primary".into(),
            host: "*".into(),
            prompt: PromptPolicy::Always,
            detection: DetectionHints::default(),
            ignore_hosts: vec!["*://*.bank.com/*".into()],
            deduplicate: true,
            description: None,
        }
    }

    #[test]
    fn default_hints_cover_common_selectors() {
        let h = DetectionHints::default();
        assert!(h
            .username_selectors
            .iter()
            .any(|s| s == "input[type=email]"));
        assert!(h
            .password_selectors
            .iter()
            .any(|s| s == "input[type=password]"));
    }

    #[test]
    fn fires_on_respects_ignore_list() {
        let s = sample();
        assert!(s.fires_on("shop.example.com"));
        assert!(!s.fires_on("online.bank.com"));
    }

    #[test]
    fn matches_host_glob_filters_non_matching() {
        let s = AuthSaverSpec {
            host: "*://*.example.com/*".into(),
            ..sample()
        };
        assert!(s.fires_on("login.example.com"));
        assert!(!s.fires_on("evil.com"));
    }

    #[test]
    fn resolve_prefers_specific_over_wildcard() {
        let mut reg = AuthSaverRegistry::new();
        reg.insert(sample()); // wildcard
        reg.insert(AuthSaverSpec {
            name: "github".into(),
            host: "*://*.github.com/*".into(),
            vault: "work".into(),
            ..sample()
        });
        let gh = reg.resolve("login.github.com").unwrap();
        assert_eq!(gh.name, "github");
        let other = reg.resolve("example.com").unwrap();
        assert_eq!(other.name, "primary");
    }

    #[test]
    fn resolve_skips_ignored_even_when_host_matches() {
        let reg = {
            let mut r = AuthSaverRegistry::new();
            r.insert(sample());
            r
        };
        assert!(reg.resolve("online.bank.com").is_none());
    }

    #[test]
    fn prompt_policy_roundtrips_through_serde() {
        for p in [
            PromptPolicy::Always,
            PromptPolicy::SilentAllowList,
            PromptPolicy::Never,
        ] {
            let s = AuthSaverSpec {
                prompt: p,
                ..sample()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: AuthSaverSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.prompt, p);
        }
    }

    #[test]
    fn captured_credential_roundtrips() {
        let c = CapturedCredential {
            url: "https://example.com/login".into(),
            username: "jane".into(),
            password: "hunter2".into(),
            flow: "login".into(),
            captured_at: 1_700_000_000,
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: CapturedCredential = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = AuthSaverRegistry::new();
        reg.insert(sample());
        reg.insert(AuthSaverSpec {
            deduplicate: false,
            ..sample()
        });
        assert_eq!(reg.len(), 1);
        assert!(!reg.specs()[0].deduplicate);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_auth_saver_form() {
        let src = r#"
            (defauth-saver :name   "primary"
                           :vault  "primary"
                           :host   "*"
                           :prompt "silent-allow-list"
                           :ignore-hosts ("*://*.bank.com/*")
                           :deduplicate #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "primary");
        assert_eq!(s.prompt, PromptPolicy::SilentAllowList);
        assert_eq!(s.ignore_hosts.len(), 1);
    }
}
