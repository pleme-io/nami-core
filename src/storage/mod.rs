//! Persistent storage for bookmarks and browsing history.

pub mod bookmarks;
pub mod history;

pub use bookmarks::{Bookmark, BookmarkStore};
pub use history::{HistoryEntry, HistoryStore};
