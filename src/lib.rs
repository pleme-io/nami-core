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

pub mod alias;
pub mod config;
pub mod content;
pub mod css;
pub mod dom;
pub mod framework;
pub mod layout;
pub mod lisp;
pub mod net;
pub mod scrape;
pub mod selector;
pub mod state;
pub mod storage;
pub mod transform;
