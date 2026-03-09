//! HTTP request pipeline for fetching web resources.
//!
//! Provides a minimal fetch client. When the `network` feature is enabled,
//! this will use todoku for authenticated HTTP with retry logic.
//! Without the feature, this provides types and a stub implementation.

use std::collections::HashMap;

use url::Url;

/// Errors that can occur during network operations.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// Invalid URL.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    /// Network/IO error.
    #[error("network error: {0}")]
    NetworkError(String),

    /// HTTP error response.
    #[error("HTTP {status}: {message}")]
    HttpError {
        /// HTTP status code.
        status: u16,
        /// Error message or status text.
        message: String,
    },

    /// Request was blocked by the content blocker.
    #[error("request blocked: {0}")]
    Blocked(String),

    /// Request timed out.
    #[error("request timed out after {0}ms")]
    Timeout(u64),
}

/// An HTTP response.
#[derive(Debug, Clone)]
pub struct Response {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: HashMap<String, String>,
    /// Response body as bytes.
    pub body: Vec<u8>,
    /// The final URL (after redirects).
    pub url: Url,
}

impl Response {
    /// Get the response body as a UTF-8 string.
    ///
    /// Returns `None` if the body is not valid UTF-8.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.body).ok()
    }

    /// Get the Content-Type header value.
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.headers
            .get("content-type")
            .or_else(|| self.headers.get("Content-Type"))
            .map(String::as_str)
    }

    /// Check if the response indicates success (2xx status).
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Check if the response content type is HTML.
    #[must_use]
    pub fn is_html(&self) -> bool {
        self.content_type()
            .is_some_and(|ct| ct.contains("text/html"))
    }

    /// Check if the response content type is CSS.
    #[must_use]
    pub fn is_css(&self) -> bool {
        self.content_type()
            .is_some_and(|ct| ct.contains("text/css"))
    }
}

/// Configuration for the fetch client.
#[derive(Debug, Clone)]
pub struct FetchConfig {
    /// User-Agent header value.
    pub user_agent: String,
    /// Request timeout in milliseconds.
    pub timeout_ms: u64,
    /// Maximum number of redirects to follow.
    pub max_redirects: u32,
    /// Whether to accept cookies.
    pub accept_cookies: bool,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            user_agent: format!("nami-core/{}", env!("CARGO_PKG_VERSION")),
            timeout_ms: 30_000,
            max_redirects: 10,
            accept_cookies: false,
        }
    }
}

/// HTTP client for fetching web resources.
#[derive(Debug, Clone)]
pub struct FetchClient {
    config: FetchConfig,
}

impl FetchClient {
    /// Create a new fetch client with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: FetchConfig::default(),
        }
    }

    /// Create a new fetch client with the given configuration.
    #[must_use]
    pub fn with_config(config: FetchConfig) -> Self {
        Self { config }
    }

    /// Get the current configuration.
    #[must_use]
    pub fn config(&self) -> &FetchConfig {
        &self.config
    }

    /// Fetch a URL and return the response.
    ///
    /// # Errors
    ///
    /// Returns `FetchError` if the request fails, times out, or the server
    /// returns an error status.
    ///
    /// Note: This is a placeholder that returns an error until a real HTTP
    /// backend (todoku or reqwest) is wired up. Enable the `network` feature
    /// to use todoku, or consumers can implement their own fetch and construct
    /// `Response` values directly.
    pub fn fetch(&self, url: &Url) -> Result<Response, FetchError> {
        // Validate scheme.
        match url.scheme() {
            "http" | "https" => {}
            scheme => {
                return Err(FetchError::InvalidUrl(format!(
                    "unsupported scheme: {scheme}"
                )));
            }
        }

        // Without a real HTTP backend, return an error indicating the
        // feature is not available. Consumers should either:
        // 1. Enable the `network` feature for todoku integration
        // 2. Use their own HTTP client and construct Response values
        Err(FetchError::NetworkError(
            "no HTTP backend configured — enable the `network` feature or provide responses directly".to_string(),
        ))
    }

    /// Create a `Response` from raw parts (for consumers that bring their own HTTP client).
    #[must_use]
    pub fn response_from_parts(
        status: u16,
        headers: HashMap<String, String>,
        body: Vec<u8>,
        url: Url,
    ) -> Response {
        Response {
            status,
            headers,
            body,
            url,
        }
    }

    /// Resolve a potentially relative URL against a base URL.
    ///
    /// # Errors
    ///
    /// Returns `FetchError::InvalidUrl` if the URL cannot be resolved.
    pub fn resolve_url(base: &Url, href: &str) -> Result<Url, FetchError> {
        base.join(href)
            .map_err(|e| FetchError::InvalidUrl(format!("{e}")))
    }
}

impl Default for FetchClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_text_extraction() {
        let resp = Response {
            status: 200,
            headers: HashMap::from([("content-type".to_string(), "text/html".to_string())]),
            body: b"<html>Hello</html>".to_vec(),
            url: Url::parse("https://example.com").unwrap(),
        };
        assert_eq!(resp.text(), Some("<html>Hello</html>"));
        assert!(resp.is_success());
        assert!(resp.is_html());
        assert!(!resp.is_css());
    }

    #[test]
    fn response_status_checks() {
        let ok = Response {
            status: 200,
            headers: HashMap::new(),
            body: Vec::new(),
            url: Url::parse("https://example.com").unwrap(),
        };
        assert!(ok.is_success());

        let not_found = Response {
            status: 404,
            headers: HashMap::new(),
            body: Vec::new(),
            url: Url::parse("https://example.com/missing").unwrap(),
        };
        assert!(!not_found.is_success());
    }

    #[test]
    fn fetch_unsupported_scheme() {
        let client = FetchClient::new();
        let url = Url::parse("ftp://example.com").unwrap();
        let result = client.fetch(&url);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), FetchError::InvalidUrl(_)));
    }

    #[test]
    fn resolve_relative_url() {
        let base = Url::parse("https://example.com/page/1").unwrap();
        let resolved = FetchClient::resolve_url(&base, "/about").unwrap();
        assert_eq!(resolved.as_str(), "https://example.com/about");

        let resolved = FetchClient::resolve_url(&base, "next").unwrap();
        assert_eq!(resolved.as_str(), "https://example.com/page/next");
    }

    #[test]
    fn default_config() {
        let config = FetchConfig::default();
        assert!(config.user_agent.starts_with("nami-core/"));
        assert_eq!(config.timeout_ms, 30_000);
        assert_eq!(config.max_redirects, 10);
        assert!(!config.accept_cookies);
    }

    #[test]
    fn response_from_parts_constructor() {
        let resp = FetchClient::response_from_parts(
            200,
            HashMap::new(),
            b"body".to_vec(),
            Url::parse("https://example.com").unwrap(),
        );
        assert_eq!(resp.status, 200);
        assert_eq!(resp.text(), Some("body"));
    }
}
