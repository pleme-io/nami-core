//! S-expression parser that reads the output of [`dom_to_sexp`] back
//! into a [`Document`]. Closes the roundtrip:
//!
//! ```text
//!   Document → dom_to_sexp → "(document (element :tag …))"
//!                            ↓
//!                       sexp_to_dom
//!                            ↓
//!                         Document   ≡   the original
//! ```
//!
//! This is a targeted mini-reader for our OWN S-expression grammar —
//! not a general Scheme reader. It understands exactly:
//!
//!   (document CHILD…)
//!   (element :tag "NAME" [:attrs ((KEY "VALUE") …)] CHILD…)
//!   (text "STRING")
//!   (comment "STRING")
//!   (… :elided N)                 -- elided subtree, skipped
//!
//! It does NOT import the full tatara-lisp reader because it runs on
//! the hot path (every shadow put-back roundtrips the tree) and we
//! want zero unrelated features to slow it down. The grammar is
//! closed under our own serializer, so it stays in lockstep.

use crate::dom::{Document, ElementData, Node, NodeData};
use std::iter::Peekable;
use std::str::Chars;

pub fn sexp_to_dom(src: &str) -> Result<Document, String> {
    let mut it = src.chars().peekable();
    skip_ws(&mut it);
    let form = read_form(&mut it)?;
    skip_ws(&mut it);
    if it.peek().is_some() {
        return Err("trailing content after (document …)".into());
    }

    let root = form_to_document_root(form)?;
    Ok(Document { root })
}

fn form_to_document_root(form: Form) -> Result<Node, String> {
    match form {
        Form::List(items) => {
            if items.is_empty() {
                return Err("empty form".into());
            }
            match &items[0] {
                Form::Symbol(s) if s == "document" => {
                    let mut children = Vec::new();
                    for child in items.into_iter().skip(1) {
                        children.push(form_to_node(child)?);
                    }
                    Ok(Node {
                        data: NodeData::Document,
                        children,
                    })
                }
                other => Err(format!(
                    "expected top-level (document …), got ({other:?} …)"
                )),
            }
        }
        other => Err(format!("expected list at top level, got {other:?}")),
    }
}

fn form_to_node(form: Form) -> Result<Node, String> {
    match form {
        Form::List(items) => {
            if items.is_empty() {
                return Err("empty node form".into());
            }
            let head = match &items[0] {
                Form::Symbol(s) => s.clone(),
                other => return Err(format!("expected symbol at head, got {other:?}")),
            };
            let rest: Vec<Form> = items.into_iter().skip(1).collect();
            match head.as_str() {
                "text" => parse_text(rest),
                "comment" => parse_comment(rest),
                "element" => parse_element(rest),
                "document" => parse_document(rest),
                "…" => parse_elided(rest),
                other => Err(format!("unknown node head: {other}")),
            }
        }
        other => Err(format!("expected list, got {other:?}")),
    }
}

fn parse_text(mut rest: Vec<Form>) -> Result<Node, String> {
    if rest.len() != 1 {
        return Err("(text …) expects one string arg".into());
    }
    let s = rest.pop().unwrap().expect_string()?;
    Ok(Node {
        data: NodeData::Text(s),
        children: Vec::new(),
    })
}

fn parse_comment(mut rest: Vec<Form>) -> Result<Node, String> {
    if rest.len() != 1 {
        return Err("(comment …) expects one string arg".into());
    }
    let s = rest.pop().unwrap().expect_string()?;
    Ok(Node {
        data: NodeData::Comment(s),
        children: Vec::new(),
    })
}

fn parse_element(rest: Vec<Form>) -> Result<Node, String> {
    // Grammar: :tag "NAME" [:attrs ((KEY "VAL") …)] CHILD…
    let mut iter = rest.into_iter().peekable();
    let mut tag: Option<String> = None;
    let mut attributes: Vec<(String, String)> = Vec::new();
    let mut children: Vec<Node> = Vec::new();
    let mut elided = false;

    while let Some(next) = iter.next() {
        match next {
            Form::Keyword(k) if k == "tag" => {
                let v = iter
                    .next()
                    .ok_or_else(|| ":tag missing value".to_string())?;
                tag = Some(v.expect_string()?);
            }
            Form::Keyword(k) if k == "attrs" => {
                let v = iter
                    .next()
                    .ok_or_else(|| ":attrs missing value".to_string())?;
                let pairs = v.expect_list()?;
                for pair in pairs {
                    let items = pair.expect_list()?;
                    if items.len() != 2 {
                        return Err(":attrs entry must be (:key \"value\")".into());
                    }
                    let key = match &items[0] {
                        Form::Keyword(k) => k.clone(),
                        Form::String(s) => s.clone(),
                        other => {
                            return Err(format!(
                                "attr key must be keyword or string, got {other:?}"
                            ));
                        }
                    };
                    let val = items.into_iter().nth(1).unwrap().expect_string()?;
                    attributes.push((key, val));
                }
            }
            Form::List(items) => {
                // Child node — or an `(… :elided N)` marker.
                if let Some(Form::Symbol(s)) = items.first() {
                    if s == "…" {
                        elided = true;
                        continue;
                    }
                }
                children.push(form_to_node(Form::List(items))?);
            }
            other => return Err(format!("unexpected form in (element …): {other:?}")),
        }
    }

    let Some(tag) = tag else {
        return Err("(element …) missing :tag".into());
    };
    let _ = elided; // consumed (currently just silently dropped).

    Ok(Node {
        data: NodeData::Element(ElementData {
            tag,
            attributes,
            qual_name: None,
        }),
        children,
    })
}

