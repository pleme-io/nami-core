//! CSS parsing and cascade resolution.
//!
//! Uses lightningcss for stylesheet parsing and provides style resolution
//! against a DOM tree.

pub mod cascade;

pub use cascade::{ComputedStyle, StyleResolver, StyleSheet, StyledNode, StyledTree};
