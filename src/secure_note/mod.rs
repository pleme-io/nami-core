//! `(defsecure-note)` — non-password secret storage.
//!
//! Absorbs 1Password Secure Notes, Bitwarden "Secure Note" items,
//! macOS Keychain "Generic Password" / "Secure Note" entries,
//! pass's freeform files. Scope covers SSH keys, API tokens,
//! recovery codes, license keys, security-question answers,
//! encrypted journal entries — anything that doesn't fit the
//! `(username, password, url)` shape.
//!
//! ```lisp
//! (defsecure-note :name        "ssh-keys"
//!                 :vault       "primary"
//!                 :kind        :ssh-key
//!                 :storage     "secure-notes"
//!                 :tags        ("infra" "github")
//!                 :expose-via-cli #t)
//!
//! (defsecure-note :name        "api-tokens"
//!                 :vault       "primary"
//!                 :kind        :api-token
//!                 :storage     "secure-notes"
//!                 :auto-expire-seconds 2592000)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Category of secure note — drives UI grouping + integration hooks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NoteKind {
    /// Freeform encrypted text.
    Text,
    /// Markdown document (rendered as rich text on view).
    Markdown,
    /// SSH private/public key pair.
    SshKey,
    /// GPG/PGP key.
    GpgKey,
    /// API bearer token / OAuth access token.
    ApiToken,
    /// Database connection string + creds.
    DatabaseCredential,
    /// Software license key.
    LicenseKey,
    /// TOTP seed not attached to a specific login.
    TotpSeed,
    /// Recovery codes issued by a provider (2FA backup).
    RecoveryCodes,
    /// Security-question answers.
    SecurityQuestions,
    /// Credit-card / bank details outside a login context.
    PaymentCard,
    /// Personal-identity document (passport, driver's license).
    Identity,
    /// Cryptocurrency wallet seed phrase.
    WalletSeed,
}

/// Secure-note profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsecure-note"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecureNoteSpec {
    pub name: String,
    /// Owning `(defpasswords)` vault.
    pub vault: String,
    pub kind: NoteKind,
    /// (defstorage) namespace for index rows. Content itself lives
    /// encrypted in the vault; this index just tracks metadata.
    pub storage: String,
    /// Freeform tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Expose the note's content through the kagibako/CLI surface
    /// so scripts can read it (e.g. `kagi get api-tokens/github`).
    #[serde(default)]
    pub expose_via_cli: bool,
    /// Seconds until the note auto-expires (and alerts for rotation).
    /// `0` = no expiry.
    #[serde(default)]
    pub auto_expire_seconds: u64,
    /// Require explicit unlock (biometric / re-auth) every access,
    /// not just a valid vault session. Used for WalletSeed / Identity.
    #[serde(default = "default_always_reauth")]
    pub always_reauth: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_always_reauth() -> bool {
    false
}

impl SecureNoteSpec {
    /// True when this profile should force re-auth on every access —
    /// automatic for high-stakes kinds, or explicit via `always_reauth`.
    #[must_use]
    pub fn requires_reauth(&self) -> bool {
        self.always_reauth
            || matches!(
                self.kind,
                NoteKind::WalletSeed
                    | NoteKind::Identity
                    | NoteKind::PaymentCard
                    | NoteKind::RecoveryCodes
            )
    }

    /// Does the expiry apply and has it passed, given `age_seconds`?
    #[must_use]
    pub fn is_expired(&self, age_seconds: u64) -> bool {
        self.auto_expire_seconds > 0 && age_seconds > self.auto_expire_seconds
    }
}

/// One stored note.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecureNote {
    /// Stable id (BLAKE3 over intrinsic fields, 26-char base32).
    pub id: String,
    /// Display title.
    pub title: String,
    /// Body — only populated when vault is unlocked; empty otherwise
    /// to prevent leaks through logs.
    #[serde(default)]
    pub body: String,
    pub kind: NoteKind,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Unix seconds.
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    /// Optional URL association (e.g., Coinbase wallet page).
    #[serde(default)]
    pub url: Option<String>,
}

