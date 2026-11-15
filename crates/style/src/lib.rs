//! CSS parsing, cascade and computed styles.
//!
//! Responsibilities of this crate:
//!
//! 1. Parse author stylesheets (`<style>`, `<link>`, `style=""`) and the
//!    built-in user-agent stylesheet.
//! 2. Match selectors against `presswork_dom::Document` nodes.
//! 3. Run the cascade (origin, specificity, source order, `!important`) and
//!    inheritance to produce a computed style for every element.

//!
//! Today only the built-in defaults in [`computed`] exist.

pub mod computed;
pub mod values;

pub use computed::{ComputedStyle, Display, Edges, FontFamily, TextAlign};
pub use values::{Color, Length};
