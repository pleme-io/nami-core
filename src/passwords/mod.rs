//! `(defpasswords)` — declarative password-vault source.
//!
//! Absorbs Chrome Password Manager, Firefox Lockwise, Safari Keychain,
//! Bitwarden, 1Password, KeePass, and pleme-io's kagibako (primary
//! target). Each spec names a vault source — Local (encrypted blob),
//! Kagibako (native CLI bridge), Bitwarden (remote API), KeePass
//! (KDBX file), or a custom Process backend — plus lookup policies
//! and the set of hosts it can unlock for.
//!
//! ```lisp
//! (defpasswords :name    "primary"
//!               :source  :kagibako
//!               :vault-path "~/.config/kagibako/vault.json"
//!               :unlock-timeout-seconds 900
//!               :require-biometrics #t
//!               :auto-fill-hosts ("*://*.example.com/*"))
//!
//! (defpasswords :name    "bw-personal"
//!               :source  :bitwarden
//!               :endpoint "https://vault.bitwarden.com"
//!               :sync-interval-seconds 3600)
//!
//! (defpasswords :name    "legacy"
//!               :source  :keepass
//!               :vault-path "~/vault.kdbx")
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Vault source kind. Covers the known password-manager landscape —
/// native Rust (Local, Kagibako), CLI bridges (every mainstream
/// commercial manager), file formats (KDBX, pass), OS-level
/// credential stores (Keychain, Credential Manager, libsecret), and
/// cloud secret stores that double as personal vaults (Akeyless,
/// HashiCorp Vault, AWS Secrets Manager, GCP Secret Manager, Azure
/// Key Vault). When nothing fits, `Process` shells out to any
/// command that emits JSON.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VaultSource {
    // ── Native + pleme-io ──
    /// Local AEAD-sealed store under a user passphrase.
    Local,
    /// pleme-io kagibako — native CLI bridge, Nix-installable.
    Kagibako,

    // ── Commercial managers (CLI bridges) ──
    /// 1Password (CLI `op`).
    OnePassword,
    /// Bitwarden / Vaultwarden (CLI `bw` or web API).
    Bitwarden,
    /// LastPass (CLI `lpass`).
    LastPass,
    /// Dashlane (CLI `dashlane`).
    Dashlane,
    /// NordPass (CLI `nordpass`).
    NordPass,
    /// Proton Pass (CLI `protonpass` / API).
    ProtonPass,
    /// Enpass (CLI `enpass`).
    Enpass,
    /// RoboForm (CLI `roboform`).
    RoboForm,
    /// Zoho Vault (API).
    ZohoVault,

    // ── File formats ──
    /// KeePass KDBX file on disk.
    Keepass,
    /// UNIX `pass` password store — GPG-encrypted file-per-entry
    /// tree under `~/.password-store`.
    Pass,
    /// `gopass` — Go rewrite of pass, same on-disk format.
    Gopass,

    // ── OS-level credential stores ──
    /// macOS Keychain via `security` CLI.
    MacOsKeychain,
    /// Windows Credential Manager via `cmdkey` / `cred`.
    WindowsCredentialManager,
    /// GNOME Keyring / KWallet / anything speaking the FreeDesktop
    /// Secret Service API (`libsecret`).
    LibSecret,

    // ── Cloud secret stores ──
    /// HashiCorp Vault (server-based).
    HashiCorpVault,
    /// pleme-io akeyless-api + akeyless-nix integration.
    Akeyless,
    /// AWS Secrets Manager.
    AwsSecretsManager,
    /// GCP Secret Manager.
    GcpSecretManager,
    /// Azure Key Vault.
    AzureKeyVault,

    // ── Fallback ──
    /// Custom subprocess returning JSON on stdout. Anything not
    /// covered above plugs in here.
    Process,
}

impl Default for VaultSource {
    fn default() -> Self {
        Self::Local
    }
}

