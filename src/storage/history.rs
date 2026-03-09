//! Browsing history storage with search and statistics.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::debug;

/// Errors from history operations.
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    /// IO error reading/writing the history file.
    #[error("history IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error.
    #[error("history serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// A single browsing history entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// The visited URL.
    pub url: String,
    /// Page title at time of visit.
    pub title: String,
    /// When this URL was last visited (Unix timestamp in seconds).
    pub visited_at: u64,
    /// Total number of times this URL has been visited.
    pub visit_count: u32,
}

impl HistoryEntry {
    /// Create a new history entry with visit count 1 and the current timestamp.
    #[must_use]
    pub fn new(url: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            title: title.into(),
            visited_at: current_timestamp(),
            visit_count: 1,
        }
    }

    /// Get the domain from the URL, if parseable.
    #[must_use]
    pub fn domain(&self) -> Option<String> {
        url::Url::parse(&self.url)
            .ok()
            .and_then(|u| u.host_str().map(String::from))
    }
}

/// Persistent browsing history storage backed by a JSON file.
#[derive(Debug)]
pub struct HistoryStore {
    /// Path to the history JSON file.
    path: PathBuf,
    /// In-memory history entries, keyed by URL for deduplication.
    entries: HashMap<String, HistoryEntry>,
    /// Maximum number of entries to keep.
    max_entries: usize,
}

impl HistoryStore {
    /// Default maximum number of history entries.
    pub const DEFAULT_MAX_ENTRIES: usize = 10_000;

    /// Create or open a history store at the given path.
    ///
    /// If the file exists, loads existing history. Otherwise, starts empty.
    ///
    /// # Errors
    ///
    /// Returns `HistoryError` if the file exists but cannot be read or parsed.
    pub fn new(path: PathBuf) -> Result<Self, HistoryError> {
        let entries = if path.exists() {
            let data = std::fs::read_to_string(&path)?;
            let list: Vec<HistoryEntry> = serde_json::from_str(&data)?;
            list.into_iter().map(|e| (e.url.clone(), e)).collect()
        } else {
            HashMap::new()
        };

        debug!("history store opened with {} entries", entries.len());
        Ok(Self {
            path,
            entries,
            max_entries: Self::DEFAULT_MAX_ENTRIES,
        })
    }

    /// Create an in-memory history store (not backed by a file).
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            path: PathBuf::new(),
            entries: HashMap::new(),
            max_entries: Self::DEFAULT_MAX_ENTRIES,
        }
    }

    /// Set the maximum number of entries to keep.
    pub fn set_max_entries(&mut self, max: usize) {
        self.max_entries = max;
    }

    /// Record a page visit. If the URL was previously visited, updates the
    /// timestamp, title, and increments the visit count.
    pub fn record(&mut self, url: impl Into<String>, title: impl Into<String>) {
        let url = url.into();
        let title = title.into();
        let now = current_timestamp();

        if let Some(entry) = self.entries.get_mut(&url) {
            entry.visited_at = now;
            entry.visit_count += 1;
            if !title.is_empty() {
                entry.title = title;
            }
        } else {
            self.entries.insert(
                url.clone(),
                HistoryEntry {
                    url,
                    title,
                    visited_at: now,
                    visit_count: 1,
                },
            );
        }

        // Evict old entries if over the limit.
        self.evict_if_needed();
    }

    /// Search history entries by query (matches URL and title).
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<&HistoryEntry> {
        let query_lower = query.to_lowercase();
        let mut results: Vec<&HistoryEntry> = self
            .entries
            .values()
            .filter(|e| {
                e.url.to_lowercase().contains(&query_lower)
                    || e.title.to_lowercase().contains(&query_lower)
            })
            .collect();

        // Sort by most recently visited.
        results.sort_by(|a, b| b.visited_at.cmp(&a.visited_at));
        results
    }

    /// Get the most recently visited entries, up to `limit`.
    #[must_use]
    pub fn recent(&self, limit: usize) -> Vec<&HistoryEntry> {
        let mut entries: Vec<&HistoryEntry> = self.entries.values().collect();
        entries.sort_by(|a, b| b.visited_at.cmp(&a.visited_at));
        entries.truncate(limit);
        entries
    }

    /// Get the most frequently visited entries, up to `limit`.
    #[must_use]
    pub fn most_visited(&self, limit: usize) -> Vec<&HistoryEntry> {
        let mut entries: Vec<&HistoryEntry> = self.entries.values().collect();
        entries.sort_by(|a, b| b.visit_count.cmp(&a.visit_count));
        entries.truncate(limit);
        entries
    }

    /// Get a single entry by URL.
    #[must_use]
    pub fn get(&self, url: &str) -> Option<&HistoryEntry> {
        self.entries.get(url)
    }

    /// Remove a single entry by URL.
    pub fn remove(&mut self, url: &str) -> Option<HistoryEntry> {
        self.entries.remove(url)
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Get the total number of history entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the history is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get per-domain visit statistics.
    #[must_use]
    pub fn domain_stats(&self) -> Vec<(String, u32)> {
        let mut domain_counts: HashMap<String, u32> = HashMap::new();
        for entry in self.entries.values() {
            if let Some(domain) = entry.domain() {
                *domain_counts.entry(domain).or_insert(0) += entry.visit_count;
            }
        }
        let mut stats: Vec<(String, u32)> = domain_counts.into_iter().collect();
        stats.sort_by(|a, b| b.1.cmp(&a.1));
        stats
    }

    /// Save history to disk.
    ///
    /// # Errors
    ///
    /// Returns `HistoryError` if the file cannot be written.
    pub fn save(&self) -> Result<(), HistoryError> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let entries: Vec<&HistoryEntry> = self.entries.values().collect();
        let json = serde_json::to_string_pretty(&entries)?;
        std::fs::write(&self.path, json)?;
        debug!("saved {} history entries to {:?}", entries.len(), self.path);
        Ok(())
    }

    /// Evict the oldest entries if over the max limit.
    fn evict_if_needed(&mut self) {
        if self.entries.len() <= self.max_entries {
            return;
        }

        // Find the entries to remove (oldest by visited_at).
        let mut by_time: Vec<(String, u64)> = self
            .entries
            .iter()
            .map(|(url, e)| (url.clone(), e.visited_at))
            .collect();
        by_time.sort_by_key(|(_, t)| *t);

        let to_remove = self.entries.len() - self.max_entries;
        for (url, _) in by_time.into_iter().take(to_remove) {
            self.entries.remove(&url);
        }
    }
}

