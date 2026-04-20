//! `(defautofill)` — form-autofill profile.
//!
//! Absorbs Chrome/Firefox/Safari autofill, 1Password field
//! detection, and LastPass form-fill. Each profile holds a set of
//! typed field entries — address / name / email / phone / card —
//! plus site-specific field-pattern overrides for forms that don't
//! match the heuristic autodetection.
//!
//! ```lisp
//! (defautofill :name "personal"
//!              :fields ((:kind email       :value "me@example.com")
//!                       (:kind full-name   :value "Jane Doe")
//!                       (:kind street      :value "742 Evergreen Terrace")
//!                       (:kind city        :value "Springfield")
//!                       (:kind postal-code :value "54321")
//!                       (:kind phone       :value "+1-555-0100"))
//!              :overrides
//!              ((:host "*://*.example.com/*"
//!                :selector "#shipping_addr1"
//!                :kind street)))
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Common semantic field categories. The autofill engine matches
/// form inputs to these via HTML autocomplete hints + heuristics +
/// user-supplied overrides.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum FieldKind {
    FullName,
    FirstName,
    LastName,
    Email,
    Phone,
    Street,
    City,
    Region,
    PostalCode,
    Country,
    Organization,
    Website,
    Username,
    // Credit-card fields — autofill only when the user opts in per-form.
    CardNumber,
    CardholderName,
    CardExpiry,
    CardCvc,
    /// Custom / freeform field the heuristics don't cover.
    Custom,
}

/// One field value in a profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FieldEntry {
    pub kind: FieldKind,
    pub value: String,
    /// Freeform label — only required when `kind == Custom`.
    #[serde(default)]
    pub label: Option<String>,
}

/// Per-site field-selector override — lets the profile fire on
/// forms whose input attributes don't match the heuristic dictionary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FieldOverride {
    /// Host glob.
    pub host: String,
    /// CSS selector for the input element.
    pub selector: String,
    /// Which entry kind to fill here.
    pub kind: FieldKind,
    /// Custom label when kind == Custom.
    #[serde(default)]
    pub label: Option<String>,
}

/// One autofill profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defautofill"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AutofillSpec {
    pub name: String,
    #[serde(default)]
    pub fields: Vec<FieldEntry>,
    #[serde(default)]
    pub overrides: Vec<FieldOverride>,
    /// Require user confirm on first use per-host (anti-phishing).
    #[serde(default = "default_confirm_first_use")]
    pub confirm_first_use: bool,
    /// Pause profile on this host list (e.g., banking).
    #[serde(default)]
    pub excluded_hosts: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_confirm_first_use() -> bool {
    true
}

impl AutofillSpec {
    /// Fetch the value for a given kind (+ optional label for Custom).
    #[must_use]
    pub fn value_for(&self, kind: FieldKind, label: Option<&str>) -> Option<&str> {
        self.fields
            .iter()
            .find(|e| {
                e.kind == kind
                    && (kind != FieldKind::Custom
                        || e.label.as_deref() == label)
            })
            .map(|e| e.value.as_str())
    }

    /// Is this profile excluded from `host`?
    #[must_use]
    pub fn is_excluded_from(&self, host: &str) -> bool {
        self.excluded_hosts
            .iter()
            .any(|g| crate::extension::glob_match_host(g, host))
    }

