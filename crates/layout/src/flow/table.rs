//! Table layout.
//!
//! A simplified version of the automatic table layout algorithm (CSS 2.1
//! section 17.5.2.2): columns are sized from the min- and max-content
//! widths of their cells, then each row is laid out as one unbreakable
//! slice. Header rows repeat at the top of every page the table continues
//! on.
//!
//! Not supported yet: cells spanning rows are placed correctly but only
//! occupy their first row; `<col>` widths; `height` on the table and row
//! groups (cells and rows take it as a minimum); the full collapsed
//! border model (shared borders are drawn once, without conflict
//! resolution).

use std::rc::Rc;

use presswork_dom::NodeId;
use presswork_style::{
    BorderCollapse, BoxSizing, ComputedStyle, Dimension, Display, Side, Sides, VerticalAlign,
};

use super::boxes::BoxGeometry;
use super::{BoxPaint, Flow, RepeatedHeader};
use crate::fragment::{Fill, Item};
use crate::geometry::Rect;

struct Cell {
    node: NodeId,
    style: ComputedStyle,
    /// First column the cell occupies.
    column: usize,
    /// Number of columns it spans.
    span: usize,
    /// Number of rows it reserves columns for.
    row_span: usize,
    border: Sides<f32>,
    padding: Sides<f32>,
    /// Min- and max-content width of the border box.
    min: f32,
    max: f32,
}

impl Cell {
    fn horizontal_edges(&self) -> f32 {
        self.border.left + self.border.right + self.padding.left + self.padding.right
    }

    fn vertical_edges(&self) -> f32 {
        self.border.top + self.border.bottom + self.padding.top + self.padding.bottom
    }

    /// The least border-box height its `height` asks for: as in browsers,
    /// a cell (and a row) grows to fit taller content. `min-height` and
    /// `max-height` do not apply to cells.
    fn min_height(&self) -> f32 {
        match (self.style.height, self.style.box_sizing) {
            (Dimension::Pt(height), BoxSizing::ContentBox) => height + self.vertical_edges(),
            (Dimension::Pt(height), BoxSizing::BorderBox) => height,
            _ => 0.0,
        }
    }
}

struct Row {
    style: ComputedStyle,
    cells: Vec<Cell>,
    /// The row belongs to a header group and repeats on each page.
    header: bool,
}

/// Column positions and widths, relative to the table's content box.
struct Grid {
    /// Left edge of each column.
    starts: Vec<f32>,
    widths: Vec<f32>,
    spacing: f32,
}

impl Grid {
    fn new(widths: Vec<f32>, spacing: f32) -> Self {
        let mut x = spacing;
        let starts = widths
            .iter()
            .map(|width| {
                let start = x;
                x += width + spacing;
                start
            })
            .collect();
        Self {
            starts,
            widths,
            spacing,
        }
    }

    /// The width of `span` columns starting at `column`, including the
    /// spacing between them.
    fn span_width(&self, column: usize, span: usize) -> f32 {
        let end = (column + span).min(self.widths.len());
        let columns = &self.widths[column.min(end)..end];
        columns.iter().sum::<f32>() + self.spacing * columns.len().saturating_sub(1) as f32
    }

    /// The width of all columns and the spacing around them.
    fn total_width(&self) -> f32 {
        self.widths.iter().sum::<f32>() + self.spacing * (self.widths.len() as f32 + 1.0)
    }
}

/// Which outer borders of the table are drawn, for collapsing cell
/// borders into them.
#[derive(Clone, Copy)]
struct TableEdges {
    collapse: bool,
    top: bool,
    right: bool,
    bottom: bool,
    left: bool,
}

