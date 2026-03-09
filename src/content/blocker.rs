//! Ad and tracker content blocking.
//!
//! Implements domain-based and pattern-based content blocking using filter lists
//! (compatible with EasyList/uBlock Origin format subsets).

use std::collections::HashSet;

use tracing::debug;
use url::Url;

/// The type of resource being loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceType {
    /// The main document (top-level page).
    Document,
    /// A CSS stylesheet.
    Stylesheet,
    /// A JavaScript file.
    Script,
    /// An image.
    Image,
    /// A font file.
    Font,
    /// An XHR/fetch request.
    XmlHttpRequest,
    /// A media file (audio/video).
    Media,
    /// A WebSocket connection.
    WebSocket,
    /// An iframe or frame.
    SubDocument,
    /// Any other resource type.
    Other,
}

/// A content blocker that decides whether to allow or block resource loads.
#[derive(Debug, Clone)]
pub struct ContentBlocker {
    /// Blocked domains (exact match).
    blocked_domains: HashSet<String>,
    /// Blocked URL patterns (substring match).
    blocked_patterns: Vec<String>,
    /// Allowed domains (exception rules).
    allowed_domains: HashSet<String>,
    /// Whether blocking is enabled.
    enabled: bool,
    /// Statistics.
    stats: BlockerStats,
}

/// Blocking statistics.
#[derive(Debug, Clone, Default)]
pub struct BlockerStats {
    /// Total number of requests checked.
    pub checked: u64,
    /// Total number of requests blocked.
    pub blocked: u64,
    /// Total number of requests allowed.
    pub allowed: u64,
}

impl ContentBlocker {
    /// Create a new content blocker with no rules loaded.
    #[must_use]
    pub fn new() -> Self {
        Self {
            blocked_domains: HashSet::new(),
            blocked_patterns: Vec::new(),
            allowed_domains: HashSet::new(),
            enabled: true,
            stats: BlockerStats::default(),
        }
    }

    /// Enable or disable the content blocker.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check whether the blocker is enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Load a filter list in a simplified format.
    ///
    /// Supported line formats:
    /// - `||domain.com^` — block domain and subdomains
    /// - `@@||domain.com^` — exception (allow) rule
    /// - `/pattern/` — block URLs matching the pattern (substring)
    /// - Lines starting with `!` or `[` are comments/headers
    /// - Empty lines are ignored
    ///
    /// # Errors
    ///
    /// Returns an error if the filter list is malformed in an unrecoverable way.
    /// Individual malformed rules are skipped with a warning.
    pub fn load_filter_list(&mut self, list: &str) -> Result<(), ContentBlockerError> {
        let mut loaded = 0u32;

        for line in list.lines() {
            let line = line.trim();

            // Skip empty lines and comments.
            if line.is_empty() || line.starts_with('!') || line.starts_with('[') {
                continue;
            }

            // Exception rule: @@||domain^
            if let Some(rest) = line.strip_prefix("@@||") {
                if let Some(domain) = rest.strip_suffix('^') {
                    self.allowed_domains.insert(domain.to_lowercase());
                    loaded += 1;
                }
                continue;
            }

            // Domain rule: ||domain^
            if let Some(rest) = line.strip_prefix("||") {
                if let Some(domain) = rest.strip_suffix('^') {
                    self.blocked_domains.insert(domain.to_lowercase());
                    loaded += 1;
                }
                continue;
            }

            // Pattern rule: any other non-comment line is treated as a substring pattern.
            if !line.is_empty() {
                self.blocked_patterns.push(line.to_lowercase());
                loaded += 1;
            }
        }

        debug!("loaded {loaded} filter rules");
        Ok(())
    }

    /// Add a single domain to the block list.
    pub fn block_domain(&mut self, domain: &str) {
        self.blocked_domains.insert(domain.to_lowercase());
    }

    /// Add a single domain to the allow list (exception).
    pub fn allow_domain(&mut self, domain: &str) {
        self.allowed_domains.insert(domain.to_lowercase());
    }

    /// Check whether a URL should be blocked.
    ///
    /// Takes the URL and the type of resource being loaded. Returns `true`
    /// if the request should be blocked.
    #[must_use]
    pub fn should_block(&self, url: &Url, _resource_type: ResourceType) -> bool {
        if !self.enabled {
            return false;
        }

        let host = url.host_str().unwrap_or("");
        let url_str = url.as_str().to_lowercase();

        // Check allow list first (exceptions).
        if self.is_domain_in_set(host, &self.allowed_domains) {
            return false;
        }

        // Check blocked domains.
        if self.is_domain_in_set(host, &self.blocked_domains) {
            return true;
        }

        // Check blocked patterns.
        self.blocked_patterns.iter().any(|p| url_str.contains(p))
    }

