//! Components — declarative DOM templates (JSX-for-Lisp).
//!
//! S-expressions are already a templating language; this module just
//! formalizes them into **named, reusable, prop-parameterized** DOM
//! fragments. A component is:
//!
//! ```lisp
//! (defcomponent :name "Card"
//!               :props ("title" "body")
//!               :template "(div :class \"card\"
//!                              (h2 (@ title))
//!                              (p  (@ body)))")
//! ```
//!
//! Template grammar (a minimal subset of our S-expression shape):
//!
//!   tree     := (TAG [KV…] [CHILD…])
//!   CHILD    := tree
//!             | STRING      -- literal text
//!             | (@ NAME)    -- prop reference (expands to the prop value)
//!   KV       := :ATTR VALUE   -- attribute
//!   VALUE    := STRING
//!             | (@ NAME)    -- dynamic attribute value
//!
//! Expansion: `expand(spec, props)` → `Vec<Node>`. Rendered nodes
//! land in nami-core's canonical `Node` shape, which means they
//! round-trip through the same DOM ↔ Lisp primitives as any parsed
//! page and integrate with transforms, scrapes, shadow, snapshot.
//!
//! This is **Layer 1** of the React-in-Lisp arc: declarative
//! templating. State (`defstate`), effects (`defeffect`), and a real
//! evaluator come in later layers. What we get today: components
//! agents can inject via `insert-before` / `replace-with` transforms
//! with parameterized content.

use crate::dom::{ElementData, Node, NodeData};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::iter::Peekable;
use std::str::Chars;

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// A declarative DOM template.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defcomponent"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComponentSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Named parameters this component consumes. Expansion fails if
    /// a required prop isn't supplied.
    #[serde(default)]
    pub props: Vec<String>,
    /// The template body as a string. Parsed on each expansion —
    /// callers that expand repeatedly should pre-parse via
    /// [`Template::parse`] and reuse.
    pub template: String,
}

/// Registry + lookup.
#[derive(Debug, Clone, Default)]
pub struct ComponentRegistry {
    specs: Vec<ComponentSpec>,
}

impl ComponentRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: ComponentSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ComponentSpec>) {
        for s in specs {
            self.insert(s);
        }
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ComponentSpec> {
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

    /// Expand a component by name with a prop map. Prop values are
    /// arbitrary JSON — strings pass through verbatim, numbers and
    /// booleans are stringified.
    pub fn expand(&self, name: &str, props: &Value) -> Result<Vec<Node>, String> {
        let spec = self
            .get(name)
            .ok_or_else(|| format!("unknown component: {name}"))?;
        expand(spec, props)
    }
}

/// Expand one spec against a prop value. Parses the template each
/// call — use [`Template::parse`] + [`Template::expand`] for hot paths.
pub fn expand(spec: &ComponentSpec, props: &Value) -> Result<Vec<Node>, String> {
    let template = Template::parse(&spec.template)?;
    template.expand(&spec.props, props)
}

/// Parsed template — cacheable across expansions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    roots: Vec<TemplateNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TemplateNode {
    Element {
        tag: String,
        attrs: Vec<(String, Value_)>,
        children: Vec<TemplateNode>,
    },
    LiteralText(String),
    PropRef(String),
}

/// Either a literal string or a prop reference (for attribute values).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value_ {
    Literal(String),
    PropRef(String),
}

impl Template {
    /// Parse template source into a reusable tree.
    pub fn parse(src: &str) -> Result<Self, String> {
        let mut reader = Reader::new(src);
        let mut roots = Vec::new();
        loop {
            reader.skip_ws();
            if reader.peek().is_none() {
                break;
            }
            roots.push(reader.read_node()?);
        }
        if roots.is_empty() {
            return Err("empty template".into());
        }
        Ok(Self { roots })
    }

    /// Expand against a prop map. `required_props` is the spec's
    /// declared prop list; expansion fails if any required prop is
    /// missing from `props`.
    pub fn expand(&self, required_props: &[String], props: &Value) -> Result<Vec<Node>, String> {
        for name in required_props {
            if props_get(props, name).is_none() {
                return Err(format!("missing prop: {name}"));
            }
        }
        self.roots
            .iter()
            .map(|n| expand_node(n, props))
            .collect::<Result<Vec<_>, _>>()
            .map(|v| v.into_iter().flatten().collect())
    }
}

