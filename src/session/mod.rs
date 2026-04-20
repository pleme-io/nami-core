//! `(defsession)` — declarative session save/restore + undo-close.
//!
//! Absorbs Firefox Session Restore, Chrome's "Continue where you left
//! off", and "Reopen closed tab" (Cmd+Shift+T everywhere) into the
//! substrate. Each profile declares persistence + window-recovery
//! policy. Closed tabs live in a ring buffer so `undo-close` can pop
//! them; the same buffer is what `session.restore_last()` reads.
//!
//! ```lisp
//! (defsession :name              "default"
//!             :restore-on-open   #t
//!             :closed-tab-limit  25
//!             :autosave-seconds  30)
//! ```
//!
//! The session store itself is pure data — `SessionStore` holds the
//! state, policy lives in `SessionSpec`, and persistence to disk
//! reuses `(defstorage)`'s append-only Lisp event log (so sessions
//! round-trip through the same BLAKE3-attestable format).

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use url::Url;

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Session-recovery profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsession"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSpec {
    pub name: String,
    /// Re-open the previous window's tabs on startup.
    #[serde(default = "default_restore_on_open")]
    pub restore_on_open: bool,
    /// Max closed-tab entries kept for Cmd+Shift+T.
    #[serde(default = "default_closed_tab_limit")]
    pub closed_tab_limit: usize,
    /// Seconds between autosaves. `0` disables autosave (the caller
    /// can still save explicitly).
    #[serde(default = "default_autosave_seconds")]
    pub autosave_seconds: u64,
    /// Persist pinned tabs even across manual session clears.
    #[serde(default = "default_preserve_pinned")]
    pub preserve_pinned: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_restore_on_open() -> bool {
    true
}
fn default_closed_tab_limit() -> usize {
    25
}
fn default_autosave_seconds() -> u64 {
    30
}
fn default_preserve_pinned() -> bool {
    true
}

impl SessionSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            restore_on_open: true,
            closed_tab_limit: 25,
            autosave_seconds: 30,
            preserve_pinned: true,
            description: Some("Default session — restore on open, 25 undo-close.".into()),
        }
    }
}

/// One recorded tab — enough to restore it later.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TabRecord {
    pub url: Url,
    pub title: String,
    /// Unix seconds when the tab closed (for "recently closed" UI).
    pub closed_at: u64,
    /// True = tab was pinned at close time.
    #[serde(default)]
    pub pinned: bool,
}

/// Live session state. Owns the closed-tab ring buffer.
#[derive(Debug, Clone, Default)]
pub struct SessionStore {
    closed: VecDeque<TabRecord>,
    open: Vec<TabRecord>,
    limit: usize,
}

impl SessionStore {
    #[must_use]
    pub fn from_spec(spec: &SessionSpec) -> Self {
        Self {
            closed: VecDeque::with_capacity(spec.closed_tab_limit.max(1)),
            open: Vec::new(),
            limit: spec.closed_tab_limit.max(1),
        }
    }

    /// Record a currently-open tab. The store tracks these so
    /// `snapshot()` can return them for on-exit persistence.
    pub fn record_open(&mut self, rec: TabRecord) {
        // De-dupe by URL — same tab updated (navigated) replaces.
        self.open.retain(|t| t.url != rec.url);
        self.open.push(rec);
    }

    /// A tab was closed. Move it to the ring buffer.
    pub fn record_close(&mut self, rec: TabRecord) {
        self.open.retain(|t| t.url != rec.url);
        self.closed.push_back(rec);
        while self.closed.len() > self.limit {
            self.closed.pop_front();
        }
    }

    /// Pop the most-recently-closed tab (undo-close). Returns the
    /// URL to reopen, or None if the ring is empty.
    pub fn undo_close(&mut self) -> Option<TabRecord> {
        self.closed.pop_back()
    }

    /// Peek the most-recently-closed tab without removing it.
    #[must_use]
    pub fn peek_last_closed(&self) -> Option<&TabRecord> {
        self.closed.back()
    }

    /// Every currently-open tab.
    #[must_use]
    pub fn open_tabs(&self) -> &[TabRecord] {
        &self.open
    }

    /// Every closed tab, newest-first.
    #[must_use]
    pub fn closed_tabs(&self) -> Vec<TabRecord> {
        self.closed.iter().rev().cloned().collect()
    }

    /// Empty the closed-tab ring.
    pub fn clear_closed(&mut self) {
        self.closed.clear();
    }

    /// Empty everything. `preserve_pinned` keeps any `pinned: true`
    /// open tab in place.
    pub fn clear(&mut self, preserve_pinned: bool) {
        if preserve_pinned {
            self.open.retain(|t| t.pinned);
        } else {
            self.open.clear();
        }
        self.closed.clear();
    }

    /// Serialize for on-disk persistence. Shape is intentionally
    /// JSON-identical to `TabRecord` array so it round-trips through
    /// `(defstorage)` events without a custom codec.
    #[must_use]
    pub fn snapshot(&self) -> Vec<TabRecord> {
        self.open.clone()
    }

    /// Rehydrate from a previous snapshot. Does not touch the
    /// closed-tab ring.
    pub fn restore(&mut self, tabs: Vec<TabRecord>) {
        self.open = tabs;
    }

    #[must_use]
    pub fn len_open(&self) -> usize {
        self.open.len()
    }

