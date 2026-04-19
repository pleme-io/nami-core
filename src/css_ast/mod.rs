//! CSS ↔ Lisp roundtrip.
//!
//! Mirrors the DOM ↔ Lisp symmetry (`lisp::dom_to_sexp` +
//! `lisp::sexp_to_dom`) for stylesheets. V1 handles the core shape:
//! top-level style rules with selector + declaration pairs. No
//! at-rules, media queries, or nested rules yet — those come as
//! the CSS dialect grammars land (SCSS/Less/Stylus via tree-sitter).
//!
//! ```text
//!   CSS text
//!     ↓ parse_css
//!   Vec<CssRule>
//!     ↓ css_to_sexp
//!   "(css-stylesheet
//!      (css-rule :selector "div.card"
//!                :declarations ((color "red")
//!                               (font-size "14px"))) …)"
//!     ↓ sexp_to_css
//!   Vec<CssRule>
//!     ↓ emit_css
//!   CSS text
//! ```
//!
//! The cycle is a fixed point on well-formed input: whitespace may
//! normalize, but structure is preserved.
//!
//! ## Why a hand-rolled parser, not lightningcss
//!
//! lightningcss structures values heavily (colors become `RGBA`,
//! lengths become `Length`, etc.) which complicates pass-through of
//! unknown-to-us properties. For Lisp-authored transforms we want
//! the raw text to survive unchanged. This V1 parser treats every
//! declaration as a `(property_name, text_value)` pair and never
//! loses a byte that wasn't whitespace.

use std::fmt::Write;

/// One style rule: a selector list + declaration block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssRule {
    pub selector: String,
    pub declarations: Vec<(String, String)>,
}

/// Parse a CSS string into a flat vec of style rules. Comments and
/// at-rules are skipped; malformed blocks are logged and dropped.
#[must_use]
pub fn parse_css(src: &str) -> Vec<CssRule> {
    let stripped = strip_comments(src);
    let mut out = Vec::new();
    let bytes = stripped.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip whitespace.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // Skip at-rules (for V1 — we don't model @media, @import, etc.).
        if bytes[i] == b'@' {
            let end = skip_at_rule(&stripped, i);
            i = end;
            continue;
        }
        // Find the next `{`.
        let Some(brace) = stripped[i..].find('{') else {
            break;
        };
        let open = i + brace;
        let selector = stripped[i..open].trim().to_owned();
        // Find the matching `}` — V1 doesn't model nested braces
        // (no supports / nested rules), so a shallow scan is fine.
        let Some(close_rel) = stripped[open + 1..].find('}') else {
            break;
        };
        let close = open + 1 + close_rel;
        let block = &stripped[open + 1..close];
        let decls = parse_declarations(block);
        if !selector.is_empty() {
            out.push(CssRule {
                selector,
                declarations: decls,
            });
        }
        i = close + 1;
    }
    out
}

fn skip_at_rule(src: &str, start: usize) -> usize {
    // An at-rule either ends with `;` (like @import) or has a `{…}`
    // block (@media, @supports, @keyframes, …). For V1 we jump past
    // the whole thing — this crate doesn't interpret it yet.
    let rest = &src[start..];
    let semi = rest.find(';');
    let brace = rest.find('{');
    match (semi, brace) {
        (Some(s), None) => start + s + 1,
        (None, Some(b)) => {
            // Skip the balanced block.
            start + b + 1 + skip_balanced_block(&rest[b + 1..])
        }
        (Some(s), Some(b)) => {
            if s < b {
                start + s + 1
            } else {
                start + b + 1 + skip_balanced_block(&rest[b + 1..])
            }
        }
        _ => src.len(),
    }
}

