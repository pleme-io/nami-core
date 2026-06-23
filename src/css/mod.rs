//! CSS parsing and cascade resolution.
//!
//! Uses lightningcss for stylesheet parsing and provides style resolution
//! against a DOM tree.

pub mod cascade;
pub mod selector;
pub mod values;

pub use cascade::{ComputedStyle, LengthProp, StyleResolver, StyleSheet, StyledNode, StyledTree};
pub use selector::{CompoundSelector, parse_selector_list};
pub use values::{Color, Display, Length, LengthContext};
