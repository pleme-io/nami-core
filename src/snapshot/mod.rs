//! `(defsnapshot)` — declarative page-snapshot recipes.
//!
//! Absorbs Firefox "Take a Screenshot", Chrome DevTools full-page
//! capture, and mobile Safari's long-screenshot into a substrate DSL.
//! Each snapshot recipe declares region, format, per-host overrides,
//! and an optional sekiban-compatible content hash pass.
//!
//! The engine only **specifies** snapshots — actual pixel capture
//! lives in the GPU layer (namimado-gpu). Nami-core models the
//! request, resolves the right recipe for a given host, and provides
//! a BLAKE3 digest API so attestation works whether the bytes come
//! from wgpu or a headless rasterizer.
//!
//! ```lisp
//! (defsnapshot :name "full-page"
//!              :region :full-page
//!              :format :png
//!              :scale  2.0
//!              :host   "*"
//!              :attest #t)
//!
//! (defsnapshot :name "viewport-jpeg"
//!              :region :viewport
//!              :format :jpeg
//!              :quality 85)
//!
//! (defsnapshot :name "selector-crop"
//!              :region :selector
//!              :selector "article"
//!              :format :png)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Capture region.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Region {
    /// Just what's visible right now.
    Viewport,
    /// Entire scroll-height page.
    FullPage,
    /// Crop bounded by a CSS selector (resolved at capture time).
    Selector,
    /// Single DOM element by id.
    Element,
}

impl Default for Region {
    fn default() -> Self {
        Self::Viewport
    }
}

/// Output format.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    Png,
    Jpeg,
    Webp,
}

impl Default for Format {
    fn default() -> Self {
        Self::Png
    }
}

/// Snapshot recipe.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsnapshot"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSpec {
    pub name: String,
    #[serde(default)]
    pub region: Region,
    #[serde(default)]
    pub format: Format,
    /// DPI multiplier — 2.0 for retina, 1.0 for baseline.
    #[serde(default = "default_scale")]
    pub scale: f32,
    /// JPEG/WebP quality [0.0, 1.0]. Ignored for PNG.
    #[serde(default = "default_quality")]
    pub quality: f32,
    /// CSS selector when region == Selector.
    #[serde(default)]
    pub selector: Option<String>,
    /// Element id when region == Element.
    #[serde(default)]
    pub element_id: Option<String>,
    /// Host glob; `"*"` matches all.
    #[serde(default = "default_host")]
    pub host: String,
    /// When true, callers should BLAKE3 the captured bytes and
    /// attach the hash to a SignatureBundle (sekiban-compatible).
    #[serde(default)]
    pub attest: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_scale() -> f32 {
    2.0
}
fn default_quality() -> f32 {
    0.9
}
fn default_host() -> String {
    "*".into()
}

impl SnapshotSpec {
    /// Built-in "full page PNG at 2x + attest" profile.
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            region: Region::FullPage,
            format: Format::Png,
            scale: 2.0,
            quality: 0.9,
            selector: None,
            element_id: None,
            host: "*".into(),
            attest: true,
            description: Some("Full-page PNG at 2x, sekiban-attested.".into()),
        }
    }

    /// Does this recipe apply to `host`?
    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        if self.host.is_empty() || self.host == "*" {
            return true;
        }
        crate::extension::glob_match_host(&self.host, host)
    }

    /// Clamp out-of-range quality to `[0.0, 1.0]`.
    #[must_use]
    pub fn clamped_quality(&self) -> f32 {
        self.quality.clamp(0.0, 1.0)
    }

    /// Clamp out-of-range scale to `[0.25, 4.0]`.
    #[must_use]
    pub fn clamped_scale(&self) -> f32 {
        self.scale.clamp(0.25, 4.0)
    }
}

/// Registry of snapshot recipes.
#[derive(Debug, Clone, Default)]
pub struct SnapshotRegistry {
    specs: Vec<SnapshotSpec>,
}

