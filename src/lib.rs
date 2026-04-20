//! # nami-core
//!
//! Shared browser core library for aranami (TUI/GPU browser) and namimado (desktop browser).
//!
//! Provides the complete web content pipeline:
//! - **DOM**: HTML parsing via html5ever, tree traversal, queries
//! - **CSS**: Stylesheet parsing via lightningcss, cascade resolution
//! - **Layout**: Flexbox/grid layout computation via taffy
//! - **Net**: HTTP request pipeline (requires `network` feature)
//! - **Content**: Ad/tracker blocking with filter lists
//! - **Storage**: Bookmarks and browsing history
//! - **Config**: Browser configuration (requires `config` feature)
//! - **Transform**: Lisp-programmable DOM mutations (requires `lisp` feature;
//!   the engine and spec types are always available, the Lisp front-end is opt-in)

pub mod accessibility;
pub mod agent;
pub mod alias;
pub mod blocker;
pub mod boost;
pub mod command;
#[cfg(feature = "ts")]
pub mod ast;
pub mod component;
pub mod config;
pub mod content;
pub mod css;
pub mod css_ast;
pub mod derived;
pub mod dom;
pub mod effect;
#[cfg(feature = "eval")]
pub mod eval;
pub mod extension;
pub mod find;
pub mod framework;
pub mod gesture;
pub mod i18n;
#[cfg(feature = "eval")]
pub mod inline_lisp;
pub mod js_runtime;
pub mod layout;
pub mod lisp;
pub mod net;
pub mod normalize;
pub mod omnibox;
pub mod pip;
pub mod plan;
pub mod predicate;
pub mod query;
pub mod reader;
pub mod route;
pub mod scrape;
pub mod security_policy;
pub mod selector;
pub mod session;
pub mod shadow;
pub mod sidebar;
pub mod snapshot;
pub mod space;
pub mod split;
pub mod state;
pub mod storage;
pub mod store;
pub mod transform;
pub mod typescape;
#[cfg(feature = "wasm")]
pub mod wasm;
#[cfg(feature = "lisp")]
pub mod wasm_agent;
pub mod zoom;
