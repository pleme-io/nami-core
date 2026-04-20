//! Ed25519 signatures for `(defextension)` bundles.
//!
//! Connects the browser-extension store to pleme-io's sekiban/tameshi
//! attestation chain. The trust model: a spec is canonicalized to
//! sorted-key JSON → hashed with BLAKE3 → signed with ed25519. The
//! verifier reconstitutes the canonical hash from the spec it has and
//! checks the signature against a trusted public-key store.
//!
//! Off by default — the `signatures` Cargo feature pulls in
//! `ed25519-dalek` and `base64`. With the feature off, the types
//! remain but verification returns
//! [`VerificationError::FeatureDisabled`] so callers can compile a
//! signature-free namimado build without #[cfg] litter.
//!
//! Manifest form — the serialized shape shipped alongside an extension:
//!
//! ```json
//! {
//!   "spec": { …ExtensionSpec fields… },
//!   "signature": {
//!     "algorithm": "ed25519",
//!     "public_key": "base64-pubkey-32B",
//!     "signature":  "base64-signature-64B",
//!     "signed_by":  "Jane Doe <jane@example.com>",
//!     "signed_at":  "2026-04-19T00:00:00Z"
//!   }
//! }
//! ```
//!
//! Canonical form for signing/verifying:
//!
//! 1. Serialize `ExtensionSpec` to JSON with keys in sorted order.
//! 2. BLAKE3 the bytes.
//! 3. Sign / verify the 32-byte hash.
//!
//! A signature therefore covers every field that survives round-
//! trip through serde_json; the `enabled` bool, rules, permissions,
//! and host_permissions are all inside the signature envelope.

use super::ExtensionSpec;
use serde::{Deserialize, Serialize};

/// Signature envelope attached to an extension bundle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignatureBundle {
    /// Algorithm tag — currently only `"ed25519"` is supported.
    pub algorithm: String,
    /// Ed25519 public key, 32 raw bytes, base64-encoded.
    pub public_key: String,
    /// Ed25519 signature, 64 raw bytes, base64-encoded.
    pub signature: String,
    /// Optional human-readable author string. Not verified — the
    /// public key is the root of trust; this is UX metadata.
    #[serde(default)]
    pub signed_by: Option<String>,
    /// Optional ISO-8601 UTC timestamp. Advisory only.
    #[serde(default)]
    pub signed_at: Option<String>,
}

/// A `(defextension)` spec plus its signature envelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignedExtension {
    pub spec: ExtensionSpec,
    pub signature: SignatureBundle,
}

/// Errors from the signing / verification path.
#[derive(Debug, thiserror::Error)]
pub enum VerificationError {
    #[error("signatures feature not enabled; rebuild nami-core with --features signatures")]
    FeatureDisabled,
    #[error("unknown signature algorithm: {0}")]
    UnknownAlgorithm(String),
    #[error("malformed base64 in {field}: {source}")]
    Base64Decode {
        field: &'static str,
        #[source]
        source: base64_error::Base64DecodeError,
    },
    #[error("public key must be 32 bytes, got {0}")]
    BadPublicKeyLength(usize),
    #[error("signature must be 64 bytes, got {0}")]
    BadSignatureLength(usize),
    #[error("signature verification failed — spec tampered or wrong key")]
    SignatureMismatch,
    #[error("public key is not in the trusted keyring")]
    UntrustedKey,
    #[error("JSON canonicalization failed: {0}")]
    Canonicalize(String),
}

/// Hand-built base64 error wrapper so callers don't need to pull
/// base64 into their own Cargo if they just want to name the error.
pub mod base64_error {
    #[derive(Debug, Clone)]
    pub struct Base64DecodeError(pub String);

    impl std::fmt::Display for Base64DecodeError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    impl std::error::Error for Base64DecodeError {}
}

/// Canonical bytes used as the signing preimage. Sorted-key JSON
/// serialization of the spec, then BLAKE3-hashed to 32 bytes.
///
/// Stable across every runtime that produces the same JSON shape;
/// the sort is alphabetical by field name at every nesting level.
#[must_use]
pub fn canonical_bytes(spec: &ExtensionSpec) -> [u8; 32] {
    let json = canonical_json(spec).unwrap_or_else(|_| serde_json::Value::Null);
    let bytes = serde_json::to_vec(&json).unwrap_or_default();
    *blake3::hash(&bytes).as_bytes()
}