    #[must_use]
    pub fn len_closed(&self) -> usize {
        self.closed.len()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SessionSpec>, String> {
    tatara_lisp::compile_typed::<SessionSpec>(src)
        .map_err(|e| format!("failed to compile defsession forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SessionSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(url: &str, title: &str) -> TabRecord {
        TabRecord {
            url: Url::parse(url).unwrap(),
            title: title.into(),
            closed_at: 0,
            pinned: false,
        }
    }

    #[test]
    fn default_profile_has_sane_defaults() {
        let s = SessionSpec::default_profile();
        assert!(s.restore_on_open);
        assert_eq!(s.closed_tab_limit, 25);
        assert_eq!(s.autosave_seconds, 30);
        assert!(s.preserve_pinned);
    }

    #[test]
    fn record_close_then_undo_pops_in_lifo_order() {
        let mut s = SessionStore::from_spec(&SessionSpec::default_profile());
        s.record_close(t("https://a.com/", "A"));
        s.record_close(t("https://b.com/", "B"));
        s.record_close(t("https://c.com/", "C"));
        let r1 = s.undo_close().unwrap();
        assert_eq!(r1.url.as_str(), "https://c.com/");
        let r2 = s.undo_close().unwrap();
        assert_eq!(r2.url.as_str(), "https://b.com/");
    }

    #[test]
    fn closed_ring_drops_oldest_at_limit() {
        let mut s = SessionStore::from_spec(&SessionSpec {
            closed_tab_limit: 3,
            ..SessionSpec::default_profile()
        });
        for i in 0..5 {
            s.record_close(t(&format!("https://site{i}.com/"), &format!("S{i}")));
        }
        assert_eq!(s.len_closed(), 3);
        let closed = s.closed_tabs();
        // Newest-first.
        assert_eq!(closed[0].url.as_str(), "https://site4.com/");
        assert_eq!(closed[2].url.as_str(), "https://site2.com/");
    }

    #[test]
    fn record_open_dedupes_by_url() {
        let mut s = SessionStore::from_spec(&SessionSpec::default_profile());
        s.record_open(TabRecord {
            title: "Old".into(),
            ..t("https://same.com/", "Old")
        });
        s.record_open(TabRecord {
            title: "New".into(),
            ..t("https://same.com/", "New")
        });
        assert_eq!(s.len_open(), 1);
        assert_eq!(s.open_tabs()[0].title, "New");
    }

    #[test]
    fn record_close_removes_from_open_list() {
        let mut s = SessionStore::from_spec(&SessionSpec::default_profile());
        s.record_open(t("https://a.com/", "A"));
        s.record_open(t("https://b.com/", "B"));
        s.record_close(t("https://a.com/", "A"));
        assert_eq!(s.len_open(), 1);
        assert_eq!(s.open_tabs()[0].url.as_str(), "https://b.com/");
    }

    #[test]
    fn undo_close_is_none_when_empty() {
        let mut s = SessionStore::from_spec(&SessionSpec::default_profile());
        assert!(s.undo_close().is_none());
    }

    #[test]
    fn peek_last_closed_does_not_consume() {
        let mut s = SessionStore::from_spec(&SessionSpec::default_profile());
        s.record_close(t("https://a.com/", "A"));
        assert_eq!(s.peek_last_closed().unwrap().url.as_str(), "https://a.com/");
        // Still there.
        assert_eq!(s.len_closed(), 1);
    }

    #[test]
    fn clear_with_preserve_pinned_keeps_pinned() {
        let mut s = SessionStore::from_spec(&SessionSpec::default_profile());
        s.record_open(TabRecord {
            pinned: true,
            ..t("https://pinned.com/", "Pinned")
        });
        s.record_open(t("https://ephemeral.com/", "Ephemeral"));
        s.record_close(t("https://closed.com/", "C"));
        s.clear(true);
        assert_eq!(s.len_open(), 1);
        assert_eq!(s.open_tabs()[0].url.as_str(), "https://pinned.com/");
        // Closed ring always wipes on clear().
        assert_eq!(s.len_closed(), 0);
    }

    #[test]
    fn snapshot_and_restore_roundtrip() {
        let mut a = SessionStore::from_spec(&SessionSpec::default_profile());
        a.record_open(t("https://x.com/", "X"));
        a.record_open(t("https://y.com/", "Y"));
        let snap = a.snapshot();

        let mut b = SessionStore::from_spec(&SessionSpec::default_profile());
        b.restore(snap);
        assert_eq!(b.len_open(), 2);
    }

    #[test]
    fn clear_closed_empties_ring_only() {
        let mut s = SessionStore::from_spec(&SessionSpec::default_profile());
        s.record_open(t("https://a.com/", "A"));
        s.record_close(t("https://b.com/", "B"));
        s.clear_closed();
        assert_eq!(s.len_closed(), 0);
        assert_eq!(s.len_open(), 1);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_session_form() {
        let src = r#"
            (defsession :name             "strict"
                        :restore-on-open  #f
                        :closed-tab-limit 10
                        :autosave-seconds 60
                        :preserve-pinned  #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "strict");
        assert!(!s.restore_on_open);
        assert_eq!(s.closed_tab_limit, 10);
        assert_eq!(s.autosave_seconds, 60);
    }
}