/// Get the current Unix timestamp in seconds.
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_retrieve() {
        let mut store = HistoryStore::in_memory();
        store.record("https://example.com", "Example");
        store.record("https://rust-lang.org", "Rust");

        assert_eq!(store.len(), 2);
        assert!(store.get("https://example.com").is_some());
        assert_eq!(
            store.get("https://example.com").unwrap().visit_count,
            1
        );
    }

    #[test]
    fn record_increments_visit_count() {
        let mut store = HistoryStore::in_memory();
        store.record("https://example.com", "Example");
        store.record("https://example.com", "Example - Updated");
        store.record("https://example.com", "Example - Updated Again");

        let entry = store.get("https://example.com").unwrap();
        assert_eq!(entry.visit_count, 3);
        assert_eq!(entry.title, "Example - Updated Again");
    }

    #[test]
    fn search_history() {
        let mut store = HistoryStore::in_memory();
        store.record("https://rust-lang.org", "Rust Programming");
        store.record("https://python.org", "Python Programming");
        store.record("https://example.com", "Example");

        let results = store.search("rust");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://rust-lang.org");

        let results = store.search("programming");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn recent_entries() {
        let mut store = HistoryStore::in_memory();
        store.record("https://a.com", "A");
        store.record("https://b.com", "B");
        store.record("https://c.com", "C");

        let recent = store.recent(2);
        assert_eq!(recent.len(), 2);
    }

    #[test]
    fn most_visited() {
        let mut store = HistoryStore::in_memory();
        store.record("https://a.com", "A");
        store.record("https://b.com", "B");
        store.record("https://b.com", "B");
        store.record("https://b.com", "B");
        store.record("https://c.com", "C");
        store.record("https://c.com", "C");

        let top = store.most_visited(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].url, "https://b.com");
        assert_eq!(top[0].visit_count, 3);
    }

    #[test]
    fn domain_stats() {
        let mut store = HistoryStore::in_memory();
        store.record("https://example.com/a", "A");
        store.record("https://example.com/b", "B");
        store.record("https://other.com/c", "C");

        let stats = store.domain_stats();
        assert!(stats.len() >= 2);
        // example.com should have 2 visits total.
        let example = stats.iter().find(|(d, _)| d == "example.com");
        assert!(example.is_some());
        assert_eq!(example.unwrap().1, 2);
    }

    #[test]
    fn remove_entry() {
        let mut store = HistoryStore::in_memory();
        store.record("https://example.com", "Example");
        assert_eq!(store.len(), 1);

        let removed = store.remove("https://example.com");
        assert!(removed.is_some());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn clear_history() {
        let mut store = HistoryStore::in_memory();
        store.record("https://a.com", "A");
        store.record("https://b.com", "B");
        store.clear();
        assert!(store.is_empty());
    }

    #[test]
    fn eviction() {
        let mut store = HistoryStore::in_memory();
        store.set_max_entries(3);

        store.record("https://1.com", "1");
        store.record("https://2.com", "2");
        store.record("https://3.com", "3");
        store.record("https://4.com", "4");

        assert_eq!(store.len(), 3);
    }

    #[test]
    fn persistence() {
        let dir = std::env::temp_dir().join("nami-core-test-history");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("history.json");

        {
            let mut store = HistoryStore::new(path.clone()).unwrap();
            store.record("https://saved.com", "Saved");
            store.save().unwrap();
        }

        {
            let store = HistoryStore::new(path.clone()).unwrap();
            assert_eq!(store.len(), 1);
            assert!(store.get("https://saved.com").is_some());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