fn canonical_json(spec: &ExtensionSpec) -> Result<serde_json::Value, VerificationError> {
    // serde_json::Map preserves insertion order; we build a new map
    // by sorted-key inserts over the original.
    let raw = serde_json::to_value(spec)
        .map_err(|e| VerificationError::Canonicalize(e.to_string()))?;
    Ok(sort_value(raw))
}

fn sort_value(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            let mut out = serde_json::Map::with_capacity(keys.len());
            for k in keys {
                let v = map.get(&k).cloned().unwrap_or(serde_json::Value::Null);
                out.insert(k, sort_value(v));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => {
            // Arrays keep order — semantic meaning in most of our
            // fields (rules, permissions) depends on it.
            serde_json::Value::Array(items.into_iter().map(sort_value).collect())
        }
        other => other,
    }
}

/// BLAKE3 content hash of the canonical form, 128 bits → 26-char
/// base32 lowercase. Same convention as tameshi/sekiban.
#[must_use]
pub fn canonical_hash(spec: &ExtensionSpec) -> String {
    let bytes = canonical_bytes(spec);
    super::base32_16(&bytes[..16])
}

/// Result of verifying a signed extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationStatus {
    /// Signature valid + key is in the caller's trust store.
    Trusted {
        public_key_b64: String,
        signed_by: Option<String>,
    },
    /// Signature valid, key is NOT in the trust store. Caller may
    /// prompt the user to TOFU-accept.
    ValidButUntrusted { public_key_b64: String },
    /// Signature didn't verify.
    Invalid(String),
}

/// A trust store — a set of allowed public keys.
#[derive(Debug, Clone, Default)]
pub struct Trustdb {
    keys: std::collections::HashSet<String>,
}

impl Trustdb {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn trust(&mut self, public_key_b64: impl Into<String>) {
        self.keys.insert(public_key_b64.into());
    }

    pub fn revoke(&mut self, public_key_b64: &str) -> bool {
        self.keys.remove(public_key_b64)
    }

    #[must_use]
    pub fn is_trusted(&self, public_key_b64: &str) -> bool {
        self.keys.contains(public_key_b64)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        let mut out: Vec<String> = self.keys.iter().cloned().collect();
        out.sort();
        out
    }
}

// ─── signing / verification (feature-gated) ──────────────────────

#[cfg(feature = "signatures")]
mod crypto {
    use super::*;
    use base64::Engine as _;
    use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

    const B64: base64::engine::general_purpose::GeneralPurpose =
        base64::engine::general_purpose::STANDARD;

    /// Deterministically derive a signing key from 32 seed bytes.
    /// Prefer `SigningKey::generate(&mut OsRng)` in production — this
    /// helper exists for tests and offline key provisioning.
    #[must_use]
    pub fn signing_key_from_seed(seed: &[u8; 32]) -> SigningKey {
        SigningKey::from_bytes(seed)
    }

    /// Sign an extension spec. Returns a SignatureBundle with the
    /// public key + signature base64-encoded.
    #[must_use]
    pub fn sign(spec: &ExtensionSpec, key: &SigningKey) -> SignatureBundle {
        let digest = canonical_bytes(spec);
        let sig: Signature = key.sign(&digest);
        let pub_bytes = key.verifying_key().to_bytes();
        SignatureBundle {
            algorithm: "ed25519".into(),
            public_key: B64.encode(pub_bytes),
            signature: B64.encode(sig.to_bytes()),
            signed_by: None,
            signed_at: None,
        }
    }