    /// Resolve a (host, selector) pair against the override list.
    /// Returns the field kind an override asserts for this input.
    #[must_use]
    pub fn override_for(&self, host: &str, selector: &str) -> Option<FieldKind> {
        self.overrides.iter().find_map(|o| {
            if o.selector == selector && crate::extension::glob_match_host(&o.host, host) {
                Some(o.kind)
            } else {
                None
            }
        })
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct AutofillRegistry {
    specs: Vec<AutofillSpec>,
}

impl AutofillRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: AutofillSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = AutofillSpec>) {
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
    pub fn specs(&self) -> &[AutofillSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&AutofillSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<AutofillSpec>, String> {
    tatara_lisp::compile_typed::<AutofillSpec>(src)
        .map_err(|e| format!("failed to compile defautofill forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<AutofillSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AutofillSpec {
        AutofillSpec {
            name: "personal".into(),
            fields: vec![
                FieldEntry {
                    kind: FieldKind::Email,
                    value: "me@example.com".into(),
                    label: None,
                },
                FieldEntry {
                    kind: FieldKind::FullName,
                    value: "Jane Doe".into(),
                    label: None,
                },
            ],
            overrides: vec![FieldOverride {
                host: "*://*.example.com/*".into(),
                selector: "#shipping_addr1".into(),
                kind: FieldKind::Street,
                label: None,
            }],
            confirm_first_use: true,
            excluded_hosts: vec!["*://*.bank.com/*".into()],
            description: None,
        }
    }

    #[test]
    fn value_for_returns_matching_field() {
        let s = sample();
        assert_eq!(
            s.value_for(FieldKind::Email, None),
            Some("me@example.com")
        );
        assert_eq!(
            s.value_for(FieldKind::FullName, None),
            Some("Jane Doe")
        );
        assert!(s.value_for(FieldKind::Phone, None).is_none());
    }

    #[test]
    fn custom_field_requires_label_match() {
        let s = AutofillSpec {
            fields: vec![
                FieldEntry {
                    kind: FieldKind::Custom,
                    value: "alpha-token".into(),
                    label: Some("api-key".into()),
                },
                FieldEntry {
                    kind: FieldKind::Custom,
                    value: "beta-token".into(),
                    label: Some("other-key".into()),
                },
            ],
            ..sample()
        };
        assert_eq!(
            s.value_for(FieldKind::Custom, Some("api-key")),
            Some("alpha-token")
        );
        assert_eq!(
            s.value_for(FieldKind::Custom, Some("other-key")),
            Some("beta-token")
        );
        assert!(s.value_for(FieldKind::Custom, Some("missing")).is_none());
    }

    #[test]
    fn excluded_from_check() {
        let s = sample();
        assert!(s.is_excluded_from("online.bank.com"));
        assert!(!s.is_excluded_from("shop.example.com"));
    }

    #[test]
    fn override_for_resolves_selector_and_host() {
        let s = sample();
        let kind = s.override_for("shop.example.com", "#shipping_addr1");
        assert_eq!(kind, Some(FieldKind::Street));
        assert!(s.override_for("shop.example.com", "#other").is_none());
        assert!(s
            .override_for("other.com", "#shipping_addr1")
            .is_none());
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = AutofillRegistry::new();
        reg.insert(sample());
        reg.insert(AutofillSpec {
            fields: vec![],
            ..sample()
        });
        assert_eq!(reg.len(), 1);
        assert!(reg.specs()[0].fields.is_empty());
    }

    #[test]
    fn field_kind_roundtrips_through_serde() {
        for k in [
            FieldKind::FullName,
            FieldKind::Email,
            FieldKind::PostalCode,
            FieldKind::CardNumber,
            FieldKind::Custom,
        ] {
            let json = serde_json::to_string(&k).unwrap();
            let back: FieldKind = serde_json::from_str(&json).unwrap();
            assert_eq!(k, back);
        }
    }

    #[test]
    fn confirm_first_use_defaults_true() {
        assert!(sample().confirm_first_use);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_autofill_form() {
        let src = r#"
            (defautofill :name "personal"
                         :fields ((:kind "email"    :value "me@example.com")
                                  (:kind "full-name" :value "Jane"))
                         :confirm-first-use #t
                         :excluded-hosts ("*://*.bank.com/*"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "personal");
        assert_eq!(s.fields.len(), 2);
        assert!(s.confirm_first_use);
        assert_eq!(s.excluded_hosts.len(), 1);
    }
}
