//! DOM tree types and operations.
//!
//! Parses HTML into a tree of [`Node`] values using html5ever,
//! and provides traversal and query methods.

pub mod node;
pub mod tree;

pub use node::{ElementData, Node, NodeData};
pub use tree::Document;
