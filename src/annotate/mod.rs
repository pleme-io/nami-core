//! `(defannotate)` — highlight + comment annotations over pages.
//!
//! Absorbs Hypothesis (hypothes.is), Diigo, and Safari's built-in
//! PDF annotations into the substrate. An annotation is a
//! page-anchored note with text selection, color, and optional
//! commentary — persisted through `(defstorage)` so it round-trips
//! with the rest of the state.
//!
//! ```lisp
//! (defannotate :name       "default"
//!              :storage    "annotations"
//!              :colors     ("yellow" "pink" "green" "blue")
//!              :default-color "yellow"
//!              :shareable  #f
//!              :show-on-load #t)
//! ```
//!
//! The spec configures the annotation *profile* — which colors to
//! offer, which storage to write to, whether annotations are
//! sharable. The payload shape ([`Annotation`]) is the wire format
//! the UI produces + stores.

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Annotation profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defannotate"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotateSpec {
    pub name: String,
    /// Storage name — must match a `(defstorage)` declaration. The
    /// substrate wires the annotation write path through that store.
    pub storage: String,
    /// Allowed highlight color tokens. UI presents these as choices;
    /// an annotation with a color NOT in this list is rejected.
    #[serde(default = "default_colors")]
    pub colors: Vec<String>,
    /// Default color when the user doesn't pick one.
    #[serde(default = "default_color")]
    pub default_color: String,
    /// Whether the profile exposes "share via link" on annotations.
    /// Requires a compatible sync backend — stays UI-only until then.
    #[serde(default)]
    pub shareable: bool,
    /// Render existing annotations automatically on navigate.
    #[serde(default = "default_show_on_load")]
    pub show_on_load: bool,
    /// Maximum comment body length. `0` = unlimited.
    #[serde(default = "default_max_comment")]
    pub max_comment_chars: usize,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_colors() -> Vec<String> {
    vec![
        "yellow".into(),
        "pink".into(),
        "green".into(),
        "blue".into(),
        "orange".into(),
    ]
}
fn default_color() -> String {
    "yellow".into()
}
fn default_show_on_load() -> bool {
    true
}
fn default_max_comment() -> usize {
    10_000
}

impl AnnotateSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            storage: "annotations".into(),
            colors: default_colors(),
            default_color: default_color(),
            shareable: false,
            show_on_load: true,
            max_comment_chars: default_max_comment(),
            description: Some("Default annotation profile — 5 colors, auto-show.".into()),
        }
    }

    #[must_use]
    pub fn accepts_color(&self, color: &str) -> bool {
        self.colors.iter().any(|c| c == color)
    }
}

/// Selector anchor — where the annotation attaches in the DOM.
/// Multiple strategies supported for resilience across edits:
/// the consumer tries each in order and uses whichever still
/// resolves.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnchorSelector {
    /// CSS selector of the containing element (best-effort).
    #[serde(default)]
    pub css: Option<String>,
    /// TextQuoteSelector (Hypothesis-style) — exact + prefix + suffix
    /// lets us re-locate text even when the containing DOM shifts.
    #[serde(default)]
    pub text_quote: Option<TextQuote>,
    /// Byte offsets inside the serialized document body — fragile,
    /// but sometimes the only thing left after heavy SPA re-renders.
    #[serde(default)]
    pub byte_range: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextQuote {
    /// Exact selected text.
    pub exact: String,
    /// Up to ~64 chars before the selection — for disambiguation.
    #[serde(default)]
    pub prefix: Option<String>,
    /// Up to ~64 chars after the selection.
    #[serde(default)]
    pub suffix: Option<String>,
}

/// One stored annotation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    /// Stable content-addressed id — BLAKE3-shaped 26-char base32.
    pub id: String,
    /// Originating URL (exact).
    pub url: String,
    /// Selection anchor.
    pub anchor: AnchorSelector,
    /// Color token — must be in the profile's `colors` list.
    pub color: String,
    /// Optional commentary body.
    #[serde(default)]
    pub comment: Option<String>,
    /// Free-form tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds; `0` when never edited.
    #[serde(default)]
    pub updated_at: i64,
}

impl Annotation {
    /// Validate against a profile. Returns the first error string or `Ok(())`.
    pub fn validate_against(&self, spec: &AnnotateSpec) -> Result<(), String> {
        if !spec.accepts_color(&self.color) {
            return Err(format!(
                "color {:?} not in profile {:?}",
                self.color, spec.name
            ));
        }
        if spec.max_comment_chars > 0 {
            if let Some(c) = &self.comment {
                if c.chars().count() > spec.max_comment_chars {
                    return Err(format!(
                        "comment exceeds {} chars",
                        spec.max_comment_chars
                    ));
                }
            }
        }
        if self.url.is_empty() {
            return Err("annotation url is empty".into());
        }
        match (&self.anchor.css, &self.anchor.text_quote, &self.anchor.byte_range) {
            (None, None, None) => {
                Err("annotation must supply at least one anchor (css/text-quote/byte-range)"
                    .into())
            }
            _ => Ok(()),
        }
    }

