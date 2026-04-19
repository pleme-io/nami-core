//! Demo: author DOM transforms in Lisp, apply them to a real HTML document.
//!
//! Run: `cargo run --example dom_transforms --features lisp`

use nami_core::dom::Document;
use nami_core::transform;

const HTML: &str = r#"
<html>
  <body>
    <header><h1>headline</h1></header>
    <aside class="ad">buy stuff</aside>
    <article>
      <p>lorem ipsum</p>
      <img src="hero.png">
      <img src="sponsor.png" class="ad">
      <a href="http://legacy.example.com/foo">link</a>
    </article>
    <footer>© 2026</footer>
  </body>
</html>
"#;

const TRANSFORMS: &str = r#"
; Strip ad-class elements entirely.
(defdom-transform :name "hide-ads"
                  :selector ".ad"
                  :action remove
                  :description "drop anything classed .ad")

; Tag unlabeled images for downstream accessibility scoring.
(defdom-transform :name "flag-images"
                  :selector "img"
                  :action add-class
                  :arg "needs-alt-review")

; Upgrade legacy http links to https.
(defdom-transform :name "upgrade-link"
                  :selector "a"
                  :action set-attr
                  :arg "href=https://example.com/migrated")
"#;

fn main() {
    transform::register();

    let specs = transform::compile(TRANSFORMS).expect("compile transforms");
    println!("=== {} transforms compiled from Lisp ===", specs.len());
    for s in &specs {
        println!(
            "  {:<14} {} → {:?}{}",
            s.name,
            s.selector,
            s.action,
            s.arg
                .as_ref()
                .map(|a| format!(" ({a})"))
                .unwrap_or_default()
        );
    }

    let mut doc = Document::parse(HTML);

    println!("\n=== before ===");
    println!("{}", doc.text_content().trim());

    let report = transform::apply(&mut doc, &specs);

    println!("\n=== applied {} transforms ===", report.applied.len());
    for hit in &report.applied {
        println!("  {:<14} {:?} <{}>", hit.transform, hit.action, hit.tag);
    }

    println!("\n=== after ===");
    println!("{}", doc.text_content().trim());
}
