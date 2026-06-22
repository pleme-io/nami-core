//! DOM → Markdown — the pleme-io absorption of Obscura's `LP.getMarkdown`
//! (`fetch --dump markdown`) as a **typed AST renderer**.
//!
//! Pure Rust, zero engine dependency: it walks the nami-core [`Document`] DOM
//! and lowers it to a typed [`Markdown`] AST whose `Display` impls are the only
//! emission surface (★★ TYPED EMISSION — no `format!()` of markup, no
//! free-form string concatenation of Markdown syntax). The agent-facing
//! "give me the page as Markdown" deliverable that the substrate's text /
//! S-expr / JSON renders did not cover.
//!
//! ```
//! use nami_core::dom::Document;
//! use nami_core::markdown::to_markdown;
//! let doc = Document::parse("<h1>Title</h1><p>Hello <a href=\"/x\">link</a></p>");
//! let md = to_markdown(&doc);
//! assert!(md.contains("# Title"));
//! assert!(md.contains("[link](/x)"));
//! ```

use std::fmt::{self, Write as _};

use crate::dom::{Document, Node, NodeData};

/// A Markdown document — an ordered sequence of typed blocks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Markdown {
    pub blocks: Vec<Block>,
}

/// A block-level Markdown construct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// `# … ###### ` — `level` is clamped to 1..=6.
    Heading { level: u8, inlines: Vec<Inline> },
    /// A paragraph of inline content.
    Paragraph(Vec<Inline>),
    /// `> ` quoted nested blocks.
    BlockQuote(Vec<Block>),
    /// A fenced code block with an optional info string (language).
    CodeBlock { lang: Option<String>, text: String },
    /// `-`/`1.` list; each item is its own block sequence.
    List { ordered: bool, items: Vec<Vec<Block>> },
    /// `---`
    ThematicBreak,
}

/// An inline Markdown construct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Code(String),
    Link { text: Vec<Inline>, href: String },
    Image { alt: String, src: String },
    /// Inter-run whitespace (renders as a single space).
    SoftBreak,
    /// `<br>` — Markdown hard line break (two trailing spaces + newline).
    HardBreak,
}

// ── rendering: Display is the ONLY emission surface ─────────────────────────

impl fmt::Display for Markdown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, block) in self.blocks.iter().enumerate() {
            if i > 0 {
                f.write_str("\n\n")?;
            }
            write!(f, "{block}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Block::Heading { level, inlines } => {
                let level = (*level).clamp(1, 6);
                for _ in 0..level {
                    f.write_char('#')?;
                }
                f.write_char(' ')?;
                write_inlines(f, inlines)
            }
            Block::Paragraph(inlines) => write_inlines(f, inlines),
            Block::BlockQuote(blocks) => {
                let inner = render_blocks(blocks);
                for (i, line) in inner.lines().enumerate() {
                    if i > 0 {
                        f.write_char('\n')?;
                    }
                    if line.is_empty() {
                        f.write_char('>')?;
                    } else {
                        write!(f, "> {line}")?;
                    }
                }
                Ok(())
            }
            Block::CodeBlock { lang, text } => {
                f.write_str("```")?;
                if let Some(lang) = lang {
                    f.write_str(lang)?;
                }
                f.write_char('\n')?;
                f.write_str(text)?;
                if !text.ends_with('\n') {
                    f.write_char('\n')?;
                }
                f.write_str("```")
            }
            Block::List { ordered, items } => {
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_char('\n')?;
                    }
                    let marker_len = if *ordered {
                        write!(f, "{}. ", i + 1)?;
                        // "N. " width for continuation indent.
                        (i + 1).to_string().len() + 2
                    } else {
                        f.write_str("- ")?;
                        2
                    };
                    let body = render_blocks(item);
                    for (j, line) in body.lines().enumerate() {
                        if j == 0 {
                            f.write_str(line)?;
                        } else {
                            f.write_char('\n')?;
                            for _ in 0..marker_len {
                                f.write_char(' ')?;
                            }
                            f.write_str(line)?;
                        }
                    }
                }
                Ok(())
            }
            Block::ThematicBreak => f.write_str("---"),
        }
    }
}

