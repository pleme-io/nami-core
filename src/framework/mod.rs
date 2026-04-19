//! Framework detection — fingerprint the renderer from the DOM.
//!
//! The goal is "normalize the web into Lisp space" — step one is
//! knowing what produced a page so scrapes + transforms can target
//! framework-specific signals sensibly.
//!
//! Runs a collection of fingerprinters over the parsed DOM and returns
//! a [`Vec<Detection>`] with evidence. Multiple detections can coexist
//! on one page (a Next.js app using htmx is plausible).
//!
//! Nothing here mutates the document; pure pattern matching over tags,
//! attributes, class patterns, and script contents.

use crate::dom::{Document, ElementData, NodeData};
use serde::Serialize;

/// A framework or rendering system we can detect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Framework {
    React,
    NextJs,
    Remix,
    Gatsby,
    Vue,
    Nuxt,
    Svelte,
    SvelteKit,
    Angular,
    Astro,
    Solid,
    Htmx,
    Alpine,
    Tailwind,
    ShadcnRadix,
    Bootstrap,
    Materialize,
    Wordpress,
    Shopify,
    GoogleTagManager,
    JqueryFallback,
}

impl Framework {
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::React => "react",
            Self::NextJs => "next.js",
            Self::Remix => "remix",
            Self::Gatsby => "gatsby",
            Self::Vue => "vue",
            Self::Nuxt => "nuxt",
            Self::Svelte => "svelte",
            Self::SvelteKit => "sveltekit",
            Self::Angular => "angular",
            Self::Astro => "astro",
            Self::Solid => "solid",
            Self::Htmx => "htmx",
            Self::Alpine => "alpine.js",
            Self::Tailwind => "tailwind",
            Self::ShadcnRadix => "shadcn/radix",
            Self::Bootstrap => "bootstrap",
            Self::Materialize => "materialize",
            Self::Wordpress => "wordpress",
            Self::Shopify => "shopify",
            Self::GoogleTagManager => "gtm",
            Self::JqueryFallback => "jquery",
        }
    }
}

/// One detection from one framework fingerprinter.
#[derive(Debug, Clone, Serialize)]
pub struct Detection {
    pub framework: Framework,
    pub name: &'static str,
    /// 0.0–1.0 — rough confidence. Multiple independent evidence
    /// lines bump it higher, a single weak signal stays low.
    pub confidence: f32,
    /// Human-readable trail of what triggered the match.
    pub evidence: Vec<String>,
}

