//! `(defpasskey)` — WebAuthn / FIDO2 passkey profile.
//!
//! Absorbs iCloud Keychain Passkeys, Android Credential Manager
//! Passkeys, 1Password/Bitwarden Passkeys, Windows Hello platform
//! authenticator, YubiKey + security-key flows. A profile declares
//! the target vault, acceptable authenticator kinds (platform vs
//! cross-platform), user-verification policy, and relying-party
//! scoping.
//!
//! ```lisp
//! (defpasskey :name            "primary"
//!             :vault           "primary"
//!             :authenticator   :any
//!             :user-verification :required
//!             :sync-passkeys   #t
//!             :allowed-rp-ids  ("example.com" "github.com"))
//!
//! (defpasskey :name            "hardware-only"
//!             :vault           "primary"
//!             :authenticator   :cross-platform
//!             :user-verification :required
//!             :sync-passkeys   #f)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Which authenticators the profile accepts. Mirrors the WebAuthn
/// `authenticatorAttachment` enum plus a convenience `Any`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Authenticator {
    /// Any — platform OR cross-platform.
    Any,
    /// Platform authenticator (Touch ID, Face ID, Windows Hello,
    /// Android screen-lock).
    Platform,
    /// Cross-platform (USB / NFC / BLE security keys like YubiKey).
    CrossPlatform,
}

impl Default for Authenticator {
    fn default() -> Self {
        Self::Any
    }
}

/// WebAuthn user-verification policy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UserVerification {
    /// Require UV (biometric / PIN) every call.
    Required,
    /// Prefer UV but accept when unavailable.
    Preferred,
    /// Never request UV — fastest but lowest assurance.
    Discouraged,
}

impl Default for UserVerification {
    fn default() -> Self {
        Self::Preferred
    }
}

/// Passkey profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defpasskey"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PasskeySpec {
    pub name: String,
    /// Owning `(defpasswords)` vault — passkeys share storage with
    /// passwords for unified lifecycle.
    pub vault: String,
    #[serde(default)]
    pub authenticator: Authenticator,
    #[serde(default)]
    pub user_verification: UserVerification,
    /// Sync passkeys via the vault's transport (iCloud, Kagibako
    /// sync, etc.). Hardware-only profiles set this to `false`.
    #[serde(default = "default_sync_passkeys")]
    pub sync_passkeys: bool,
    /// When non-empty, only these Relying Party IDs may use the
    /// profile. `["*"]` or empty = any RP.
    #[serde(default)]
    pub allowed_rp_ids: Vec<String>,
    /// Specific RP IDs to exclude (e.g., ban `evil-phish.com`).
    #[serde(default)]
    pub blocked_rp_ids: Vec<String>,
    /// Resident key (discoverable credential) preference. `true` =
    /// require resident — lets accounts appear in the credential
    /// picker without the site sending an `allowCredentials` list.
    #[serde(default = "default_resident_key")]
    pub resident_key: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_sync_passkeys() -> bool {
    true
}
fn default_resident_key() -> bool {
    true
}

/// A stored passkey.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyRecord {
    /// Credential id (base64-url encoded).
    pub credential_id: String,
    /// Relying Party ID (domain the key is bound to).
    pub rp_id: String,
    /// RP display name.
    pub rp_name: String,
    /// User-handle bytes (base64-url encoded).
    pub user_handle: String,
    /// User display name shown in the chooser.
    pub user_name: String,
    /// Algorithm — COSE identifier (e.g. -7 = ES256).
    pub algorithm: i32,
    pub authenticator: Authenticator,
    pub user_verification: UserVerification,
    /// Public key bytes (CBOR, base64-url encoded).
    pub public_key: String,
    /// Unix seconds.
    pub created_at: i64,
    #[serde(default)]
    pub last_used_at: i64,
    /// Signature counter — monotonically increases to detect cloning.
    #[serde(default)]
    pub sign_count: u32,
}

impl PasskeyRecord {
    /// Sanitized copy — strips public key + user handle for logging.
    #[must_use]
    pub fn redacted(&self) -> Self {
        Self {
            public_key: String::new(),
            user_handle: String::new(),
            ..self.clone()
        }
    }
}

