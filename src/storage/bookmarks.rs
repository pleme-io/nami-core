//! Bookmark storage with CRUD operations and search.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::debug;
use url::Url;

/// Errors from bookmark operations.
#[derive(Debug, thiserror::Error)]
pub enum BookmarkError {
    /// IO error reading/writing the bookmark file.
    #[error("bookmark IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error.
    #[error("bookmark serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Bookmark not found.
    #[error("bookmark not found: {0}")]
    NotFound(String),

    /// Duplicate bookmark.
    #[error("bookmark already exists: {0}")]
    Duplicate(String),
}

/// A single bookmark entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    /// The bookmarked URL.
    pub url: String,
    /// Display title.
    pub title: String,
    /// User-assigned tags for categorization.
    pub tags: Vec<String>,
    /// When the bookmark was created (Unix timestamp in seconds).
    pub created_at: u64,
    /// Optional folder/group name.
    pub folder: Option<String>,
}

impl Bookmark {
    /// Create a new bookmark with the current timestamp.
    #[must_use]
    pub fn new(url: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            title: title.into(),
            tags: Vec::new(),
            created_at: current_timestamp(),
            folder: None,
        }
    }

    /// Create a bookmark with tags.
    #[must_use]
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Create a bookmark in a folder.
    #[must_use]
    pub fn in_folder(mut self, folder: impl Into<String>) -> Self {
        self.folder = Some(folder.into());
        self
    }

    /// Check if this bookmark has a given tag.
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t.eq_ignore_ascii_case(tag))
    }

    /// Parse the URL, returning `None` if invalid.
    #[must_use]
    pub fn parsed_url(&self) -> Option<Url> {
        Url::parse(&self.url).ok()
    }

    /// Get the domain from the URL, if parseable.
    #[must_use]
    pub fn domain(&self) -> Option<String> {
        self.parsed_url()
            .and_then(|u| u.host_str().map(String::from))
    }
}

/// Persistent bookmark storage backed by a JSON file.
#[derive(Debug)]
pub struct BookmarkStore {
    /// Path to the bookmarks JSON file.
    path: PathBuf,
    /// In-memory bookmark list.
    bookmarks: Vec<Bookmark>,
}

impl BookmarkStore {
    /// Create or open a bookmark store at the given path.
    ///
    /// If the file exists, loads existing bookmarks. Otherwise, starts empty
    /// and creates the file on first save.
    ///
    /// # Errors
    ///
    /// Returns `BookmarkError` if the file exists but cannot be read or parsed.
    pub fn new(path: PathBuf) -> Result<Self, BookmarkError> {
        let bookmarks = if path.exists() {
            let data = std::fs::read_to_string(&path)?;
            serde_json::from_str(&data)?
        } else {
            Vec::new()
        };

        debug!("bookmark store opened with {} entries", bookmarks.len());
        Ok(Self { path, bookmarks })
    }

