//! `(defdownload)` — declarative download manager.
//!
//! Absorbs every browser's DL panel (Chrome, Firefox, Safari, Edge)
//! plus resume-aware managers (uGet, aria2 UI, JDownloader). A policy
//! names the target folder, per-MIME quarantine rules, and optional
//! post-download BLAKE3 verification — connects straight to sekiban's
//! attestation chain.
//!
//! ```lisp
//! (defdownload :name             "default"
//!              :folder            "~/Downloads"
//!              :quarantine-mime   ("application/octet-stream"
//!                                  "application/x-executable"
//!                                  "application/x-msdownload"
//!                                  "application/zip"
//!                                  "application/x-apple-diskimage")
//!              :hash-verify      :sha256
//!              :auto-open-mime   ("application/pdf" "image/*")
//!              :concurrency      4
//!              :resume           #t)
//! ```
//!
//! The DSL describes policy; the fetch pipeline does the actual I/O
//! and calls back with a [`DownloadRecord`]. The record includes
//! `content_hash` (BLAKE3 → 26-char base32) so downloads flow into
//! sekiban attestation + `(defextension)` signature verification
//! without a separate pipeline.

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// How to verify the downloaded file's content hash.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HashVerify {
    /// No verification (accept as-is).
    None,
    /// BLAKE3 — substrate default, connects to sekiban.
    Blake3,
    /// SHA-256 — broad ecosystem compat.
    Sha256,
    /// SHA-512.
    Sha512,
}

impl Default for HashVerify {
    fn default() -> Self {
        Self::Blake3
    }
}

/// Download profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defdownload"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadSpec {
    pub name: String,
    /// Target folder. `~` expands; relative paths resolve to the
    /// caller's working dir.
    #[serde(default = "default_folder")]
    pub folder: String,
    /// MIME types (or `type/*` globs) that land in a quarantine subdir
    /// until the user explicitly releases them. Tor-Browser-style.
    #[serde(default)]
    pub quarantine_mime: Vec<String>,
    /// MIME types auto-opened after download. `type/*` globs supported.
    #[serde(default)]
    pub auto_open_mime: Vec<String>,
    #[serde(default)]
    pub hash_verify: HashVerify,
    /// Maximum concurrent downloads. `0` = unlimited.
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
    /// Re-use partial downloads via HTTP Range.
    #[serde(default = "default_resume")]
    pub resume: bool,
    /// Reject files larger than this. `0` = no cap.
    #[serde(default)]
    pub max_size_bytes: u64,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_folder() -> String {
    "~/Downloads".into()
}
fn default_concurrency() -> u32 {
    4
}
fn default_resume() -> bool {
    true
}

impl DownloadSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            folder: default_folder(),
            quarantine_mime: vec![
                "application/octet-stream".into(),
                "application/x-executable".into(),
                "application/x-msdownload".into(),
                "application/zip".into(),
                "application/x-apple-diskimage".into(),
            ],
            auto_open_mime: vec![],
            hash_verify: HashVerify::Blake3,
            concurrency: 4,
            resume: true,
            max_size_bytes: 0,
            description: Some(
                "Default download policy — BLAKE3-verify, 4-way concurrent."
                    .into(),
            ),
        }
    }

    /// Should `mime` be quarantined?
    #[must_use]
    pub fn should_quarantine(&self, mime: &str) -> bool {
        self.quarantine_mime.iter().any(|p| mime_matches(p, mime))
    }

    /// Should `mime` auto-open?
    #[must_use]
    pub fn should_auto_open(&self, mime: &str) -> bool {
        self.auto_open_mime.iter().any(|p| mime_matches(p, mime))
    }

    /// True when `bytes` would be rejected for exceeding the cap.
    #[must_use]
    pub fn exceeds_size_cap(&self, bytes: u64) -> bool {
        self.max_size_bytes > 0 && bytes > self.max_size_bytes
    }
}

fn mime_matches(pattern: &str, mime: &str) -> bool {
    // `type/*` matches anything starting with `type/`.
    if let Some(prefix) = pattern.strip_suffix("/*") {
        return mime.starts_with(prefix) && mime.len() > prefix.len();
    }
    pattern.eq_ignore_ascii_case(mime)
}

/// State of one download at a point in time.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DownloadState {
    Pending,
    InProgress,
    Paused,
    Complete,
    Quarantined,
    Failed,
    Cancelled,
}

/// One download record — what the fetch pipeline writes back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRecord {
    /// BLAKE3 of the source URL + creation time — stable id.
    pub id: String,
    pub url: String,
    pub filename: String,
    pub target_path: String,
    pub mime: String,
    pub size_bytes: u64,
    pub bytes_downloaded: u64,
    pub state: DownloadState,
    /// Content hash — present once the download completes and verify
    /// ran. 26-char base32 when hash_verify was Blake3 (tameshi-shape);
    /// hex for sha256/sha512.
    #[serde(default)]
    pub content_hash: Option<String>,
    /// Unix seconds.
    pub started_at: i64,
    #[serde(default)]
    pub completed_at: i64,
    /// Error detail when state == Failed.
    #[serde(default)]
    pub error: Option<String>,
}

impl DownloadRecord {
    /// Progress fraction [0.0, 1.0]; returns 0.0 for zero-size files.
    #[must_use]
    pub fn progress(&self) -> f32 {
        if self.size_bytes == 0 {
            return 0.0;
        }
        (self.bytes_downloaded as f64 / self.size_bytes as f64) as f32
    }

    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            DownloadState::Complete
                | DownloadState::Quarantined
                | DownloadState::Failed
                | DownloadState::Cancelled
        )
    }
}

