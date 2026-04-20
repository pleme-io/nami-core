//! `(deflocale)` — per-host locale override.
//!
//! Absorbs Chrome/Firefox/Safari `Accept-Language` + navigator.
//! language + navigator.languages + `Intl.*` defaults. Browsers
//! apply locale globally; no UI lets you say "read nytimes.com in
//! English but google.jp in Japanese". This DSL fills the gap by
//! declaring per-host overrides for every locale surface the web
//! platform exposes.
//!
//! ```lisp
//! (deflocale :name        "japan-japanese"
//!            :host        "*://*.example.jp/*"
//!            :primary     "ja-JP"
//!            :accept-languages ("ja-JP" "ja" "en;q=0.5")
//!            :timezone    "Asia/Tokyo"
//!            :date-format :ja-jp
//!            :first-day-of-week :monday
//!            :measurement :metric
//!            :currency    "JPY"
//!            :numbering-system :latn)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Measurement system override.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Measurement {
    /// Honor whatever the locale default ships (en-US = imperial,
    /// rest = metric). No override.
    #[default]
    Passthrough,
    Metric,
    Imperial,
    /// UK style (miles + kg).
    Uk,
}

/// First day of the week override (per calendar locale).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FirstDay {
    #[default]
    Locale,
    Sunday,
    Monday,
    Saturday,
}

/// Intl.NumberFormat numbering-system hint.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum NumberingSystem {
    #[default]
    Latn,
    Arab,
    Arabext,
    Beng,
    Deva,
    Thai,
    Hanidec,
    Roman,
}

/// Time-format preference (12-hour vs 24-hour).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TimeFormat {
    #[default]
    Locale,
    H12,
    H24,
}

/// Date-format style.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DateFormat {
    #[default]
    Locale,
    /// MM/DD/YYYY.
    UsSlash,
    /// DD/MM/YYYY.
    EuSlash,
    /// YYYY-MM-DD (ISO 8601).
    Iso,
    /// YYYY/MM/DD (Japan).
    JaJp,
    /// DD.MM.YYYY (DACH / Russia).
    Dotted,
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "deflocale"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocaleSpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    /// Primary locale tag (e.g. "en-US", "ja-JP", "de-AT"). Empty =
    /// passthrough.
    #[serde(default)]
    pub primary: String,
    /// Ordered Accept-Language list — rendered with q-values.
    /// Entries may include a q-value suffix `"en;q=0.5"` verbatim;
    /// otherwise the renderer auto-scales.
    #[serde(default)]
    pub accept_languages: Vec<String>,
    /// IANA timezone (e.g. "Asia/Tokyo"). Empty = device default.
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub date_format: DateFormat,
    #[serde(default)]
    pub time_format: TimeFormat,
    #[serde(default)]
    pub first_day_of_week: FirstDay,
    #[serde(default)]
    pub measurement: Measurement,
    /// ISO 4217 currency code override (e.g. "JPY", "EUR").
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub numbering_system: NumberingSystem,
    /// Override navigator.language / .languages reported to JS —
    /// turning this off preserves the device-level value while
    /// still swapping Accept-Language at the HTTP layer (useful for
    /// anti-fingerprint).
    #[serde(default = "default_expose_to_js")]
    pub expose_to_js: bool,
    /// Hosts exempt from this profile (e.g. bank that needs native locale).
    #[serde(default)]
    pub exempt_hosts: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_expose_to_js() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

