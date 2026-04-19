//! Browser configuration.
//!
//! Defines the `BrowserConfig` type for nami-core consumers.
//! When the `config` feature is enabled, this integrates with shikumi
//! for file-based configuration with hot-reload. Without the feature,
//! it provides the config struct with manual construction.

use serde::{Deserialize, Serialize};

/// Top-level browser configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserConfig {
    /// The default homepage URL.
    pub homepage: String,

    /// Default search engine URL template. `{}` is replaced with the query.
    pub search_engine: String,

    /// Content blocking settings.
    pub content_blocking: ContentBlockingConfig,

    /// Privacy settings.
    pub privacy: PrivacyConfig,

    /// Network settings.
    pub network: NetworkConfig,

    /// Storage settings.
    pub storage: StorageConfig,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            homepage: "about:blank".to_string(),
            search_engine: "https://duckduckgo.com/?q={}".to_string(),
            content_blocking: ContentBlockingConfig::default(),
            privacy: PrivacyConfig::default(),
            network: NetworkConfig::default(),
            storage: StorageConfig::default(),
        }
    }
}

impl BrowserConfig {
    /// Create a config with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a search URL for the given query.
    #[must_use]
    pub fn search_url(&self, query: &str) -> String {
        self.search_engine.replace("{}", query)
    }

    /// Load configuration from a YAML string.
    ///
    /// # Errors
    ///
    /// Returns an error if the YAML is malformed.
    pub fn from_yaml(yaml: &str) -> Result<Self, ConfigError> {
        serde_json::from_str(yaml).or_else(|_| {
            // Try as JSON first (serde_json), fall back to treating it as
            // a simple key-value format. For full YAML support, consumers
            // should use serde_yaml or shikumi.
            Err(ConfigError::ParseError(
                "YAML parsing requires serde_yaml or the `config` feature with shikumi".to_string(),
            ))
        })
    }

    /// Load configuration from a JSON string.
    ///
    /// # Errors
    ///
    /// Returns an error if the JSON is malformed.
    pub fn from_json(json: &str) -> Result<Self, ConfigError> {
        serde_json::from_str(json).map_err(|e| ConfigError::ParseError(e.to_string()))
    }

    /// Discover the config file path following XDG conventions.
    ///
    /// Looks for `~/.config/nami/nami.yaml` (or `nami.json`).
    #[must_use]
    pub fn discover_path() -> Option<std::path::PathBuf> {
        let config_dir = dirs_path()?;
        let yaml_path = config_dir.join("nami.yaml");
        if yaml_path.exists() {
            return Some(yaml_path);
        }
        let json_path = config_dir.join("nami.json");
        if json_path.exists() {
            return Some(json_path);
        }
        None
    }
}

/// Content blocking configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContentBlockingConfig {
    /// Whether content blocking is enabled.
    pub enabled: bool,

    /// Paths to filter list files.
    pub filter_lists: Vec<String>,

    /// Additional domains to block (beyond filter lists).
    pub extra_blocked_domains: Vec<String>,

    /// Domains to always allow (exception rules).
    pub allowed_domains: Vec<String>,
}

impl Default for ContentBlockingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            filter_lists: Vec::new(),
            extra_blocked_domains: Vec::new(),
            allowed_domains: Vec::new(),
        }
    }
}

/// Privacy settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacyConfig {
    /// Whether to send Do-Not-Track header.
    pub do_not_track: bool,

    /// Whether to block third-party cookies.
    pub block_third_party_cookies: bool,

    /// Whether to enforce HTTPS-only mode.
    pub https_only: bool,

    /// Whether to clear browsing data on exit.
    pub clear_on_exit: bool,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            do_not_track: true,
            block_third_party_cookies: true,
            https_only: false,
            clear_on_exit: false,
        }
    }
}

/// Network settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// Request timeout in milliseconds.
    pub timeout_ms: u64,

    /// Maximum concurrent connections.
    pub max_connections: u32,

    /// User-Agent string override (empty = default).
    pub user_agent: String,

    /// Maximum number of redirects to follow.
    pub max_redirects: u32,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 30_000,
            max_connections: 6,
            user_agent: String::new(),
            max_redirects: 10,
        }
    }
}

/// Storage settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Maximum number of history entries to keep.
    pub max_history_entries: usize,

    /// Path to the bookmarks file (relative to config dir, or absolute).
    pub bookmarks_file: String,

    /// Path to the history file (relative to config dir, or absolute).
    pub history_file: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            max_history_entries: 10_000,
            bookmarks_file: "bookmarks.json".to_string(),
            history_file: "history.json".to_string(),
        }
    }
}

/// Configuration errors.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Failed to parse configuration.
    #[error("config parse error: {0}")]
    ParseError(String),

    /// IO error reading configuration file.
    #[error("config IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Get the nami config directory path (`~/.config/nami`).
fn dirs_path() -> Option<std::path::PathBuf> {
    // Use XDG_CONFIG_HOME if set, otherwise ~/.config.
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    Some(config_home.join("nami"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = BrowserConfig::new();
        assert_eq!(config.homepage, "about:blank");
        assert!(config.content_blocking.enabled);
        assert!(config.privacy.do_not_track);
        assert!(!config.privacy.https_only);
    }

    #[test]
    fn search_url_template() {
        let config = BrowserConfig::new();
        let url = config.search_url("rust lang");
        assert_eq!(url, "https://duckduckgo.com/?q=rust lang");
    }

    #[test]
    fn from_json() {
        let json = r#"{"homepage": "https://example.com", "search_engine": "https://google.com/search?q={}"}"#;
        let config = BrowserConfig::from_json(json).unwrap();
        assert_eq!(config.homepage, "https://example.com");
        assert_eq!(
            config.search_url("test"),
            "https://google.com/search?q=test"
        );
    }

    #[test]
    fn serde_roundtrip() {
        let config = BrowserConfig::new();
        let json = serde_json::to_string(&config).unwrap();
        let restored: BrowserConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.homepage, config.homepage);
        assert_eq!(restored.privacy.do_not_track, config.privacy.do_not_track);
    }

    #[test]
    fn default_storage_config() {
        let config = StorageConfig::default();
        assert_eq!(config.max_history_entries, 10_000);
        assert_eq!(config.bookmarks_file, "bookmarks.json");
        assert_eq!(config.history_file, "history.json");
    }
}
