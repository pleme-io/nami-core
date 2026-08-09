//! The swappable web-content engine seam.
//!
//! BROWSER.md's "the trait seam IS the whole strategy", made real in code.
//! Every concrete content engine — Servo (the pure-Rust product engine), wry
//! (the interim OS-WebView escape hatch), or the [`SubstrateNullEngine`]
//! (no pixels, the substrate text render shows through) — implements this one
//! [`BrowserEngine`] trait. A host (namimado's `gpu.rs`, aranami) holds a
//! `Box<dyn BrowserEngine>` chosen ONCE at construction; swapping engines is a
//! construction choice, not a `#[cfg]` forest threaded through the event loop.
//!
//! The trait lives in nami-core (the owned engine abstraction) so both
//! browsers share one seam; the engine *implementations* that link `servo` /
//! `wry` live in the consumer crate (namimado) where those deps belong —
//! nami-core itself pulls in no web-engine dependency, only the contract.
//!
//! A pixel-painting pure-Rust engine (namimado's `NamiNativeEngine`) runs
//! nami-core's own DOM → CSS → layout → [`crate::paint`] pipeline and hands
//! the resulting [`crate::paint::DisplayList`] to the host via
//! [`BrowserEngine::display_list`] for GPU rendering. That method has a
//! default returning a `&'static` empty list, so the existing
//! [`SubstrateNullEngine`] (and any engine that paints nothing or owns a
//! native subview) stays correct without overriding it.
//!
//! ## Why the per-frame read is a BORROW (a fixed defect class, 2026-08-09)
//!
//! This seam previously exposed `take_display_list(&mut self) -> DisplayList`,
//! whose doc invited the host to call it "once per frame" while the engine
//! `std::mem::take`s its cached list. Those two sentences cannot both hold: a
//! list recomputed only on `navigate`/`resize` but *consumed* every frame
//! survives exactly ONE frame. namimado shipped that pairing and the operator
//! saw the symptom — a page "renders in a flash then goes away" (a loaded
//! page painted for one frame, then a permanently blank content area, because
//! `renders_pixels() == true` also suppresses the substrate-text fallback).
//!
//! The fix is the type, not a caller convention: the per-frame accessor hands
//! out `&DisplayList`. **A shared borrow cannot clear the thing it borrows**,
//! so "the frame that ate the page" has no code path through this trait. Any
//! host may read it every frame, forever, and get the same pixels.
//!
//! Consequence for engine authors: build your list in `navigate`/`resize`,
//! keep it, and return `&self.display_list`. Do not clear on read. If you
//! need to *invalidate*, do it where the content actually changes.

use url::Url;

/// The content rectangle an engine paints into — logical pixels, positioned
/// below the chrome (address bar) and above the status bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContentRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ContentRect {
    #[must_use]
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// A typed engine failure. A `Result::Err` here is a *visible gap*, never a
/// silent wrong answer — e.g. the null engine returns [`EngineError::Unsupported`]
/// for `evaluate_js` so a caller learns "no JS host yet" mechanically.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EngineError {
    /// The engine cannot do `what` (named so the gap is self-describing).
    #[error("{engine} engine does not support {what} yet")]
    Unsupported {
        what: &'static str,
        engine: &'static str,
    },
    /// Engine construction / boot failed.
    #[error("engine init failed: {0}")]
    Init(String),
}

/// One swappable web-content engine. Object-safe by construction: the host
/// drives `Box<dyn BrowserEngine>` through this surface and nothing else.
///
/// Deliberately NOT `Send`. The host drives the engine entirely on the
/// windowing/event-loop thread (winit's `ApplicationHandler` is single-threaded
/// by design), so a `Send` bound buys nothing — and it makes whole engine
/// classes *unrepresentable*: a `wry::WebView` on macOS is main-thread-bound
/// (its Objective-C delegates hold `RefCell`/`Rc`/main-thread-only retained
/// objects, so it is `!Send`). Requiring `Send` here would forbid the wry
/// adapter entirely for no real-world benefit. If a future consumer needs to
/// hand an engine to another thread, it adds `Send` at *its own* boundary
/// (`Box<dyn BrowserEngine + Send>`) rather than the trait forcing it on every
/// engine.
pub trait BrowserEngine {
    /// Stable lowercase name for logs / the inspector (`"servo"`, `"wry"`,
    /// `"substrate-null"`).
    fn name(&self) -> &'static str;

    /// Navigate the active view to `url`.
    fn navigate(&mut self, url: &Url);