    /// Verify a signed extension against a trust store.
    pub fn verify(
        signed: &SignedExtension,
        trustdb: &Trustdb,
    ) -> Result<VerificationStatus, VerificationError> {
        if signed.signature.algorithm != "ed25519" {
            return Err(VerificationError::UnknownAlgorithm(
                signed.signature.algorithm.clone(),
            ));
        }

        let pub_bytes = B64
            .decode(&signed.signature.public_key)
            .map_err(|e| VerificationError::Base64Decode {
                field: "public_key",
                source: base64_error::Base64DecodeError(e.to_string()),
            })?;
        if pub_bytes.len() != 32 {
            return Err(VerificationError::BadPublicKeyLength(pub_bytes.len()));
        }
        let pub_arr: [u8; 32] = pub_bytes
            .as_slice()
            .try_into()
            .expect("length checked above");
        let verifying = VerifyingKey::from_bytes(&pub_arr)
            .map_err(|_| VerificationError::SignatureMismatch)?;

        let sig_bytes = B64
            .decode(&signed.signature.signature)
            .map_err(|e| VerificationError::Base64Decode {
                field: "signature",
                source: base64_error::Base64DecodeError(e.to_string()),
            })?;
        if sig_bytes.len() != 64 {
            return Err(VerificationError::BadSignatureLength(sig_bytes.len()));
        }
        let sig_arr: [u8; 64] = sig_bytes
            .as_slice()
            .try_into()
            .expect("length checked above");
        let sig = Signature::from_bytes(&sig_arr);

        let digest = canonical_bytes(&signed.spec);
        if verifying.verify(&digest, &sig).is_err() {
            return Err(VerificationError::SignatureMismatch);
        }

        let pk_b64 = signed.signature.public_key.clone();
        Ok(if trustdb.is_trusted(&pk_b64) {
            VerificationStatus::Trusted {
                public_key_b64: pk_b64,
                signed_by: signed.signature.signed_by.clone(),
            }
        } else {
            VerificationStatus::ValidButUntrusted { public_key_b64: pk_b64 }
        })
    }
}

#[cfg(feature = "signatures")]
pub use crypto::{sign, signing_key_from_seed, verify};

#[cfg(not(feature = "signatures"))]
pub fn verify(
    _signed: &SignedExtension,
    _trustdb: &Trustdb,
) -> Result<VerificationStatus, VerificationError> {
    Err(VerificationError::FeatureDisabled)
}