impl LocaleSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            primary: String::new(),
            accept_languages: vec![],
            timezone: String::new(),
            date_format: DateFormat::Locale,
            time_format: TimeFormat::Locale,
            first_day_of_week: FirstDay::Locale,
            measurement: Measurement::Passthrough,
            currency: None,
            numbering_system: NumberingSystem::Latn,
            expose_to_js: true,
            exempt_hosts: vec![],
            enabled: true,
            description: Some(
                "Default locale — passthrough. Set primary + accept-languages to override.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        if self.host.is_empty() || self.host == "*" {
            return true;
        }
        crate::extension::glob_match_host(&self.host, host)
    }

    #[must_use]
    pub fn is_exempt(&self, host: &str) -> bool {
        self.exempt_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    /// Render the Accept-Language HTTP header value. Entries that
    /// already contain `;q=` are passed through; bare tags get
    /// auto-scaled q-values descending from 1.0.
    #[must_use]
    pub fn render_accept_language(&self) -> String {
        if self.accept_languages.is_empty() {
            return self.primary.clone();
        }
        let mut out: Vec<String> = Vec::with_capacity(self.accept_languages.len());
        let mut implicit_idx = 0usize;
        for entry in &self.accept_languages {
            if entry.contains(";q=") || entry.contains("; q=") {
                out.push(entry.clone());
                continue;
            }
            // Auto-scale — first entry = q=1.0 (omit), subsequent
            // descend by 0.1 but never below 0.1.
            let q = if implicit_idx == 0 {
                None
            } else {
                Some(format!("{:.1}", (1.0 - 0.1 * implicit_idx as f32).max(0.1)))
            };
            implicit_idx += 1;
            out.push(match q {
                None => entry.clone(),
                Some(v) => format!("{entry};q={v}"),
            });
        }
        out.join(", ")
    }

    /// Primary language tag — `primary` if set, else first entry of
    /// `accept_languages`, else empty.
    #[must_use]
    pub fn primary_tag(&self) -> &str {
        if !self.primary.is_empty() {
            return &self.primary;
        }
        self.accept_languages
            .first()
            .map(String::as_str)
            .unwrap_or("")
    }

    /// The navigator.languages array as the JS engine should see it.
    /// Strips q-values.
    #[must_use]
    pub fn js_languages(&self) -> Vec<String> {
        if !self.expose_to_js {
            return vec![];
        }
        let mut out: Vec<String> = Vec::with_capacity(self.accept_languages.len() + 1);
        if !self.primary.is_empty() {
            out.push(self.primary.clone());
        }
        for e in &self.accept_languages {
            let tag = e
                .split([';', ','])
                .next()
                .unwrap_or("")
                .trim()
                .to_owned();
            if !tag.is_empty() && !out.contains(&tag) {
                out.push(tag);
            }
        }
        out
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct LocaleRegistry {
    specs: Vec<LocaleSpec>,
}

impl LocaleRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: LocaleSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = LocaleSpec>) {
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
    pub fn specs(&self) -> &[LocaleSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&LocaleSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<LocaleSpec>, String> {
    tatara_lisp::compile_typed::<LocaleSpec>(src)
        .map_err(|e| format!("failed to compile deflocale forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<LocaleSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_passthrough() {
        let s = LocaleSpec::default_profile();
        assert!(s.primary.is_empty());
        assert!(s.accept_languages.is_empty());
        assert!(s.expose_to_js);
    }

    #[test]
    fn render_accept_language_auto_scales_q_values() {
        let s = LocaleSpec {
            accept_languages: vec!["ja-JP".into(), "ja".into(), "en".into()],
            ..LocaleSpec::default_profile()
        };
        let h = s.render_accept_language();
        assert_eq!(h, "ja-JP, ja;q=0.9, en;q=0.8");
    }

    #[test]
    fn render_accept_language_passes_through_explicit_q() {
        let s = LocaleSpec {
            accept_languages: vec!["ja-JP".into(), "en;q=0.5".into(), "de;q=0.3".into()],
            ..LocaleSpec::default_profile()
        };
        let h = s.render_accept_language();
        assert_eq!(h, "ja-JP, en;q=0.5, de;q=0.3");
    }

    #[test]
    fn render_accept_language_empty_falls_back_to_primary() {
        let s = LocaleSpec {
            primary: "fr-FR".into(),
            ..LocaleSpec::default_profile()
        };
        assert_eq!(s.render_accept_language(), "fr-FR");
    }

    #[test]
    fn render_accept_language_q_floor_is_point_one() {
        let langs: Vec<String> = (0..15).map(|i| format!("l{i}")).collect();
        let s = LocaleSpec {
            accept_languages: langs,
            ..LocaleSpec::default_profile()
        };
        let h = s.render_accept_language();
        // The last few entries should clamp to q=0.1 (auto-scale
        // never bottoms out below it).
        assert!(h.ends_with("q=0.1"));
    }

    #[test]
    fn primary_tag_prefers_primary_field() {
        let s = LocaleSpec {
            primary: "fr-FR".into(),
            accept_languages: vec!["en".into()],
            ..LocaleSpec::default_profile()
        };
        assert_eq!(s.primary_tag(), "fr-FR");
    }

    #[test]
    fn primary_tag_falls_back_to_first_accept() {
        let s = LocaleSpec {
            primary: String::new(),
            accept_languages: vec!["ja".into(), "en".into()],
            ..LocaleSpec::default_profile()
        };
        assert_eq!(s.primary_tag(), "ja");
    }

    #[test]
    fn primary_tag_empty_when_no_fields() {
        let s = LocaleSpec::default_profile();
        assert_eq!(s.primary_tag(), "");
    }

    #[test]
    fn js_languages_strips_q_values_and_dedupes() {
        let s = LocaleSpec {
            primary: "ja-JP".into(),
            accept_languages: vec![
                "ja-JP".into(),
                "ja;q=0.9".into(),
                "en;q=0.5".into(),
                "en;q=0.3".into(),
            ],
            ..LocaleSpec::default_profile()
        };
        let js = s.js_languages();
        assert_eq!(js, vec!["ja-JP", "ja", "en"]);
    }

    #[test]
    fn js_languages_empty_when_exposed_off() {
        let s = LocaleSpec {
            primary: "ja-JP".into(),
            expose_to_js: false,
            ..LocaleSpec::default_profile()
        };
        assert!(s.js_languages().is_empty());
    }

    #[test]
    fn is_exempt_matches_glob() {
        let s = LocaleSpec {
            exempt_hosts: vec!["*://*.bank.com/*".into()],
            ..LocaleSpec::default_profile()
        };
        assert!(s.is_exempt("my.bank.com"));
        assert!(!s.is_exempt("example.com"));
    }

    #[test]
    fn measurement_roundtrips_through_serde() {
        for m in [
            Measurement::Passthrough,
            Measurement::Metric,
            Measurement::Imperial,
            Measurement::Uk,
        ] {
            let s = LocaleSpec {
                measurement: m,
                ..LocaleSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: LocaleSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.measurement, m);
        }
    }

    #[test]
    fn date_format_roundtrips_through_serde() {
        for d in [
            DateFormat::Locale,
            DateFormat::UsSlash,
            DateFormat::EuSlash,
            DateFormat::Iso,
            DateFormat::JaJp,
            DateFormat::Dotted,
        ] {
            let s = LocaleSpec {
                date_format: d,
                ..LocaleSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: LocaleSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.date_format, d);
        }
    }

    #[test]
    fn time_format_first_day_numbering_system_roundtrip() {
        let s = LocaleSpec {
            time_format: TimeFormat::H24,
            first_day_of_week: FirstDay::Monday,
            numbering_system: NumberingSystem::Arab,
            ..LocaleSpec::default_profile()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: LocaleSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.time_format, TimeFormat::H24);
        assert_eq!(back.first_day_of_week, FirstDay::Monday);
        assert_eq!(back.numbering_system, NumberingSystem::Arab);
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = LocaleRegistry::new();
        reg.insert(LocaleSpec::default_profile());
        reg.insert(LocaleSpec {
            name: "jp".into(),
            host: "*://*.example.jp/*".into(),
            primary: "ja-JP".into(),
            ..LocaleSpec::default_profile()
        });
        let jp = reg.resolve("www.example.jp").unwrap();
        assert_eq!(jp.primary, "ja-JP");
        let other = reg.resolve("example.com").unwrap();
        assert_eq!(other.name, "default");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = LocaleRegistry::new();
        reg.insert(LocaleSpec {
            enabled: false,
            ..LocaleSpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_locale_form() {
        let src = r#"
            (deflocale :name "japan-japanese"
                       :host "*://*.example.jp/*"
                       :primary "ja-JP"
                       :accept-languages ("ja-JP" "ja" "en;q=0.5")
                       :timezone "Asia/Tokyo"
                       :date-format "ja-jp"
                       :first-day-of-week "monday"
                       :measurement "metric"
                       :currency "JPY"
                       :numbering-system "latn"
                       :expose-to-js #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.primary, "ja-JP");
        assert_eq!(s.timezone, "Asia/Tokyo");
        assert_eq!(s.date_format, DateFormat::JaJp);
        assert_eq!(s.first_day_of_week, FirstDay::Monday);
        assert_eq!(s.measurement, Measurement::Metric);
        assert_eq!(s.currency.as_deref(), Some("JPY"));
    }
}
