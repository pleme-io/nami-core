//! Content blocking (ad/tracker filtering).
//!
//! Implements a filter-list-based content blocker that can block requests
//! to known ad and tracker domains.

pub mod blocker;

pub use blocker::{ContentBlocker, ResourceType};