    /// Record a blocking decision and update stats.
    pub fn record_decision(&mut self, url: &Url, resource_type: ResourceType) -> bool {
        let blocked = self.should_block(url, resource_type);
        self.stats.checked += 1;
        if blocked {
            self.stats.blocked += 1;
        } else {
            self.stats.allowed += 1;
        }
        blocked
    }

    /// Get current blocking statistics.
    #[must_use]
    pub fn stats(&self) -> &BlockerStats {
        &self.stats
    }

    /// Reset blocking statistics.
    pub fn reset_stats(&mut self) {
        self.stats = BlockerStats::default();
    }

    /// Get the number of loaded rules.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.blocked_domains.len() + self.blocked_patterns.len() + self.allowed_domains.len()
    }

    /// Check if a host matches any domain in the given set (including subdomain matching).
    fn is_domain_in_set(&self, host: &str, set: &HashSet<String>) -> bool {
        let host_lower = host.to_lowercase();

        // Exact match.
        if set.contains(&host_lower) {
            return true;
        }

        // Subdomain match: if host is "sub.example.com" and "example.com" is in the set.
        let mut parts = host_lower.as_str();
        while let Some(pos) = parts.find('.') {
            parts = &parts[pos + 1..];
            if set.contains(parts) {
                return true;
            }
        }

        false
    }
}

impl Default for ContentBlocker {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors from content blocker operations.
#[derive(Debug, thiserror::Error)]
pub enum ContentBlockerError {
    /// Filter list parsing error.
    #[error("filter list error: {0}")]
    ParseError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_domain() {
        let mut blocker = ContentBlocker::new();
        blocker.block_domain("ads.example.com");

        let url = Url::parse("https://ads.example.com/banner.js").unwrap();
        assert!(blocker.should_block(&url, ResourceType::Script));

        let url = Url::parse("https://example.com/page").unwrap();
        assert!(!blocker.should_block(&url, ResourceType::Document));
    }

    #[test]
    fn block_subdomain() {
        let mut blocker = ContentBlocker::new();
        blocker.block_domain("tracker.com");

        // Subdomain should also be blocked.
        let url = Url::parse("https://cdn.tracker.com/pixel.gif").unwrap();
        assert!(blocker.should_block(&url, ResourceType::Image));
    }

    #[test]
    fn exception_rule() {
        let mut blocker = ContentBlocker::new();
        blocker.block_domain("ads.com");
        blocker.allow_domain("safe.ads.com");

        let blocked = Url::parse("https://ads.com/banner").unwrap();
        assert!(blocker.should_block(&blocked, ResourceType::Script));

        let allowed = Url::parse("https://safe.ads.com/widget").unwrap();
        assert!(!blocker.should_block(&allowed, ResourceType::Script));
    }

    #[test]
    fn load_filter_list() {
        let list = r"! EasyList sample
[Adblock Plus 2.0]
||doubleclick.net^
||google-analytics.com^
@@||analytics.allowed.com^
/tracking-pixel
";
        let mut blocker = ContentBlocker::new();
        blocker.load_filter_list(list).unwrap();

        assert_eq!(blocker.rule_count(), 4);

        let url = Url::parse("https://doubleclick.net/ad").unwrap();
        assert!(blocker.should_block(&url, ResourceType::Script));

        let url = Url::parse("https://example.com/tracking-pixel?id=1").unwrap();
        assert!(blocker.should_block(&url, ResourceType::Image));

        let url = Url::parse("https://analytics.allowed.com/script.js").unwrap();
        assert!(!blocker.should_block(&url, ResourceType::Script));
    }

    #[test]
    fn disabled_blocker_allows_all() {
        let mut blocker = ContentBlocker::new();
        blocker.block_domain("ads.com");
        blocker.set_enabled(false);

        let url = Url::parse("https://ads.com/banner").unwrap();
        assert!(!blocker.should_block(&url, ResourceType::Script));
    }

    #[test]
    fn statistics_tracking() {
        let mut blocker = ContentBlocker::new();
        blocker.block_domain("ads.com");

        let blocked_url = Url::parse("https://ads.com/ad").unwrap();
        let allowed_url = Url::parse("https://example.com/page").unwrap();

        blocker.record_decision(&blocked_url, ResourceType::Script);
        blocker.record_decision(&allowed_url, ResourceType::Document);

        assert_eq!(blocker.stats().checked, 2);
        assert_eq!(blocker.stats().blocked, 1);
        assert_eq!(blocker.stats().allowed, 1);

        blocker.reset_stats();
        assert_eq!(blocker.stats().checked, 0);
    }
}