impl PasskeySpec {
    /// Does this profile permit `rp_id`?
    #[must_use]
    pub fn allows_rp(&self, rp_id: &str) -> bool {
        // Block-list wins first.
        if self.blocked_rp_ids.iter().any(|r| r == rp_id) {
            return false;
        }
        // Empty allow-list OR wildcard means "allow any unblocked".
        if self.allowed_rp_ids.is_empty()
            || self.allowed_rp_ids.iter().any(|r| r == "*")
        {
            return true;
        }
        // Exact match OR domain-suffix match.
        self.allowed_rp_ids.iter().any(|allowed| {
            allowed == rp_id || rp_id.ends_with(&format!(".{allowed}"))
        })
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct PasskeyRegistry {
    specs: Vec<PasskeySpec>,
}

impl PasskeyRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: PasskeySpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = PasskeySpec>) {
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
    pub fn specs(&self) -> &[PasskeySpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&PasskeySpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// Every profile that permits `rp_id` — passkey chooser UI
    /// sources its list from here.
    #[must_use]
    pub fn applicable(&self, rp_id: &str) -> Vec<&PasskeySpec> {
        self.specs.iter().filter(|s| s.allows_rp(rp_id)).collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<PasskeySpec>, String> {
    tatara_lisp::compile_typed::<PasskeySpec>(src)
        .map_err(|e| format!("failed to compile defpasskey forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<PasskeySpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PasskeySpec {
        PasskeySpec {
            name: "primary".into(),
            vault: "primary".into(),
            authenticator: Authenticator::Any,
            user_verification: UserVerification::Required,
            sync_passkeys: true,
            allowed_rp_ids: vec!["example.com".into(), "github.com".into()],
            blocked_rp_ids: vec![],
            resident_key: true,
            description: None,
        }
    }

    #[test]
    fn allows_rp_exact_match() {
        let s = sample();
        assert!(s.allows_rp("example.com"));
        assert!(s.allows_rp("github.com"));
    }

    #[test]
    fn allows_rp_subdomain_match() {
        let s = sample();
        assert!(s.allows_rp("login.github.com"));
    }

    #[test]
    fn allows_rp_rejects_non_listed() {
        let s = sample();
        assert!(!s.allows_rp("evil.com"));
    }

    #[test]
    fn empty_allow_list_permits_any_unblocked() {
        let s = PasskeySpec {
            allowed_rp_ids: vec![],
            ..sample()
        };
        assert!(s.allows_rp("anything.com"));
    }

    #[test]
    fn wildcard_permits_any_unblocked() {
        let s = PasskeySpec {
            allowed_rp_ids: vec!["*".into()],
            ..sample()
        };
        assert!(s.allows_rp("anything.com"));
    }

    #[test]
    fn block_list_takes_precedence() {
        let s = PasskeySpec {
            allowed_rp_ids: vec!["example.com".into()],
            blocked_rp_ids: vec!["example.com".into()],
            ..sample()
        };
        assert!(!s.allows_rp("example.com"));
    }

    #[test]
    fn registry_dedupes_by_name_and_filters_by_rp() {
        let mut reg = PasskeyRegistry::new();
        reg.insert(sample());
        reg.insert(PasskeySpec {
            name: "hardware".into(),
            authenticator: Authenticator::CrossPlatform,
            allowed_rp_ids: vec!["bank.com".into()],
            ..sample()
        });
        assert_eq!(reg.len(), 2);
        let on_github = reg.applicable("github.com");
        assert_eq!(on_github.len(), 1);
        assert_eq!(on_github[0].name, "primary");
        let on_bank = reg.applicable("bank.com");
        assert_eq!(on_bank.len(), 1);
        assert_eq!(on_bank[0].name, "hardware");
    }

    #[test]
    fn record_redacted_strips_key_material() {
        let r = PasskeyRecord {
            credential_id: "cred".into(),
            rp_id: "example.com".into(),
            rp_name: "Example".into(),
            user_handle: "handle".into(),
            user_name: "jane".into(),
            algorithm: -7,
            authenticator: Authenticator::Platform,
            user_verification: UserVerification::Required,
            public_key: "long-b64".into(),
            created_at: 1,
            last_used_at: 2,
            sign_count: 5,
        };
        let r2 = r.redacted();
        assert!(r2.public_key.is_empty());
        assert!(r2.user_handle.is_empty());
        assert_eq!(r2.user_name, "jane");
    }

    #[test]
    fn enums_roundtrip_through_serde() {
        for a in [
            Authenticator::Any,
            Authenticator::Platform,
            Authenticator::CrossPlatform,
        ] {
            let s = PasskeySpec {
                authenticator: a,
                ..sample()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: PasskeySpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.authenticator, a);
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_passkey_form() {
        let src = r#"
            (defpasskey :name "primary"
                        :vault "primary"
                        :authenticator "platform"
                        :user-verification "required"
                        :sync-passkeys #t
                        :allowed-rp-ids ("example.com" "github.com")
                        :resident-key #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.authenticator, Authenticator::Platform);
        assert_eq!(s.user_verification, UserVerification::Required);
        assert!(s.sync_passkeys);
        assert_eq!(s.allowed_rp_ids.len(), 2);
    }
}