/// Run every fingerprinter and return detections sorted by confidence
/// (highest first).
#[must_use]
pub fn detect(doc: &Document) -> Vec<Detection> {
    let mut out = Vec::new();
    out.extend(detect_react(doc));
    out.extend(detect_nextjs(doc));
    out.extend(detect_remix(doc));
    out.extend(detect_gatsby(doc));
    out.extend(detect_vue(doc));
    out.extend(detect_nuxt(doc));
    out.extend(detect_svelte(doc));
    out.extend(detect_sveltekit(doc));
    out.extend(detect_angular(doc));
    out.extend(detect_astro(doc));
    out.extend(detect_solid(doc));
    out.extend(detect_htmx(doc));
    out.extend(detect_alpine(doc));
    out.extend(detect_tailwind(doc));
    out.extend(detect_shadcn(doc));
    out.extend(detect_bootstrap(doc));
    out.extend(detect_wordpress(doc));
    out.extend(detect_shopify(doc));
    out.extend(detect_gtm(doc));
    out.extend(detect_jquery(doc));

    out.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

// ── fingerprinters ────────────────────────────────────────────────

fn detect_react(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;

    if any_attr_match(doc, |k, _| k == "data-reactroot") {
        ev.push("element with data-reactroot".into());
        conf += 0.6;
    }
    if any_attr_match(doc, |k, _| k.starts_with("data-reactid")) {
        ev.push("data-reactid on element".into());
        conf += 0.2;
    }
    if any_attr_match(doc, |k, _| k == "data-react-helmet") {
        ev.push("react-helmet head management".into());
        conf += 0.2;
    }
    if script_contents_contain(doc, "__REACT_DEVTOOLS_GLOBAL_HOOK__") {
        ev.push("React devtools hook in script".into());
        conf += 0.3;
    }

    finalize(Framework::React, "react", conf, ev)
}

fn detect_nextjs(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if any_attr_match(doc, |k, v| k == "id" && v == "__next") {
        ev.push("<div id=__next>".into());
        conf += 0.6;
    }
    if script_with_id(doc, "__NEXT_DATA__") {
        ev.push("<script id=__NEXT_DATA__>".into());
        conf += 0.5;
    }
    if script_contents_contain(doc, "_next/static") {
        ev.push("_next/static asset reference".into());
        conf += 0.2;
    }
    finalize(Framework::NextJs, "next.js", conf, ev)
}

fn detect_remix(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if script_contents_contain(doc, "__remixContext") {
        ev.push("window.__remixContext in script".into());
        conf += 0.6;
    }
    if any_attr_match(doc, |k, _| k.starts_with("data-remix-")) {
        ev.push("data-remix-* attribute".into());
        conf += 0.3;
    }
    finalize(Framework::Remix, "remix", conf, ev)
}

fn detect_gatsby(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if any_attr_match(doc, |k, v| k == "id" && v == "___gatsby") {
        ev.push("#___gatsby root".into());
        conf += 0.7;
    }
    if script_contents_contain(doc, "window.___gatsby") {
        ev.push("window.___gatsby in script".into());
        conf += 0.2;
    }
    finalize(Framework::Gatsby, "gatsby", conf, ev)
}

fn detect_vue(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if any_attr_match(doc, |k, _| k.starts_with("data-v-")) {
        ev.push("data-v-* scoped-css attrs".into());
        conf += 0.4;
    }
    if any_attr_match(doc, |k, _| {
        matches!(k, "v-if" | "v-for" | "v-model" | "v-show")
    }) {
        ev.push("v-* directive attr".into());
        conf += 0.4;
    }
    finalize(Framework::Vue, "vue", conf, ev)
}

fn detect_nuxt(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if any_attr_match(doc, |k, v| k == "id" && v == "__nuxt") {
        ev.push("#__nuxt root".into());
        conf += 0.6;
    }
    if script_with_id(doc, "__NUXT_DATA__") {
        ev.push("<script id=__NUXT_DATA__>".into());
        conf += 0.3;
    }
    finalize(Framework::Nuxt, "nuxt", conf, ev)
}

fn detect_svelte(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    let mut hash_classes = 0usize;
    for_each_element(doc, |el| {
        if let Some(cls) = el.get_attribute("class") {
            for c in cls.split_whitespace() {
                if c.starts_with("svelte-") && c.len() > 10 {
                    hash_classes += 1;
                }
            }
        }
    });
    if hash_classes > 0 {
        ev.push(format!("{hash_classes} svelte-xxxxx hashed class(es)"));
        conf += (hash_classes.min(5) as f32) * 0.15;
    }
    finalize(Framework::Svelte, "svelte", conf, ev)
}

fn detect_sveltekit(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if any_attr_match(doc, |k, _| k.starts_with("data-sveltekit-")) {
        ev.push("data-sveltekit-* attribute".into());
        conf += 0.6;
    }
    if script_contents_contain(doc, "__sveltekit_") {
        ev.push("__sveltekit_ in script".into());
        conf += 0.2;
    }
    finalize(Framework::SvelteKit, "sveltekit", conf, ev)
}

fn detect_angular(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if any_attr_match(doc, |k, _| k == "ng-version") {
        ev.push("ng-version attribute".into());
        conf += 0.7;
    }
    if any_attr_match(doc, |k, _| {
        k.starts_with("_ngcontent-") || k.starts_with("_nghost-")
    }) {
        ev.push("_ngcontent/_nghost view encapsulation attrs".into());
        conf += 0.2;
    }
    finalize(Framework::Angular, "angular", conf, ev)
}

fn detect_astro(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if any_tag_equals(doc, "astro-island") {
        ev.push("<astro-island> hydration tag".into());
        conf += 0.6;
    }
    if any_attr_match(doc, |k, _| k == "astro-script") {
        ev.push("astro-script attr".into());
        conf += 0.2;
    }
    finalize(Framework::Astro, "astro", conf, ev)
}

fn detect_solid(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if script_contents_contain(doc, "solid-js") || script_contents_contain(doc, "_$DX_DELEGATE") {
        ev.push("solid-js signal in script".into());
        conf += 0.5;
    }
    finalize(Framework::Solid, "solid", conf, ev)
}

fn detect_htmx(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    let mut count = 0usize;
    for_each_element(doc, |el| {
        for (k, _) in &el.attributes {
            if k.starts_with("hx-") {
                count += 1;
            }
        }
    });
    if count > 0 {
        ev.push(format!("{count} hx-* attribute(s)"));
        conf += (count.min(10) as f32) * 0.1;
    }
    finalize(Framework::Htmx, "htmx", conf, ev)
}

fn detect_alpine(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    let mut x_count = 0usize;
    let mut at_count = 0usize;
    for_each_element(doc, |el| {
        for (k, _) in &el.attributes {
            if k.starts_with("x-") || k.starts_with("@") {
                x_count += 1;
            }
            if k.starts_with("@") {
                at_count += 1;
            }
        }
    });
    if x_count > 0 {
        ev.push(format!("{x_count} alpine directive attr(s)"));
        conf += (x_count.min(5) as f32) * 0.15;
    }
    if at_count > 0 {
        conf += 0.1;
    }
    finalize(Framework::Alpine, "alpine.js", conf, ev)
}

fn detect_tailwind(doc: &Document) -> Option<Detection> {
    // Heuristic: many elements carry classes like `flex`, `grid`,
    // `text-*`, `bg-*`, `p-*`, `m-*`, `w-*` etc. We count sentinel
    // utility classes across the document.
    const SENTINEL: &[&str] = &[
        "flex",
        "grid",
        "inline-flex",
        "hidden",
        "block",
        "container",
    ];
    let mut count = 0usize;
    for_each_element(doc, |el| {
        if let Some(cls) = el.get_attribute("class") {
            for c in cls.split_whitespace() {
                if SENTINEL.contains(&c)
                    || c.starts_with("text-")
                    || c.starts_with("bg-")
                    || c.starts_with("p-")
                    || c.starts_with("m-")
                    || c.starts_with("w-")
                    || c.starts_with("h-")
                {
                    count += 1;
                }
            }
        }
    });
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if count >= 5 {
        ev.push(format!("{count} tailwind-style utility class usage(s)"));
        conf = ((count.min(30) as f32) / 30.0) * 0.7;
    }
    finalize(Framework::Tailwind, "tailwind", conf, ev)
}

fn detect_shadcn(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    let mut slot = 0usize;
    let mut state = 0usize;
    let mut orientation = 0usize;
    for_each_element(doc, |el| {
        for (k, _) in &el.attributes {
            match k.as_str() {
                "data-slot" => slot += 1,
                "data-state" => state += 1,
                "data-orientation" => orientation += 1,
                _ => {}
            }
        }
    });
    if slot > 0 {
        ev.push(format!("{slot} data-slot attrs (shadcn convention)"));
        conf += (slot.min(5) as f32) * 0.15;
    }
    if state > 0 && orientation > 0 {
        ev.push(format!(
            "data-state({state}) + data-orientation({orientation}) — radix pattern"
        ));
        conf += 0.2;
    }
    finalize(Framework::ShadcnRadix, "shadcn/radix", conf, ev)
}

fn detect_bootstrap(doc: &Document) -> Option<Detection> {
    let mut count = 0usize;
    for_each_element(doc, |el| {
        if let Some(cls) = el.get_attribute("class") {
            for c in cls.split_whitespace() {
                if matches!(c, "container" | "row" | "col")
                    || c.starts_with("col-")
                    || c.starts_with("btn-")
                    || c.starts_with("bs-")
                {
                    count += 1;
                }
            }
        }
    });
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if count >= 3 {
        ev.push(format!("{count} bootstrap-style class(es)"));
        conf = ((count.min(20) as f32) / 20.0) * 0.5;
    }
    finalize(Framework::Bootstrap, "bootstrap", conf, ev)
}

fn detect_wordpress(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if script_contents_contain(doc, "/wp-content/") || script_contents_contain(doc, "/wp-includes/")
    {
        ev.push("wp-content / wp-includes asset path".into());
        conf += 0.5;
    }
    if any_attr_match(doc, |k, v| k == "name" && v == "generator") && has_generator_wordpress(doc) {
        ev.push("<meta name=generator> = WordPress".into());
        conf += 0.4;
    }
    finalize(Framework::Wordpress, "wordpress", conf, ev)
}

fn detect_shopify(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if script_contents_contain(doc, "Shopify.theme")
        || script_contents_contain(doc, "cdn.shopify.com")
    {
        ev.push("Shopify.theme / cdn.shopify.com".into());
        conf += 0.6;
    }
    finalize(Framework::Shopify, "shopify", conf, ev)
}

fn detect_gtm(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if script_contents_contain(doc, "googletagmanager.com/gtm.js")
        || script_contents_contain(doc, "dataLayer = window.dataLayer")
    {
        ev.push("googletagmanager.com gtm.js snippet".into());
        conf += 0.5;
    }
    finalize(Framework::GoogleTagManager, "gtm", conf, ev)
}

fn detect_jquery(doc: &Document) -> Option<Detection> {
    let mut ev = Vec::new();
    let mut conf = 0.0;
    if script_src_contains(doc, "jquery") {
        ev.push("<script src=…jquery…>".into());
        conf += 0.4;
    }
    finalize(Framework::JqueryFallback, "jquery", conf, ev)
}

// ── helpers ──────────────────────────────────────────────────────

fn finalize(
    framework: Framework,
    name: &'static str,
    confidence: f32,
    evidence: Vec<String>,
) -> Option<Detection> {
    if confidence <= 0.0 || evidence.is_empty() {
        return None;
    }
    Some(Detection {
        framework,
        name,
        confidence: confidence.min(1.0),
        evidence,
    })
}

fn for_each_element(doc: &Document, mut f: impl FnMut(&ElementData)) {
    for node in doc.root.descendants() {
        if let NodeData::Element(el) = &node.data {
            f(el);
        }
    }
}

fn any_attr_match(doc: &Document, pred: impl Fn(&str, &str) -> bool) -> bool {
    for node in doc.root.descendants() {
        if let NodeData::Element(el) = &node.data {
            for (k, v) in &el.attributes {
                if pred(k, v) {
                    return true;
                }
            }
        }
    }
    false
}

fn any_tag_equals(doc: &Document, tag: &str) -> bool {
    for node in doc.root.descendants() {
        if let NodeData::Element(el) = &node.data {
            if el.tag.eq_ignore_ascii_case(tag) {
                return true;
            }
        }
    }
    false
}

fn script_with_id(doc: &Document, id: &str) -> bool {
    for node in doc.root.descendants() {
        if let NodeData::Element(el) = &node.data {
            if el.tag == "script" && el.get_attribute("id") == Some(id) {
                return true;
            }
        }
    }
    false
}

fn script_src_contains(doc: &Document, needle: &str) -> bool {
    for node in doc.root.descendants() {
        if let NodeData::Element(el) = &node.data {
            if el.tag == "script" {
                if let Some(src) = el.get_attribute("src") {
                    if src.contains(needle) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn script_contents_contain(doc: &Document, needle: &str) -> bool {
    for node in doc.root.descendants() {
        if let NodeData::Element(el) = &node.data {
            if el.tag == "script" && node.text_content().contains(needle) {
                return true;
            }
        }
    }
    false
}

fn has_generator_wordpress(doc: &Document) -> bool {
    for node in doc.root.descendants() {
        if let NodeData::Element(el) = &node.data {
            if el.tag == "meta"
                && el.get_attribute("name") == Some("generator")
                && el
                    .get_attribute("content")
                    .is_some_and(|c| c.to_ascii_lowercase().contains("wordpress"))
            {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Document {
        Document::parse(s)
    }

    #[test]
    fn detects_nextjs() {
        let doc = parse(
            r#"<html><body><div id="__next"><p>x</p></div><script id="__NEXT_DATA__">{}</script></body></html>"#,
        );
        let d = detect(&doc);
        assert!(d.iter().any(|x| x.framework == Framework::NextJs));
    }

    #[test]
    fn detects_htmx_by_hx_attrs() {
        let doc =
            parse(r##"<html><body><button hx-get="/a" hx-target="#o">x</button></body></html>"##);
        let d = detect(&doc);
        assert!(d.iter().any(|x| x.framework == Framework::Htmx));
    }

    #[test]
    fn detects_alpine() {
        let doc = parse(
            r#"<html><body><div x-data="{a:1}"><button @click="a++">+</button></div></body></html>"#,
        );
        let d = detect(&doc);
        assert!(d.iter().any(|x| x.framework == Framework::Alpine));
    }

    #[test]
    fn detects_tailwind_by_utility_classes() {
        let doc = parse(
            r#"<html><body><div class="flex items-center p-4 bg-white text-gray-900 w-full"><span class="text-sm m-2">x</span></div></body></html>"#,
        );
        let d = detect(&doc);
        assert!(d.iter().any(|x| x.framework == Framework::Tailwind));
    }

    #[test]
    fn detects_shadcn_by_data_slots_and_state() {
        let doc = parse(
            r#"<html><body>
                <div data-slot="card" data-state="open" data-orientation="vertical">
                    <div data-slot="card-header"></div>
                    <div data-slot="card-content"></div>
                </div>
            </body></html>"#,
        );
        let d = detect(&doc);
        assert!(d.iter().any(|x| x.framework == Framework::ShadcnRadix));
    }

    #[test]
    fn detects_angular() {
        let doc = parse(
            r#"<html><body ng-version="17.0.0"><app-root _ngcontent-abc="">x</app-root></body></html>"#,
        );
        let d = detect(&doc);
        assert!(d.iter().any(|x| x.framework == Framework::Angular));
    }

    #[test]
    fn detects_multiple_simultaneously() {
        // A Next.js + htmx + tailwind page is entirely plausible.
        let doc = parse(
            r##"<html><body>
                <div id="__next" class="flex h-full w-full bg-slate-900 text-slate-50">
                    <nav class="flex p-4 m-2 bg-blue-500">
                        <button hx-get="/api" class="flex p-2 m-1 text-white w-32">x</button>
                        <a href="#" class="p-2 m-1 text-sm">home</a>
                    </nav>
                </div>
                <script id="__NEXT_DATA__">{}</script>
            </body></html>"##,
        );
        let d = detect(&doc);
        let names: Vec<_> = d.iter().map(|x| x.framework).collect();
        assert!(names.contains(&Framework::NextJs), "got {d:?}");
        assert!(names.contains(&Framework::Htmx), "got {d:?}");
        assert!(names.contains(&Framework::Tailwind), "got {d:?}");
    }

    #[test]
    fn no_framework_plain_html_is_empty() {
        let doc = parse("<html><body><p>plain</p></body></html>");
        let d = detect(&doc);
        assert!(d.is_empty());
    }

    #[test]
    fn detections_sort_by_confidence() {
        let doc = parse(
            r#"<html><body><div id="__next"><p x-data="{}">x</p></div><script id="__NEXT_DATA__"></script></body></html>"#,
        );
        let d = detect(&doc);
        for w in d.windows(2) {
            assert!(w[0].confidence >= w[1].confidence);
        }
    }
}
