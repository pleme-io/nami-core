//! Framework-aliased selectors.
//!
//! Users author `@card` once; the resolver picks the right raw
//! selector based on which framework actually produced the page.
//!
//! ```lisp
//! (defframework-alias :name "@card"
//!                     :shadcn   "[data-slot=\"card\"]"
//!                     :mui      "div.MuiCard-root"
//!                     :bootstrap "div.card"
//!                     :tailwind "div.card"
//!                     :fallback "div.card")
//!
//! ; The transform can be written once and work against any site:
//! (defdom-transform :name "pretty-cards"
//!                   :selector "@card"
//!                   :action add-class
//!                   :arg "nami-card")
//! ```
//!
//! Alias resolution rule: iterate detected frameworks in confidence
//! order (as returned by [`crate::framework::detect`]); the first one
//! with a non-`None` field on the [`AliasSpec`] wins. If no detected
//! framework matches, use [`AliasSpec::fallback`].
//!
//! This is pure data + string substitution. No new runtime behavior
//! beyond expansion. The selector it expands TO then runs through
//! [`crate::selector::Selector`] exactly as if the user had typed the
//! raw selector themselves.

use crate::framework::{Detection, Framework};
use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// A named alias with per-framework expansions + a fallback.
///
/// All per-framework fields are `Option<String>` — you only list the
/// frameworks you want to target. If nothing matches, `fallback` is
/// used, which is the one required field (so every alias resolves).
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defframework-alias"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AliasSpec {
    /// The alias identifier, e.g. `"@card"`. Conventionally starts with `@`.
    pub name: String,
    /// Selector to use when the detected frameworks don't match any
    /// specific field. REQUIRED so every alias resolves to *something*.
    pub fallback: String,
    #[serde(default)]
    pub description: Option<String>,

    // ── per-framework overrides (all optional) ─────────────────────
    #[serde(default)]
    pub shadcn: Option<String>,
    #[serde(default)]
    pub mui: Option<String>,
    #[serde(default)]
    pub tailwind: Option<String>,
    #[serde(default)]
    pub bootstrap: Option<String>,
    #[serde(default)]
    pub react: Option<String>,
    #[serde(default)]
    pub nextjs: Option<String>,
    #[serde(default)]
    pub remix: Option<String>,
    #[serde(default)]
    pub gatsby: Option<String>,
    #[serde(default)]
    pub vue: Option<String>,
    #[serde(default)]
    pub nuxt: Option<String>,
    #[serde(default)]
    pub svelte: Option<String>,
    #[serde(default)]
    pub sveltekit: Option<String>,
    #[serde(default)]
    pub angular: Option<String>,
    #[serde(default)]
    pub astro: Option<String>,
    #[serde(default)]
    pub solid: Option<String>,
    #[serde(default)]
    pub htmx: Option<String>,
    #[serde(default)]
    pub alpine: Option<String>,
    #[serde(default)]
    pub wordpress: Option<String>,
    #[serde(default)]
    pub shopify: Option<String>,
}

impl AliasSpec {
    /// Pick the per-framework selector for a single [`Framework`],
    /// or `None` if this spec doesn't override that framework.
    #[must_use]
    pub fn for_framework(&self, f: Framework) -> Option<&str> {
        let s = match f {
            Framework::ShadcnRadix => &self.shadcn,
            Framework::Tailwind => &self.tailwind,
            Framework::Bootstrap => &self.bootstrap,
            Framework::React => &self.react,
            Framework::NextJs => &self.nextjs,
            Framework::Remix => &self.remix,
            Framework::Gatsby => &self.gatsby,
            Framework::Vue => &self.vue,
            Framework::Nuxt => &self.nuxt,
            Framework::Svelte => &self.svelte,
            Framework::SvelteKit => &self.sveltekit,
            Framework::Angular => &self.angular,
            Framework::Astro => &self.astro,
            Framework::Solid => &self.solid,
            Framework::Htmx => &self.htmx,
            Framework::Alpine => &self.alpine,
            Framework::Wordpress => &self.wordpress,
            Framework::Shopify => &self.shopify,
            // MUI is a design system that rides on React; we detect it
            // separately from React in practice, but for now it maps to
            // the `mui` field and users manually opt in.
            Framework::Materialize | Framework::GoogleTagManager | Framework::JqueryFallback => {
                &None
            }
        };
        s.as_deref().or_else(|| {
            // Special case: shadcn-radix alias can also use the generic `mui` slot
            // if the user defined it under a more-canonical framework.
            if f == Framework::ShadcnRadix && self.shadcn.is_none() {
                self.mui.as_deref()
            } else {
                None
            }
        })
    }