fn skip_balanced_block(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut depth = 1;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    s.len()
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // skip to closing */
            if let Some(end_rel) = src[i + 2..].find("*/") {
                i += 2 + end_rel + 2;
                continue;
            }
            break; // unterminated comment — drop rest
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn parse_declarations(block: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in block.split(';') {
        let decl = raw.trim();
        if decl.is_empty() {
            continue;
        }
        let Some(colon) = decl.find(':') else {
            continue;
        };
        let (prop, value) = decl.split_at(colon);
        let prop = prop.trim().to_owned();
        let value = value[1..].trim().to_owned();
        if prop.is_empty() || value.is_empty() {
            continue;
        }
        out.push((prop, value));
    }
    out
}

/// Emit a CSS text representation. Pretty-printed with 2-space
/// indent, one declaration per line. Idempotent under parse →
/// emit → parse.
#[must_use]
pub fn emit_css(rules: &[CssRule]) -> String {
    let mut out = String::new();
    for (i, rule) in rules.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        write!(out, "{} {{\n", rule.selector).ok();
        for (prop, value) in &rule.declarations {
            writeln!(out, "  {prop}: {value};").ok();
        }
        out.push_str("}\n");
    }
    out
}

/// Serialize rules into our canonical Lisp sexp form.
#[must_use]
pub fn css_to_sexp(rules: &[CssRule]) -> String {
    let mut out = String::new();
    out.push_str("(css-stylesheet");
    for rule in rules {
        out.push_str("\n  (css-rule");
        out.push_str(" :selector ");
        write_quoted(&mut out, &rule.selector);
        out.push_str("\n    :declarations (");
        for (i, (prop, value)) in rule.declarations.iter().enumerate() {
            if i > 0 {
                out.push_str("\n                   ");
            }
            out.push('(');
            write_quoted(&mut out, prop);
            out.push(' ');
            write_quoted(&mut out, value);
            out.push(')');
        }
        out.push_str("))");
    }
    out.push_str("\n)");
    out
}

/// Parse our own sexp form back into rules.
///
/// Grammar:
///   form      = `(css-stylesheet RULE*)`
///   RULE      = `(css-rule :selector STRING :declarations (DECL*))`
///   DECL      = `(STRING STRING)`
pub fn sexp_to_css(src: &str) -> Result<Vec<CssRule>, String> {
    let mut it = src.chars().peekable();
    skip_ws(&mut it);
    expect(&mut it, '(')?;
    let tag = read_symbol(&mut it);
    if tag != "css-stylesheet" {
        return Err(format!("expected css-stylesheet, got {tag:?}"));
    }
    let mut rules = Vec::new();
    loop {
        skip_ws(&mut it);
        match it.peek() {
            Some(&')') => {
                it.next();
                break;
            }
            Some(&'(') => rules.push(read_rule(&mut it)?),
            Some(c) => return Err(format!("unexpected char {c:?}")),
            None => return Err("unexpected EOF".into()),
        }
    }
    Ok(rules)
}

use std::iter::Peekable;
use std::str::Chars;

type It<'a> = Peekable<Chars<'a>>;

fn skip_ws(it: &mut It<'_>) {
    while let Some(&c) = it.peek() {
        if c.is_whitespace() {
            it.next();
        } else {
            break;
        }
    }
}

fn expect(it: &mut It<'_>, want: char) -> Result<(), String> {
    skip_ws(it);
    match it.next() {
        Some(c) if c == want => Ok(()),
        Some(c) => Err(format!("expected {want:?}, got {c:?}")),
        None => Err(format!("expected {want:?}, got EOF")),
    }
}

fn read_symbol(it: &mut It<'_>) -> String {
    skip_ws(it);
    let mut s = String::new();
    while let Some(&c) = it.peek() {
        if c.is_alphanumeric() || c == '-' || c == '_' || c == ':' || c == '?' || c == '!' {
            s.push(c);
            it.next();
        } else {
            break;
        }
    }
    s
}

