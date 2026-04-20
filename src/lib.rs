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
pub mod annotate;
pub mod auth_saver;
pub mod autofill;
pub mod chat;
pub mod blocker;
pub mod boost;
pub mod bridge;
pub mod command;
#[cfg(feature = "ts")]
pub mod ast;
pub mod component;
pub mod config;
pub mod content;
pub mod css;
pub mod css_ast;
pub mod derived;
pub mod dns;
pub mod dom;
pub mod download;
pub mod effect;
#[cfg(feature = "eval")]
pub mod eval;
pub mod extension;
pub mod feed;
pub mod find;
pub mod framework;
pub mod gesture;
pub mod i18n;
#[cfg(feature = "eval")]
pub mod inline_lisp;
pub mod js_runtime;
pub mod layout;
pub mod lisp;
pub mod llm;
pub mod llm_completion;
pub mod net;
pub mod normalize;
pub mod offline;
pub mod omnibox;
pub mod outline;
pub mod passkey;
pub mod passwords;
pub mod pip;
pub mod plan;
pub mod predicate;
pub mod pull_refresh;
pub mod query;
pub mod reader;
pub mod redirect;
pub mod route;
pub mod routing;
pub mod scrape;
pub mod script_policy;
pub mod secure_note;
pub mod security_policy;
pub mod selector;
pub mod session;
pub mod shadow;
pub mod share;
pub mod sidebar;
pub mod snapshot;
pub mod space;
pub mod split;
pub mod spoof;
pub mod state;
pub mod storage;
pub mod store;
pub mod summarize;
pub mod transform;
pub mod typescape;
pub mod url_clean;
#[cfg(feature = "wasm")]
pub mod wasm;
#[cfg(feature = "lisp")]
pub mod wasm_agent;
pub mod zoom;