    /// Resolve this alias against a detected-framework list.
    /// Frameworks should come in confidence order (highest first);
    /// the first one we have an override for wins.
    #[must_use]
    pub fn resolve(&self, detections: &[Detection]) -> &str {
        for d in detections {
            if let Some(s) = self.for_framework(d.framework) {
                return s;
            }
        }
        &self.fallback
    }
}

/// Index of aliases by name.
#[derive(Debug, Clone, Default)]
pub struct AliasRegistry {
    specs: Vec<AliasSpec>,
}

impl AliasRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: AliasSpec) {
        // Replace-by-name so later definitions override earlier.
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = AliasSpec>) {
        for s in specs {
            self.insert(s);
        }
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&AliasSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// Expand `@name` references in a selector string, using the given
    /// framework detections. Unknown aliases pass through unchanged
    /// (selector parser will error out later — same as mistyped raw
    /// selectors).
    #[must_use]
    pub fn expand(&self, selector: &str, detections: &[Detection]) -> String {
        expand_aliases(selector, self, detections)
    }

    /// Return a copy of the transform specs with every `@alias` in
    /// the selectors resolved against the given framework detections.
    /// Non-alias selectors pass through byte-identical.
    #[must_use]
    pub fn expand_transforms(
        &self,
        specs: &[crate::transform::DomTransformSpec],
        detections: &[Detection],
    ) -> Vec<crate::transform::DomTransformSpec> {
        specs
            .iter()
            .map(|s| {
                let mut s = s.clone();
                s.selector = self.expand(&s.selector, detections);
                s
            })
            .collect()
    }

    /// Same idea for scrape specs.
    #[must_use]
    pub fn expand_scrapes(
        &self,
        specs: &[crate::scrape::ScrapeSpec],
        detections: &[Detection],
    ) -> Vec<crate::scrape::ScrapeSpec> {
        specs
            .iter()
            .map(|s| {
                let mut s = s.clone();
                s.selector = self.expand(&s.selector, detections);
                s
            })
            .collect()
    }
}