fn read_string(it: &mut It<'_>) -> Result<String, String> {
    skip_ws(it);
    expect(it, '"')?;
    let mut s = String::new();
    while let Some(c) = it.next() {
        match c {
            '"' => return Ok(s),
            '\\' => match it.next() {
                Some('n') => s.push('\n'),
                Some('r') => s.push('\r'),
                Some('t') => s.push('\t'),
                Some(other) => s.push(other),
                None => return Err("EOF in escape".into()),
            },
            _ => s.push(c),
        }
    }
    Err("unterminated string".into())
}

fn read_rule(it: &mut It<'_>) -> Result<CssRule, String> {
    expect(it, '(')?;
    let tag = read_symbol(it);
    if tag != "css-rule" {
        return Err(format!("expected css-rule, got {tag:?}"));
    }
    let mut selector: Option<String> = None;
    let mut declarations: Vec<(String, String)> = Vec::new();
    loop {
        skip_ws(it);
        match it.peek() {
            Some(&')') => {
                it.next();
                break;
            }
            Some(&':') => {
                it.next();
                let key = read_symbol(it);
                skip_ws(it);
                match key.as_str() {
                    "selector" => {
                        selector = Some(read_string(it)?);
                    }
                    "declarations" => {
                        expect(it, '(')?;
                        loop {
                            skip_ws(it);
                            match it.peek() {
                                Some(&')') => {
                                    it.next();
                                    break;
                                }
                                Some(&'(') => {
                                    it.next();
                                    let prop = read_string(it)?;
                                    let value = read_string(it)?;
                                    skip_ws(it);
                                    expect(it, ')')?;
                                    declarations.push((prop, value));
                                }
                                other => {
                                    return Err(format!("expected decl or ), got {other:?}"));
                                }
                            }
                        }
                    }
                    other => return Err(format!("unknown css-rule key {other:?}")),
                }
            }
            Some(c) => return Err(format!("unexpected char in rule: {c:?}")),
            None => return Err("EOF in rule".into()),
        }
    }
    Ok(CssRule {
        selector: selector.ok_or("missing :selector")?,
        declarations,
    })
}

