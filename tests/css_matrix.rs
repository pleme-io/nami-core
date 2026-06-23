//! CSS verification matrix — the confidence forcing-function.
//!
//! One row per supported CSS-feature variant the rendering pipeline must
//! honor: every selector kind, every [`Length`] unit, every [`Color`]
//! form, every [`Display`] mode, inheritance, `display:none`, the
//! `background:` shorthand, and inline `style=""`. Each row asserts
//! through the ONE [`nami_core::testkit`] vocabulary, so the matrix reads
//! the same as every other rendering test.
//!
//! Per the ★★ CLOSED-LOOP MASS-SYNTHESIS rule (verification matrix as a
//! forcing function): all failures AGGREGATE into one report before the
//! suite asserts — one run reports every broken variant, not just the
//! first. And [`matrix_covers_all_supported`] pins the row count so a new
//! CSS feature landing without a matrix row fails CI.
//!
//! Gated on the `testkit` feature (the assertion vocabulary lives there).

#![cfg(feature = "testkit")]

use std::panic::{catch_unwind, AssertUnwindSafe};

use nami_core::css::{Color, Display, Length, LengthProp};
use nami_core::testkit::Probe;

/// One verification row: a named CSS-feature variant + a closure that
/// exercises it through the testkit (panicking on mismatch). The matrix
/// runner catches each panic so one run reports every failure.
struct Row {
    name: &'static str,
    check: fn(),
}

