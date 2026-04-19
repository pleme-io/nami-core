//! CSS-style selectors — shared matcher across nami-core and nami.
//!
//! V2 supports:
//!
//! - **Compound**: tag + class + id on one element
//!   - `"div"`, `".ad"`, `"#hero"`
//!   - `"div.card"`, `"a.link#main"`, `"img.lazy#hero"`
//!   - `"*"` matches any element
//! - **Descendant combinator** (space): `"article p"` — any `<p>` inside an `<article>`
//! - **Child combinator** (`>`): `"ul > li"` — `<li>` that is a *direct* child
//!
//! V2 deliberately does NOT yet support: attribute selectors (`[href*="x"]`),
//! pseudo-classes (`:first-child`), adjacent / general sibling combinators
//! (`+`, `~`), or selector lists (`a, b`). Each is a clean follow-up.
//!
//! The matcher is generic over a [`SelectorNode`] trait so nami's
//! `Element` (children in `Element`) and nami-core's `ElementData`
//! (children in wrapping `Node`) both plug in.
//!
//! ```
//! use nami_core::selector::Selector;
//!
//! let s = Selector::parse("article p").unwrap();
//! // s matches a <p> whose ancestor chain contains an <article>.
//! ```

mod parse;

pub use parse::parse;

/// A parsed selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    /// All simple parts must match the same element.
    Compound(Vec<SimplePart>),
    /// `ancestor descendant` — descendant's last element matches,
    /// and some prefix of the ancestor path matches the ancestor.
    Descendant(Box<Selector>, Box<Selector>),
    /// `parent > child` — child's last element matches, and the element
    /// immediately above matches the parent.
    Child(Box<Selector>, Box<Selector>),
}

/// A single constraint on an element's own attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimplePart {
    /// `*` — matches any element (universal selector).
    Universal,
    /// `div`, `article`, etc. (case-insensitive).
    Tag(String),
    /// `.foo` — element has that class.
    Class(String),
    /// `#bar` — element has that id.
    Id(String),
}

/// What a DOM node must expose for selectors to match against it.
///
/// Intentionally minimal: the three things CSS compounds ask for.
/// Implemented by both `nami_core::dom::ElementData` and nami's
/// `Element`.
pub trait SelectorNode {
    fn tag(&self) -> &str;
    fn has_class(&self, class: &str) -> bool;
    fn id(&self) -> Option<&str>;
}

impl Selector {
    /// Parse a selector string. Whitespace-insensitive around combinators;
    /// `" "` = descendant, `" > "` = child.
    pub fn parse(s: &str) -> Result<Self, String> {
        parse::parse(s)
    }

    /// Match this selector against an ancestor path ending in the element
    /// we're testing. Path is root-to-leaf; `path.last()` is the candidate.
    pub fn matches<N: SelectorNode>(&self, path: &[&N]) -> bool {
        match self {
            Self::Compound(parts) => {
                let Some(leaf) = path.last() else {
                    return false;
                };
                parts.iter().all(|p| p.matches(*leaf))
            }
            Self::Descendant(ancestor, descendant) => {
                if !descendant.matches(path) {
                    return false;
                }
                // The right-hand side pinned to the leaf. The left-hand
                // side matches the leaf of some strictly-shorter prefix.
                if path.len() < 2 {
                    return false;
                }
                for i in (0..path.len() - 1).rev() {
                    if ancestor.matches(&path[..=i]) {
                        return true;
                    }
                }
                false
            }
            Self::Child(parent, child) => {
                if !child.matches(path) {
                    return false;
                }
                if path.len() < 2 {
                    return false;
                }
                parent.matches(&path[..path.len() - 1])
            }
        }
    }
}

impl SimplePart {
    fn matches<N: SelectorNode>(&self, node: &N) -> bool {
        match self {
            Self::Universal => true,
            Self::Tag(t) => node.tag().eq_ignore_ascii_case(t),
            Self::Class(c) => node.has_class(c),
            Self::Id(i) => node.id() == Some(i.as_str()),
        }
    }
}

// ── SelectorNode for nami-core's own ElementData ──────────────────

impl SelectorNode for crate::dom::ElementData {
    fn tag(&self) -> &str {
        &self.tag
    }

    fn has_class(&self, class: &str) -> bool {
        crate::dom::ElementData::has_class(self, class)
    }