    /// The content rectangle changed (window resize / chrome relayout).
    fn resize(&mut self, rect: ContentRect);

    /// Paint one frame of content into the host surface. No-op for engines
    /// that own their own native subview (wry) or that paint nothing (null).
    fn paint(&mut self) {}

    /// Pump the engine's internal event loop one tick. No-op unless the engine
    /// drives its own loop (Servo's `spin_event_loop`). Returns `false` to ask
    /// the host to shut down.
    fn pump(&mut self) -> bool {
        true
    }

    /// Evaluate JavaScript in the active page. Defaults to a typed
    /// [`EngineError::Unsupported`] so engines without a JS host surface the
    /// gap rather than silently succeeding.
    ///
    /// # Errors
    /// Returns [`EngineError::Unsupported`] when the engine has no JS host.
    fn evaluate_js(&mut self, _script: &str) -> Result<(), EngineError> {
        Err(EngineError::Unsupported {
            what: "evaluate_js",
            engine: self.name(),
        })
    }

    /// Whether this engine paints real web pixels. `false` ⇒ the host draws
    /// its substrate-text fallback in the content rect (the null engine).
    fn renders_pixels(&self) -> bool;

    /// Borrow the engine's current [`DisplayList`](crate::paint::DisplayList)
    /// — the typed paint IR the host renders into GPU pixels.
    ///
    /// **This is the per-frame accessor, and it is deliberately a borrow.**
    /// An engine that builds a display list (the pure-Rust `NamiNativeEngine`)
    /// overrides this to return `&self.display_list`, computed on the last
    /// `navigate`/`resize`. The default returns a `&'static` empty list, so
    /// engines that paint nothing ([`SubstrateNullEngine`]) or own their own
    /// native subview (wry) stay correct without overriding.
    ///
    /// A host may call this on every frame indefinitely and always get the
    /// same content — reading cannot consume. See the module docs for the
    /// defect class this signature makes unrepresentable.
    fn display_list(&self) -> &crate::paint::DisplayList {
        &EMPTY_DISPLAY_LIST
    }
}

/// The empty list every non-painting engine borrows. A `static` (not a
/// `const`) so the borrow is genuinely `'static`: `DisplayList` owns a `Vec`
/// and therefore has drop glue, which blocks const-promotion of a temporary.
static EMPTY_DISPLAY_LIST: crate::paint::DisplayList = crate::paint::DisplayList::new();

/// The default pure-Rust engine: paints nothing, runs nothing. The host keeps
/// rendering the nami-core substrate text/inspector in the content rect. This
/// is what the pure-Rust default build uses until Servo is wired — so the
/// browser is always *driven through the seam*, never a special-cased path.
#[derive(Debug, Default, Clone, Copy)]
pub struct SubstrateNullEngine;