impl SecureNote {
    /// Return a copy with `body` cleared for logging safety.
    #[must_use]
    pub fn redacted(&self) -> Self {
        Self {
            body: String::new(),
            ..self.clone()
        }
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct SecureNoteRegistry {
    specs: Vec<SecureNoteSpec>,
}

impl SecureNoteRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SecureNoteSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SecureNoteSpec>) {
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
    pub fn specs(&self) -> &[SecureNoteSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&SecureNoteSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// Specs owned by `vault`.
    #[must_use]
    pub fn in_vault(&self, vault: &str) -> Vec<&SecureNoteSpec> {
        self.specs.iter().filter(|s| s.vault == vault).collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SecureNoteSpec>, String> {
    tatara_lisp::compile_typed::<SecureNoteSpec>(src)
        .map_err(|e| format!("failed to compile defsecure-note forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SecureNoteSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, kind: NoteKind) -> SecureNoteSpec {
        SecureNoteSpec {
            name: name.into(),
            vault: "primary".into(),
            kind,
            storage: "secure-notes".into(),
            tags: vec![],
            expose_via_cli: false,
            auto_expire_seconds: 0,
            always_reauth: false,
            description: None,
        }
    }

    #[test]
    fn requires_reauth_auto_triggers_for_sensitive_kinds() {
        assert!(sample("w", NoteKind::WalletSeed).requires_reauth());
        assert!(sample("i", NoteKind::Identity).requires_reauth());
        assert!(sample("c", NoteKind::PaymentCard).requires_reauth());
        assert!(sample("r", NoteKind::RecoveryCodes).requires_reauth());
        assert!(!sample("t", NoteKind::Text).requires_reauth());
    }

    #[test]
    fn requires_reauth_respects_explicit_opt_in() {
        let mut s = sample("t", NoteKind::Text);
        assert!(!s.requires_reauth());
        s.always_reauth = true;
        assert!(s.requires_reauth());
    }

    #[test]
    fn is_expired_honors_auto_expire_seconds() {
        let s = SecureNoteSpec {
            auto_expire_seconds: 100,
            ..sample("x", NoteKind::ApiToken)
        };
        assert!(!s.is_expired(99));
        assert!(s.is_expired(101));
    }

    #[test]
    fn is_expired_zero_means_never() {
        let s = sample("x", NoteKind::ApiToken);
        assert!(!s.is_expired(u64::MAX));
    }

    #[test]
    fn note_redacted_strips_body() {
        let n = SecureNote {
            id: "abc".into(),
            title: "t".into(),
            body: "secret".into(),
            kind: NoteKind::Text,
            tags: vec![],
            created_at: 0,
            updated_at: 0,
            url: None,
        };
        assert!(n.redacted().body.is_empty());
        assert_eq!(n.redacted().title, "t");
    }

    #[test]
    fn registry_dedupes_by_name_and_lists_by_vault() {
        let mut reg = SecureNoteRegistry::new();
        reg.insert(sample("a", NoteKind::ApiToken));
        reg.insert(SecureNoteSpec {
            vault: "work".into(),
            ..sample("b", NoteKind::SshKey)
        });
        assert_eq!(reg.in_vault("primary").len(), 1);
        assert_eq!(reg.in_vault("work").len(), 1);
    }

    #[test]
    fn note_kind_roundtrips_through_serde() {
        for k in [
            NoteKind::Text,
            NoteKind::Markdown,
            NoteKind::SshKey,
            NoteKind::ApiToken,
            NoteKind::RecoveryCodes,
            NoteKind::WalletSeed,
        ] {
            let s = sample("x", k);
            let json = serde_json::to_string(&s).unwrap();
            let back: SecureNoteSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.kind, k);
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_secure_note_form() {
        let src = r#"
            (defsecure-note :name    "ssh-keys"
                            :vault   "primary"
                            :kind    "ssh-key"
                            :storage "secure-notes"
                            :tags    ("infra" "github")
                            :expose-via-cli #t
                            :auto-expire-seconds 2592000)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.kind, NoteKind::SshKey);
        assert!(s.expose_via_cli);
        assert_eq!(s.auto_expire_seconds, 2_592_000);
    }
}
