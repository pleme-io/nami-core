//! Typed selector matching for the cascade.
//!
//! Replaces the old substring-against-Debug-output hack with a typed
//! [`CompoundSelector`] — `tag` + `id` + `classes` — that matches against
//! an [`ElementData`](crate::dom::ElementData) field-by-field.
//!
//! The cascade currently matches a rule against an element with **no
//! ancestor context** (it asks "does this element match this rule?", not
//! "does this element match in this tree position?"). So for a complex
//! selector like `.nav a` we take the **rightmost compound** (`a`) — the
//! part that constrains the *subject* element — and match that. This is
//! behavior-preserving (the old hack also ignored combinators) and strictly
//! more correct: `.box` now matches *only* elements carrying that class,
//! not any element whose Debug string happened to contain "box".
//!
//! Full ancestor-aware combinator matching lives in
//! [`crate::selector`](crate::selector) (the `Selector` matcher), which the
//! cascade can adopt once it threads an ancestor path; until then the
//! rightmost-compound rule is the load-bearing correctness fix.
//!
//! ## Unrepresentability tier
//!
//! *Parse-time-rejected / total*: [`parse_selector_list`] never fails — it
//! produces zero or more [`CompoundSelector`]s. An empty / unparseable
//! compound becomes the universal match-all (the safe CSS-ish default for a
//! selector the cascade can't refine), never a silently-wrong match against
//! the wrong element. [`CompoundSelector::matches`] is a pure predicate over
//! typed fields — there is no string to misinterpret.

use crate::dom::ElementData;

/// A single compound selector — constraints that must all hold on **one**
/// element: an optional tag, an optional id, and zero or more classes.
///
/// An all-`None`/empty compound is the universal selector (`*` or an
/// unparseable fragment) and matches every element.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompoundSelector {
    /// Tag name constraint (case-insensitive), or `None` for "any tag".
    pub tag: Option<String>,
    /// Id constraint (exact match), or `None`.
    pub id: Option<String>,
    /// Class constraints — the element must carry **all** of these.
    pub classes: Vec<String>,
}

impl CompoundSelector {
    /// Parse one compound selector fragment (no combinators) like
    /// `"div.card#hero"` into a typed [`CompoundSelector`].
    ///
    /// Tokenizes on `.` (class) and `#` (id); the leading unmarked run is
    /// the tag (unless it is `*`, the universal selector). A `:` begins a
    /// pseudo-class/element which is dropped (e.g. `a:hover` → tag `a`),
    /// since the cascade can't evaluate dynamic pseudo state here.
    #[must_use]
    pub fn parse(compound: &str) -> CompoundSelector {
        let compound = compound.trim();
        let mut sel = CompoundSelector::default();
        if compound.is_empty() {
            return sel; // universal match-all
        }

        // Walk the string accumulating tokens. State is "what kind of
        // token are we currently reading": tag (default), class (after
        // '.'), or id (after '#').
        #[derive(Clone, Copy)]
        enum Kind {
            Tag,
            Class,
            Id,
        }
        let mut kind = Kind::Tag;
        let mut buf = String::new();
        let mut in_pseudo = false;

        let flush = |kind: Kind, buf: &mut String, sel: &mut CompoundSelector| {
            if buf.is_empty() {
                return;
            }
            let token = std::mem::take(buf);
            match kind {
                Kind::Tag => {
                    // A leading run that is "*" is the universal selector —
                    // leave tag as None (match any). Otherwise it's the tag.
                    if token != "*" {
                        sel.tag = Some(token);
                    }
                }
                Kind::Class => sel.classes.push(token),
                Kind::Id => sel.id = Some(token),
            }
        };

        for ch in compound.chars() {
            match ch {
                // Pseudo-class / pseudo-element — drop everything from the
                // first ':' to the next combinator-ish boundary. Since this
                // is one compound (no combinators), drop to end of token.
                ':' => {
                    flush(kind, &mut buf, &mut sel);
                    in_pseudo = true;
                }
                '.' if !in_pseudo => {
                    flush(kind, &mut buf, &mut sel);
                    kind = Kind::Class;
                }
                '#' if !in_pseudo => {
                    flush(kind, &mut buf, &mut sel);
                    kind = Kind::Id;
                }
                _ if in_pseudo => {
                    // Skip pseudo-class chars; a '.' or '#' could end the
                    // pseudo and start a class/id (e.g. `a:hover.x`), so
                    // re-arm on those.
                    if ch == '.' {
                        in_pseudo = false;
                        kind = Kind::Class;
                    } else if ch == '#' {
                        in_pseudo = false;
                        kind = Kind::Id;
                    }
                    // else: part of the pseudo name — drop it.
                }
                _ => buf.push(ch),
            }
        }
        if !in_pseudo {
            flush(kind, &mut buf, &mut sel);
        }
        sel
    }