impl Flow<'_, '_> {
    /// Lays out the table `id` in a containing block whose content box
    /// starts at `x` and is `width` wide.
    pub(super) fn table(&mut self, id: NodeId, style: &ComputedStyle, x: f32, width: f32) {
        let (captions, mut rows) = self.collect_rows(id, style);
        for (caption, caption_style) in &captions {
            self.block(*caption, caption_style, x, width);
        }

        let columns = assign_columns(&mut rows);
        for cell in rows.iter_mut().flat_map(|row| &mut row.cells) {
            self.measure_cell(cell);
        }

        let collapse = style.border_collapse == BorderCollapse::Collapse;
        let spacing = if collapse { 0.0 } else { style.border_spacing };
        let gaps = spacing * (columns as f32 + 1.0);
        let (min, max, percent) = column_constraints(&rows, columns, spacing);
        let (min_total, max_total) = (
            min.iter().sum::<f32>() + gaps,
            max.iter().sum::<f32>() + gaps,
        );

        // The table's own box. Its content width is the grid's width.
        let probe = BoxGeometry::resolve(style, width, Some(0.0));
        let edges = probe.border_box_width();
        let specified = BoxGeometry::specified_content_width(style, width, edges);
        if self.is_measuring() {
            let outer = edges
                + style.margin.left.resolve(0.0).unwrap_or(0.0)
                + style.margin.right.resolve(0.0).unwrap_or(0.0);
            let fixed = match style.width {
                Dimension::Pt(_) => specified,
                _ => None,
            };
            self.record_extent(
                outer + fixed.unwrap_or(min_total).max(min_total),
                outer + fixed.unwrap_or(max_total).max(min_total),
            );
            return;
        }
        let margins = style.margin.left.resolve(width).unwrap_or(0.0)
            + style.margin.right.resolve(width).unwrap_or(0.0);
        let available = (width - margins - edges).max(0.0);
        let content_width = match specified {
            Some(specified) => specified.max(min_total),
            None => max_total.min(available).max(min_total),
        };
        let grid = Grid::new(
            distribute(&min, &max, &percent, content_width - gaps),
            spacing,
        );
        let geometry = BoxGeometry::resolve(style, width, Some(grid.total_width()));
        let content_x = x + geometry.content_x();

        let paint = BoxPaint::from_style(style);
        let has = |side| paint.is_some_and(|paint| paint.has_border(side));
        let table_edges = TableEdges {
            collapse,
            top: has(Side::Top),
            right: has(Side::Right),
            bottom: has(Side::Bottom),
            left: has(Side::Left),
        };

        let open = self.open_box(style, &geometry, x, false);
        self.settle_margin();
        self.y += f64::from(spacing);

        let mut header_items = Vec::new();
        let mut header_height = 0.0;
        let mut header: Option<Rc<RepeatedHeader>> = None;
        let last = rows.len().saturating_sub(1);
        let mut previous_has_bottom_border = false;
        for (index, row) in rows.iter().enumerate() {
            let position = RowPosition {
                first: index == 0,
                last: index == last,
                below_border: previous_has_bottom_border,
            };
            let (height, items) = self.layout_row(row, &grid, content_x, table_edges, position);
            previous_has_bottom_border = row.cells.iter().any(|cell| cell.border.bottom > 0.0);

            if row.header {
                header_items.extend(
                    items
                        .iter()
                        .cloned()
                        .map(|item| item.translate(0.0, header_height)),
                );
                header_height += height + spacing;
            } else if header.is_none() && header_height > 0.0 {
                header = Some(Rc::new(RepeatedHeader {
                    height: header_height,
                    items: std::mem::take(&mut header_items),
                }));
            }

            self.push_slice(self.y, height, items);
            if !row.header
                && let Some(slice) = self.canvas.slices.last_mut()
            {
                slice.header.clone_from(&header);
            }
            self.y += f64::from(height + spacing);
        }

        self.close_box(style, &geometry, open);
    }

