//! CSS parsing, cascade and computed styles.
//!
//! Responsibilities of this crate:
//!
//! 1. Parse author stylesheets (`<style>`, `<link>`, `style=""`) and the
//!    built-in user-agent stylesheet.
//! 2. Match selectors against `presswork_dom::Document` nodes.
//! 3. Run the cascade (origin, specificity, source order, `!important`) and
//!    inheritance to produce a computed style for every element.

pub mod values;

pub use values::{Color, Length};