    /// Create an in-memory bookmark store (not backed by a file).
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            path: PathBuf::new(),
            bookmarks: Vec::new(),
        }
    }

    /// Add a bookmark. Returns an error if a bookmark with the same URL already exists.
    ///
    /// # Errors
    ///
    /// Returns `BookmarkError::Duplicate` if the URL is already bookmarked.
    pub fn add(&mut self, bookmark: Bookmark) -> Result<(), BookmarkError> {
        if self.bookmarks.iter().any(|b| b.url == bookmark.url) {
            return Err(BookmarkError::Duplicate(bookmark.url));
        }
        self.bookmarks.push(bookmark);
        Ok(())
    }

    /// Remove a bookmark by URL. Returns the removed bookmark.
    ///
    /// # Errors
    ///
    /// Returns `BookmarkError::NotFound` if no bookmark with that URL exists.
    pub fn remove(&mut self, url: &str) -> Result<Bookmark, BookmarkError> {
        let pos = self
            .bookmarks
            .iter()
            .position(|b| b.url == url)
            .ok_or_else(|| BookmarkError::NotFound(url.to_string()))?;
        Ok(self.bookmarks.remove(pos))
    }

    /// Get a bookmark by URL.
    #[must_use]
    pub fn get(&self, url: &str) -> Option<&Bookmark> {
        self.bookmarks.iter().find(|b| b.url == url)
    }

    /// Check if a URL is bookmarked.
    #[must_use]
    pub fn contains(&self, url: &str) -> bool {
        self.bookmarks.iter().any(|b| b.url == url)
    }

    /// Search bookmarks by query string (matches against URL, title, and tags).
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<&Bookmark> {
        let query_lower = query.to_lowercase();
        self.bookmarks
            .iter()
            .filter(|b| {
                b.url.to_lowercase().contains(&query_lower)
                    || b.title.to_lowercase().contains(&query_lower)
                    || b.tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query_lower))
            })
            .collect()
    }

    /// Get all bookmarks in a folder.
    #[must_use]
    pub fn in_folder(&self, folder: &str) -> Vec<&Bookmark> {
        self.bookmarks
            .iter()
            .filter(|b| b.folder.as_deref() == Some(folder))
            .collect()
    }

    /// Get all bookmarks with a given tag.
    #[must_use]
    pub fn with_tag(&self, tag: &str) -> Vec<&Bookmark> {
        self.bookmarks.iter().filter(|b| b.has_tag(tag)).collect()
    }

    /// Get all bookmarks.
    #[must_use]
    pub fn all(&self) -> &[Bookmark] {
        &self.bookmarks
    }

    /// Get the total number of bookmarks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bookmarks.len()
    }

    /// Check if the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bookmarks.is_empty()
    }

    /// Get all unique tags across all bookmarks.
    #[must_use]
    pub fn all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .bookmarks
            .iter()
            .flat_map(|b| b.tags.iter().cloned())
            .collect();
        tags.sort();
        tags.dedup();
        tags
    }

    /// Save bookmarks to disk.
    ///
    /// # Errors
    ///
    /// Returns `BookmarkError` if the file cannot be written.
    pub fn save(&self) -> Result<(), BookmarkError> {
        if self.path.as_os_str().is_empty() {
            // In-memory store, nothing to save.
            return Ok(());
        }

        // Ensure parent directory exists.
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(&self.bookmarks)?;
        std::fs::write(&self.path, json)?;
        debug!(
            "saved {} bookmarks to {:?}",
            self.bookmarks.len(),
            self.path
        );
        Ok(())
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
    fn create_bookmark() {
        let bm = Bookmark::new("https://example.com", "Example")
            .with_tags(vec!["test".into(), "web".into()])
            .in_folder("work");

        assert_eq!(bm.url, "https://example.com");
        assert_eq!(bm.title, "Example");
        assert!(bm.has_tag("test"));
        assert!(bm.has_tag("web"));
        assert!(!bm.has_tag("other"));
        assert_eq!(bm.folder, Some("work".to_string()));
        assert!(bm.created_at > 0);
    }

    #[test]
    fn bookmark_domain() {
        let bm = Bookmark::new("https://www.rust-lang.org/learn", "Learn Rust");
        assert_eq!(bm.domain(), Some("www.rust-lang.org".to_string()));
    }

    #[test]
    fn store_add_remove() {
        let mut store = BookmarkStore::in_memory();
        assert!(store.is_empty());

        store.add(Bookmark::new("https://a.com", "A")).unwrap();
        store.add(Bookmark::new("https://b.com", "B")).unwrap();
        assert_eq!(store.len(), 2);

        // Duplicate should fail.
        let result = store.add(Bookmark::new("https://a.com", "A again"));
        assert!(result.is_err());

        // Remove.
        let removed = store.remove("https://a.com").unwrap();
        assert_eq!(removed.title, "A");
        assert_eq!(store.len(), 1);

        // Remove non-existent should fail.
        let result = store.remove("https://missing.com");
        assert!(result.is_err());
    }

    #[test]
    fn store_search() {
        let mut store = BookmarkStore::in_memory();
        store
            .add(
                Bookmark::new("https://rust-lang.org", "Rust Programming")
                    .with_tags(vec!["programming".into()]),
            )
            .unwrap();
        store
            .add(
                Bookmark::new("https://python.org", "Python Programming")
                    .with_tags(vec!["programming".into()]),
            )
            .unwrap();
        store
            .add(Bookmark::new("https://example.com", "Example Site"))
            .unwrap();

        let results = store.search("rust");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://rust-lang.org");

        let results = store.search("programming");
        assert_eq!(results.len(), 2);

        let results = store.search("nonexistent");
        assert!(results.is_empty());
    }

    #[test]
    fn store_tags_and_folders() {
        let mut store = BookmarkStore::in_memory();
        store
            .add(
                Bookmark::new("https://a.com", "A")
                    .with_tags(vec!["work".into(), "rust".into()])
                    .in_folder("dev"),
            )
            .unwrap();
        store
            .add(
                Bookmark::new("https://b.com", "B")
                    .with_tags(vec!["fun".into()])
                    .in_folder("personal"),
            )
            .unwrap();
        store
            .add(
                Bookmark::new("https://c.com", "C")
                    .with_tags(vec!["work".into()])
                    .in_folder("dev"),
            )
            .unwrap();

        let dev = store.in_folder("dev");
        assert_eq!(dev.len(), 2);

        let work = store.with_tag("work");
        assert_eq!(work.len(), 2);

        let all_tags = store.all_tags();
        assert_eq!(all_tags, vec!["fun", "rust", "work"]);
    }

    #[test]
    fn store_contains() {
        let mut store = BookmarkStore::in_memory();
        store.add(Bookmark::new("https://a.com", "A")).unwrap();
        assert!(store.contains("https://a.com"));
        assert!(!store.contains("https://b.com"));
    }

    #[test]
    fn store_persistence() {
        let dir = std::env::temp_dir().join("nami-core-test-bookmarks");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("bookmarks.json");

        // Create and save.
        {
            let mut store = BookmarkStore::new(path.clone()).unwrap();
            store
                .add(Bookmark::new("https://saved.com", "Saved"))
                .unwrap();
            store.save().unwrap();
        }

        // Reload and verify.
        {
            let store = BookmarkStore::new(path.clone()).unwrap();
            assert_eq!(store.len(), 1);
            assert!(store.contains("https://saved.com"));
        }

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