/// Password vault declaration.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defpasswords"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PasswordsSpec {
    pub name: String,
    #[serde(default)]
    pub source: VaultSource,
    /// Path to local vault file (Local / KeePass / Kagibako).
    #[serde(default)]
    pub vault_path: Option<String>,
    /// Remote endpoint (Bitwarden).
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Subprocess command for Process backend.
    #[serde(default)]
    pub command: Option<Vec<String>>,
    /// Minutes of inactivity before the vault re-locks. `0` = never
    /// auto-lock (not recommended).
    #[serde(default = "default_unlock_timeout")]
    pub unlock_timeout_seconds: u64,
    /// Require platform biometric on unlock.
    #[serde(default = "default_require_biometrics")]
    pub require_biometrics: bool,
    /// How often to pull new entries from remote sources.
    #[serde(default = "default_sync_interval")]
    pub sync_interval_seconds: u64,
    /// Host patterns this vault auto-fills into. Empty = every host.
    #[serde(default)]
    pub auto_fill_hosts: Vec<String>,
    /// Host patterns that NEVER receive auto-fill from this vault
    /// (take precedence over auto_fill_hosts).
    #[serde(default)]
    pub blocked_hosts: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_unlock_timeout() -> u64 {
    900 // 15 minutes
}
fn default_require_biometrics() -> bool {
    true
}
fn default_sync_interval() -> u64 {
    3600 // 1 hour
}

/// One credential record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CredentialRecord {
    /// Display title.
    pub title: String,
    /// Username / email / account id.
    pub username: String,
    /// Secret — only populated after unlock. Empty when the vault is
    /// locked so serialization doesn't leak.
    #[serde(default)]
    pub password: String,
    /// URL the credential is registered for (matched via host glob).
    pub url: String,
    /// TOTP secret (base32) when the site supports it.
    #[serde(default)]
    pub totp_secret: Option<String>,
    /// Extra labeled fields (security questions, recovery codes).
    #[serde(default)]
    pub fields: Vec<(String, String)>,
    /// Unix seconds; `0` = unknown.
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

impl CredentialRecord {
    /// True if the credential applies to `host` via URL host-glob.
    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(&self.url) {
            if let Some(credential_host) = parsed.host_str() {
                // Exact match OR domain suffix (credentials on example.com
                // should also fill login.example.com).
                return host == credential_host
                    || host.ends_with(&format!(".{credential_host}"));
            }
        }
        false
    }

    /// A deep-redacted shape safe to log — password + totp stripped.
    #[must_use]
    pub fn redacted(&self) -> Self {
        Self {
            password: String::new(),
            totp_secret: None,
            ..self.clone()
        }
    }
}

