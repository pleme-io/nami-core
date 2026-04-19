//! Parser for V2 selectors.
//!
//! Grammar:
//!
//!   selector    := compound (combinator compound)*
//!   combinator  := WS+             (descendant)
//!                | WS* '>' WS*     (child)
//!   compound    := ('*' | atom)+
//!   atom        := '.' IDENT       (class)
//!                | '#' IDENT       (id)
//!                | IDENT           (tag)
//!
//! Whitespace is collapsed to a single descendant combinator; surrounding
//! a `>` with whitespace is optional.

use super::{Selector, SimplePart};

pub fn parse(input: &str) -> Result<Selector, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("empty selector".into());
    }

    // Tokenize into compounds + combinators.
    let tokens = tokenize(s)?;
    if tokens.is_empty() {
        return Err("empty selector".into());
    }

    // Build a left-associative tree.
    let mut iter = tokens.into_iter();
    let first = match iter.next() {
        Some(Token::Compound(parts)) => Selector::Compound(parts),
        Some(Token::Combinator(_)) => return Err("selector cannot begin with a combinator".into()),
        None => unreachable!(),
    };

    let mut acc = first;
    loop {
        let Some(combinator) = iter.next() else {
            break;
        };
        let combinator = match combinator {
            Token::Combinator(c) => c,
            Token::Compound(_) => return Err("two compounds without a combinator".into()),
        };
        let rhs_parts = match iter.next() {
            Some(Token::Compound(parts)) => parts,
            Some(Token::Combinator(_)) => return Err("combinator followed by combinator".into()),
            None => return Err("dangling combinator at end".into()),
        };
        let rhs = Selector::Compound(rhs_parts);
        acc = match combinator {
            Combinator::Descendant => Selector::Descendant(Box::new(acc), Box::new(rhs)),
            Combinator::Child => Selector::Child(Box::new(acc), Box::new(rhs)),
        };
    }

    Ok(acc)
}

#[derive(Debug)]
enum Token {
    Compound(Vec<SimplePart>),
    Combinator(Combinator),
}

#[derive(Debug, Clone, Copy)]
enum Combinator {
    Descendant,
    Child,
}