impl fmt::Display for Inline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Inline::Text(s) => f.write_str(s),
            Inline::Strong(v) => {
                f.write_str("**")?;
                write_inlines(f, v)?;
                f.write_str("**")
            }
            Inline::Emphasis(v) => {
                f.write_char('*')?;
                write_inlines(f, v)?;
                f.write_char('*')
            }
            Inline::Code(s) => write!(f, "`{s}`"),
            Inline::Link { text, href } => {
                f.write_char('[')?;
                write_inlines(f, text)?;
                write!(f, "]({href})")
            }
            Inline::Image { alt, src } => write!(f, "![{alt}]({src})"),
            Inline::SoftBreak => f.write_char(' '),
            Inline::HardBreak => f.write_str("  \n"),
        }
    }
}

fn write_inlines(f: &mut fmt::Formatter<'_>, inlines: &[Inline]) -> fmt::Result {
    for inline in inlines {
        write!(f, "{inline}")?;
    }
    Ok(())
}

fn render_blocks(blocks: &[Block]) -> String {
    let mut out = String::new();
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        // Display into a String — the Block Display impl is the emission surface.
        let _ = write!(out, "{block}");
    }
    out
}

// ── DOM → AST lowering ──────────────────────────────────────────────────────

/// Render a parsed [`Document`] to a Markdown string.
#[must_use]
pub fn to_markdown(doc: &Document) -> String {
    document_to_markdown(doc).to_string()
}

/// Lower a parsed [`Document`] to the typed [`Markdown`] AST. Prefers the
/// `<body>` subtree; falls back to the whole tree.
#[must_use]
pub fn document_to_markdown(doc: &Document) -> Markdown {
    let root = find_body(&doc.root).unwrap_or(&doc.root);
    let mut blocks = Vec::new();
    let mut pending: Vec<Inline> = Vec::new();
    convert_children(root, &mut blocks, &mut pending);
    flush(&mut blocks, &mut pending);
    Markdown { blocks }
}

fn find_body(node: &Node) -> Option<&Node> {
    node.descendants()
        .find(|n| n.as_element().is_some_and(|el| el.tag.eq_ignore_ascii_case("body")))
}

/// Walk a node's children in block context, accumulating loose inline content
/// in `pending` and flushing it as a paragraph at every block boundary.
fn convert_children(node: &Node, blocks: &mut Vec<Block>, pending: &mut Vec<Inline>) {
    for child in &node.children {
        match &child.data {
            NodeData::Comment(_) | NodeData::Document => {}
            NodeData::Text(t) => {
                push_text(pending, t);
            }
            NodeData::Element(el) => {
                let tag = el.tag.to_ascii_lowercase();
                if is_skipped(&tag) {
                    continue;
                }
                if let Some(block) = block_for(&tag, child) {
                    flush(blocks, pending);
                    blocks.push(block);
                } else if is_inline(&tag) {
                    if let Some(inline) = inline_for(&tag, child) {
                        pending.push(inline);
                    }
                } else {
                    // Transparent container (div/section/article/main/…):
                    // recurse so its block children surface at this level.
                    convert_children(child, blocks, pending);
                }
            }
        }
    }
}

fn flush(blocks: &mut Vec<Block>, pending: &mut Vec<Inline>) {
    trim_inlines(pending);
    if !pending.is_empty() {
        blocks.push(Block::Paragraph(std::mem::take(pending)));
    } else {
        pending.clear();
    }
}