impl PasswordsSpec {
    /// Structural validation — source-dependent required fields.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("passwords spec name is empty".into());
        }
        match self.source {
            // File-path sources.
            VaultSource::Local | VaultSource::Keepass => {
                if self.vault_path.is_none() {
                    return Err(format!(
                        "{:?} vault '{}' requires :vault-path",
                        self.source, self.name
                    ));
                }
            }

            // Remote endpoint sources.
            VaultSource::Bitwarden
            | VaultSource::ZohoVault
            | VaultSource::HashiCorpVault
            | VaultSource::Akeyless => {
                if self.endpoint.is_none() {
                    return Err(format!(
                        "{:?} vault '{}' requires :endpoint",
                        self.source, self.name
                    ));
                }
            }

            // Subprocess fallback.
            VaultSource::Process => {
                if self.command.as_ref().is_none_or(Vec::is_empty) {
                    return Err(format!(
                        "process vault '{}' requires :command",
                        self.name
                    ));
                }
            }

            // Everything else — CLI-bridge / OS-keychain / cloud secret
            // store — takes zero required spec fields. Credentials
            // resolve at lookup time through the backend's own auth
            // (native biometric, pre-configured SDK profile, etc.).
            VaultSource::Kagibako
            | VaultSource::OnePassword
            | VaultSource::LastPass
            | VaultSource::Dashlane
            | VaultSource::NordPass
            | VaultSource::ProtonPass
            | VaultSource::Enpass
            | VaultSource::RoboForm
            | VaultSource::Pass
            | VaultSource::Gopass
            | VaultSource::MacOsKeychain
            | VaultSource::WindowsCredentialManager
            | VaultSource::LibSecret
            | VaultSource::AwsSecretsManager
            | VaultSource::GcpSecretManager
            | VaultSource::AzureKeyVault => {}
        }
        Ok(())
    }

    /// Does this vault auto-fill into `host`?
    #[must_use]
    pub fn auto_fills_host(&self, host: &str) -> bool {
        if self
            .blocked_hosts
            .iter()
            .any(|g| crate::extension::glob_match_host(g, host))
        {
            return false;
        }
        if self.auto_fill_hosts.is_empty() {
            return true;
        }
        self.auto_fill_hosts
            .iter()
            .any(|g| crate::extension::glob_match_host(g, host))
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct PasswordsRegistry {
    specs: Vec<PasswordsSpec>,
}

impl PasswordsRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: PasswordsSpec) -> Result<(), String> {
        spec.validate()?;
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
        Ok(())
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = PasswordsSpec>) {
        for s in specs {
            if let Err(e) = self.insert(s.clone()) {
                tracing::warn!("defpasswords '{}' rejected: {}", s.name, e);
            }
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
    pub fn specs(&self) -> &[PasswordsSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&PasswordsSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// Every spec that auto-fills into `host` (respecting blocklists).
    #[must_use]
    pub fn applicable(&self, host: &str) -> Vec<&PasswordsSpec> {
        self.specs
            .iter()
            .filter(|s| s.auto_fills_host(host))
            .collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<PasswordsSpec>, String> {
    tatara_lisp::compile_typed::<PasswordsSpec>(src)
        .map_err(|e| format!("failed to compile defpasswords forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<PasswordsSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_kagibako() -> PasswordsSpec {
        PasswordsSpec {
            name: "primary".into(),
            source: VaultSource::Kagibako,
            vault_path: Some("~/.config/kagibako/vault.json".into()),
            endpoint: None,
            command: None,
            unlock_timeout_seconds: 900,
            require_biometrics: true,
            sync_interval_seconds: 3600,
            auto_fill_hosts: vec![],
            blocked_hosts: vec![],
            description: None,
        }
    }

    #[test]
    fn validate_local_requires_vault_path() {
        let s = PasswordsSpec {
            source: VaultSource::Local,
            vault_path: None,
            ..sample_kagibako()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_bitwarden_requires_endpoint() {
        let s = PasswordsSpec {
            source: VaultSource::Bitwarden,
            endpoint: None,
            ..sample_kagibako()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_process_requires_non_empty_command() {
        let s = PasswordsSpec {
            source: VaultSource::Process,
            command: Some(vec![]),
            ..sample_kagibako()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_kagibako_accepts_minimal_form() {
        assert!(sample_kagibako().validate().is_ok());
    }

    #[test]
    fn auto_fills_host_empty_list_is_all() {
        let s = sample_kagibako();
        assert!(s.auto_fills_host("anywhere.com"));
    }

    #[test]
    fn auto_fills_host_respects_allow_list() {
        let s = PasswordsSpec {
            auto_fill_hosts: vec!["*://*.example.com/*".into()],
            ..sample_kagibako()
        };
        assert!(s.auto_fills_host("shop.example.com"));
        assert!(!s.auto_fills_host("evil.com"));
    }

    #[test]
    fn blocked_hosts_take_precedence() {
        let s = PasswordsSpec {
            auto_fill_hosts: vec![],
            blocked_hosts: vec!["*://*.bank.com/*".into()],
            ..sample_kagibako()
        };
        assert!(!s.auto_fills_host("online.bank.com"));
        assert!(s.auto_fills_host("other.com"));
    }

    #[test]
    fn credential_matches_host_exact_and_subdomain() {
        let c = CredentialRecord {
            title: "Example".into(),
            username: "jane".into(),
            password: "secret".into(),
            url: "https://example.com".into(),
            totp_secret: None,
            fields: vec![],
            created_at: 0,
            updated_at: 0,
        };
        assert!(c.matches_host("example.com"));
        assert!(c.matches_host("login.example.com"));
        assert!(!c.matches_host("example.org"));
        assert!(!c.matches_host("notexample.com"));
    }

    #[test]
    fn redacted_strips_secrets() {
        let c = CredentialRecord {
            title: "t".into(),
            username: "u".into(),
            password: "p".into(),
            url: "https://example.com".into(),
            totp_secret: Some("TOTP".into()),
            fields: vec![],
            created_at: 0,
            updated_at: 0,
        };
        let r = c.redacted();
        assert!(r.password.is_empty());
        assert!(r.totp_secret.is_none());
        assert_eq!(r.username, "u");
    }

    #[test]
    fn registry_dedupes_by_name_and_applicable() {
        let mut reg = PasswordsRegistry::new();
        reg.insert(sample_kagibako()).unwrap();
        let banking = PasswordsSpec {
            name: "bank".into(),
            auto_fill_hosts: vec!["*://*.bank.com/*".into()],
            ..sample_kagibako()
        };
        reg.insert(banking).unwrap();
        assert_eq!(reg.len(), 2);
        let on_bank = reg.applicable("online.bank.com");
        assert_eq!(on_bank.len(), 2);
    }

    #[test]
    fn validate_hashicorp_vault_requires_endpoint() {
        let s = PasswordsSpec {
            source: VaultSource::HashiCorpVault,
            endpoint: None,
            ..sample_kagibako()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_akeyless_requires_endpoint() {
        let s = PasswordsSpec {
            source: VaultSource::Akeyless,
            endpoint: None,
            ..sample_kagibako()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_cli_bridge_sources_need_no_config() {
        // Every CLI-only source accepts a minimal form — lookup
        // auth lives in the backend's own SDK / keychain.
        for source in [
            VaultSource::OnePassword,
            VaultSource::LastPass,
            VaultSource::Dashlane,
            VaultSource::NordPass,
            VaultSource::ProtonPass,
            VaultSource::Enpass,
            VaultSource::RoboForm,
            VaultSource::Pass,
            VaultSource::Gopass,
            VaultSource::MacOsKeychain,
            VaultSource::WindowsCredentialManager,
            VaultSource::LibSecret,
            VaultSource::AwsSecretsManager,
            VaultSource::GcpSecretManager,
            VaultSource::AzureKeyVault,
        ] {
            let s = PasswordsSpec {
                source,
                vault_path: None,
                endpoint: None,
                ..sample_kagibako()
            };
            assert!(
                s.validate().is_ok(),
                "{source:?} should validate with minimal form"
            );
        }
    }

    #[test]
    fn registry_extend_drops_invalid() {
        let mut reg = PasswordsRegistry::new();
        reg.extend(vec![
            sample_kagibako(),
            PasswordsSpec {
                name: "bad".into(),
                source: VaultSource::Bitwarden,
                endpoint: None,
                ..sample_kagibako()
            },
        ]);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].name, "primary");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_passwords_form() {
        let src = r#"
            (defpasswords :name "primary"
                          :source "kagibako"
                          :vault-path "~/.config/kagibako/vault.json"
                          :unlock-timeout-seconds 600
                          :require-biometrics #t
                          :auto-fill-hosts ("*://*.example.com/*"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.source, VaultSource::Kagibako);
        assert_eq!(s.unlock_timeout_seconds, 600);
        assert!(s.require_biometrics);
    }
}