    /// Whether `el` satisfies every constraint of this compound.
    ///
    /// - tag: case-insensitive equality (when constrained),
    /// - id: exact equality with `el.id()` (when constrained),
    /// - classes: `el.has_class` for **all** required classes.
    ///
    /// An all-empty compound (universal) matches every element.
    #[must_use]
    pub fn matches(&self, el: &ElementData) -> bool {
        if let Some(tag) = &self.tag {
            if !el.tag.eq_ignore_ascii_case(tag) {
                return false;
            }
        }
        if let Some(id) = &self.id {
            if el.id() != Some(id.as_str()) {
                return false;
            }
        }
        for class in &self.classes {
            if !el.has_class(class) {
                return false;
            }
        }
        true
    }

    /// Whether this compound constrains nothing — the universal selector.
    #[must_use]
    pub fn is_universal(&self) -> bool {
        self.tag.is_none() && self.id.is_none() && self.classes.is_empty()
    }
}

/// Parse a full selector-list string (the canonical text lightningcss's
/// `to_css_string` produces) into the typed [`CompoundSelector`]s the
/// cascade matches against.
///
/// 1. Comma-split into individual complex selectors.
/// 2. For each complex selector, take the **rightmost compound** — split
///    on any combinator/whitespace (`' '`, `>`, `+`, `~`, tab, newline)
///    and keep the last non-empty fragment (the subject).
/// 3. Parse each subject fragment via [`CompoundSelector::parse`].
///
/// Always succeeds (returns a possibly-empty `Vec`); a fragment it can't
/// refine becomes the universal match-all rather than a wrong match.
#[must_use]
pub fn parse_selector_list(text: &str) -> Vec<CompoundSelector> {
    text.split(',')
        .filter_map(|complex| {
            let subject = rightmost_compound(complex);
            if subject.is_empty() {
                None
            } else {
                Some(CompoundSelector::parse(subject))
            }
        })
        .collect()
}