impl SnapshotRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SnapshotSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SnapshotSpec>) {
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
    pub fn specs(&self) -> &[SnapshotSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&SnapshotSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// First recipe whose host filter matches `host` (insertion order).
    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&SnapshotSpec> {
        self.specs.iter().find(|s| s.matches_host(host))
    }
}

/// Compute a sekiban-compatible BLAKE3 digest of captured bytes.
/// Same hash shape as extension::signature::canonical_hash — 128
/// bits → 26-char base32. Use this for snapshot-pinned attestation.
#[must_use]
pub fn attest(bytes: &[u8]) -> String {
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

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SnapshotSpec>, String> {
    tatara_lisp::compile_typed::<SnapshotSpec>(src)
        .map_err(|e| format!("failed to compile defsnapshot forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SnapshotSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_full_page_png_2x() {
        let s = SnapshotSpec::default_profile();
        assert_eq!(s.region, Region::FullPage);
        assert_eq!(s.format, Format::Png);
        assert!((s.scale - 2.0).abs() < f32::EPSILON);
        assert!(s.attest);
    }

    #[test]
    fn host_match_reuses_extension_glob() {
        let s = SnapshotSpec {
            host: "*://*.example.com/*".into(),
            ..SnapshotSpec::default_profile()
        };
        assert!(s.matches_host("blog.example.com"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn wildcard_matches_everything() {
        let s = SnapshotSpec::default_profile();
        assert!(s.matches_host("anywhere.com"));
    }

    #[test]
    fn clamped_quality_bounds_to_unit_interval() {
        let s = SnapshotSpec {
            quality: 5.0,
            ..SnapshotSpec::default_profile()
        };
        assert!((s.clamped_quality() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn clamped_scale_bounds_to_valid_range() {
        let s = SnapshotSpec {
            scale: 99.0,
            ..SnapshotSpec::default_profile()
        };
        assert!((s.clamped_scale() - 4.0).abs() < f32::EPSILON);
        let s2 = SnapshotSpec {
            scale: 0.01,
            ..SnapshotSpec::default_profile()
        };
        assert!((s2.clamped_scale() - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = SnapshotRegistry::new();
        reg.insert(SnapshotSpec::default_profile());
        reg.insert(SnapshotSpec {
            format: Format::Jpeg,
            ..SnapshotSpec::default_profile()
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("default").unwrap().format, Format::Jpeg);
    }

    #[test]
    fn resolve_returns_first_matching_recipe() {
        let mut reg = SnapshotRegistry::new();
        reg.insert(SnapshotSpec {
            name: "default".into(),
            host: "*".into(),
            ..SnapshotSpec::default_profile()
        });
        reg.insert(SnapshotSpec {
            name: "gh".into(),
            host: "*://*.github.com/*".into(),
            ..SnapshotSpec::default_profile()
        });
        // Order matters — default wins because it was inserted first.
        // Callers who want host-specific priority should insert those
        // first, like the security-policy registry demonstrates.
        let hit = reg.resolve("blog.github.com").unwrap();
        assert_eq!(hit.name, "default");
    }

    #[test]
    fn attest_produces_26_char_base32_hash() {
        let h = attest(b"snapshot-bytes-here");
        assert_eq!(h.len(), 26);
        for ch in h.chars() {
            assert!(ch.is_ascii_lowercase() || ch.is_ascii_digit());
        }
    }

    #[test]
    fn attest_is_deterministic() {
        let a = attest(b"same bytes");
        let b = attest(b"same bytes");
        assert_eq!(a, b);
    }

    #[test]
    fn attest_changes_on_payload_mutation() {
        let a = attest(b"snapshot-bytes-here");
        let b = attest(b"snapshot-bytes-here!");
        assert_ne!(a, b);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_snapshot_form() {
        let src = r#"
            (defsnapshot :name    "viewport-jpeg"
                         :region  "viewport"
                         :format  "jpeg"
                         :quality 0.85
                         :scale   1.0
                         :attest  #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "viewport-jpeg");
        assert_eq!(s.region, Region::Viewport);
        assert_eq!(s.format, Format::Jpeg);
        assert!((s.quality - 0.85).abs() < 0.001);
        assert!(s.attest);
    }
}