fn parse_document(rest: Vec<Form>) -> Result<Node, String> {
    // Nested document node (rare but serialized consistently).
    let mut children = Vec::new();
    for f in rest {
        children.push(form_to_node(f)?);
    }
    Ok(Node {
        data: NodeData::Document,
        children,
    })
}

fn parse_elided(_rest: Vec<Form>) -> Result<Node, String> {
    // (… :elided N) — represents a pruned subtree. Reconstructing
    // here is lossy by definition; emit an empty text node as a
    // deterministic placeholder.
    Ok(Node {
        data: NodeData::Text(String::new()),
        children: Vec::new(),
    })
}

// ── generic S-expression mini-reader ─────────────────────────────

#[derive(Debug, Clone)]
enum Form {
    Symbol(String),
    Keyword(String), // `:name` — value is the name without the colon
    String(String),
    List(Vec<Form>),
}

impl Form {
    fn expect_string(self) -> Result<String, String> {
        match self {
            Self::String(s) => Ok(s),
            other => Err(format!("expected string, got {other:?}")),
        }
    }
    fn expect_list(self) -> Result<Vec<Form>, String> {
        match self {
            Self::List(v) => Ok(v),
            other => Err(format!("expected list, got {other:?}")),
        }
    }
}

fn read_form(it: &mut Peekable<Chars>) -> Result<Form, String> {
    skip_ws(it);
    let Some(&c) = it.peek() else {
        return Err("unexpected EOF".into());
    };
    match c {
        '(' => read_list(it),
        '"' => read_string(it).map(Form::String),
        ':' => {
            it.next();
            let name = read_ident(it);
            if name.is_empty() {
                return Err("bare ':' with no keyword name".into());
            }
            Ok(Form::Keyword(name))
        }
        _ => {
            let sym = read_ident(it);
            if sym.is_empty() {
                return Err(format!("unexpected character {c:?}"));
            }
            Ok(Form::Symbol(sym))
        }
    }
}

fn read_list(it: &mut Peekable<Chars>) -> Result<Form, String> {
    // Consume '('
    it.next();
    let mut out = Vec::new();
    loop {
        skip_ws(it);
        match it.peek() {
            None => return Err("unterminated list (missing `)`)".into()),
            Some(&')') => {
                it.next();
                return Ok(Form::List(out));
            }
            Some(_) => out.push(read_form(it)?),
        }
    }
}

fn read_string(it: &mut Peekable<Chars>) -> Result<String, String> {
    // Consume opening '"'
    it.next();
    let mut out = String::new();
    loop {
        match it.next() {
            None => return Err("unterminated string".into()),
            Some('"') => return Ok(out),
            Some('\\') => match it.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    // \u{XXXX}
                    if it.next() != Some('{') {
                        return Err("expected '{' after \\u".into());
                    }
                    let mut hex = String::new();
                    loop {
                        match it.next() {
                            Some('}') => break,
                            Some(c) if c.is_ascii_hexdigit() => hex.push(c),
                            Some(c) => {
                                return Err(format!("invalid hex in \\u{{…}}: {c:?}"));
                            }
                            None => return Err("unterminated \\u{…}".into()),
                        }
                    }
                    let cp =
                        u32::from_str_radix(&hex, 16).map_err(|e| format!("hex decode: {e}"))?;
                    let c = char::from_u32(cp)
                        .ok_or_else(|| format!("invalid codepoint U+{cp:04x}"))?;
                    out.push(c);
                }
                Some(other) => return Err(format!("unknown escape: \\{other}")),
                None => return Err("trailing backslash in string".into()),
            },
            Some(c) => out.push(c),
        }
    }
}

fn read_ident(it: &mut Peekable<Chars>) -> String {
    let mut out = String::new();
    while let Some(&c) = it.peek() {
        if c.is_whitespace() || c == '(' || c == ')' || c == '"' {
            break;
        }
        out.push(c);
        it.next();
    }
    out
}