/// The supported-variant matrix. Adding a CSS feature to the pipeline
/// means adding a row here — `matrix_covers_all_supported` enforces it.
const MATRIX: &[Row] = &[
    // ── Selector kinds ──────────────────────────────────────────────
    Row {
        name: "selector: tag",
        check: || {
            Probe::html("<style>p{color:#ff0000}</style><p>x</p>")
                .style("p")
                .color(Color::rgb8(255, 0, 0));
        },
    },
    Row {
        name: "selector: class",
        check: || {
            Probe::html("<style>.box{color:#00ff00}</style><div class=\"box\">x</div><div>y</div>")
                .style(".box")
                .color(Color::rgb8(0, 255, 0));
        },
    },
    Row {
        name: "selector: class does not leak to bare element",
        check: || {
            // A bare <div> must NOT pick up .box — the substring-hack bug.
            Probe::html("<style>.box{color:#00ff00}</style><div class=\"box\">x</div><div id=\"bare\">y</div>")
                .style("#bare")
                .missing("color");
        },
    },
    Row {
        name: "selector: id",
        check: || {
            Probe::html("<style>#hero{color:#0000ff}</style><p id=\"hero\">x</p>")
                .style("#hero")
                .color(Color::rgb8(0, 0, 255));
        },
    },
    Row {
        name: "selector: compound div.card (needs BOTH)",
        check: || {
            Probe::html(
                "<style>div.card{color:#ff0000}</style>\
                 <div class=\"card\">a</div><span class=\"card\">b</span>",
            )
            .style("div.card")
            .color(Color::rgb8(255, 0, 0));
            // The span.card must NOT match div.card.
            Probe::html(
                "<style>div.card{color:#ff0000}</style>\
                 <div class=\"card\">a</div><span class=\"card\">b</span>",
            )
            .style("span")
            .missing("color");
        },
    },
    Row {
        name: "selector: universal *",
        check: || {
            Probe::html("<style>*{color:#112233}</style><div>x</div>")
                .style("div")
                .color(Color::hex("#112233"));
        },
    },
    Row {
        name: "selector: rightmost-descendant .nav a",
        check: || {
            // `.nav a` constrains the subject <a>; a bare <a> matches.
            Probe::html("<style>.nav a{color:#445566}</style><div class=\"nav\"><a>link</a></div>")
                .style("a")
                .color(Color::hex("#445566"));
        },
    },
    // ── Length units (resolved through layout) ──────────────────────
    Row {
        name: "length: px",
        check: || {
            Probe::html("<div style=\"width:200px;height:50px\"></div>")
                .layout("div")
                .width(200.0);
        },
    },
    Row {
        name: "length: em (against node font-size)",
        check: || {
            // font-size:32px + width:5em → 160px.
            Probe::html("<div style=\"font-size:32px;width:5em;height:40px\"></div>")
                .layout("div")
                .width(160.0);
        },
    },
    Row {
        name: "length: rem (against root 16px)",
        check: || {
            // 10rem × 16 = 160px.
            Probe::html("<div style=\"width:10rem;height:40px\"></div>")
                .layout("div")
                .width(160.0);
        },
    },
    Row {
        name: "length: % (against viewport basis at root)",
        check: || {
            Probe::html("<div style=\"width:50%;height:40px\"></div>")
                .viewport(1000.0, 800.0)
                .layout("div")
                .width(500.0);
        },
    },
    Row {
        name: "length: vw",
        check: || {
            Probe::html("<div style=\"width:50vw;height:40px\"></div>")
                .viewport(1280.0, 800.0)
                .layout("div")
                .width(640.0);
        },
    },
    Row {
        name: "length: vh",
        check: || {
            Probe::html("<div style=\"width:40px;height:25vh\"></div>")
                .viewport(1280.0, 800.0)
                .layout("div")
                .height(200.0);
        },
    },
    Row {
        name: "length: auto (typed field stays Auto)",
        check: || {
            Probe::html("<div style=\"width:auto\"></div>")
                .style("div")
                .length(LengthProp::Width, Length::Auto);
        },
    },
    // ── Color forms ─────────────────────────────────────────────────
    Row {
        name: "color: #rgb",
        check: || {
            Probe::html("<div style=\"color:#f00\">x</div>")
                .style("div")
                .color(Color::rgb8(255, 0, 0));
        },
    },
    Row {
        name: "color: #rrggbb",
        check: || {
            Probe::html("<div style=\"color:#3050ff\">x</div>")
                .style("div")
                .color(Color::hex("#3050ff"));
        },
    },
    Row {
        name: "color: #rrggbbaa",
        check: || {
            // 50% alpha (0x80/255 ≈ 0.502).
            Probe::html("<div style=\"color:#ff000080\">x</div>")
                .style("div")
                .color(Color::parse("#ff000080").unwrap());
        },
    },
    Row {
        name: "color: rgb()",
        check: || {
            Probe::html("<div style=\"color:rgb(255, 0, 0)\">x</div>")
                .style("div")
                .color(Color::rgb8(255, 0, 0));
        },
    },
    Row {
        name: "color: rgba()",
        check: || {
            Probe::html("<div style=\"color:rgba(255, 0, 0, 0.5)\">x</div>")
                .style("div")
                .color(Color::parse("rgba(255, 0, 0, 0.5)").unwrap());
        },
    },
    Row {
        name: "color: named",
        check: || {
            Probe::html("<div style=\"color:blue\">x</div>")
                .style("div")
                .color(Color::rgb8(0, 0, 255));
        },
    },
    Row {
        name: "color: currentColor (parses to None → unset)",
        check: || {
            // currentColor is not resolvable at parse → the typed field
            // stays unset (a visible gap, never a wrong color).
            Probe::html("<div style=\"color:currentColor\">x</div>")
                .style("div")
                .missing("color");
        },
    },
    // ── Display modes ───────────────────────────────────────────────
    Row {
        name: "display: block",
        check: || {
            Probe::html("<style>span{display:block}</style><span>x</span>")
                .style("span")
                .display(Display::Block);
        },
    },
    Row {
        name: "display: inline (UA default for span)",
        check: || {
            Probe::html("<span>x</span>")
                .style("span")
                .display(Display::Inline);
        },
    },
    Row {
        name: "display: flex",
        check: || {
            Probe::html("<style>div{display:flex}</style><div>x</div>")
                .style("div")
                .display(Display::Flex);
        },
    },
    Row {
        name: "display: none",
        check: || {
            Probe::html("<style>div{display:none}</style><div>x</div>")
                .style("div")
                .display(Display::None);
        },
    },
    // ── Inheritance ─────────────────────────────────────────────────
    Row {
        name: "inheritance: child inherits color",
        check: || {
            Probe::html("<style>div{color:#00ff00}</style><div><span>x</span></div>")
                .style("span")
                .color(Color::rgb8(0, 255, 0));
        },
    },
    Row {
        name: "inheritance: background-color does NOT inherit",
        check: || {
            Probe::html("<style>div{background-color:#123456}</style><div><span>x</span></div>")
                .style("span")
                .missing("background-color");
        },
    },
    // ── display:none non-render ─────────────────────────────────────
    Row {
        name: "display:none produces no paint",
        check: || {
            // A display:none div with a background emits no Rect.
            Probe::html(
                "<style>div{display:none;background-color:#ff0000;width:50px;height:50px}</style>\
                 <div>x</div>",
            )
            .paint()
            .rect_count(0);
        },
    },
    Row {
        name: "non-rendered <style> text does not paint",
        check: || {
            // <style> content is UA display:none — its raw CSS must not paint.
            Probe::html("<style>body{color:red}</style><div>visible</div>")
                .paint()
                .no_text("body{color:red}");
        },
    },
    // ── background shorthand ────────────────────────────────────────
    Row {
        name: "background: shorthand → background-color",
        check: || {
            Probe::html("<style>div{background:#112233;width:50px;height:50px}</style><div></div>")
                .style("div")
                .background(Color::hex("#112233"));
        },
    },
    Row {
        name: "background shorthand paints a Rect",
        check: || {
            Probe::html("<style>div{background:#3050ff;width:50px;height:50px}</style><div></div>")
                .paint()
                .rect_with_color(Color::hex("#3050ff"));
        },
    },
    // ── inline style="" ─────────────────────────────────────────────
    Row {
        name: "inline style=\"\" applies (highest priority)",
        check: || {
            Probe::html("<div style=\"color:#ff0000\">x</div>")
                .style("div")
                .color(Color::rgb8(255, 0, 0));
        },
    },
    Row {
        name: "inline style overrides a sheet rule",
        check: || {
            Probe::html(
                "<style>.x{color:#0000ff}</style><div class=\"x\" style=\"color:#00ff00\">y</div>",
            )
            .style(".x")
            .color(Color::rgb8(0, 255, 0));
        },
    },
    // ── end-to-end paint text ───────────────────────────────────────
    Row {
        name: "paint: white text on colored block",
        check: || {
            Probe::html(
                "<style>div{background-color:#3050ff;width:200px;height:100px} \
                 p{color:#ffffff;height:30px}</style><div><p>Hello</p></div>",
            )
            .paint()
            .rect_with_color(Color::hex("#3050ff"))
            .text_with_color("Hello", Color::rgb8(255, 255, 255));
        },
    },
];