    /// Content-hash the annotation's intrinsic fields → 26-char
    /// base32 (same shape as tameshi/sekiban). Useful as a stable id
    /// when the caller doesn't have one yet.
    #[must_use]
    pub fn content_id(&self) -> String {
        let mut snapshot = self.clone();
        snapshot.id = String::new();
        snapshot.updated_at = 0;
        let bytes = serde_json::to_vec(&snapshot).unwrap_or_default();
        let h = blake3::hash(&bytes);
        base32_16(&h.as_bytes()[..16])
    }
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

/// Registry of annotation profiles.
#[derive(Debug, Clone, Default)]
pub struct AnnotateRegistry {
    specs: Vec<AnnotateSpec>,
}

impl AnnotateRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: AnnotateSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = AnnotateSpec>) {
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
    pub fn specs(&self) -> &[AnnotateSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&AnnotateSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<AnnotateSpec>, String> {
    tatara_lisp::compile_typed::<AnnotateSpec>(src)
        .map_err(|e| format!("failed to compile defannotate forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<AnnotateSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_annotation() -> Annotation {
        Annotation {
            id: "abc".into(),
            url: "https://example.com/article".into(),
            anchor: AnchorSelector {
                css: Some("article > p:nth-child(3)".into()),
                text_quote: Some(TextQuote {
                    exact: "the interesting sentence".into(),
                    prefix: Some("…before ".into()),
                    suffix: Some(" after…".into()),
                }),
                byte_range: Some((1024, 1065)),
            },
            color: "yellow".into(),
            comment: Some("A thought on this.".into()),
            tags: vec!["important".into()],
            created_at: 1_700_000_000,
            updated_at: 0,
        }
    }

    #[test]
    fn default_profile_accepts_common_colors() {
        let s = AnnotateSpec::default_profile();
        for c in &["yellow", "pink", "green", "blue", "orange"] {
            assert!(s.accepts_color(c));
        }
        assert!(!s.accepts_color("neon"));
    }

    #[test]
    fn validate_requires_at_least_one_anchor() {
        let spec = AnnotateSpec::default_profile();
        let mut ann = sample_annotation();
        ann.anchor = AnchorSelector {
            css: None,
            text_quote: None,
            byte_range: None,
        };
        assert!(ann.validate_against(&spec).is_err());
    }

    #[test]
    fn validate_requires_url() {
        let spec = AnnotateSpec::default_profile();
        let mut ann = sample_annotation();
        ann.url = String::new();
        assert!(ann.validate_against(&spec).is_err());
    }

    #[test]
    fn validate_rejects_color_not_in_profile() {
        let spec = AnnotateSpec::default_profile();
        let mut ann = sample_annotation();
        ann.color = "neon".into();
        assert!(ann.validate_against(&spec).is_err());
    }

    #[test]
    fn validate_enforces_comment_length() {
        let spec = AnnotateSpec {
            max_comment_chars: 10,
            ..AnnotateSpec::default_profile()
        };
        let mut ann = sample_annotation();
        ann.comment = Some("x".repeat(20));
        assert!(ann.validate_against(&spec).is_err());
        ann.comment = Some("short".into());
        assert!(ann.validate_against(&spec).is_ok());
    }

    #[test]
    fn validate_zero_max_is_unlimited() {
        let spec = AnnotateSpec {
            max_comment_chars: 0,
            ..AnnotateSpec::default_profile()
        };
        let mut ann = sample_annotation();
        ann.comment = Some("x".repeat(50_000));
        assert!(ann.validate_against(&spec).is_ok());
    }

    #[test]
    fn content_id_is_26_char_base32() {
        let id = sample_annotation().content_id();
        assert_eq!(id.len(), 26);
        for ch in id.chars() {
            assert!(ch.is_ascii_lowercase() || ch.is_ascii_digit());
        }
    }

    #[test]
    fn content_id_is_deterministic() {
        let a = sample_annotation().content_id();
        let b = sample_annotation().content_id();
        assert_eq!(a, b);
    }

    #[test]
    fn content_id_changes_on_comment_edit() {
        let a = sample_annotation().content_id();
        let mut b = sample_annotation();
        b.comment = Some("different thought".into());
        assert_ne!(a, b.content_id());
    }

    #[test]
    fn content_id_ignores_updated_at() {
        let a = sample_annotation();
        let mut b = a.clone();
        b.updated_at = 999_999;
        // updated_at is excluded from the hash.
        assert_eq!(a.content_id(), b.content_id());
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = AnnotateRegistry::new();
        reg.insert(AnnotateSpec::default_profile());
        reg.insert(AnnotateSpec {
            shareable: true,
            ..AnnotateSpec::default_profile()
        });
        assert_eq!(reg.len(), 1);
        assert!(reg.specs()[0].shareable);
    }

    #[test]
    fn annotation_roundtrips_through_json() {
        let a = sample_annotation();
        let json = serde_json::to_string(&a).unwrap();
        let back: Annotation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_annotate_form() {
        let src = r#"
            (defannotate :name      "reader"
                         :storage   "reader-notes"
                         :colors    ("yellow" "pink")
                         :default-color "pink"
                         :shareable #t
                         :show-on-load #t
                         :max-comment-chars 500)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "reader");
        assert_eq!(s.storage, "reader-notes");
        assert_eq!(s.default_color, "pink");
        assert!(s.shareable);
        assert_eq!(s.max_comment_chars, 500);
    }
}