// ─── tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::{ExtensionSpec, Permission};

    fn sample_spec() -> ExtensionSpec {
        ExtensionSpec {
            name: "dark-reader".into(),
            version: "1.0.0".into(),
            description: Some("Per-site dark-mode CSS".into()),
            author: Some("Jane".into()),
            homepage_url: None,
            icon: None,
            permissions: vec![Permission::Storage, Permission::ActiveTab],
            host_permissions: vec!["*://*.example.com/*".into()],
            rules: vec!["dark-reader/css".into()],
            enabled: true,
        }
    }

    #[test]
    fn canonical_bytes_is_stable() {
        let s = sample_spec();
        let a = canonical_bytes(&s);
        let b = canonical_bytes(&s);
        assert_eq!(a, b);
    }

    #[test]
    fn canonical_bytes_changes_on_mutation() {
        let s = sample_spec();
        let a = canonical_bytes(&s);
        let mut s2 = s.clone();
        s2.rules.push("dark-reader/injected".into());
        let b = canonical_bytes(&s2);
        assert_ne!(a, b);
    }

    #[test]
    fn canonical_hash_matches_pleme_io_attestation_shape() {
        let h = canonical_hash(&sample_spec());
        assert_eq!(h.len(), 26);
        for ch in h.chars() {
            assert!(ch.is_ascii_lowercase() || ch.is_ascii_digit());
        }
    }

    #[test]
    fn trustdb_roundtrip() {
        let mut db = Trustdb::new();
        db.trust("abc".to_owned());
        db.trust("def".to_owned());
        assert_eq!(db.len(), 2);
        assert!(db.is_trusted("abc"));
        assert!(!db.is_trusted("ghi"));
        assert!(db.revoke("abc"));
        assert!(!db.is_trusted("abc"));
        assert!(!db.revoke("never"));
    }

    #[test]
    fn trustdb_keys_are_sorted() {
        let mut db = Trustdb::new();
        db.trust("c".to_owned());
        db.trust("a".to_owned());
        db.trust("b".to_owned());
        assert_eq!(db.keys(), vec!["a", "b", "c"]);
    }

    #[cfg(not(feature = "signatures"))]
    #[test]
    fn verify_without_feature_returns_feature_disabled() {
        let signed = SignedExtension {
            spec: sample_spec(),
            signature: SignatureBundle {
                algorithm: "ed25519".into(),
                public_key: "AA".into(),
                signature: "AA".into(),
                signed_by: None,
                signed_at: None,
            },
        };
        let err = verify(&signed, &Trustdb::new()).unwrap_err();
        assert!(matches!(err, VerificationError::FeatureDisabled));
    }

    #[cfg(feature = "signatures")]
    mod signed {
        use super::*;

        fn fresh_key() -> ed25519_dalek::SigningKey {
            let seed = [7u8; 32];
            signing_key_from_seed(&seed)
        }

        #[test]
        fn sign_verify_roundtrips() {
            let key = fresh_key();
            let spec = sample_spec();
            let bundle = sign(&spec, &key);
            let signed = SignedExtension { spec, signature: bundle };

            let mut trust = Trustdb::new();
            trust.trust(signed.signature.public_key.clone());

            match verify(&signed, &trust).unwrap() {
                VerificationStatus::Trusted { public_key_b64, .. } => {
                    assert_eq!(public_key_b64, signed.signature.public_key);
                }
                other => panic!("expected Trusted, got {other:?}"),
            }
        }

        #[test]
        fn verify_with_untrusted_key_reports_valid_but_untrusted() {
            let key = fresh_key();
            let spec = sample_spec();
            let bundle = sign(&spec, &key);
            let signed = SignedExtension { spec, signature: bundle };
            // Empty trust store.
            let trust = Trustdb::new();
            match verify(&signed, &trust).unwrap() {
                VerificationStatus::ValidButUntrusted { .. } => (),
                other => panic!("expected ValidButUntrusted, got {other:?}"),
            }
        }

        #[test]
        fn verify_rejects_tampered_spec() {
            let key = fresh_key();
            let spec = sample_spec();
            let bundle = sign(&spec, &key);
            // Mutate the spec AFTER signing.
            let mut tampered = spec.clone();
            tampered.rules.push("evil/rule".into());
            let signed = SignedExtension { spec: tampered, signature: bundle };
            let trust = Trustdb::new();
            let err = verify(&signed, &trust).unwrap_err();
            assert!(matches!(err, VerificationError::SignatureMismatch));
        }

        #[test]
        fn verify_rejects_wrong_key() {
            let attacker = signing_key_from_seed(&[99u8; 32]);
            let spec = sample_spec();
            let mut bundle = sign(&spec, &attacker);

            // Swap the public_key field so it doesn't match the
            // actual signer — the signature now verifies against a
            // different (but syntactically valid) public key.
            let bystander = signing_key_from_seed(&[42u8; 32]);
            let bystander_pub = {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD
                    .encode(bystander.verifying_key().to_bytes())
            };
            bundle.public_key = bystander_pub;

            let signed = SignedExtension { spec, signature: bundle };
            let trust = Trustdb::new();
            let err = verify(&signed, &trust).unwrap_err();
            assert!(matches!(err, VerificationError::SignatureMismatch));
        }

        #[test]
        fn verify_rejects_bad_algorithm() {
            let key = fresh_key();
            let spec = sample_spec();
            let mut bundle = sign(&spec, &key);
            bundle.algorithm = "rsa".into();
            let signed = SignedExtension { spec, signature: bundle };
            let err = verify(&signed, &Trustdb::new()).unwrap_err();
            assert!(matches!(err, VerificationError::UnknownAlgorithm(_)));
        }

        #[test]
        fn verify_rejects_malformed_base64() {
            let key = fresh_key();
            let spec = sample_spec();
            let mut bundle = sign(&spec, &key);
            bundle.signature = "not-valid-base64!!!".into();
            let signed = SignedExtension { spec, signature: bundle };
            let err = verify(&signed, &Trustdb::new()).unwrap_err();
            assert!(matches!(err, VerificationError::Base64Decode { .. }));
        }

        #[test]
        fn sign_embeds_public_key_matching_signing_key() {
            let key = fresh_key();
            let bundle = sign(&sample_spec(), &key);
            use base64::Engine as _;
            let pub_bytes = base64::engine::general_purpose::STANDARD
                .decode(&bundle.public_key)
                .unwrap();
            assert_eq!(pub_bytes.len(), 32);
            assert_eq!(&pub_bytes[..], &key.verifying_key().to_bytes()[..]);
        }

        #[test]
        fn deterministic_key_from_seed() {
            let a = signing_key_from_seed(&[1u8; 32]);
            let b = signing_key_from_seed(&[1u8; 32]);
            assert_eq!(a.verifying_key().to_bytes(), b.verifying_key().to_bytes());
        }

        #[test]
        fn signed_extension_roundtrips_through_json() {
            let key = fresh_key();
            let spec = sample_spec();
            let bundle = sign(&spec, &key);
            let signed = SignedExtension { spec, signature: bundle };
            let json = serde_json::to_string(&signed).unwrap();
            let back: SignedExtension = serde_json::from_str(&json).unwrap();
            let mut trust = Trustdb::new();
            trust.trust(back.signature.public_key.clone());
            assert!(matches!(
                verify(&back, &trust).unwrap(),
                VerificationStatus::Trusted { .. }
            ));
        }
    }
}
