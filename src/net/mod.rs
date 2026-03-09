//! Network request pipeline.
//!
//! Provides HTTP fetching for loading web pages, stylesheets, and other resources.
//! When the `network` feature is enabled, uses todoku for authenticated HTTP.
//! Without the feature, provides a minimal implementation.

pub mod fetch;

pub use fetch::{FetchClient, FetchError, Response};
