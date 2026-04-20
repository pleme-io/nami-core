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
pub mod audit_trail;
pub mod auth_saver;
pub mod autofill;
pub mod chat;
pub mod bfcache_policy;
pub mod blocker;
pub mod boost;
pub mod bridge;
pub mod cast;
pub mod clear_site_data;
pub mod command;
#[cfg(feature = "ts")]
pub mod ast;
pub mod component;
pub mod config;
pub mod console_rule;
pub mod content;
pub mod cookie_jar;
pub mod crdt_room;
pub mod csp_policy;
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
pub mod fingerprint_randomize;
pub mod framework;
pub mod gesture;
pub mod high_contrast;
pub mod history_policy;
pub mod i18n;
pub mod identity;
#[cfg(feature = "eval")]
pub mod inline_lisp;
pub mod inspector;
pub mod js_runtime;
pub mod layout;
pub mod lisp;
pub mod llm;
pub mod llm_completion;
pub mod media_session;
pub mod multiplayer_cursor;
pub mod navigation_intent;
pub mod net;
pub mod normalize;
pub mod offline;
pub mod omnibox;
pub mod outline;
pub mod passkey;
pub mod passwords;
pub mod permission_policy;
pub mod permission_prompt;
pub mod pip;
pub mod plan;
pub mod predicate;
pub mod prerender_rule;
pub mod presence;
pub mod profiler;
pub mod pull_refresh;
pub mod query;
pub mod reader;
pub mod reader_aloud;
pub mod redirect;
pub mod resource_hint;
pub mod route;
pub mod routing;
pub mod scrape;
pub mod script_policy;
pub mod search_bang;
pub mod search_engine;
pub mod secure_note;
pub mod security_policy;
pub mod selector;
pub mod service_worker;
pub mod session;
pub mod shadow;
pub mod share;
pub mod sidebar;
pub mod simplify;
pub mod snapshot;
pub mod space;
pub mod split;
pub mod spoof;
pub mod state;
pub mod storage;
pub mod storage_quota;
pub mod store;
pub mod subtitle;
pub mod suggestion_ranker;
pub mod suggestion_source;
pub mod summarize;
pub mod sync_channel;
pub mod tab_group;
pub mod tab_hibernate;
pub mod tab_preview;
pub mod totp;
pub mod transform;
pub mod typescape;
pub mod url_clean;
pub mod viewport;
#[cfg(feature = "wasm")]
pub mod wasm;
pub mod webgpu_policy;
#[cfg(feature = "lisp")]
pub mod wasm_agent;
pub mod zoom;