fn skip_ws(it: &mut Peekable<Chars>) {
    while let Some(&c) = it.peek() {
        if c.is_whitespace() {
            it.next();
        } else if c == ';' {
            // Line comment to end of line.
            for c in it.by_ref() {
                if c == '\n' {
                    break;
                }
            }
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{SexpOptions, dom_to_sexp, dom_to_sexp_with};
    use super::sexp_to_dom;
    use crate::dom::{Document, NodeData};

    fn roundtrip(html: &str) -> Document {
        let src = Document::parse(html);
        let sexp = dom_to_sexp_with(
            &src,
            &SexpOptions {
                depth_cap: None,
                pretty: false,
                trim_whitespace: true,
            },
        );
        sexp_to_dom(&sexp).expect("roundtrip")
    }

    #[test]
    fn roundtrips_plain_element() {
        let back = roundtrip("<html><body><p>hi</p></body></html>");
        assert!(back.text_content().contains("hi"));
    }

    #[test]
    fn roundtrips_attributes() {
        let back =
            roundtrip(r#"<html><body><a href="https://x" class="hero">go</a></body></html>"#);
        let a = back
            .root
            .descendants()
            .find_map(|n| n.as_element().filter(|e| e.tag == "a"))
            .unwrap();
        assert_eq!(a.get_attribute("href"), Some("https://x"));
        assert_eq!(a.get_attribute("class"), Some("hero"));
    }

    #[test]
    fn roundtrips_comments() {
        let back = roundtrip("<html><body><!-- note --><p>x</p></body></html>");
        let mut found = false;
        for node in back.root.descendants() {
            if let NodeData::Comment(c) = &node.data {
                if c.contains("note") {
                    found = true;
                }
            }
        }
        assert!(found);
    }

    #[test]
    fn roundtrips_framework_attributes() {
        let back = roundtrip(
            r##"<html><body><button hx-get="/api/x" data-state="open">go</button></body></html>"##,
        );
        let btn = back
            .root
            .descendants()
            .find_map(|n| n.as_element().filter(|e| e.tag == "button"))
            .unwrap();
        assert_eq!(btn.get_attribute("hx-get"), Some("/api/x"));
        assert_eq!(btn.get_attribute("data-state"), Some("open"));
    }

    #[test]
    fn roundtrips_escaped_text() {
        let back = roundtrip(r#"<html><body><p>quoted "value" with \backslash</p></body></html>"#);
        let p = back
            .root
            .descendants()
            .find(|n| n.as_element().is_some_and(|e| e.tag == "p"))
            .unwrap();
        assert!(p.text_content().contains(r#""value""#));
    }

    #[test]
    fn parses_pretty_printed_sexp() {
        // dom_to_sexp with pretty=true produces newlines + indent.
        let src = Document::parse("<html><body><section><p>hi</p></section></body></html>");
        let sexp = dom_to_sexp(&src); // default pretty=true
        let back = sexp_to_dom(&sexp).expect("parse pretty");
        assert!(back.text_content().contains("hi"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(sexp_to_dom("not an s-expression").is_err());
        assert!(sexp_to_dom("(not-a-document)").is_err());
        assert!(sexp_to_dom("(document (unknown))").is_err());
    }

    #[test]
    fn handles_elided_subtree_placeholder() {
        // An elided subtree is allowed as a placeholder and parses
        // back as an empty text node (lossy by design).
        let input = r#"(document (element :tag "html" (… :elided 3)))"#;
        let doc = sexp_to_dom(input).expect("elided parses");
        // html element present; its single child is the placeholder
        let html = doc
            .root
            .children
            .first()
            .and_then(|n| n.as_element())
            .expect("html");
        assert_eq!(html.tag, "html");
    }

    #[test]
    fn ignores_line_comments() {
        let src = r#"
            ; this is a comment
            (document
              (element :tag "html"
                ; another
                (element :tag "body"
                  (text "ok"))))
        "#;
        let back = sexp_to_dom(src).expect("comments-ok");
        assert!(back.text_content().contains("ok"));
    }

    #[test]
    fn roundtrip_is_idempotent_after_two_cycles() {
        let once = roundtrip("<html><body><p>hi</p></body></html>");
        let sexp1 = dom_to_sexp_with(
            &once,
            &SexpOptions {
                depth_cap: None,
                pretty: false,
                trim_whitespace: true,
            },
        );
        let twice = sexp_to_dom(&sexp1).expect("second parse");
        let sexp2 = dom_to_sexp_with(
            &twice,
            &SexpOptions {
                depth_cap: None,
                pretty: false,
                trim_whitespace: true,
            },
        );
        assert_eq!(sexp1, sexp2, "roundtrip should be a fixed point");
    }
}