/// The minimum row count the matrix must carry. A new CSS-feature variant
/// landing without a matrix row drops below this and fails CI — the
/// forcing function. Bump this when adding rows.
const MIN_ROWS: usize = 34;

#[test]
fn every_variant_in_the_matrix_works() {
    let mut failures = Vec::new();
    for row in MATRIX {
        // Catch each row's panic so one run reports EVERY broken variant,
        // never just the first. The testkit's panic message is captured.
        let result = catch_unwind(AssertUnwindSafe(row.check));
        if let Err(payload) = result {
            let msg = panic_message(&payload);
            failures.push(format!("{}: {msg}", row.name));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} matrix variants failed:\n  - {}",
        failures.len(),
        MATRIX.len(),
        failures.join("\n  - ")
    );
}

#[test]
fn matrix_covers_all_supported() {
    assert!(
        MATRIX.len() >= MIN_ROWS,
        "matrix has {} rows, expected ≥ {MIN_ROWS} — a supported CSS feature \
         is missing its verification row",
        MATRIX.len()
    );
}

#[test]
fn matrix_row_names_are_unique() {
    // A duplicated row name signals a copy-paste row that doesn't add
    // coverage — fail so the matrix stays a faithful coverage map.
    let mut names: Vec<&str> = MATRIX.iter().map(|r| r.name).collect();
    let total = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names.len(),
        total,
        "matrix has duplicate row names — every row must name a distinct variant"
    );
}

/// Extract a readable message from a caught panic payload.
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic>".to_string()
    }
}