/// Build a block from a block-level element, or `None` if `tag` is not block.
fn block_for(tag: &str, node: &Node) -> Option<Block> {
    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = tag.as_bytes()[1] - b'0';
            Some(Block::Heading {
                level,
                inlines: inlines_of(node),
            })
        }
        "p" => Some(Block::Paragraph(inlines_of(node))),
        "blockquote" => {
            let mut inner = Vec::new();
            let mut pending = Vec::new();
            convert_children(node, &mut inner, &mut pending);
            flush(&mut inner, &mut pending);
            Some(Block::BlockQuote(inner))
        }
        "pre" => Some(Block::CodeBlock {
            lang: code_lang(node),
            text: node.text_content(),
        }),
        "ul" | "ol" => {
            let ordered = tag == "ol";
            let mut items = Vec::new();
            for li in &node.children {
                if li.as_element().is_some_and(|e| e.tag.eq_ignore_ascii_case("li")) {
                    let mut inner = Vec::new();
                    let mut pending = Vec::new();
                    convert_children(li, &mut inner, &mut pending);
                    flush(&mut inner, &mut pending);
                    items.push(inner);
                }
            }
            Some(Block::List { ordered, items })
        }
        "hr" => Some(Block::ThematicBreak),
        _ => None,
    }
}

/// Collect the inline content of a node (recursively), collapsing whitespace.
fn inlines_of(node: &Node) -> Vec<Inline> {
    let mut out = Vec::new();
    collect_inlines(node, &mut out);
    trim_inlines(&mut out);
    out
}

fn collect_inlines(node: &Node, out: &mut Vec<Inline>) {
    for child in &node.children {
        match &child.data {
            NodeData::Text(t) => push_text(out, t),
            NodeData::Element(el) => {
                let tag = el.tag.to_ascii_lowercase();
                if is_skipped(&tag) {
                    continue;
                }
                if let Some(inline) = inline_for(&tag, child) {
                    out.push(inline);
                } else {
                    // Unknown/transparent inline wrapper: descend.
                    collect_inlines(child, out);
                }
            }
            NodeData::Comment(_) | NodeData::Document => {}
        }
    }
}

/// Build an inline from an inline-level element, or `None` to descend.
fn inline_for(tag: &str, node: &Node) -> Option<Inline> {
    match tag {
        "strong" | "b" => Some(Inline::Strong(inlines_of(node))),
        "em" | "i" => Some(Inline::Emphasis(inlines_of(node))),
        "code" => Some(Inline::Code(node.text_content())),
        "br" => Some(Inline::HardBreak),
        "a" => {
            let href = node
                .as_element()
                .and_then(|el| el.get_attribute("href"))
                .unwrap_or_default()
                .to_string();
            Some(Inline::Link {
                text: inlines_of(node),
                href,
            })
        }
        "img" => {
            let el = node.as_element();
            let src = el
                .and_then(|e| e.get_attribute("src"))
                .unwrap_or_default()
                .to_string();
            let alt = el
                .and_then(|e| e.get_attribute("alt"))
                .unwrap_or_default()
                .to_string();
            Some(Inline::Image { alt, src })
        }
        _ => None,
    }
}

/// Push text, collapsing internal whitespace runs to single spaces and
/// emitting a leading/trailing `SoftBreak` where the source had whitespace.
fn push_text(out: &mut Vec<Inline>, raw: &str) {
    if raw.trim().is_empty() {
        if raw.contains(char::is_whitespace) && !out.is_empty() {
            out.push(Inline::SoftBreak);
        }
        return;
    }
    let leading = raw.starts_with(char::is_whitespace);
    let trailing = raw.ends_with(char::is_whitespace);
    if leading && !out.is_empty() {
        out.push(Inline::SoftBreak);
    }
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    out.push(Inline::Text(collapsed));
    if trailing {
        out.push(Inline::SoftBreak);
    }
}

/// Drop leading/trailing soft breaks from an inline run.
fn trim_inlines(inlines: &mut Vec<Inline>) {
    while matches!(inlines.first(), Some(Inline::SoftBreak)) {
        inlines.remove(0);
    }
    while matches!(inlines.last(), Some(Inline::SoftBreak)) {
        inlines.pop();
    }
}

fn code_lang(node: &Node) -> Option<String> {
    // <pre><code class="language-rust"> → "rust"
    node.descendants()
        .filter_map(Node::as_element)
        .find(|el| el.tag.eq_ignore_ascii_case("code"))
        .and_then(|el| el.get_attribute("class"))
        .and_then(|class| {
            class
                .split_whitespace()
                .find_map(|c| c.strip_prefix("language-").map(str::to_string))
        })
}

