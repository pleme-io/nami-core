//! Regression: `<script>` / `<style>` source must never paint into the page.
//!
//! Operator report 2026-08-09: namimado on https://www.google.com painted
//! Google's minified JS (`(function(){var _g={kEI:'…'`) as body text. This
//! reproduces the class against the real pipeline — parse → cascade → layout →
//! display list — so the failure is located rather than guessed at.

use nami_core::css::cascade::StyleResolver;
use nami_core::dom::Document;
use nami_core::layout::{LayoutEngine, Size};
use nami_core::paint::{self, DrawCmd};

/// Every string a `DrawCmd::Text` would paint.
fn painted_text(html: &str) -> Vec<String> {
    let doc = Document::parse(html);
    let resolver = StyleResolver::new();
    let styled = resolver.resolve(&doc);
    let mut engine = LayoutEngine::new();
    let layout = engine.compute(&styled, Size::new(1280.0, 800.0));
    let list = paint::build_display_list(&layout, &styled, &doc);
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn script_source_never_reaches_the_display_list() {
    let html = r#"<!DOCTYPE html><html><head>
<script nonce="abc">(function(){var _g={kEI:'SECRET_JS_MARKER'};})();</script>
</head><body><p>VISIBLE_MARKER</p></body></html>"#;

    let painted = painted_text(html);
    let joined = painted.join(" | ");
    assert!(
        joined.contains("VISIBLE_MARKER"),
        "the real paragraph must paint, else the test proves nothing: {joined}"
    );
    assert!(
        !joined.contains("SECRET_JS_MARKER"),
        "script source painted into the page: {joined}"
    );
}

/// The same, with the script in `<body>` — where Google puts most of theirs,
/// and where the parser's insertion mode differs from `<head>`.
#[test]
fn script_in_body_never_reaches_the_display_list() {
    let html = r#"<!DOCTYPE html><html><body>
<p>VISIBLE_MARKER</p>
<script>var x = "BODY_JS_MARKER"; function f(){return 1;}</script>
</body></html>"#;

    let joined = painted_text(html).join(" | ");
    assert!(joined.contains("VISIBLE_MARKER"), "control: {joined}");
    assert!(
        !joined.contains("BODY_JS_MARKER"),
        "body script source painted into the page: {joined}"
    );
}

#[test]
fn style_source_never_reaches_the_display_list() {
    let html = r#"<!DOCTYPE html><html><head>
<style>.cls { color: red; } /* STYLE_MARKER */</style>
</head><body><p>VISIBLE_MARKER</p></body></html>"#;

    let joined = painted_text(html).join(" | ");
    assert!(joined.contains("VISIBLE_MARKER"), "control: {joined}");
    assert!(
        !joined.contains("STYLE_MARKER"),
        "style source painted into the page: {joined}"
    );
}

/// Google-shaped: no `<head>`/`<body>` tags authored, script immediately after
/// a bare text node. html5ever must still nest the source under `<script>`.
#[test]
fn implicit_body_with_bare_script_does_not_leak() {
    let html = r#"<!DOCTYPE html><html><head><title>Google</title></head><body>Google<script nonce="x">(function(){var _g={kEI:'IMPLICIT_MARKER'};})();</script></body></html>"#;

    let joined = painted_text(html).join(" | ");
    assert!(
        !joined.contains("IMPLICIT_MARKER"),
        "script source painted into the page: {joined}"
    );
}

/// `<title>` is metadata — it names the tab, and must not paint into the page.
#[test]
fn title_text_never_reaches_the_display_list() {
    let html = r#"<!DOCTYPE html><html><head><title>TITLE_MARKER</title></head>
<body><p>VISIBLE_MARKER</p></body></html>"#;

    let joined = painted_text(html).join(" | ");
    assert!(joined.contains("VISIBLE_MARKER"), "control: {joined}");
    assert!(
        !joined.contains("TITLE_MARKER"),
        "title text painted into the page body: {joined}"
    );
}

// ── With AUTHOR CSS, the way namimado actually renders ──────────────
//
// namimado's NamiNativeEngine collects every <style> element's text and
// `add_sheet`s it before resolving. The cascade applies the UA sheet FIRST so
// author rules win — which is correct for `div{display:inline}`, but means a
// mis-matching selector could override `script{display:none}` and leak source.
// nami-core's own docs note "Specificity is not yet fully implemented", so this
// is the difference between the passing tests above and the live failure.

fn painted_text_with_author_css(html: &str) -> Vec<String> {
    use nami_core::css::StyleSheet;
    let doc = Document::parse(html);
    let mut resolver = StyleResolver::new();
    // Mirror namimado: collect <style> text, parse, add as an author sheet.
    let css = collect_style_text_test(&doc.root);
    if !css.trim().is_empty() {
        if let Ok(sheet) = StyleSheet::parse(&css) {
            resolver.add_sheet(sheet);
        }
    }
    let styled = resolver.resolve(&doc);
    let mut engine = LayoutEngine::new();
    let layout = engine.compute(&styled, Size::new(1280.0, 800.0));
    paint::build_display_list(&layout, &styled, &doc)
        .cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn collect_style_text_test(node: &nami_core::dom::Node) -> String {
    let mut out = String::new();
    fn walk(node: &nami_core::dom::Node, out: &mut String) {
        if let Some(el) = node.as_element() {
            if el.tag == "style" {
                out.push_str(&node.text_content());
                out.push('\n');
                return;
            }
        }
        for c in &node.children {
            walk(c, out);
        }
    }
    walk(node, &mut out);
    out
}

/// Google-shaped author CSS — universal + grouped + attribute selectors, the
/// kinds most likely to over-match in a partial selector engine.
#[test]
fn author_css_must_not_override_script_display_none() {
    let html = r#"<!DOCTYPE html><html><head>
<style>*{box-sizing:border-box}html,body{margin:0;font:13px arial}a{color:#1a0dab}
div,span{display:block}[data-x]{display:block}</style>
<script nonce="q">(function(){var _g={kEI:'AUTHORCSS_MARKER'};})();</script>
</head><body><p>VISIBLE_MARKER</p></body></html>"#;

    let joined = painted_text_with_author_css(html).join(" | ");
    assert!(joined.contains("VISIBLE_MARKER"), "control: {joined}");
    assert!(
        !joined.contains("AUTHORCSS_MARKER"),
        "author CSS let script source paint: {joined}"
    );
}

/// The <style> text itself must not paint even when it IS parsed as a sheet.
#[test]
fn style_text_does_not_paint_even_when_parsed_as_author_css() {
    let html = r#"<!DOCTYPE html><html><head>
<style>.a{color:red}/*STYLETEXT_MARKER*/</style></head>
<body><p>VISIBLE_MARKER</p></body></html>"#;

    let joined = painted_text_with_author_css(html).join(" | ");
    assert!(joined.contains("VISIBLE_MARKER"), "control: {joined}");
    assert!(
        !joined.contains("STYLETEXT_MARKER"),
        "style source painted: {joined}"
    );
}

/// THE REAL PAGE. Synthetic HTML passes every assertion above, so the failure
/// lives in something only a real document has. Point `NAMI_HTML_FIXTURE` at a
/// saved page and this reports exactly what would paint.
///
/// Ignored by default: it needs a fixture on disk, so as a normal test it would
/// pass or fail for reasons unrelated to this code.
///
/// ```text
/// curl -sL https://www.google.com -o /tmp/google.html
/// NAMI_HTML_FIXTURE=/tmp/google.html cargo test --test script_text_never_paints \
///   real_page -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs NAMI_HTML_FIXTURE=<path> to a saved page"]
fn real_page_does_not_leak_script_source() {
    let Ok(path) = std::env::var("NAMI_HTML_FIXTURE") else {
        panic!("set NAMI_HTML_FIXTURE=<path>");
    };
    let html = std::fs::read_to_string(&path).expect("read fixture");
    println!("fixture {path}: {} bytes", html.len());

    let painted = painted_text_with_author_css(&html);
    println!("painted text commands: {}", painted.len());

    // Anything that looks like source rather than prose.
    let smells = ["function(", "var ", "kEI", "=function", "){", ";}"];
    let mut leaked: Vec<(usize, &String)> = Vec::new();
    for (i, t) in painted.iter().enumerate() {
        if smells.iter().any(|s| t.contains(s)) {
            leaked.push((i, t));
        }
    }
    for (i, t) in leaked.iter().take(5) {
        let head: String = t.chars().take(160).collect();
        println!("LEAK[{i}]: {head}");
    }
    println!("total leaking commands: {}", leaked.len());
    for (i, t) in painted.iter().take(8).enumerate() {
        let head: String = t.chars().take(100).collect();
        println!("  painted[{i}]: {head}");
    }
    assert!(
        leaked.is_empty(),
        "{} painted commands look like JS source",
        leaked.len()
    );
}

/// DIAGNOSTIC: `build_display_list` maps a layout box to its DOM node by
/// `node_index`, assuming the cascade numbers nodes in the SAME preorder that
/// `descendants()` walks. If those ever diverge, a visible box adopts ANOTHER
/// node's text — which is exactly what a script-source leak looks like. This
/// finds the first index where the two disagree.
#[test]
#[ignore = "needs NAMI_HTML_FIXTURE=<path>"]
fn styled_and_dom_indices_agree_on_a_real_page() {
    use nami_core::css::cascade::StyledNode;

    let path = std::env::var("NAMI_HTML_FIXTURE").expect("set NAMI_HTML_FIXTURE");
    let html = std::fs::read_to_string(&path).expect("read fixture");
    let doc = Document::parse(&html);
    let styled = StyleResolver::new().resolve(&doc);

    let dom: Vec<&nami_core::dom::Node> = doc.root.descendants().collect();

    fn flatten<'a>(n: &'a StyledNode, out: &mut Vec<&'a StyledNode>) {
        out.push(n);
        for c in &n.children {
            flatten(c, out);
        }
    }
    let mut styled_nodes: Vec<&StyledNode> = Vec::new();
    flatten(&styled.root, &mut styled_nodes);

    println!("dom nodes: {} | styled nodes: {}", dom.len(), styled_nodes.len());

    let dom_tag = |n: &nami_core::dom::Node| -> String {
        match n.as_element() {
            Some(el) => el.tag.clone(),
            None if n.as_text().is_some() => "#text".into(),
            None => "#other".into(),
        }
    };

    let mut divergences = 0usize;
    for s in &styled_nodes {
        let Some(d) = dom.get(s.node_index) else {
            println!("node_index {} is OUT OF RANGE (dom len {})", s.node_index, dom.len());
            divergences += 1;
            continue;
        };
        let dt = dom_tag(d);
        let st = &s.tag;
        let same = st == &dt
            || (st == "#text" && dt == "#text")
            || (st == "#document" && dt == "#other")
            || (st == "#comment" && dt == "#other");
        if !same {
            if divergences < 10 {
                println!("MISMATCH at node_index {}: styled={st:?} dom={dt:?}", s.node_index);
            }
            divergences += 1;
        }
    }
    println!("total divergences: {divergences}");
    assert_eq!(divergences, 0, "styled/dom index mapping diverges");
}

/// DIAGNOSTIC: what `display` does the cascade actually compute for the real
/// page's <script> elements, and do they carry text?
#[test]
#[ignore = "needs NAMI_HTML_FIXTURE=<path>"]
fn script_nodes_compute_display_none_on_a_real_page() {
    use nami_core::css::cascade::StyledNode;
    use nami_core::css::values::Display;

    let path = std::env::var("NAMI_HTML_FIXTURE").expect("set NAMI_HTML_FIXTURE");
    let html = std::fs::read_to_string(&path).expect("read fixture");
    let doc = Document::parse(&html);
    let styled = StyleResolver::new().resolve(&doc);

    fn flatten<'a>(n: &'a StyledNode, out: &mut Vec<&'a StyledNode>) {
        out.push(n);
        for c in &n.children {
            flatten(c, out);
        }
    }
    let mut all: Vec<&StyledNode> = Vec::new();
    flatten(&styled.root, &mut all);

    let mut bad = 0usize;
    for s in &all {
        if s.tag == "script" || s.tag == "style" || s.tag == "title" {
            let d = s.style.display();
            if d != Display::None {
                if bad < 8 {
                    println!("NOT-NONE: <{}> idx={} display={:?}", s.tag, s.node_index, d);
                }
                bad += 1;
            }
        }
    }
    let n_script = all.iter().filter(|s| s.tag == "script").count();
    println!("script nodes: {n_script} | non-display-none among script/style/title: {bad}");

    // And: where does the JS TEXT actually live? Print the parent tag of every
    // text node whose content smells like source.
    let dom: Vec<&nami_core::dom::Node> = doc.root.descendants().collect();
    let mut homes: std::collections::BTreeMap<String, usize> = Default::default();
    fn walk(n: &nami_core::dom::Node, parent: &str, homes: &mut std::collections::BTreeMap<String, usize>) {
        if let Some(t) = n.as_text() {
            if t.contains("function(") || t.contains("kEXPI") {
                *homes.entry(parent.to_string()).or_insert(0) += 1;
            }
        }
        let tag = n.as_element().map(|e| e.tag.as_str()).unwrap_or(parent);
        for c in &n.children {
            walk(c, tag, homes);
        }
    }
    walk(&doc.root, "#root", &mut homes);
    println!("JS-looking text nodes live under: {homes:?}");
    let _ = dom;
    assert_eq!(bad, 0, "script/style/title did not compute display:none");
}

/// THE HYPOTHESIS: `display` is not an inherited property, so the TEXT NODE
/// child of a `display:none` <script> computes a default display. The paint
/// guard checks a node's OWN display, so the element is skipped while its text
/// child is not — and `display:none` must suppress the whole SUBTREE.
#[test]
#[ignore = "needs NAMI_HTML_FIXTURE=<path>"]
fn text_children_of_display_none_also_compute_none() {
    use nami_core::css::cascade::StyledNode;
    use nami_core::css::values::Display;

    let path = std::env::var("NAMI_HTML_FIXTURE").expect("set NAMI_HTML_FIXTURE");
    let html = std::fs::read_to_string(&path).expect("read fixture");
    let doc = Document::parse(&html);
    let styled = StyleResolver::new().resolve(&doc);

    // Walk the styled tree carrying the parent's tag + display.
    fn walk(n: &StyledNode, parent: Option<(&str, Display)>, bad: &mut Vec<(String, String, Display)>) {
        if let Some((ptag, pdisp)) = parent {
            if pdisp == Display::None && n.style.display() != Display::None {
                bad.push((ptag.to_string(), n.tag.clone(), n.style.display()));
            }
        }
        let me = (n.tag.as_str(), n.style.display());
        for c in &n.children {
            walk(c, Some(me), bad);
        }
    }
    let mut bad = Vec::new();
    walk(&styled.root, None, &mut bad);

    for (ptag, ctag, d) in bad.iter().take(10) {
        println!("SUBTREE LEAK: <{ptag} display:none> child <{ctag}> display={d:?}");
    }
    println!("children of display:none that are NOT none: {}", bad.len());
    assert!(
        bad.is_empty(),
        "{} children of display:none elements do not inherit suppression",
        bad.len()
    );
}

/// DIAGNOSTIC: for each leaking painted string, find the DOM node whose direct
/// text it is, and report that node's tag + its styled node's display. Answers
/// "who painted this?" without duplicating paint's logic.
#[test]
#[ignore = "needs NAMI_HTML_FIXTURE=<path>"]
fn attribute_each_leak_to_its_node() {
    use nami_core::css::cascade::StyledNode;

    let path = std::env::var("NAMI_HTML_FIXTURE").expect("set NAMI_HTML_FIXTURE");
    let html = std::fs::read_to_string(&path).expect("read fixture");
    let doc = Document::parse(&html);
    let styled_tree = StyleResolver::new().resolve(&doc);

    let dom: Vec<&nami_core::dom::Node> = doc.root.descendants().collect();
    fn flatten<'a>(n: &'a StyledNode, out: &mut Vec<&'a StyledNode>) {
        out.push(n);
        for c in &n.children {
            flatten(c, out);
        }
    }
    let mut styled_nodes: Vec<&StyledNode> = Vec::new();
    flatten(&styled_tree.root, &mut styled_nodes);

    let direct_text = |n: &nami_core::dom::Node| -> String {
        let mut s = String::new();
        for c in &n.children {
            if let Some(t) = c.as_text() {
                s.push_str(t);
            }
        }
        s
    };

    let painted = painted_text_with_author_css(&html);
    let leaks: Vec<&String> = painted
        .iter()
        .filter(|t| t.contains("function(") || t.contains("@font-face") || t.contains("kEXPI"))
        .collect();
    println!("leaking commands: {}", leaks.len());

    for leak in leaks.iter().take(6) {
        let mut found = false;
        for (i, n) in dom.iter().enumerate() {
            if &direct_text(n) == *leak {
                let tag = n.as_element().map(|e| e.tag.clone()).unwrap_or_else(|| {
                    if n.as_text().is_some() { "#text".into() } else { "#other".into() }
                });
                let sd = styled_nodes
                    .iter()
                    .find(|s| s.node_index == i)
                    .map(|s| format!("{:?} tag={}", s.style.display(), s.tag));
                println!("  leak owned by dom[{i}] <{tag}> styled={sd:?}");
                found = true;
                break;
            }
        }
        if !found {
            let head: String = leak.chars().take(60).collect();
            println!("  leak NOT the direct_text of any node: {head}");
        }
    }
}