/// Take the rightmost compound from one complex selector: split on
/// combinators + whitespace and return the last non-empty fragment.
fn rightmost_compound(complex: &str) -> &str {
    complex
        .split([' ', '>', '+', '~', '\t', '\n', '\r'])
        .filter(|s| !s.is_empty())
        .last()
        .unwrap_or("")
        .trim()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::ElementData;

    fn el(tag: &str, attrs: &[(&str, &str)]) -> ElementData {
        ElementData::with_attributes(
            tag,
            attrs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        )
    }

    // ── CompoundSelector::parse ──────────────────────────────────────

    #[test]
    fn parse_tag_only() {
        let s = CompoundSelector::parse("div");
        assert_eq!(s.tag.as_deref(), Some("div"));
        assert!(s.id.is_none());
        assert!(s.classes.is_empty());
    }

    #[test]
    fn parse_class_only() {
        let s = CompoundSelector::parse(".box");
        assert!(s.tag.is_none());
        assert_eq!(s.classes, vec!["box".to_string()]);
    }

    #[test]
    fn parse_id_only() {
        let s = CompoundSelector::parse("#hero");
        assert_eq!(s.id.as_deref(), Some("hero"));
    }

    #[test]
    fn parse_compound_tag_class_id() {
        let s = CompoundSelector::parse("a.link#main");
        assert_eq!(s.tag.as_deref(), Some("a"));
        assert_eq!(s.id.as_deref(), Some("main"));
        assert_eq!(s.classes, vec!["link".to_string()]);
    }

    #[test]
    fn parse_multiple_classes() {
        let s = CompoundSelector::parse("div.a.b.c");
        assert_eq!(s.tag.as_deref(), Some("div"));
        assert_eq!(
            s.classes,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn parse_universal_has_no_tag() {
        let s = CompoundSelector::parse("*");
        assert!(s.is_universal());
    }

    #[test]
    fn parse_drops_pseudo() {
        let s = CompoundSelector::parse("a:hover");
        assert_eq!(s.tag.as_deref(), Some("a"));
        assert!(s.classes.is_empty());
        let s = CompoundSelector::parse("button:focus-visible");
        assert_eq!(s.tag.as_deref(), Some("button"));
        // class after a pseudo still picked up.
        let s = CompoundSelector::parse("a:hover.x");
        assert_eq!(s.tag.as_deref(), Some("a"));
        assert_eq!(s.classes, vec!["x".to_string()]);
    }

    #[test]
    fn parse_empty_is_universal() {
        assert!(CompoundSelector::parse("").is_universal());
        assert!(CompoundSelector::parse("   ").is_universal());
    }

    // ── CompoundSelector::matches ────────────────────────────────────

    #[test]
    fn class_matches_only_elements_with_that_class() {
        let s = CompoundSelector::parse(".box");
        assert!(s.matches(&el("div", &[("class", "box")])));
        assert!(s.matches(&el("span", &[("class", "a box c")])));
        // bare <div> with no class must NOT match (the old hack's bug).
        assert!(!s.matches(&el("div", &[])));
        assert!(!s.matches(&el("div", &[("class", "notbox")])));
    }

    #[test]
    fn id_matches_exact() {
        let s = CompoundSelector::parse("#hero");
        assert!(s.matches(&el("section", &[("id", "hero")])));
        assert!(!s.matches(&el("section", &[("id", "hero2")])));
        assert!(!s.matches(&el("section", &[])));
    }

    #[test]
    fn tag_matches_case_insensitive() {
        let s = CompoundSelector::parse("div");
        assert!(s.matches(&el("div", &[])));
        assert!(s.matches(&el("DIV", &[])));
        assert!(!s.matches(&el("span", &[])));
    }

    #[test]
    fn compound_requires_all_parts() {
        // div.card needs BOTH the tag and the class.
        let s = CompoundSelector::parse("div.card");
        assert!(s.matches(&el("div", &[("class", "card")])));
        assert!(!s.matches(&el("div", &[]))); // missing class
        assert!(!s.matches(&el("span", &[("class", "card")]))); // wrong tag
    }

    #[test]
    fn universal_matches_anything() {
        let s = CompoundSelector::parse("*");
        assert!(s.matches(&el("div", &[])));
        assert!(s.matches(&el("anything", &[("class", "x")])));
    }

    #[test]
    fn multiple_classes_all_required() {
        let s = CompoundSelector::parse(".a.b");
        assert!(s.matches(&el("div", &[("class", "a b c")])));
        assert!(!s.matches(&el("div", &[("class", "a")]))); // missing b
    }

    // ── parse_selector_list ──────────────────────────────────────────

    #[test]
    fn selector_list_comma_split() {
        let list = parse_selector_list("h1, h2, .title");
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].tag.as_deref(), Some("h1"));
        assert_eq!(list[1].tag.as_deref(), Some("h2"));
        assert_eq!(list[2].classes, vec!["title".to_string()]);
    }

    #[test]
    fn selector_list_takes_rightmost_compound() {
        // ".nav a" → match <a> (the subject), not .nav.
        let list = parse_selector_list(".nav a");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].tag.as_deref(), Some("a"));
        assert!(list[0].classes.is_empty());

        let a = el("a", &[]);
        assert!(list[0].matches(&a));
        // .nav (a div with that class) must NOT be matched by ".nav a".
        let nav = el("div", &[("class", "nav")]);
        assert!(!list[0].matches(&nav));
    }

    #[test]
    fn selector_list_child_combinator_rightmost() {
        let list = parse_selector_list("ul > li");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].tag.as_deref(), Some("li"));
    }

    #[test]
    fn selector_list_compound_rightmost() {
        // "article div.card" → rightmost is div.card.
        let list = parse_selector_list("article div.card");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].tag.as_deref(), Some("div"));
        assert_eq!(list[0].classes, vec!["card".to_string()]);
    }

    #[test]
    fn selector_list_sibling_combinators() {
        assert_eq!(parse_selector_list("h1 + p")[0].tag.as_deref(), Some("p"));
        assert_eq!(parse_selector_list("h1 ~ p")[0].tag.as_deref(), Some("p"));
    }

    #[test]
    fn selector_list_no_false_positive_across_elements() {
        // ".card" must match only the card, not a sibling .deck.
        let list = parse_selector_list(".card");
        let card = el("div", &[("class", "card")]);
        let deck = el("div", &[("class", "deck")]);
        assert!(list[0].matches(&card));
        assert!(!list[0].matches(&deck));
    }

    #[test]
    fn selector_list_handles_extra_whitespace() {
        let list = parse_selector_list("  div  ,  .box  ");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].tag.as_deref(), Some("div"));
        assert_eq!(list[1].classes, vec!["box".to_string()]);
    }
}
