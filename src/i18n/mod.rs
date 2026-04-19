//! `(defi18n)` — declarative localized message bundles.
//!
//! Absorbs `chrome.i18n` + Firefox `browser.i18n.getMessage` +
//! Safari NSLocalizedString-ish translation flows into the substrate.
//! Each form ships one namespace's strings for one locale; the
//! registry merges by namespace and resolves with a fallback chain:
//!
//!   exact namespace+locale → namespace+"en" → raw key
//!
//! ```lisp
//! (defi18n :namespace "core"
//!          :locale    "en"
//!          :strings   ((:hello  . "Hello")
//!                      (:bye    . "Goodbye")
//!                      (:newTab . "New tab")))
//!
//! (defi18n :namespace "core"
//!          :locale    "ja"
//!          :strings   ((:hello  . "こんにちは")
//!                      (:bye    . "さようなら")
//!                      (:newTab . "新しいタブ")))
//!
//! (defi18n :namespace "dark-reader"
//!          :locale    "en"
//!          :strings   ((:toggleDescription . "Toggle dark mode for this site.")))
//! ```
//!
//! Parameterized strings (`"Hello, {name}"`) evaluate by simple
//! `{placeholder}` substitution via [`MessageRegistry::format`]. No
//! ICU plurals / genders yet — landing those is a V2 orthogonal to
//! the substrate pattern, keyed off the same spec type.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// One (namespace, locale) bundle. Authored as a single
/// `(defi18n)` form.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defi18n"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MessageSpec {
    /// Scope — typically "core" for the browser chrome, or an
    /// extension name for per-extension strings.
    pub namespace: String,
    /// BCP-47 locale tag — "en", "en-US", "ja", "pt-BR".
    pub locale: String,
    /// Key → translated string. Keys are stable across locales.
    #[serde(default)]
    pub strings: HashMap<String, String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Merged registry of translations, indexed by (namespace, locale).
#[derive(Debug, Clone, Default)]
pub struct MessageRegistry {
    /// `(namespace, locale) → key → value`.
    bundles: HashMap<(String, String), HashMap<String, String>>,
}