/// DIAGNOSTIC: walk the real LAYOUT tree and check each box's node_index/tag
/// against the styled node paint would look up. `extract_layout` zips taffy
/// children with styled children POSITIONALLY, so any length/order difference
/// pairs every later child with the wrong styled node.
#[test]
#[ignore = "needs NAMI_HTML_FIXTURE=<path>"]
fn layout_boxes_agree_with_styled_nodes() {
    use nami_core::css::cascade::StyledNode;
    use nami_core::layout::LayoutBox;

    let path = std::env::var("NAMI_HTML_FIXTURE").expect("set NAMI_HTML_FIXTURE");
    let html = std::fs::read_to_string(&path).expect("read fixture");
    let doc = Document::parse(&html);
    let styled_tree = StyleResolver::new().resolve(&doc);
    let mut engine = LayoutEngine::new();
    let layout = engine.compute(&styled_tree, Size::new(1280.0, 800.0));

    let mut styled_nodes: Vec<&StyledNode> = Vec::new();
    fn flatten<'a>(n: &'a StyledNode, out: &mut Vec<&'a StyledNode>) {
        out.push(n);
        for c in &n.children {
            flatten(c, out);
        }
    }
    flatten(&styled_tree.root, &mut styled_nodes);

    let mut boxes = 0usize;
    let mut mismatches = 0usize;
    fn walk(
        b: &LayoutBox,
        styled_nodes: &[&StyledNode],
        boxes: &mut usize,
        mismatches: &mut usize,
    ) {
        *boxes += 1;
        let s = styled_nodes.iter().find(|s| s.node_index == b.node_index);
        match s {
            Some(s) if s.tag != b.tag => {
                if *mismatches < 8 {
                    println!(
                        "BOX/STYLED MISMATCH idx={} box.tag={:?} styled.tag={:?}",
                        b.node_index, b.tag, s.tag
                    );
                }
                *mismatches += 1;
            }
            None => {
                if *mismatches < 8 {
                    println!("BOX idx={} tag={:?} has NO styled node", b.node_index, b.tag);
                }
                *mismatches += 1;
            }
            _ => {}
        }
        for c in &b.children {
            walk(c, styled_nodes, boxes, mismatches);
        }
    }
    walk(&layout.root, &styled_nodes, &mut boxes, &mut mismatches);
    println!("layout boxes: {boxes} | styled nodes: {} | mismatches: {mismatches}", styled_nodes.len());

    // And: is there a box for a script/style element at all?
    let mut script_boxes = 0usize;
    fn count_script(b: &LayoutBox, n: &mut usize) {
        if b.tag == "script" || b.tag == "style" {
            *n += 1;
        }
        for c in &b.children {
            count_script(c, n);
        }
    }
    count_script(&layout.root, &mut script_boxes);
    println!("script/style boxes present in layout: {script_boxes}");
    assert_eq!(mismatches, 0, "layout/styled pairing diverges");
}