fn expand_node(tn: &TemplateNode, props: &Value) -> Result<Vec<Node>, String> {
    match tn {
        TemplateNode::Element {
            tag,
            attrs,
            children,
        } => {
            let mut attributes = Vec::with_capacity(attrs.len());
            for (k, v) in attrs {
                let resolved = match v {
                    Value_::Literal(s) => s.clone(),
                    Value_::PropRef(name) => props_as_string(props, name)
                        .ok_or_else(|| format!("missing prop: {name}"))?,
                };
                attributes.push((k.clone(), resolved));
            }
            let mut child_nodes = Vec::new();
            for c in children {
                child_nodes.extend(expand_node(c, props)?);
            }
            Ok(vec![Node {
                data: NodeData::Element(ElementData {
                    tag: tag.clone(),
                    attributes,
                    qual_name: None,
                }),
                children: child_nodes,
            }])
        }
        TemplateNode::LiteralText(s) => Ok(vec![Node {
            data: NodeData::Text(s.clone()),
            children: vec![],
        }]),
        TemplateNode::PropRef(name) => {
            let s = props_as_string(props, name).ok_or_else(|| format!("missing prop: {name}"))?;
            Ok(vec![Node {
                data: NodeData::Text(s),
                children: vec![],
            }])
        }
    }
}

fn props_get<'a>(props: &'a Value, name: &str) -> Option<&'a Value> {
    props.get(name)
}

fn props_as_string(props: &Value, name: &str) -> Option<String> {
    props.get(name).map(|v| match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    })
}

/// Register the `defcomponent` keyword.
#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ComponentSpec>();
}

/// Compile a Lisp source file of `(defcomponent …)` forms.
#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ComponentSpec>, String> {
    tatara_lisp::compile_typed::<ComponentSpec>(src).map_err(|e| format!("{e}"))
}

// ── Reader ──────────────────────────────────────────────────────

struct Reader<'a> {
    chars: Peekable<Chars<'a>>,
}