    /// Finds the table's captions and rows. Rows in header groups come
    /// first and rows in footer groups last, whatever the source order.
    #[allow(clippy::type_complexity, reason = "a private pair of lists")]
    fn collect_rows(
        &mut self,
        table: NodeId,
        style: &ComputedStyle,
    ) -> (Vec<(NodeId, ComputedStyle)>, Vec<Row>) {
        let doc = self.cx.doc;
        let styles = self.cx.styles;
        let mut captions = Vec::new();
        let (mut head, mut body, mut foot) = (Vec::new(), Vec::new(), Vec::new());

        let make_row = |row: NodeId, row_style: ComputedStyle, header: bool| {
            let cells = doc
                .children(row)
                .filter_map(|cell| {
                    let element = doc.element(cell)?;
                    let cell_style = styles.style(doc, cell, Some(&row_style));
                    if cell_style.display != Display::TableCell {
                        return None;
                    }
                    let span = |name| {
                        element
                            .attr(name)
                            .and_then(|s| s.trim().parse::<usize>().ok())
                            .unwrap_or(1)
                            .clamp(1, 1000)
                    };
                    Some(Cell {
                        node: cell,
                        column: 0,
                        span: span("colspan"),
                        row_span: span("rowspan"),
                        border: cell_style.border_widths(),
                        // Percentage padding on cells is not supported.
                        padding: cell_style.padding.map(|p| p.resolve(0.0).unwrap_or(0.0)),
                        min: 0.0,
                        max: 0.0,
                        style: cell_style,
                    })
                })
                .collect();
            Row {
                style: row_style,
                cells,
                header,
            }
        };

        for child in doc.children(table) {
            if doc.element(child).is_none() {
                continue;
            }
            let child_style = styles.style(doc, child, Some(style));
            match child_style.display {
                Display::TableCaption => captions.push((child, child_style)),
                Display::TableRow => body.push(make_row(child, child_style, false)),
                Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup => {
                    let header = child_style.display == Display::TableHeaderGroup;
                    let group = match child_style.display {
                        Display::TableHeaderGroup => &mut head,
                        Display::TableFooterGroup => &mut foot,
                        _ => &mut body,
                    };
                    for row in doc.children(child) {
                        if doc.element(row).is_none() {
                            continue;
                        }
                        let row_style = styles.style(doc, row, Some(&child_style));
                        if row_style.display == Display::TableRow {
                            group.push(make_row(row, row_style, header));
                        }
                    }
                }
                _ => {}
            }
        }

        head.extend(body);
        head.extend(foot);
        (captions, head)
    }

    /// Measures the min- and max-content width of a cell's border box.
    fn measure_cell(&mut self, cell: &mut Cell) {
        let mut flow = Flow::measuring(self.cx);
        flow.block_contents(cell.node, &cell.style, 0.0, 0.0);
        let content = flow.extent();
        let edges = cell.horizontal_edges();
        cell.min = content.min + edges;
        cell.max = content.max + edges;
        if let Dimension::Pt(width) = cell.style.width {
            let width = match cell.style.box_sizing {
                BoxSizing::ContentBox => width + edges,
                BoxSizing::BorderBox => width,
            };
            // A specified width is the preferred width, but never
            // narrower than the content can go.
            cell.max = width.max(cell.min);
        }
    }