/// The gap in my own earlier diagnostics: computed display was checked WITHOUT
/// author CSS, while the leak was measured WITH it. This checks display with the
/// real page's own stylesheets applied, exactly as namimado renders.
#[test]
#[ignore = "needs NAMI_HTML_FIXTURE=<path>"]
fn script_display_none_survives_real_author_css() {
    use nami_core::css::cascade::StyledNode;
    use nami_core::css::values::Display;
    use nami_core::css::StyleSheet;

    let path = std::env::var("NAMI_HTML_FIXTURE").expect("set NAMI_HTML_FIXTURE");
    let html = std::fs::read_to_string(&path).expect("read fixture");
    let doc = Document::parse(&html);

    let mut resolver = StyleResolver::new();
    let css = collect_style_text_test(&doc.root);
    println!("author css bytes: {}", css.len());
    match StyleSheet::parse(&css) {
        Ok(sheet) => {
            println!("author rules parsed: {}", sheet.rules.len());
            resolver.add_sheet(sheet);
        }
        Err(e) => println!("author css FAILED to parse: {e}"),
    }
    let styled = resolver.resolve(&doc);

    fn flatten<'a>(n: &'a StyledNode, out: &mut Vec<&'a StyledNode>) {
        out.push(n);
        for c in &n.children {
            flatten(c, out);
        }
    }
    let mut all: Vec<&StyledNode> = Vec::new();
    flatten(&styled.root, &mut all);

    let mut bad = 0usize;
    for s in &all {
        if matches!(s.tag.as_str(), "script" | "style" | "title") && s.style.display() != Display::None {
            if bad < 6 {
                println!("NOT-NONE with author css: <{}> idx={} display={:?}", s.tag, s.node_index, s.style.display());
            }
            bad += 1;
        }
    }
    // And the children, which is what actually paints.
    let mut bad_children = 0usize;
    fn walk(n: &StyledNode, parent_none: bool, bad: &mut usize) {
        if parent_none && n.style.display() != Display::None {
            *bad += 1;
        }
        let me_none = n.style.display() == Display::None;
        for c in &n.children {
            walk(c, parent_none || me_none, bad);
        }
    }
    walk(&styled.root, false, &mut bad_children);
    println!("script/style/title NOT display:none (author css applied): {bad}");
    println!("descendants of display:none still renderable: {bad_children}");
    assert_eq!(bad, 0, "author CSS re-enabled non-rendered elements");
    assert_eq!(bad_children, 0, "suppression did not reach descendants");
}

