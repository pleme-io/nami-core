//! `(deftotp)` — RFC 6238 Time-based One-Time Passwords.
//!
//! Absorbs Authy, Google Authenticator, 1Password TOTP, Bitwarden
//! TOTP, Yubico Authenticator, macOS Passwords (TOTP), Aegis. Each
//! profile stores the shared secret (base32), configurable digits,
//! period, and HMAC algorithm, plus the issuer + account-name pair
//! that appears in the UI.
//!
//! Complements [`crate::passwords`] + [`crate::passkey`] — passkeys
//! replace passwords where supported; TOTP fills the 2FA gap for
//! everything that still asks for a 6-digit code.
//!
//! ```lisp
//! (deftotp :name         "github-2fa"
//!          :issuer       "GitHub"
//!          :account-name "jane@example.com"
//!          :secret       "JBSWY3DPEHPK3PXP"
//!          :algorithm    :sha1
//!          :digits       6
//!          :period       30)
//! ```

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// HMAC hash used for the HOTP/TOTP computation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TotpAlgorithm {
    /// RFC 6238 default.
    #[default]
    Sha1,
    Sha256,
    Sha512,
}

/// TOTP profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "deftotp"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TotpSpec {
    pub name: String,
    /// Issuer shown in the UI (e.g. "GitHub").
    #[serde(default)]
    pub issuer: String,
    /// Account name ("jane@example.com").
    #[serde(default)]
    pub account_name: String,
    /// Base32-encoded HMAC secret.
    pub secret: String,
    #[serde(default)]
    pub algorithm: TotpAlgorithm,
    /// Number of digits in the generated code (6, 7, or 8).
    #[serde(default = "default_digits")]
    pub digits: u8,
    /// Time step in seconds (RFC 6238 default: 30).
    #[serde(default = "default_period")]
    pub period: u32,
    /// Which identities this TOTP belongs to.
    #[serde(default)]
    pub identities: Vec<String>,
    /// Associated vault name.
    #[serde(default)]
    pub vault: Option<String>,
    /// Favicon URL (for identity rendering).
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_digits() -> u8 {
    6
}
fn default_period() -> u32 {
    30
}
fn default_enabled() -> bool {
    true
}

/// Per-spec error surface kept minimal — bad base32 or out-of-range
/// digit count are the only things that make generation fail.
#[derive(Debug, thiserror::Error)]
pub enum TotpError {
    #[error("secret is not valid base32")]
    InvalidSecret,
    #[error("digit count {0} must be 6, 7, or 8")]
    InvalidDigits(u8),
    #[error("period must be > 0")]
    InvalidPeriod,
}

impl TotpSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "example".into(),
            issuer: "example.com".into(),
            account_name: "user@example.com".into(),
            // "Hello!" in base32 — deterministic placeholder.
            secret: "JBSWY3DPEHPK3PXP".into(),
            algorithm: TotpAlgorithm::Sha1,
            digits: 6,
            period: 30,
            identities: vec![],
            vault: None,
            icon: None,
            enabled: true,
            description: Some("Example TOTP profile — SHA1, 6 digits, 30 s period.".into()),
        }
    }

    /// Generate the TOTP code for the current system time.
    pub fn generate_now(&self) -> Result<String, TotpError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.generate_at(now)
    }

    /// Generate the TOTP code at Unix time `seconds`. Pure — useful
    /// for testing against RFC 6238 vectors.
    pub fn generate_at(&self, seconds: u64) -> Result<String, TotpError> {
        if self.period == 0 {
            return Err(TotpError::InvalidPeriod);
        }
        if !matches!(self.digits, 6 | 7 | 8) {
            return Err(TotpError::InvalidDigits(self.digits));
        }
        let counter = seconds / u64::from(self.period);
        let key = base32_decode(&self.secret).ok_or(TotpError::InvalidSecret)?;
        let digest = hmac(self.algorithm, &key, &counter.to_be_bytes());
        let truncated = dynamic_truncate(&digest);
        let modulus = 10u32.pow(u32::from(self.digits));
        let code = truncated % modulus;
        Ok(format!("{code:0>width$}", width = self.digits as usize))
    }

    /// Seconds until the current code rolls over.
    pub fn seconds_remaining_now(&self) -> u32 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.seconds_remaining_at(now)
    }

    pub fn seconds_remaining_at(&self, seconds: u64) -> u32 {
        if self.period == 0 {
            return 0;
        }
        let period = u64::from(self.period);
        let r = period - (seconds % period);
        u32::try_from(r).unwrap_or(0)
    }

    /// Render a standard otpauth:// URI (the QR-code format).
    #[must_use]
    pub fn otpauth_uri(&self) -> String {
        let algo = match self.algorithm {
            TotpAlgorithm::Sha1 => "SHA1",
            TotpAlgorithm::Sha256 => "SHA256",
            TotpAlgorithm::Sha512 => "SHA512",
        };
        let issuer = url_encode(&self.issuer);
        let account = url_encode(&self.account_name);
        let secret = &self.secret;
        format!(
            "otpauth://totp/{issuer}:{account}?secret={secret}&issuer={issuer}&algorithm={algo}&digits={d}&period={p}",
            d = self.digits,
            p = self.period,
        )
    }
}