fn tokenize(s: &str) -> Result<Vec<Token>, String> {
    let mut out: Vec<Token> = Vec::new();
    let mut chars = s.chars().peekable();
    let mut current_compound: Vec<SimplePart> = Vec::new();

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            // End current compound, emit descendant combinator (will be
            // promoted to child if '>' follows).
            if !current_compound.is_empty() {
                out.push(Token::Compound(std::mem::take(&mut current_compound)));
            }
            // Skip all whitespace.
            while matches!(chars.peek(), Some(c) if c.is_whitespace()) {
                chars.next();
            }
            // Only emit a combinator if more content follows.
            if chars.peek().is_some() {
                // Decide descendant vs child based on next char.
                if chars.peek() == Some(&'>') {
                    chars.next();
                    // Skip trailing whitespace around '>'.
                    while matches!(chars.peek(), Some(c) if c.is_whitespace()) {
                        chars.next();
                    }
                    out.push(Token::Combinator(Combinator::Child));
                } else {
                    out.push(Token::Combinator(Combinator::Descendant));
                }
            }
            continue;
        }

        if c == '>' {
            // Child combinator without surrounding whitespace (e.g. "ul>li").
            if !current_compound.is_empty() {
                out.push(Token::Compound(std::mem::take(&mut current_compound)));
            }
            chars.next();
            while matches!(chars.peek(), Some(c) if c.is_whitespace()) {
                chars.next();
            }
            out.push(Token::Combinator(Combinator::Child));
            continue;
        }

        if c == '.' {
            chars.next();
            let ident = read_ident(&mut chars);
            if ident.is_empty() {
                return Err("'.' not followed by class name".into());
            }
            current_compound.push(SimplePart::Class(ident));
            continue;
        }

        if c == '#' {
            chars.next();
            let ident = read_ident(&mut chars);
            if ident.is_empty() {
                return Err("'#' not followed by id".into());
            }
            current_compound.push(SimplePart::Id(ident));
            continue;
        }

        if c == '*' {
            chars.next();
            current_compound.push(SimplePart::Universal);
            continue;
        }

        if is_ident_start(c) {
            let ident = read_ident(&mut chars);
            current_compound.push(SimplePart::Tag(ident));
            continue;
        }

        return Err(format!("unexpected character in selector: {c:?}"));
    }

    if !current_compound.is_empty() {
        out.push(Token::Compound(current_compound));
    }

    Ok(out)
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '-'
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn read_ident(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut out = String::new();
    while let Some(&c) = chars.peek() {
        if is_ident_continue(c) {
            out.push(c);
            chars.next();
        } else {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::{Selector, SimplePart};

    fn compound(parts: Vec<SimplePart>) -> Selector {
        Selector::Compound(parts)
    }

    #[test]
    fn parse_tag() {
        let s = Selector::parse("div").unwrap();
        assert_eq!(s, compound(vec![SimplePart::Tag("div".into())]));
    }

    #[test]
    fn parse_class() {
        let s = Selector::parse(".foo").unwrap();
        assert_eq!(s, compound(vec![SimplePart::Class("foo".into())]));
    }

    #[test]
    fn parse_id() {
        let s = Selector::parse("#bar").unwrap();
        assert_eq!(s, compound(vec![SimplePart::Id("bar".into())]));
    }

    #[test]
    fn parse_universal() {
        let s = Selector::parse("*").unwrap();
        assert_eq!(s, compound(vec![SimplePart::Universal]));
    }

    #[test]
    fn parse_compound_tag_class() {
        let s = Selector::parse("div.card").unwrap();
        assert_eq!(
            s,
            compound(vec![
                SimplePart::Tag("div".into()),
                SimplePart::Class("card".into()),
            ])
        );
    }

    #[test]
    fn parse_compound_tag_class_id() {
        let s = Selector::parse("a.link#main").unwrap();
        assert_eq!(
            s,
            compound(vec![
                SimplePart::Tag("a".into()),
                SimplePart::Class("link".into()),
                SimplePart::Id("main".into()),
            ])
        );
    }

    #[test]
    fn parse_descendant() {
        let s = Selector::parse("article p").unwrap();
        assert_eq!(
            s,
            Selector::Descendant(
                Box::new(compound(vec![SimplePart::Tag("article".into())])),
                Box::new(compound(vec![SimplePart::Tag("p".into())])),
            )
        );
    }

    #[test]
    fn parse_child_with_spaces() {
        let s = Selector::parse("ul > li").unwrap();
        assert_eq!(
            s,
            Selector::Child(
                Box::new(compound(vec![SimplePart::Tag("ul".into())])),
                Box::new(compound(vec![SimplePart::Tag("li".into())])),
            )
        );
    }

    #[test]
    fn parse_child_without_spaces() {
        let s = Selector::parse("ul>li").unwrap();
        assert_eq!(
            s,
            Selector::Child(
                Box::new(compound(vec![SimplePart::Tag("ul".into())])),
                Box::new(compound(vec![SimplePart::Tag("li".into())])),
            )
        );
    }

    #[test]
    fn parse_chained_left_associative() {
        // article > section p  =  Descendant(Child(article, section), p)
        let s = Selector::parse("article > section p").unwrap();
        match s {
            Selector::Descendant(left, right) => {
                assert!(matches!(*left, Selector::Child(_, _)));
                assert_eq!(*right, compound(vec![SimplePart::Tag("p".into())]));
            }
            other => panic!("expected Descendant, got {other:?}"),
        }
    }

    #[test]
    fn empty_is_err() {
        assert!(Selector::parse("").is_err());
        assert!(Selector::parse("   ").is_err());
    }

    #[test]
    fn dangling_combinator_is_err() {
        assert!(Selector::parse("div >").is_err());
        assert!(Selector::parse("> div").is_err());
    }

    #[test]
    fn bare_dot_or_hash_is_err() {
        assert!(Selector::parse(".").is_err());
        assert!(Selector::parse("#").is_err());
    }
}
