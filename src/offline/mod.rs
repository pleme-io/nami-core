//! `(defoffline)` — declarative save-for-later / offline cache.
//!
//! Absorbs Pocket, Instapaper, Raindrop, Safari Reading List, Firefox
//! Read It Later, Chrome "Save for later". Each profile declares a
//! storage namespace, TTL, and how much of the page to snapshot.
//!
//! ```lisp
//! (defoffline :name             "reading-list"
//!             :storage           "offline-pages"
//!             :ttl-seconds       604800
//!             :include-assets    (:html :images :css)
//!             :max-size-bytes    5242880
//!             :auto-save-tags    ("longform" "research"))
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Asset classes to include in the offline snapshot.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum AssetKind {
    /// The HTML document (always on — listing it is a no-op, but
    /// explicit makes the `:include-assets` list self-documenting).
    Html,
    /// Stylesheets.
    Css,
    /// Images (raster + svg).
    Images,
    /// Fonts.
    Fonts,
    /// JS (rarely useful offline; opt-in).
    Scripts,
    /// Embedded media (video/audio).
    Media,
}

/// Offline-save profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defoffline"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OfflineSpec {
    pub name: String,
    /// (defstorage) name holding the offline entries.
    pub storage: String,
    /// Time before an entry expires. `0` = never.
    #[serde(default = "default_ttl")]
    pub ttl_seconds: u64,
    /// Asset classes to fetch + cache alongside the HTML.
    #[serde(default = "default_assets")]
    pub include_assets: Vec<AssetKind>,
    /// Per-entry size cap in bytes. `0` = unlimited.
    #[serde(default = "default_max_size")]
    pub max_size_bytes: u64,
    /// Pages tagged with any of these auto-save on navigate.
    #[serde(default)]
    pub auto_save_tags: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_ttl() -> u64 {
    604_800 // 7 days
}

fn default_assets() -> Vec<AssetKind> {
    vec![AssetKind::Html, AssetKind::Css, AssetKind::Images]
}

fn default_max_size() -> u64 {
    10 * 1024 * 1024 // 10 MiB
}

impl OfflineSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            storage: "offline-pages".into(),
            ttl_seconds: default_ttl(),
            include_assets: default_assets(),
            max_size_bytes: default_max_size(),
            auto_save_tags: vec![],
            description: Some(
                "Default offline — HTML+CSS+images, 7-day TTL, 10MiB cap.".into(),
            ),
        }
    }

    #[must_use]
    pub fn includes(&self, kind: AssetKind) -> bool {
        self.include_assets.contains(&kind)
    }

    /// Would this profile auto-save a page tagged with any of `tags`?
    #[must_use]
    pub fn auto_saves(&self, tags: &[String]) -> bool {
        if self.auto_save_tags.is_empty() {
            return false;
        }
        tags.iter().any(|t| self.auto_save_tags.contains(t))
    }
}

/// One saved entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OfflineEntry {
    pub url: String,
    pub title: String,
    /// Unix seconds.
    pub saved_at: i64,
    /// Snapshot bytes (may be content-addressed elsewhere; this is the
    /// index row). Empty when the actual blob lives in a content store.
    #[serde(default)]
    pub size_bytes: u64,
    /// BLAKE3 content hash (26-char base32 — tameshi-shape).
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct OfflineRegistry {
    specs: Vec<OfflineSpec>,
}

impl OfflineRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: OfflineSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = OfflineSpec>) {
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
    pub fn specs(&self) -> &[OfflineSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&OfflineSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<OfflineSpec>, String> {
    tatara_lisp::compile_typed::<OfflineSpec>(src)
        .map_err(|e| format!("failed to compile defoffline forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<OfflineSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_includes_html_css_images() {
        let s = OfflineSpec::default_profile();
        assert!(s.includes(AssetKind::Html));
        assert!(s.includes(AssetKind::Css));
        assert!(s.includes(AssetKind::Images));
        assert!(!s.includes(AssetKind::Scripts));
    }

    #[test]
    fn auto_saves_checks_tag_overlap() {
        let s = OfflineSpec {
            auto_save_tags: vec!["longform".into(), "research".into()],
            ..OfflineSpec::default_profile()
        };
        assert!(s.auto_saves(&["longform".into()]));
        assert!(s.auto_saves(&["research".into(), "misc".into()]));
        assert!(!s.auto_saves(&["misc".into()]));
    }

    #[test]
    fn auto_saves_empty_tags_never_triggers() {
        let s = OfflineSpec::default_profile();
        assert!(!s.auto_saves(&["anything".into()]));
    }

    #[test]
    fn default_ttl_is_one_week() {
        assert_eq!(OfflineSpec::default_profile().ttl_seconds, 604_800);
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = OfflineRegistry::new();
        reg.insert(OfflineSpec::default_profile());
        reg.insert(OfflineSpec {
            max_size_bytes: 100,
            ..OfflineSpec::default_profile()
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].max_size_bytes, 100);
    }

    #[test]
    fn asset_kind_roundtrips_through_serde() {
        for k in [
            AssetKind::Html,
            AssetKind::Css,
            AssetKind::Images,
            AssetKind::Fonts,
            AssetKind::Scripts,
            AssetKind::Media,
        ] {
            let json = serde_json::to_string(&k).unwrap();
            let back: AssetKind = serde_json::from_str(&json).unwrap();
            assert_eq!(k, back);
        }
    }

    #[test]
    fn entry_roundtrips_through_json() {
        let e = OfflineEntry {
            url: "https://example.com/article".into(),
            title: "A Long Read".into(),
            saved_at: 1_700_000_000,
            size_bytes: 42_000,
            content_hash: Some("abcdefghijklmnopqrstuvwxy2".into()),
            tags: vec!["longform".into()],
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: OfflineEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_offline_form() {
        let src = r#"
            (defoffline :name           "reading"
                        :storage        "offline-pages"
                        :ttl-seconds    86400
                        :include-assets ("html" "css" "images")
                        :max-size-bytes 1048576
                        :auto-save-tags ("longform"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "reading");
        assert_eq!(s.ttl_seconds, 86400);
        assert_eq!(s.max_size_bytes, 1_048_576);
        assert!(s.auto_saves(&["longform".into()]));
    }
}