fn write_quoted(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_is_empty() {
        assert!(parse_css("").is_empty());
        assert!(parse_css("   \n  ").is_empty());
        assert!(parse_css("/* just a comment */").is_empty());
    }

    #[test]
    fn parse_single_rule() {
        let rules = parse_css("div.card { color: red; font-size: 14px; }");
        assert_eq!(rules.len(), 1);
        let r = &rules[0];
        assert_eq!(r.selector, "div.card");
        assert_eq!(r.declarations.len(), 2);
        assert_eq!(r.declarations[0], ("color".into(), "red".into()));
        assert_eq!(r.declarations[1], ("font-size".into(), "14px".into()));
    }

    #[test]
    fn parse_multiple_rules() {
        let src = "a { color: red; } b { color: blue; } p.main { margin: 0; padding: 1em; }";
        let rules = parse_css(src);
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].selector, "a");
        assert_eq!(rules[1].selector, "b");
        assert_eq!(rules[2].selector, "p.main");
        assert_eq!(rules[2].declarations.len(), 2);
    }

    #[test]
    fn parse_strips_line_comments_and_block_comments() {
        let src = "/* header */ a { color: red; /* inline */ font-size: 1em; } /* tail */";
        let rules = parse_css(src);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].declarations.len(), 2);
    }

    #[test]
    fn parse_skips_at_rules_for_now() {
        let src = r#"
            @import url("x.css");
            a { color: red; }
            @media (max-width: 600px) {
                b { color: green; }
            }
            c { color: blue; }
        "#;
        let rules = parse_css(src);
        // V1 drops @media + @import; top-level rules outside them survive.
        assert!(rules.iter().any(|r| r.selector == "a"));
        assert!(rules.iter().any(|r| r.selector == "c"));
        assert!(rules.iter().all(|r| !r.selector.starts_with('@')));
    }

    #[test]
    fn parse_tolerates_trailing_semicolon_and_whitespace() {
        let rules = parse_css("a {\n  color: red ;   \n  font-size:  14px;\n}\n\n");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].declarations[0], ("color".into(), "red".into()));
        assert_eq!(rules[0].declarations[1], ("font-size".into(), "14px".into()));
    }

    #[test]
    fn parse_skips_declarations_without_colon() {
        let rules = parse_css("a { color: red; stray-token; font-size: 1em; }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].declarations.len(), 2);
    }

    #[test]
    fn emit_single_rule_is_deterministic() {
        let rules = vec![CssRule {
            selector: "div".into(),
            declarations: vec![
                ("color".into(), "red".into()),
                ("font-size".into(), "14px".into()),
            ],
        }];
        let a = emit_css(&rules);
        let b = emit_css(&rules);
        assert_eq!(a, b);
        assert!(a.contains("div {"));
        assert!(a.contains("  color: red;"));
        assert!(a.contains("  font-size: 14px;"));
    }

    #[test]
    fn parse_emit_roundtrip_normalizes_whitespace_but_preserves_data() {
        let src = "a { color: red; font-size: 14px; }";
        let rules_1 = parse_css(src);
        let emitted = emit_css(&rules_1);
        let rules_2 = parse_css(&emitted);
        assert_eq!(rules_1, rules_2, "parse → emit → parse must be fixed");
    }

    #[test]
    fn css_to_sexp_emits_our_canonical_form() {
        let rules = parse_css("div.card { color: red; font-size: 14px; }");
        let sexp = css_to_sexp(&rules);
        assert!(sexp.contains("(css-stylesheet"));
        assert!(sexp.contains("(css-rule"));
        assert!(sexp.contains("\"div.card\""));
        assert!(sexp.contains("\"color\" \"red\""));
        assert!(sexp.contains("\"font-size\" \"14px\""));
    }

    #[test]
    fn sexp_roundtrip_is_a_fixed_point() {
        let rules_1 = parse_css("a { color: red; } p.x { margin: 0; padding: 1em; }");
        let sexp_1 = css_to_sexp(&rules_1);
        let rules_2 = sexp_to_css(&sexp_1).expect("sexp_to_css");
        assert_eq!(rules_1, rules_2);
        let sexp_2 = css_to_sexp(&rules_2);
        assert_eq!(sexp_1, sexp_2, "sexp roundtrip isn't a fixed point");
    }

    #[test]
    fn css_to_sexp_to_css_roundtrip() {
        // Full chain: CSS → rules → sexp → rules → CSS.
        let src = "div { color: red; } span { font-weight: bold; }";
        let rules_1 = parse_css(src);
        let sexp = css_to_sexp(&rules_1);
        let rules_2 = sexp_to_css(&sexp).expect("sexp_to_css");
        let emitted = emit_css(&rules_2);
        let rules_3 = parse_css(&emitted);
        assert_eq!(rules_1, rules_3);
    }

    #[test]
    fn sexp_parser_rejects_wrong_top_tag() {
        let bad = "(document)";
        assert!(sexp_to_css(bad).is_err());
    }

    #[test]
    fn sexp_parser_rejects_missing_selector() {
        let bad = "(css-stylesheet (css-rule :declarations ()))";
        assert!(sexp_to_css(bad).is_err());
    }

    #[test]
    fn sexp_parser_handles_escaped_quotes_in_values() {
        // Quotes in attribute values are common in CSS strings.
        let rules = parse_css(r#"a { content: "hello world"; }"#);
        let sexp = css_to_sexp(&rules);
        let back = sexp_to_css(&sexp).expect("roundtrip");
        assert_eq!(rules, back);
    }

    #[test]
    fn empty_stylesheet_sexp_parses_back_to_empty() {
        let rules: Vec<CssRule> = Vec::new();
        let sexp = css_to_sexp(&rules);
        assert_eq!(sexp_to_css(&sexp).unwrap(), rules);
    }
}