    fn id(&self) -> Option<&str> {
        self.get_attribute("id")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny fake node for parser-independent matcher tests.
    struct Fake<'a> {
        tag: &'a str,
        classes: Vec<&'a str>,
        id: Option<&'a str>,
    }

    impl<'a> SelectorNode for Fake<'a> {
        fn tag(&self) -> &str {
            self.tag
        }
        fn has_class(&self, c: &str) -> bool {
            self.classes.iter().any(|x| *x == c)
        }
        fn id(&self) -> Option<&str> {
            self.id
        }
    }

    fn n<'a>(tag: &'a str, classes: &[&'a str], id: Option<&'a str>) -> Fake<'a> {
        Fake {
            tag,
            classes: classes.to_vec(),
            id,
        }
    }

    #[test]
    fn compound_all_parts_must_match() {
        let s = Selector::parse("div.card#hero").unwrap();
        let leaf = n("div", &["card"], Some("hero"));
        assert!(s.matches(&[&leaf]));
        let leaf = n("div", &["card"], None);
        assert!(!s.matches(&[&leaf])); // missing id
        let leaf = n("div", &[], Some("hero"));
        assert!(!s.matches(&[&leaf])); // missing class
        let leaf = n("span", &["card"], Some("hero"));
        assert!(!s.matches(&[&leaf])); // wrong tag
    }

    #[test]
    fn universal_matches_anything() {
        let s = Selector::parse("*").unwrap();
        assert!(s.matches(&[&n("anything", &[], None)]));
    }

    #[test]
    fn tag_matching_is_case_insensitive() {
        let s = Selector::parse("DIV").unwrap();
        assert!(s.matches(&[&n("div", &[], None)]));
        assert!(s.matches(&[&n("DIV", &[], None)]));
    }

    #[test]
    fn descendant_combinator() {
        let s = Selector::parse("article p").unwrap();
        let article = n("article", &[], None);
        let section = n("section", &[], None);
        let p = n("p", &[], None);
        // article > section > p → matches (article is some ancestor of p)
        assert!(s.matches(&[&article, &section, &p]));
        // article > p → matches
        assert!(s.matches(&[&article, &p]));
        // just p (no ancestor) → no match
        assert!(!s.matches(&[&p]));
        // div > p (no article) → no match
        let div = n("div", &[], None);
        assert!(!s.matches(&[&div, &p]));
    }

    #[test]
    fn child_combinator() {
        let s = Selector::parse("ul > li").unwrap();
        let ul = n("ul", &[], None);
        let li = n("li", &[], None);
        let div = n("div", &[], None);
        // ul > li → match
        assert!(s.matches(&[&ul, &li]));
        // ul > div > li → NOT a match (li is grandchild, not direct child)
        assert!(!s.matches(&[&ul, &div, &li]));
    }

    #[test]
    fn descendant_plus_compound() {
        let s = Selector::parse("article p.note").unwrap();
        let article = n("article", &[], None);
        let p_note = n("p", &["note"], None);
        let p_plain = n("p", &[], None);
        assert!(s.matches(&[&article, &p_note]));
        assert!(!s.matches(&[&article, &p_plain]));
    }

    #[test]
    fn chained_combinators() {
        // article > section p  — p descendant of section, section direct child of article
        let s = Selector::parse("article > section p").unwrap();
        let article = n("article", &[], None);
        let section = n("section", &[], None);
        let div = n("div", &[], None);
        let p = n("p", &[], None);
        // article > section > p  ✓
        assert!(s.matches(&[&article, &section, &p]));
        // article > section > div > p  ✓ (p descendant of section)
        assert!(s.matches(&[&article, &section, &div, &p]));
        // article > div > section > p  ✗ (section is grandchild of article, not direct)
        assert!(!s.matches(&[&article, &div, &section, &p]));
    }

    #[test]
    fn legacy_single_atom_selectors_still_work() {
        assert!(
            Selector::parse("div")
                .unwrap()
                .matches(&[&n("div", &[], None)])
        );
        assert!(
            Selector::parse(".foo")
                .unwrap()
                .matches(&[&n("x", &["foo"], None)])
        );
        assert!(
            Selector::parse("#bar")
                .unwrap()
                .matches(&[&n("x", &[], Some("bar"))])
        );
    }
}