/// DIAGNOSTIC: which author rules match a <script> element? Google's CSS does
/// not target scripts, so any match is the selector engine OVER-MATCHING.
#[test]
#[ignore = "needs NAMI_HTML_FIXTURE=<path>"]
fn which_author_rules_match_a_script_element() {
    use nami_core::css::StyleSheet;

    let path = std::env::var("NAMI_HTML_FIXTURE").expect("set NAMI_HTML_FIXTURE");
    let html = std::fs::read_to_string(&path).expect("read fixture");
    let doc = Document::parse(&html);
    let css = collect_style_text_test(&doc.root);
    let sheet = StyleSheet::parse(&css).expect("parse author css");

    // Find a <script> element in the DOM.
    let script = doc
        .root
        .descendants()
        .find(|n| n.as_element().map(|e| e.tag == "script").unwrap_or(false))
        .and_then(|n| n.as_element())
        .expect("a script element");

    let mut matched = 0usize;
    for (i, rule) in sheet.rules.iter().enumerate() {
        if rule.compounds.iter().any(|c| c.matches(script)) {
            let decls: Vec<String> = rule
                .declarations
                .iter()
                .map(|d| format!("{}:{}", d.property, d.value))
                .collect();
            if matched < 10 {
                println!("rule[{i}] MATCHES script — compounds={:?} decls={:?}",
                    rule.compounds.len(), decls);
            }
            matched += 1;
        }
    }
    println!("author rules matching <script>: {matched} of {}", sheet.rules.len());
}