impl MessageRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Install a bundle. Existing entries under the same
    /// `(namespace, locale)` are merged key-wise — last spec wins
    /// per key, so later forms override earlier defaults.
    pub fn insert(&mut self, spec: MessageSpec) {
        let bundle = self
            .bundles
            .entry((spec.namespace, spec.locale))
            .or_default();
        for (k, v) in spec.strings {
            bundle.insert(k, v);
        }
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = MessageSpec>) {
        for s in specs {
            self.insert(s);
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.bundles.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bundles.is_empty()
    }

    /// Resolve a key → translated string.
    ///
    /// Lookup order (first hit wins):
    ///   1. `(namespace, locale)`
    ///   2. `(namespace, locale-prefix)` — `"en-US"` falls back to `"en"`
    ///   3. `(namespace, "en")`
    ///   4. raw `key` (so UIs degrade gracefully during translation work)
    #[must_use]
    pub fn get(&self, namespace: &str, locale: &str, key: &str) -> String {
        if let Some(hit) = self.lookup(namespace, locale, key) {
            return hit;
        }
        if let Some((prefix, _)) = locale.split_once('-') {
            if let Some(hit) = self.lookup(namespace, prefix, key) {
                return hit;
            }
        }
        if locale != "en" {
            if let Some(hit) = self.lookup(namespace, "en", key) {
                return hit;
            }
        }
        key.to_owned()
    }

    /// Like [`get`] but returns `None` when no bundle contains the key,
    /// so callers can distinguish "raw key" from "translated string
    /// happens to equal the key".
    #[must_use]
    pub fn lookup(&self, namespace: &str, locale: &str, key: &str) -> Option<String> {
        self.bundles
            .get(&(namespace.to_owned(), locale.to_owned()))
            .and_then(|b| b.get(key))
            .cloned()
    }

    /// Resolve + substitute `{placeholder}` tokens. Placeholders
    /// absent from the args map are left literal (so it's obvious
    /// something's missing in translation review).
    #[must_use]
    pub fn format(
        &self,
        namespace: &str,
        locale: &str,
        key: &str,
        args: &HashMap<String, String>,
    ) -> String {
        let template = self.get(namespace, locale, key);
        let mut out = String::with_capacity(template.len());
        let mut chars = template.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '{' {
                let mut token = String::new();
                let mut closed = false;
                while let Some(&next) = chars.peek() {
                    if next == '}' {
                        chars.next();
                        closed = true;
                        break;
                    }
                    token.push(next);
                    chars.next();
                }
                if closed {
                    if let Some(v) = args.get(&token) {
                        out.push_str(v);
                    } else {
                        out.push('{');
                        out.push_str(&token);
                        out.push('}');
                    }
                } else {
                    out.push('{');
                    out.push_str(&token);
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    /// Every (namespace, locale) pair registered. Useful for /i18n
    /// inspector and "is my translation coverage complete?" checks.
    #[must_use]
    pub fn pairs(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self.bundles.keys().cloned().collect();
        out.sort();
        out
    }

    /// All keys present in `(namespace, locale)`. Returns an empty
    /// Vec if the bundle doesn't exist.
    #[must_use]
    pub fn keys(&self, namespace: &str, locale: &str) -> Vec<String> {
        match self
            .bundles
            .get(&(namespace.to_owned(), locale.to_owned()))
        {
            Some(b) => {
                let mut keys: Vec<String> = b.keys().cloned().collect();
                keys.sort();
                keys
            }
            None => Vec::new(),
        }
    }

    /// Every locale present for `namespace`, sorted.
    #[must_use]
    pub fn locales_for(&self, namespace: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .bundles
            .keys()
            .filter(|(ns, _)| ns == namespace)
            .map(|(_, loc)| loc.clone())
            .collect();
        out.sort();
        out
    }

    /// Keys present in `(namespace, "en")` but missing from
    /// `(namespace, locale)`. A "what's left to translate" view.
    #[must_use]
    pub fn missing(&self, namespace: &str, locale: &str) -> Vec<String> {
        let en: std::collections::HashSet<String> = self
            .keys(namespace, "en")
            .into_iter()
            .collect();
        let here: std::collections::HashSet<String> = self
            .keys(namespace, locale)
            .into_iter()
            .collect();
        let mut out: Vec<String> = en.difference(&here).cloned().collect();
        out.sort();
        out
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<MessageSpec>, String> {
    tatara_lisp::compile_typed::<MessageSpec>(src)
        .map_err(|e| format!("failed to compile defi18n forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<MessageSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(ns: &str, loc: &str, pairs: &[(&str, &str)]) -> MessageSpec {
        MessageSpec {
            namespace: ns.into(),
            locale: loc.into(),
            strings: pairs
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            description: None,
        }
    }

    #[test]
    fn exact_match_wins() {
        let mut reg = MessageRegistry::new();
        reg.insert(spec("core", "en", &[("hello", "Hello")]));
        reg.insert(spec("core", "ja", &[("hello", "こんにちは")]));
        assert_eq!(reg.get("core", "en", "hello"), "Hello");
        assert_eq!(reg.get("core", "ja", "hello"), "こんにちは");
    }

    #[test]
    fn falls_back_to_prefix_locale() {
        let mut reg = MessageRegistry::new();
        reg.insert(spec("core", "en", &[("color", "color")]));
        reg.insert(spec("core", "en-GB", &[("color", "colour")]));
        // Exact hit.
        assert_eq!(reg.get("core", "en-GB", "color"), "colour");
        // Requested "en-AU", not present → prefix "en" wins.
        assert_eq!(reg.get("core", "en-AU", "color"), "color");
    }

    #[test]
    fn falls_back_to_en_when_target_locale_missing() {
        let mut reg = MessageRegistry::new();
        reg.insert(spec("core", "en", &[("hello", "Hello")]));
        // No Japanese bundle at all → falls back to English.
        assert_eq!(reg.get("core", "ja", "hello"), "Hello");
    }

    #[test]
    fn falls_back_to_raw_key_when_nothing_matches() {
        let reg = MessageRegistry::new();
        assert_eq!(reg.get("extension-foo", "en", "unknown.key"), "unknown.key");
    }

    #[test]
    fn locale_fallback_chain_does_not_touch_other_namespaces() {
        let mut reg = MessageRegistry::new();
        reg.insert(spec("core", "en", &[("hello", "Hello")]));
        // "dark-reader" has no en bundle → must NOT bleed "core"'s.
        assert_eq!(reg.get("dark-reader", "en", "hello"), "hello");
    }

    #[test]
    fn format_substitutes_placeholders() {
        let mut reg = MessageRegistry::new();
        reg.insert(spec(
            "core",
            "en",
            &[("greeting", "Hello, {name}! You have {count} messages.")],
        ));
        let args: HashMap<String, String> = [
            ("name".to_owned(), "Jane".to_owned()),
            ("count".to_owned(), "3".to_owned()),
        ]
        .into();
        assert_eq!(
            reg.format("core", "en", "greeting", &args),
            "Hello, Jane! You have 3 messages."
        );
    }

    #[test]
    fn format_leaves_unknown_placeholders_literal() {
        let mut reg = MessageRegistry::new();
        reg.insert(spec("core", "en", &[("k", "Hi {name} — {missing}")]));
        let args: HashMap<String, String> =
            [("name".to_owned(), "A".to_owned())].into();
        assert_eq!(
            reg.format("core", "en", "k", &args),
            "Hi A — {missing}",
        );
    }

    #[test]
    fn format_preserves_text_without_placeholders() {
        let mut reg = MessageRegistry::new();
        reg.insert(spec("core", "en", &[("k", "just text")]));
        let args: HashMap<String, String> = HashMap::new();
        assert_eq!(reg.format("core", "en", "k", &args), "just text");
    }

    #[test]
    fn format_handles_unclosed_brace_literally() {
        let mut reg = MessageRegistry::new();
        reg.insert(spec("core", "en", &[("k", "text {oops")]));
        let args: HashMap<String, String> = HashMap::new();
        // Unclosed → emit literal so translators see the error.
        assert_eq!(reg.format("core", "en", "k", &args), "text {oops");
    }

    #[test]
    fn extend_merges_multiple_forms_into_one_bundle() {
        let mut reg = MessageRegistry::new();
        reg.extend(vec![
            spec("core", "en", &[("a", "A")]),
            spec("core", "en", &[("b", "B")]),
        ]);
        // Two specs, same (ns, loc) → merged into one bundle with both keys.
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.keys("core", "en"), vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn later_form_overrides_earlier_on_same_key() {
        let mut reg = MessageRegistry::new();
        reg.insert(spec("core", "en", &[("hello", "first")]));
        reg.insert(spec("core", "en", &[("hello", "second")]));
        assert_eq!(reg.get("core", "en", "hello"), "second");
    }

    #[test]
    fn missing_lists_untranslated_keys() {
        let mut reg = MessageRegistry::new();
        reg.insert(spec("core", "en", &[("a", "A"), ("b", "B"), ("c", "C")]));
        reg.insert(spec("core", "ja", &[("a", "ア"), ("c", "シ")]));
        assert_eq!(reg.missing("core", "ja"), vec!["b".to_owned()]);
    }

    #[test]
    fn locales_for_returns_every_locale_in_ns() {
        let mut reg = MessageRegistry::new();
        reg.insert(spec("core", "en", &[("a", "A")]));
        reg.insert(spec("core", "ja", &[("a", "ア")]));
        reg.insert(spec("core", "pt-BR", &[("a", "A")]));
        reg.insert(spec("other", "en", &[("a", "A")]));
        assert_eq!(
            reg.locales_for("core"),
            vec!["en".to_owned(), "ja".to_owned(), "pt-BR".to_owned()]
        );
    }

    #[test]
    fn pairs_returns_sorted_ns_locale_tuples() {
        let mut reg = MessageRegistry::new();
        reg.insert(spec("core", "en", &[("a", "A")]));
        reg.insert(spec("core", "ja", &[("a", "ア")]));
        reg.insert(spec("dark-reader", "en", &[("x", "X")]));
        let pairs = reg.pairs();
        assert_eq!(
            pairs,
            vec![
                ("core".to_owned(), "en".to_owned()),
                ("core".to_owned(), "ja".to_owned()),
                ("dark-reader".to_owned(), "en".to_owned()),
            ]
        );
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_i18n_form() {
        // tatara-lisp encodes HashMap<String,String> as keyword→value.
        let src = r#"
            (defi18n :namespace "core"
                     :locale    "en"
                     :strings   (:hello  "Hello"
                                 :bye    "Goodbye"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.namespace, "core");
        assert_eq!(s.locale, "en");
        assert_eq!(s.strings.get("hello").map(String::as_str), Some("Hello"));
        assert_eq!(s.strings.get("bye").map(String::as_str), Some("Goodbye"));
    }
}
