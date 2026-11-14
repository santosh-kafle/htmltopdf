//! PDF output.
//!
//! Walks the per-page fragment tree produced by `presswork-layout` and emits
//! PDF content: text runs, borders, backgrounds, images, links and the
//! document outline (bookmarks).

use presswork_layout::{Orientation, PageSize};

/// Document-level options for the generated PDF.
#[derive(Debug, Clone, Default)]
pub struct PdfOptions {
    /// Default page size when the document has no `@page { size }` rule.
    pub page_size: PageSize,
    pub orientation: Orientation,
    /// `Title` entry in the PDF metadata.
    pub title: Option<String>,
    /// `Author` entry in the PDF metadata.
    pub author: Option<String>,
}