fn hmac(algo: TotpAlgorithm, key: &[u8], msg: &[u8]) -> Vec<u8> {
    match algo {
        TotpAlgorithm::Sha1 => {
            let mut mac = Hmac::<Sha1>::new_from_slice(key).expect("hmac-sha1 accepts any key length");
            mac.update(msg);
            mac.finalize().into_bytes().to_vec()
        }
        TotpAlgorithm::Sha256 => {
            let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac-sha256 accepts any key length");
            mac.update(msg);
            mac.finalize().into_bytes().to_vec()
        }
        TotpAlgorithm::Sha512 => {
            let mut mac = Hmac::<Sha512>::new_from_slice(key).expect("hmac-sha512 accepts any key length");
            mac.update(msg);
            mac.finalize().into_bytes().to_vec()
        }
    }
}

fn dynamic_truncate(digest: &[u8]) -> u32 {
    // RFC 4226 §5.3 — bottom 4 bits of the last byte is the offset.
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let b0 = u32::from(digest[offset] & 0x7f);
    let b1 = u32::from(digest[offset + 1]);
    let b2 = u32::from(digest[offset + 2]);
    let b3 = u32::from(digest[offset + 3]);
    (b0 << 24) | (b1 << 16) | (b2 << 8) | b3
}

fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim().trim_end_matches('=').to_ascii_uppercase();
    let s: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(s.len() * 5 / 8);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for c in s.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            '2'..='7' => c as u32 - '2' as u32 + 26,
            _ => return None,
        };
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct TotpRegistry {
    specs: Vec<TotpSpec>,
}

