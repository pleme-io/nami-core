//! Persistent storage.
//!
//! * [`bookmarks`] — domain-specific JSON-backed store.
//! * [`history`] — domain-specific JSON-backed store.
//! * [`kv`] — Lisp-declared generic key/value stores (`(defstorage)`)
//!   with append-only event-log persistence. Pure tatara-lisp, no
//!   SQLite dep. Covers cookies, session restore, user prefs, and
//!   anything else key→value that benefits from being a first-class
//!   substrate citizen.

pub mod bookmarks;
pub mod history;
pub mod kv;

pub use bookmarks::{Bookmark, BookmarkStore};
pub use history::{HistoryEntry, HistoryStore};
