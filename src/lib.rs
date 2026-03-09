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

pub mod config;
pub mod content;
pub mod css;
pub mod dom;
pub mod layout;
pub mod net;
pub mod storage;