    /// Lays out one row. Returns its height and its items, positioned
    /// relative to the canvas's left edge and the row's top.
    fn layout_row(
        &mut self,
        row: &Row,
        grid: &Grid,
        content_x: f32,
        edges: TableEdges,
        position: RowPosition,
    ) -> (f32, Vec<Item>) {
        // Lay out every cell first: the tallest sets the row's height.
        let mut laid_out = Vec::with_capacity(row.cells.len());
        let mut height = 0.0_f32;
        for cell in &row.cells {
            let width = grid.span_width(cell.column, cell.span);
            let mut flow = Flow::new(self.cx);
            flow.block_contents(
                cell.node,
                &cell.style,
                0.0,
                (width - cell.horizontal_edges()).max(0.0),
            );
            let (canvas, content_height) = flow.finish();
            let content_height = content_height as f32;
            height = height
                .max(content_height + cell.vertical_edges())
                .max(cell.min_height());
            laid_out.push((canvas, content_height, width));
        }

        if let Dimension::Pt(min) = row.style.height {
            height = height.max(min);
        }

        let mut items = Vec::new();
        if !row.style.background_color.is_transparent() && !grid.widths.is_empty() {
            let width = grid.total_width() - 2.0 * grid.spacing;
            items.push(Item::Fill(Fill {
                rect: Rect::new(content_x + grid.spacing, 0.0, width, height),
                color: row.style.background_color,
            }));
        }

        let last_column = grid.widths.len();
        let mut previous_has_right_border = false;
        for (cell, (canvas, content_height, width)) in row.cells.iter().zip(laid_out) {
            let cell_x = content_x + grid.starts.get(cell.column).copied().unwrap_or(0.0);

            if let Some(mut paint) = BoxPaint::from_style(&cell.style) {
                if edges.collapse {
                    // Borders shared with a neighbor or the table are
                    // drawn once, by whoever comes first.
                    let at_left = cell.column == 0;
                    let at_right = cell.column + cell.span >= last_column;
                    let hidden = [
                        (
                            Side::Left,
                            if at_left {
                                edges.left
                            } else {
                                previous_has_right_border
                            },
                        ),
                        (
                            Side::Top,
                            if position.first {
                                edges.top
                            } else {
                                position.below_border
                            },
                        ),
                        (Side::Right, at_right && edges.right),
                        (Side::Bottom, position.last && edges.bottom),
                    ];
                    for (side, hide) in hidden {
                        if hide {
                            paint = paint.without_border(side);
                        }
                    }
                }
                paint.fills(Rect::new(cell_x, 0.0, width, height), &mut items);
            }
            previous_has_right_border = cell.border.right > 0.0;

            let free = (height - cell.vertical_edges() - content_height).max(0.0);
            let shift = match cell.style.vertical_align {
                VerticalAlign::Middle => free / 2.0,
                VerticalAlign::Bottom => free,
                // Baseline alignment across cells is not implemented.
                _ => 0.0,
            };
            canvas.into_items(
                cell_x + cell.border.left + cell.padding.left,
                cell.border.top + cell.padding.top + shift,
                &mut items,
            );
        }
        (height, items)
    }
}

/// Where a row sits in its table, for collapsed borders.
#[derive(Clone, Copy)]
struct RowPosition {
    first: bool,
    last: bool,
    /// The row above has a bottom border.
    below_border: bool,
}

/// Assigns each cell its first column, skipping columns still occupied by
/// cells spanning rows from above. Returns the number of columns.
fn assign_columns(rows: &mut [Row]) -> usize {
    // For each column, how many more rows a cell from above occupies it.
    let mut occupied: Vec<usize> = Vec::new();
    for row in rows.iter_mut() {
        let mut column = 0;
        for cell in &mut row.cells {
            while occupied.get(column).is_some_and(|&rows| rows > 0) {
                column += 1;
            }
            cell.column = column;
            if occupied.len() < column + cell.span {
                occupied.resize(column + cell.span, 0);
            }
            occupied[column..column + cell.span].fill(cell.row_span);
            column += cell.span;
        }
        for rows in &mut occupied {
            *rows = rows.saturating_sub(1);
        }
    }
    occupied.len()
}

/// The min-content width, max-content width and percentage width of each
/// column.
fn column_constraints(
    rows: &[Row],
    columns: usize,
    spacing: f32,
) -> (Vec<f32>, Vec<f32>, Vec<Option<f32>>) {
    let mut min = vec![0.0_f32; columns];
    let mut max = vec![0.0_f32; columns];
    let mut percent = vec![None; columns];
    let cells = || rows.iter().flat_map(|row| &row.cells);

    for cell in cells().filter(|cell| cell.span == 1) {
        let column = cell.column;
        min[column] = min[column].max(cell.min);
        max[column] = max[column].max(cell.max);
        if let Dimension::Percent(fraction) = cell.style.width {
            percent[column] = Some(percent[column].unwrap_or(0.0_f32).max(fraction));
        }
    }
    // A spanning cell widens its columns equally if they are too narrow.
    for cell in cells().filter(|cell| cell.span > 1) {
        let range = cell.column..(cell.column + cell.span).min(columns);
        let count = range.len() as f32;
        if count == 0.0 {
            continue;
        }
        let between = spacing * (count - 1.0);
        for (widths, needed) in [(&mut min, cell.min), (&mut max, cell.max)] {
            let current = widths[range.clone()].iter().sum::<f32>() + between;
            if needed > current {
                let extra = (needed - current) / count;
                widths[range.clone()].iter_mut().for_each(|w| *w += extra);
            }
        }
    }
    for (max, min) in max.iter_mut().zip(&min) {
        *max = max.max(*min);
    }
    (min, max, percent)
}