/// Walk a selector string, substitute every `@ident` token with the
/// resolved raw selector. Identifiers are ASCII-alphanumeric + `-`/`_`.
fn expand_aliases(selector: &str, reg: &AliasRegistry, detections: &[Detection]) -> String {
    let mut out = String::with_capacity(selector.len());
    let bytes = selector.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_')
            {
                i += 1;
            }
            let ident = &selector[start..i];
            match reg.get(ident) {
                Some(spec) => out.push_str(spec.resolve(detections)),
                None => out.push_str(ident),
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Compile a Lisp document of `(defframework-alias …)` forms.
#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<AliasSpec>, String> {
    tatara_lisp::compile_typed::<AliasSpec>(src).map_err(|e| format!("{e}"))
}

/// Register the `defframework-alias` keyword in the global tatara registry.
#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<AliasSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detection(framework: Framework, confidence: f32) -> Detection {
        Detection {
            framework,
            name: framework.name(),
            confidence,
            evidence: vec![],
        }
    }

    fn card_alias() -> AliasSpec {
        AliasSpec {
            name: "@card".into(),
            fallback: "div.card".into(),
            description: None,
            shadcn: Some(r#"[data-slot="card"]"#.into()),
            mui: Some("div.MuiCard-root".into()),
            tailwind: Some("div.card".into()),
            bootstrap: Some("div.card".into()),
            react: None,
            nextjs: None,
            remix: None,
            gatsby: None,
            vue: None,
            nuxt: None,
            svelte: None,
            sveltekit: None,
            angular: None,
            astro: None,
            solid: None,
            htmx: None,
            alpine: None,
            wordpress: None,
            shopify: None,
        }
    }

    #[test]
    fn resolves_to_fallback_when_no_framework_matches() {
        let spec = card_alias();
        let out = spec.resolve(&[]);
        assert_eq!(out, "div.card");
    }

    #[test]
    fn resolves_to_shadcn_when_detected() {
        let spec = card_alias();
        let out = spec.resolve(&[detection(Framework::ShadcnRadix, 0.75)]);
        assert_eq!(out, r#"[data-slot="card"]"#);
    }

    #[test]
    fn resolves_in_confidence_order() {
        // Two frameworks detected; shadcn higher → wins even though
        // tailwind is also defined.
        let spec = card_alias();
        let out = spec.resolve(&[
            detection(Framework::ShadcnRadix, 0.75),
            detection(Framework::Tailwind, 0.7),
        ]);
        assert_eq!(out, r#"[data-slot="card"]"#);

        // Reverse the order → tailwind wins.
        let out = spec.resolve(&[
            detection(Framework::Tailwind, 0.9),
            detection(Framework::ShadcnRadix, 0.5),
        ]);
        assert_eq!(out, "div.card");
    }

    #[test]
    fn skips_frameworks_without_overrides() {
        // React detected but alias has no react override; we fall
        // through to the next detected framework or fallback.
        let spec = card_alias();
        let out = spec.resolve(&[
            detection(Framework::React, 0.9),
            detection(Framework::ShadcnRadix, 0.7),
        ]);
        assert_eq!(out, r#"[data-slot="card"]"#);
    }

    #[test]
    fn registry_insert_and_lookup() {
        let mut reg = AliasRegistry::new();
        reg.insert(card_alias());
        assert_eq!(reg.len(), 1);
        assert!(reg.get("@card").is_some());
        assert!(reg.get("@unknown").is_none());
    }

    #[test]
    fn registry_replace_by_name() {
        let mut reg = AliasRegistry::new();
        reg.insert(card_alias());
        let mut updated = card_alias();
        updated.fallback = "div.overridden".into();
        reg.insert(updated);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("@card").unwrap().fallback, "div.overridden");
    }

    #[test]
    fn expand_leaves_non_alias_selectors_alone() {
        let reg = AliasRegistry::new();
        assert_eq!(reg.expand("div.card > a", &[]), "div.card > a");
    }

    #[test]
    fn expand_substitutes_alias_with_fallback() {
        let mut reg = AliasRegistry::new();
        reg.insert(card_alias());
        assert_eq!(reg.expand("@card", &[]), "div.card");
    }

    #[test]
    fn expand_substitutes_alias_with_framework_specific() {
        let mut reg = AliasRegistry::new();
        reg.insert(card_alias());
        let out = reg.expand("@card > a", &[detection(Framework::ShadcnRadix, 0.75)]);
        assert_eq!(out, r#"[data-slot="card"] > a"#);
    }

    #[test]
    fn expand_preserves_surrounding_selector_syntax() {
        let mut reg = AliasRegistry::new();
        reg.insert(card_alias());
        // nested inside a descendant combinator + compound
        let out = reg.expand(
            "article @card.highlight > h2",
            &[detection(Framework::Bootstrap, 0.6)],
        );
        assert_eq!(out, "article div.card.highlight > h2");
    }

    #[test]
    fn expand_unknown_alias_passes_through_unchanged() {
        let reg = AliasRegistry::new();
        assert_eq!(reg.expand("@unknown > p", &[]), "@unknown > p");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_round_trip_alias_spec() {
        let src = r#"
            (defframework-alias :name "@card"
                                :shadcn "[data-slot=\"card\"]"
                                :tailwind "div.card"
                                :fallback "div.card")
            (defframework-alias :name "@nav"
                                :shadcn "nav[data-slot=\"nav\"]"
                                :bootstrap "nav.navbar"
                                :fallback "nav")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, "@card");
        assert_eq!(specs[1].name, "@nav");
        assert_eq!(specs[1].bootstrap.as_deref(), Some("nav.navbar"));
    }
}
