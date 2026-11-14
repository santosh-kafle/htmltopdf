//! Box tree construction, layout and pagination.
//!
//! Pipeline stages owned by this crate:
//!
//! 1. **Box generation**: turn styled DOM nodes into a box tree, inserting
//!    anonymous boxes and `::before` / `::after` / list-marker content.
//! 2. **Layout**: block, inline (text), table, flex and grid formatting.
//! 3. **Fragmentation**: split the laid-out content across pages, honoring
//!    `break-*` properties, orphans and widows.
//!
//! The output is a fragment tree per page, consumed by `presswork-paint`.
//! All geometry is expressed in typographic points (1pt = 1/72 in).

pub mod geometry;
pub mod page;

pub use geometry::{EdgeSizes, Point, Rect, Size};
pub use page::{Orientation, PageSize};