impl SubstrateNullEngine {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl BrowserEngine for SubstrateNullEngine {
    fn name(&self) -> &'static str {
        "substrate-null"
    }
    fn navigate(&mut self, _url: &Url) {}
    fn resize(&mut self, _rect: ContentRect) {}
    fn renders_pixels(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Records every call — the testability contract for the whole strategy:
    /// the host can be exercised end-to-end with zero real engine present.
    #[derive(Debug, Default)]
    struct MockEngine {
        log: Arc<Mutex<Vec<String>>>,
        pixels: bool,
        pump_result: bool,
    }

    impl MockEngine {
        fn new(pixels: bool) -> (Self, Arc<Mutex<Vec<String>>>) {
            let log = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    log: log.clone(),
                    pixels,
                    pump_result: true,
                },
                log,
            )
        }
    }

    impl BrowserEngine for MockEngine {
        fn name(&self) -> &'static str {
            "mock"
        }
        fn navigate(&mut self, url: &Url) {
            self.log.lock().unwrap().push(format!("navigate:{url}"));
        }
        fn resize(&mut self, rect: ContentRect) {
            self.log
                .lock()
                .unwrap()
                .push(format!("resize:{}x{}", rect.width, rect.height));
        }
        fn paint(&mut self) {
            self.log.lock().unwrap().push("paint".into());
        }
        fn pump(&mut self) -> bool {
            self.log.lock().unwrap().push("pump".into());
            self.pump_result
        }
        fn evaluate_js(&mut self, script: &str) -> Result<(), EngineError> {
            self.log.lock().unwrap().push(format!("js:{script}"));
            Ok(())
        }
        fn renders_pixels(&self) -> bool {
            self.pixels
        }
    }

    fn drive(engine: &mut dyn BrowserEngine) {
        engine.navigate(&Url::parse("https://example.com/").unwrap());
        engine.resize(ContentRect::new(0.0, 56.0, 1280.0, 712.0));
        engine.paint();
        let _ = engine.pump();
    }

    #[test]
    fn host_drives_engine_through_the_seam() {
        let (mock, log) = MockEngine::new(true);
        let mut boxed: Box<dyn BrowserEngine> = Box::new(mock);
        drive(boxed.as_mut());
        let calls = log.lock().unwrap().clone();
        assert_eq!(
            calls,
            vec![
                "navigate:https://example.com/",
                "resize:1280x712",
                "paint",
                "pump",
            ]
        );
    }

    #[test]
    fn null_engine_is_a_no_op_that_shows_substrate_text() {
        let mut engine = SubstrateNullEngine::new();
        assert_eq!(engine.name(), "substrate-null");
        assert!(!engine.renders_pixels());
        // navigate/resize/paint do nothing and must not panic.
        drive(&mut engine);
        // evaluate_js surfaces the gap mechanically — never a silent success.
        assert_eq!(
            engine.evaluate_js("1+1"),
            Err(EngineError::Unsupported {
                what: "evaluate_js",
                engine: "substrate-null"
            })
        );
    }

    #[test]
    fn engine_is_object_safe_and_swappable() {
        // Two different engines behind the same Box — the swap is a value choice.
        let (m, _) = MockEngine::new(true);
        let engines: Vec<Box<dyn BrowserEngine>> =
            vec![Box::new(SubstrateNullEngine::new()), Box::new(m)];
        let names: Vec<_> = engines.iter().map(|e| e.name()).collect();
        assert_eq!(names, vec!["substrate-null", "mock"]);
        let renders: Vec<_> = engines.iter().map(|e| e.renders_pixels()).collect();
        assert_eq!(renders, vec![false, true]);
    }

    #[test]
    fn pump_false_signals_shutdown() {
        let (mut mock, _) = MockEngine::new(false);
        mock.pump_result = false;
        assert!(!mock.pump());
    }

    #[test]
    fn default_display_list_is_empty_for_null_and_mock() {
        // SubstrateNullEngine + MockEngine do not override display_list →
        // the default `&'static` empty list keeps them green.
        let null = SubstrateNullEngine::new();
        assert!(null.display_list().is_empty());

        let (mock, _) = MockEngine::new(true);
        assert!(mock.display_list().is_empty());
    }

    /// The seal on the "frame that ate the page" class: reading the display
    /// list is NON-DESTRUCTIVE, so a host that reads every frame keeps
    /// painting the same content forever. This is enforced by the signature
    /// (`&self` → `&DisplayList`) — the test pins the behaviour that
    /// signature buys, driven through `dyn BrowserEngine` exactly as a host
    /// does.
    #[test]
    fn reading_the_display_list_never_consumes_it() {
        use crate::paint::{DisplayList, DrawCmd};

        struct Painting(DisplayList);
        impl BrowserEngine for Painting {
            fn name(&self) -> &'static str {
                "painting"
            }
            fn navigate(&mut self, _u: &Url) {}
            fn resize(&mut self, _r: ContentRect) {}
            fn renders_pixels(&self) -> bool {
                true
            }
            fn display_list(&self) -> &DisplayList {
                &self.0
            }
        }

        let mut list = DisplayList::new();
        list.push(DrawCmd::Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            color: crate::paint::Rgba::rgb8(1, 2, 3),
        });
        let engine: Box<dyn BrowserEngine> = Box::new(Painting(list));

        // 120 consecutive "frames" — every one still paints the page.
        for frame in 0..120 {
            assert_eq!(
                engine.display_list().cmds.len(),
                1,
                "frame {frame} lost the page — a per-frame read consumed it"
            );
        }
    }

    #[test]
    fn default_evaluate_js_is_unsupported() {
        // An engine that doesn't override evaluate_js gets the typed gap.
        struct Bare;
        impl BrowserEngine for Bare {
            fn name(&self) -> &'static str {
                "bare"
            }
            fn navigate(&mut self, _u: &Url) {}
            fn resize(&mut self, _r: ContentRect) {}
            fn renders_pixels(&self) -> bool {
                false
            }
        }
        let mut b = Bare;
        assert!(matches!(
            b.evaluate_js("x"),
            Err(EngineError::Unsupported { engine: "bare", .. })
        ));
    }
}