impl<'a> Reader<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            chars: src.chars().peekable(),
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn next(&mut self) -> Option<char> {
        self.chars.next()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.next();
            } else if c == ';' {
                // line comment
                for c in self.chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn read_node(&mut self) -> Result<TemplateNode, String> {
        self.skip_ws();
        match self.peek() {
            Some('(') => self.read_list(),
            Some('"') => self.read_string().map(TemplateNode::LiteralText),
            Some(_) => Err("expected '(' or string literal at top of child".into()),
            None => Err("unexpected EOF while reading node".into()),
        }
    }

    fn read_list(&mut self) -> Result<TemplateNode, String> {
        // consume '('
        self.next();
        self.skip_ws();
        // Special form: (@ prop-name)
        if self.peek() == Some('@') {
            self.next();
            self.skip_ws();
            let ident = self.read_ident();
            if ident.is_empty() {
                return Err("(@ …) missing prop name".into());
            }
            self.skip_ws();
            if self.next() != Some(')') {
                return Err("(@ …) expected ')'".into());
            }
            return Ok(TemplateNode::PropRef(ident));
        }

        // Otherwise a tag form: (tag [kv…] [child…])
        let tag = self.read_ident();
        if tag.is_empty() {
            return Err("expected tag name".into());
        }

        let mut attrs: Vec<(String, Value_)> = Vec::new();
        let mut children: Vec<TemplateNode> = Vec::new();

        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err("unterminated (…)".into()),
                Some(')') => {
                    self.next();
                    return Ok(TemplateNode::Element {
                        tag,
                        attrs,
                        children,
                    });
                }
                Some(':') => {
                    self.next();
                    let key = self.read_ident();
                    if key.is_empty() {
                        return Err("':' not followed by attribute name".into());
                    }
                    self.skip_ws();
                    let value = self.read_value()?;
                    attrs.push((key, value));
                }
                Some('"') => {
                    // text child
                    let s = self.read_string()?;
                    children.push(TemplateNode::LiteralText(s));
                }
                Some('(') => {
                    children.push(self.read_node()?);
                }
                Some(c) => return Err(format!("unexpected char in list body: {c:?}")),
            }
        }
    }

    fn read_value(&mut self) -> Result<Value_, String> {
        self.skip_ws();
        match self.peek() {
            Some('"') => Ok(Value_::Literal(self.read_string()?)),
            Some('(') => {
                // Must be a (@ prop) prop-ref.
                self.next();
                self.skip_ws();
                if self.peek() != Some('@') {
                    return Err("attribute value list must be (@ prop-name)".into());
                }
                self.next();
                self.skip_ws();
                let ident = self.read_ident();
                if ident.is_empty() {
                    return Err("(@ …) missing prop name".into());
                }
                self.skip_ws();
                if self.next() != Some(')') {
                    return Err("(@ …) expected ')'".into());
                }
                Ok(Value_::PropRef(ident))
            }
            Some(c) => Err(format!(
                "expected string or (@ …) for attr value, got {c:?}"
            )),
            None => Err("EOF while reading attribute value".into()),
        }
    }

    fn read_string(&mut self) -> Result<String, String> {
        // consume '"'
        self.next();
        let mut out = String::new();
        loop {
            match self.next() {
                None => return Err("unterminated string".into()),
                Some('"') => return Ok(out),
                Some('\\') => match self.next() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(c) => return Err(format!("unknown escape: \\{c}")),
                    None => return Err("trailing backslash".into()),
                },
                Some(c) => out.push(c),
            }
        }
    }

    fn read_ident(&mut self) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || c == '(' || c == ')' || c == '"' || c == ':' || c == '@' {
                break;
            }
            out.push(c);
            self.next();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(name: &str, props: &[&str], template: &str) -> ComponentSpec {
        ComponentSpec {
            name: name.into(),
            description: None,
            props: props.iter().map(|s| s.to_string()).collect(),
            template: template.into(),
        }
    }

    #[test]
    fn expand_simple_element_no_props() {
        let s = spec("Hi", &[], r#"(p "hello")"#);
        let nodes = expand(&s, &json!({})).unwrap();
        assert_eq!(nodes.len(), 1);
        let p = nodes[0].as_element().unwrap();
        assert_eq!(p.tag, "p");
        assert_eq!(nodes[0].children.len(), 1);
        assert!(matches!(&nodes[0].children[0].data, NodeData::Text(t) if t == "hello"));
    }

    #[test]
    fn expand_element_with_literal_attrs() {
        let s = spec(
            "Link",
            &[],
            r#"(a :href "https://x" :class "cta" "click me")"#,
        );
        let nodes = expand(&s, &json!({})).unwrap();
        let a = nodes[0].as_element().unwrap();
        assert_eq!(a.tag, "a");
        assert_eq!(a.get_attribute("href"), Some("https://x"));
        assert_eq!(a.get_attribute("class"), Some("cta"));
    }

    #[test]
    fn expand_with_text_prop_ref() {
        let s = spec("Greet", &["who"], r#"(p "hello " (@ who) "!")"#);
        let nodes = expand(&s, &json!({"who": "world"})).unwrap();
        let p = nodes[0].as_element().unwrap();
        assert_eq!(p.tag, "p");
        // Children: "hello ", Text("world"), "!"
        assert_eq!(nodes[0].children.len(), 3);
        assert!(matches!(&nodes[0].children[0].data, NodeData::Text(t) if t == "hello "));
        assert!(matches!(&nodes[0].children[1].data, NodeData::Text(t) if t == "world"));
        assert!(matches!(&nodes[0].children[2].data, NodeData::Text(t) if t == "!"));
    }

    #[test]
    fn expand_with_attr_prop_ref() {
        let s = spec("Avatar", &["src"], r#"(img :src (@ src) :alt "avatar")"#);
        let nodes = expand(&s, &json!({"src": "/me.png"})).unwrap();
        let img = nodes[0].as_element().unwrap();
        assert_eq!(img.get_attribute("src"), Some("/me.png"));
        assert_eq!(img.get_attribute("alt"), Some("avatar"));
    }

    #[test]
    fn expand_nested_card_component() {
        let s = spec(
            "Card",
            &["title", "body"],
            r#"(div :class "card"
                   (h2 (@ title))
                   (p  (@ body)))"#,
        );
        let nodes = expand(&s, &json!({"title": "Hi", "body": "there"})).unwrap();
        let div = nodes[0].as_element().unwrap();
        assert_eq!(div.tag, "div");
        assert_eq!(div.get_attribute("class"), Some("card"));
        assert_eq!(nodes[0].children.len(), 2);
        let h2 = nodes[0].children[0].as_element().unwrap();
        assert_eq!(h2.tag, "h2");
        assert!(matches!(
            &nodes[0].children[0].children[0].data,
            NodeData::Text(t) if t == "Hi"
        ));
        let p = nodes[0].children[1].as_element().unwrap();
        assert_eq!(p.tag, "p");
    }

    #[test]
    fn expand_missing_required_prop_errors() {
        let s = spec("G", &["who"], r#"(p (@ who))"#);
        let err = expand(&s, &json!({})).unwrap_err();
        assert!(err.contains("missing prop"));
    }

    #[test]
    fn expand_numeric_and_bool_props_stringify() {
        let s = spec("S", &["n", "b"], r#"(p "n=" (@ n) " b=" (@ b))"#);
        let nodes = expand(&s, &json!({"n": 42, "b": true})).unwrap();
        let p = nodes[0].as_element().unwrap();
        assert_eq!(p.tag, "p");
        assert!(matches!(&nodes[0].children[1].data, NodeData::Text(t) if t == "42"));
        assert!(matches!(&nodes[0].children[3].data, NodeData::Text(t) if t == "true"));
    }

    #[test]
    fn multi_root_template_is_a_fragment() {
        let s = spec("Pair", &[], r#"(h1 "Title") (p "Body")"#);
        let nodes = expand(&s, &json!({})).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].as_element().unwrap().tag, "h1");
        assert_eq!(nodes[1].as_element().unwrap().tag, "p");
    }

    #[test]
    fn registry_insert_and_expand() {
        let mut reg = ComponentRegistry::new();
        reg.insert(spec("Hi", &["who"], r#"(p "hello " (@ who))"#));
        let nodes = reg.expand("Hi", &json!({"who": "Lisp"})).unwrap();
        assert_eq!(nodes[0].children.len(), 2);
    }

    #[test]
    fn registry_unknown_component_errors() {
        let reg = ComponentRegistry::new();
        assert!(reg.expand("ghost", &json!({})).is_err());
    }

    #[test]
    fn template_can_be_pre_parsed_and_reused() {
        let t = Template::parse(r#"(p (@ name))"#).unwrap();
        let a = t
            .expand(&["name".into()], &json!({"name": "Alice"}))
            .unwrap();
        let b = t.expand(&["name".into()], &json!({"name": "Bob"})).unwrap();
        // Different props produce different text children.
        let a_text = match &a[0].children[0].data {
            NodeData::Text(t) => t.clone(),
            _ => String::new(),
        };
        let b_text = match &b[0].children[0].data {
            NodeData::Text(t) => t.clone(),
            _ => String::new(),
        };
        assert_eq!(a_text, "Alice");
        assert_eq!(b_text, "Bob");
    }

    #[test]
    fn template_rejects_invalid_syntax() {
        assert!(Template::parse("").is_err());
        assert!(Template::parse("(").is_err());
        assert!(Template::parse("(@ )").is_err());
        assert!(Template::parse("(tag :no-value-here)").is_err());
    }

    #[test]
    fn template_line_comments_ignored() {
        let src = r#"
            ; this is a comment
            (div :class "main"
              ; another
              (p "hi"))
        "#;
        let t = Template::parse(src).unwrap();
        let nodes = t.expand(&[], &json!({})).unwrap();
        assert_eq!(nodes[0].as_element().unwrap().tag, "div");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_round_trip_component_specs() {
        let src = r#"
            (defcomponent :name "Card"
                          :props ("title" "body")
                          :template "(div :class \"card\"
                                          (h2 (@ title))
                                          (p  (@ body)))"
                          :description "a simple card")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].props, vec!["title", "body"]);
    }
}