/// Compute the tameshi-shape content hash of a byte slice (BLAKE3,
/// 128 bits → 26-char base32 lowercase). Same function as
/// extension::signature::canonical_hash but reused here so download
/// pipelines don't pull the signatures feature in.
#[must_use]
pub fn blake3_content_hash(bytes: &[u8]) -> String {
    let h = blake3::hash(bytes);
    base32_16(&h.as_bytes()[..16])
}

fn base32_16(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::new();
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &b in bytes {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buf >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

/// Registry of download profiles.
#[derive(Debug, Clone, Default)]
pub struct DownloadRegistry {
    specs: Vec<DownloadSpec>,
}

impl DownloadRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: DownloadSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = DownloadSpec>) {
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
    pub fn specs(&self) -> &[DownloadSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&DownloadSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<DownloadSpec>, String> {
    tatara_lisp::compile_typed::<DownloadSpec>(src)
        .map_err(|e| format!("failed to compile defdownload forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<DownloadSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_matches_exact_and_glob() {
        assert!(mime_matches("application/pdf", "application/pdf"));
        assert!(mime_matches("image/*", "image/png"));
        assert!(mime_matches("image/*", "image/jpeg"));
        assert!(!mime_matches("image/*", "image"));
        assert!(!mime_matches("application/pdf", "application/octet-stream"));
    }

    #[test]
    fn default_profile_quarantines_dangerous_mimes() {
        let s = DownloadSpec::default_profile();
        assert!(s.should_quarantine("application/octet-stream"));
        assert!(s.should_quarantine("application/zip"));
        assert!(!s.should_quarantine("text/plain"));
    }

    #[test]
    fn auto_open_glob_catches_subclasses() {
        let s = DownloadSpec {
            auto_open_mime: vec!["image/*".into(), "application/pdf".into()],
            ..DownloadSpec::default_profile()
        };
        assert!(s.should_auto_open("image/png"));
        assert!(s.should_auto_open("application/pdf"));
        assert!(!s.should_auto_open("application/zip"));
    }

    #[test]
    fn exceeds_size_cap_zero_is_unlimited() {
        let s = DownloadSpec::default_profile();
        assert!(!s.exceeds_size_cap(u64::MAX));
    }

    #[test]
    fn exceeds_size_cap_enforces_when_set() {
        let s = DownloadSpec {
            max_size_bytes: 1024,
            ..DownloadSpec::default_profile()
        };
        assert!(!s.exceeds_size_cap(500));
        assert!(!s.exceeds_size_cap(1024));
        assert!(s.exceeds_size_cap(2048));
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = DownloadRegistry::new();
        reg.insert(DownloadSpec::default_profile());
        reg.insert(DownloadSpec {
            concurrency: 10,
            ..DownloadSpec::default_profile()
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].concurrency, 10);
    }

    #[test]
    fn record_progress_and_terminal_state() {
        let mut r = DownloadRecord {
            id: "abc".into(),
            url: "https://example.com/x.pdf".into(),
            filename: "x.pdf".into(),
            target_path: "/tmp/x.pdf".into(),
            mime: "application/pdf".into(),
            size_bytes: 1_000,
            bytes_downloaded: 500,
            state: DownloadState::InProgress,
            content_hash: None,
            started_at: 1,
            completed_at: 0,
            error: None,
        };
        assert!((r.progress() - 0.5).abs() < 0.01);
        assert!(!r.is_terminal());
        r.state = DownloadState::Complete;
        assert!(r.is_terminal());
    }

    #[test]
    fn zero_size_progress_is_zero() {
        let r = DownloadRecord {
            id: "a".into(),
            url: "u".into(),
            filename: "f".into(),
            target_path: "t".into(),
            mime: "text/plain".into(),
            size_bytes: 0,
            bytes_downloaded: 0,
            state: DownloadState::Pending,
            content_hash: None,
            started_at: 0,
            completed_at: 0,
            error: None,
        };
        assert_eq!(r.progress(), 0.0);
    }

    #[test]
    fn blake3_content_hash_is_26_char_base32() {
        let h = blake3_content_hash(b"some-bytes");
        assert_eq!(h.len(), 26);
        for ch in h.chars() {
            assert!(ch.is_ascii_lowercase() || ch.is_ascii_digit());
        }
    }

    #[test]
    fn blake3_content_hash_is_deterministic() {
        assert_eq!(
            blake3_content_hash(b"same"),
            blake3_content_hash(b"same")
        );
    }

    #[test]
    fn blake3_content_hash_changes_on_input() {
        assert_ne!(
            blake3_content_hash(b"a"),
            blake3_content_hash(b"b")
        );
    }

    #[test]
    fn hash_verify_default_is_blake3() {
        assert_eq!(HashVerify::default(), HashVerify::Blake3);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_download_form() {
        let src = r#"
            (defdownload :name           "strict"
                         :folder         "~/secure-dl"
                         :quarantine-mime ("application/octet-stream"
                                           "application/zip")
                         :hash-verify    "sha256"
                         :concurrency    2
                         :resume         #t
                         :max-size-bytes 10485760)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "strict");
        assert_eq!(s.folder, "~/secure-dl");
        assert_eq!(s.hash_verify, HashVerify::Sha256);
        assert_eq!(s.concurrency, 2);
        assert_eq!(s.max_size_bytes, 10_485_760);
    }
}