impl TotpRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: TotpSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = TotpSpec>) {
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
    pub fn specs(&self) -> &[TotpSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&TotpSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// All enabled TOTP profiles belonging to a named identity.
    #[must_use]
    pub fn for_identity(&self, identity: &str) -> Vec<&TotpSpec> {
        self.specs
            .iter()
            .filter(|s| s.enabled && s.identities.iter().any(|i| i == identity))
            .collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<TotpSpec>, String> {
    tatara_lisp::compile_typed::<TotpSpec>(src)
        .map_err(|e| format!("failed to compile deftotp forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<TotpSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_sha1_6_30() {
        let s = TotpSpec::default_profile();
        assert_eq!(s.algorithm, TotpAlgorithm::Sha1);
        assert_eq!(s.digits, 6);
        assert_eq!(s.period, 30);
    }

    /// RFC 6238 Appendix B — SHA1 vectors. Secret is
    /// "12345678901234567890" encoded as base32 (GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ).
    #[test]
    fn rfc6238_sha1_vectors() {
        let s = TotpSpec {
            secret: "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".into(),
            algorithm: TotpAlgorithm::Sha1,
            digits: 8,
            period: 30,
            ..TotpSpec::default_profile()
        };
        // From the RFC table:
        assert_eq!(s.generate_at(59).unwrap(), "94287082");
        assert_eq!(s.generate_at(1_111_111_109).unwrap(), "07081804");
        assert_eq!(s.generate_at(1_111_111_111).unwrap(), "14050471");
        assert_eq!(s.generate_at(1_234_567_890).unwrap(), "89005924");
        assert_eq!(s.generate_at(2_000_000_000).unwrap(), "69279037");
    }

    #[test]
    fn rfc6238_sha256_vectors() {
        // SHA256 secret is 32 bytes: "12345678901234567890123456789012"
        // base32: GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZA
        let s = TotpSpec {
            secret: "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZA".into(),
            algorithm: TotpAlgorithm::Sha256,
            digits: 8,
            period: 30,
            ..TotpSpec::default_profile()
        };
        assert_eq!(s.generate_at(59).unwrap(), "46119246");
        assert_eq!(s.generate_at(1_111_111_109).unwrap(), "68084774");
    }

    #[test]
    fn invalid_digits_returns_error() {
        let s = TotpSpec {
            digits: 9,
            ..TotpSpec::default_profile()
        };
        assert!(matches!(s.generate_at(0), Err(TotpError::InvalidDigits(9))));
    }

    #[test]
    fn invalid_period_returns_error() {
        let s = TotpSpec {
            period: 0,
            ..TotpSpec::default_profile()
        };
        assert!(matches!(s.generate_at(0), Err(TotpError::InvalidPeriod)));
    }

    #[test]
    fn invalid_secret_returns_error() {
        let s = TotpSpec {
            secret: "NOT-BASE32-!@#".into(),
            ..TotpSpec::default_profile()
        };
        assert!(matches!(s.generate_at(0), Err(TotpError::InvalidSecret)));
    }

    #[test]
    fn seconds_remaining_counts_down() {
        let s = TotpSpec {
            period: 30,
            ..TotpSpec::default_profile()
        };
        assert_eq!(s.seconds_remaining_at(0), 30);
        assert_eq!(s.seconds_remaining_at(29), 1);
        assert_eq!(s.seconds_remaining_at(30), 30);
        assert_eq!(s.seconds_remaining_at(45), 15);
    }

    #[test]
    fn otpauth_uri_encodes_issuer_and_account() {
        let s = TotpSpec {
            name: "github".into(),
            issuer: "GitHub".into(),
            account_name: "jane@example.com".into(),
            secret: "JBSWY3DPEHPK3PXP".into(),
            algorithm: TotpAlgorithm::Sha256,
            digits: 6,
            period: 30,
            ..TotpSpec::default_profile()
        };
        let uri = s.otpauth_uri();
        assert!(uri.starts_with("otpauth://totp/GitHub:jane%40example.com?"));
        assert!(uri.contains("secret=JBSWY3DPEHPK3PXP"));
        assert!(uri.contains("issuer=GitHub"));
        assert!(uri.contains("algorithm=SHA256"));
    }

    #[test]
    fn base32_decode_handles_padding_and_whitespace() {
        // "JBSWY3DPEE======" = "Hello!" (6 bytes, padded).
        let d = base32_decode("JBSWY3DPEE======").unwrap();
        assert_eq!(d, b"Hello!");
        // Same content, whitespace splits ignored.
        let d2 = base32_decode("JBSW Y3DP EE==").unwrap();
        assert_eq!(d2, b"Hello!");
        // Non-base32 chars cause None.
        assert!(base32_decode("NOT-BASE32-!@#").is_none());
    }

    #[test]
    fn algorithm_roundtrips_through_serde() {
        for a in [
            TotpAlgorithm::Sha1,
            TotpAlgorithm::Sha256,
            TotpAlgorithm::Sha512,
        ] {
            let s = TotpSpec {
                algorithm: a,
                ..TotpSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: TotpSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.algorithm, a);
        }
    }

    #[test]
    fn registry_for_identity_filters() {
        let mut reg = TotpRegistry::new();
        reg.insert(TotpSpec {
            name: "gh".into(),
            identities: vec!["work".into()],
            ..TotpSpec::default_profile()
        });
        reg.insert(TotpSpec {
            name: "aws".into(),
            identities: vec!["personal".into()],
            ..TotpSpec::default_profile()
        });
        assert_eq!(reg.for_identity("work").len(), 1);
        assert_eq!(reg.for_identity("work")[0].name, "gh");
    }

    #[test]
    fn registry_dedupes_on_name_insert() {
        let mut reg = TotpRegistry::new();
        reg.insert(TotpSpec::default_profile());
        let replaced = TotpSpec {
            digits: 7,
            ..TotpSpec::default_profile()
        };
        reg.insert(replaced);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("example").unwrap().digits, 7);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_totp_form() {
        let src = r#"
            (deftotp :name "github-2fa"
                     :issuer "GitHub"
                     :account-name "jane@example.com"
                     :secret "JBSWY3DPEHPK3PXP"
                     :algorithm "sha1"
                     :digits 6
                     :period 30
                     :identities ("work"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "github-2fa");
        assert_eq!(s.issuer, "GitHub");
        assert_eq!(s.algorithm, TotpAlgorithm::Sha1);
        assert_eq!(s.identities, vec!["work".to_string()]);
        // Sanity — the spec can actually produce a code.
        assert_eq!(s.generate_at(0).unwrap().len(), 6);
    }
}