fn is_skipped(tag: &str) -> bool {
    matches!(
        tag,
        "script" | "style" | "noscript" | "template" | "head" | "title" | "meta" | "link"
    )
}

fn is_inline(tag: &str) -> bool {
    matches!(
        tag,
        "a" | "b"
            | "strong"
            | "i"
            | "em"
            | "code"
            | "br"
            | "img"
            | "span"
            | "small"
            | "sub"
            | "sup"
            | "mark"
            | "u"
            | "abbr"
            | "cite"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(html: &str) -> String {
        to_markdown(&Document::parse(html))
    }

    #[test]
    fn heading_levels() {
        assert!(md("<h1>One</h1>").contains("# One"));
        assert!(md("<h3>Three</h3>").contains("### Three"));
    }

    #[test]
    fn paragraph_with_link() {
        let out = md(r#"<p>see <a href="https://x.io">here</a> ok</p>"#);
        assert!(out.contains("see [here](https://x.io) ok"), "got: {out}");
    }

    #[test]
    fn bold_and_italic() {
        let out = md("<p>a <strong>bold</strong> and <em>it</em></p>");
        assert!(out.contains("a **bold** and *it*"), "got: {out}");
    }

    #[test]
    fn inline_code_and_pre() {
        let out = md(r#"<p>run <code>cargo test</code></p><pre><code class="language-rust">fn main(){}</code></pre>"#);
        assert!(out.contains("run `cargo test`"), "got: {out}");
        assert!(out.contains("```rust"), "got: {out}");
        assert!(out.contains("fn main(){}"), "got: {out}");
    }

    #[test]
    fn unordered_list() {
        let out = md("<ul><li>one</li><li>two</li></ul>");
        assert!(out.contains("- one"), "got: {out}");
        assert!(out.contains("- two"), "got: {out}");
    }

    #[test]
    fn ordered_list() {
        let out = md("<ol><li>first</li><li>second</li></ol>");
        assert!(out.contains("1. first"), "got: {out}");
        assert!(out.contains("2. second"), "got: {out}");
    }

    #[test]
    fn blockquote() {
        let out = md("<blockquote><p>quoted</p></blockquote>");
        assert!(out.contains("> quoted"), "got: {out}");
    }

    #[test]
    fn image() {
        let out = md(r#"<p><img src="/a.png" alt="cat"></p>"#);
        assert!(out.contains("![cat](/a.png)"), "got: {out}");
    }

    #[test]
    fn thematic_break() {
        assert!(md("<p>a</p><hr><p>b</p>").contains("---"));
    }

    #[test]
    fn script_and_style_are_skipped() {
        let out = md("<p>keep</p><script>var x=1</script><style>.a{}</style>");
        assert!(out.contains("keep"));
        assert!(!out.contains("var x"), "got: {out}");
        assert!(!out.contains(".a{}"), "got: {out}");
    }

    #[test]
    fn nested_containers_flatten() {
        let out = md("<div><section><h2>T</h2><p>body</p></section></div>");
        assert!(out.contains("## T"), "got: {out}");
        assert!(out.contains("body"), "got: {out}");
    }

    #[test]
    fn blocks_separated_by_blank_line() {
        let out = md("<h1>T</h1><p>p1</p><p>p2</p>");
        assert!(out.contains("# T\n\np1\n\np2"), "got: {out:?}");
    }

    #[test]
    fn empty_document_is_empty() {
        assert_eq!(md("<html><body></body></html>").trim(), "");
    }

    #[test]
    fn ast_is_typed_and_reusable() {
        let ast = document_to_markdown(&Document::parse("<h1>Hi</h1>"));
        assert_eq!(
            ast.blocks,
            vec![Block::Heading {
                level: 1,
                inlines: vec![Inline::Text("Hi".to_string())]
            }]
        );
    }
}