/// Divides `available` width among columns.
///
/// Percentage columns get their share first. The rest get their
/// max-content width if everything fits (sharing any surplus in proportion
/// to it), otherwise a width between min- and max-content, and never less
/// than min-content.
fn distribute(min: &[f32], max: &[f32], percent: &[Option<f32>], available: f32) -> Vec<f32> {
    let mut widths = vec![0.0; min.len()];
    let mut remaining = available;
    for (i, fraction) in percent.iter().enumerate() {
        if let Some(fraction) = fraction {
            widths[i] = (fraction * available).max(min[i]);
            remaining -= widths[i];
        }
    }

    let flexible: Vec<usize> = (0..min.len()).filter(|&i| percent[i].is_none()).collect();
    if flexible.is_empty() {
        return widths;
    }
    let min_total: f32 = flexible.iter().map(|&i| min[i]).sum();
    let max_total: f32 = flexible.iter().map(|&i| max[i]).sum();
    for &i in &flexible {
        widths[i] = if remaining >= max_total {
            let surplus = remaining - max_total;
            let share = if max_total > 0.0 {
                max[i] / max_total
            } else {
                1.0 / flexible.len() as f32
            };
            max[i] + surplus * share
        } else if remaining > min_total {
            let t = (remaining - min_total) / (max_total - min_total);
            min[i] + (max[i] - min[i]) * t
        } else {
            min[i]
        };
    }
    widths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.01)
    }

    #[test]
    fn columns_get_max_content_when_it_fits() {
        let widths = distribute(&[10.0, 20.0], &[30.0, 60.0], &[None, None], 90.0);
        assert!(approx(&widths, &[30.0, 60.0]), "{widths:?}");
    }

    #[test]
    fn surplus_is_shared_in_proportion() {
        let widths = distribute(&[10.0, 20.0], &[30.0, 60.0], &[None, None], 180.0);
        assert!(approx(&widths, &[60.0, 120.0]), "{widths:?}");
    }

    #[test]
    fn tight_space_interpolates_between_min_and_max() {
        let widths = distribute(&[10.0, 20.0], &[30.0, 60.0], &[None, None], 60.0);
        assert!(approx(&widths, &[20.0, 40.0]), "{widths:?}");
    }

    #[test]
    fn columns_never_shrink_below_min_content() {
        let widths = distribute(&[10.0, 20.0], &[30.0, 60.0], &[None, None], 5.0);
        assert!(approx(&widths, &[10.0, 20.0]), "{widths:?}");
    }

    #[test]
    fn percentage_columns_take_their_share_first() {
        let widths = distribute(&[10.0, 10.0], &[50.0, 50.0], &[Some(0.75), None], 200.0);
        assert!(approx(&widths, &[150.0, 50.0]), "{widths:?}");
    }

    #[test]
    fn grid_positions_columns_with_spacing() {
        let grid = Grid::new(vec![10.0, 20.0, 30.0], 2.0);
        assert_eq!(grid.starts, [2.0, 14.0, 36.0]);
        assert_eq!(grid.span_width(0, 2), 32.0);
        assert_eq!(grid.span_width(2, 5), 30.0);
        assert_eq!(grid.total_width(), 68.0);
    }
}
