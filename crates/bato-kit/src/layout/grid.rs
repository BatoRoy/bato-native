//! CSS grid with column tracks, `col-span`, auto-flow rows and stretched
//! cells (`align-items: stretch`), which iced's `row` cannot express (a
//! `Fill`-height child is squeezed instead).

use iced::advanced::layout;
use iced::advanced::widget::Widget;
use iced::{Element, Length, Point, Size};

use super::{forward_cells, into_element};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// A column track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Track {
    /// A fixed width in px (`3rem` = 48).
    Px(f32),
    /// A share of the free space (`1fr`).
    Fr(f32),
}

#[derive(Debug, Clone, PartialEq)]
enum Columns {
    Tracks(Vec<Track>),
    /// `repeat(auto-fill, minmax(min, 1fr))`
    AutoFill(f32),
}

pub struct Grid<'a, Message> {
    cells: Vec<El<'a, Message>>,
    spans: Vec<usize>,
    columns: Columns,
    gap_x: f32,
    gap_y: f32,
    equal_rows: bool,
}

/// `grid grid-cols-N gap-G` with one cell per column (a single row of equal,
/// equally tall cells; the Home page's two cards).
pub fn grid<'a, Message>(cells: Vec<El<'a, Message>>, gap: f32) -> Grid<'a, Message> {
    let n = cells.len().max(1);
    let spans = vec![1; cells.len()];
    Grid { cells, spans, columns: Columns::Tracks(vec![Track::Fr(1.0); n]), gap_x: gap, gap_y: gap, equal_rows: false }
}

impl<'a, Message> Grid<'a, Message> {
    /// `grid-cols-N`: N equal columns, no cells yet.
    pub fn columns(n: usize) -> Self {
        Self::tracks(vec![Track::Fr(1.0); n.max(1)])
    }

    /// `grid-cols-[…]`: explicit tracks, e.g. `[Px(48.0), Fr(1.0) × 7]`.
    pub fn tracks(tracks: Vec<Track>) -> Self {
        Grid { cells: Vec::new(), spans: Vec::new(), columns: Columns::Tracks(tracks), gap_x: 0.0, gap_y: 0.0, equal_rows: false }
    }

    /// AutoGrid: `repeat(auto-fill, minmax(min, 1fr))`.
    pub fn auto_fill(min: f32) -> Self {
        Grid { cells: Vec::new(), spans: Vec::new(), columns: Columns::AutoFill(min), gap_x: 0.0, gap_y: 0.0, equal_rows: false }
    }

    /// `gap-G` (both axes).
    pub fn gap(mut self, g: f32) -> Self {
        self.gap_x = g;
        self.gap_y = g;
        self
    }
    pub fn gap_x(mut self, g: f32) -> Self {
        self.gap_x = g;
        self
    }
    pub fn gap_y(mut self, g: f32) -> Self {
        self.gap_y = g;
        self
    }
    /// `auto-rows-fr`: every row as tall as the tallest.
    pub fn equal_rows(mut self) -> Self {
        self.equal_rows = true;
        self
    }
    pub fn push(self, cell: impl Into<El<'a, Message>>) -> Self {
        self.push_span(cell, 1)
    }
    /// A cell spanning `span` columns (`col-span-2`; clamped to the grid).
    pub fn push_span(mut self, cell: impl Into<El<'a, Message>>, span: usize) -> Self {
        self.cells.push(cell.into());
        self.spans.push(span.max(1));
        self
    }
}

/// Column widths for a total width.
fn column_widths(columns: &Columns, total: f32, gap: f32) -> Vec<f32> {
    let tracks = match columns {
        Columns::Tracks(t) => t.clone(),
        Columns::AutoFill(min) => {
            let n = (((total + gap) / (min + gap)).floor() as usize).max(1);
            vec![Track::Fr(1.0); n]
        }
    };
    let n = tracks.len().max(1);
    let fixed: f32 = tracks.iter().map(|t| if let Track::Px(p) = t { *p } else { 0.0 }).sum();
    let frs: f32 = tracks.iter().map(|t| if let Track::Fr(f) = t { *f } else { 0.0 }).sum();
    let free = (total - gap * (n as f32 - 1.0) - fixed).max(0.0);
    tracks
        .iter()
        .map(|t| match t {
            Track::Px(p) => *p,
            Track::Fr(f) if frs > 0.0 => free * f / frs,
            Track::Fr(_) => 0.0,
        })
        .collect()
}

/// Auto-flow placement: (row, first column, span) per cell.
fn place(spans: &[usize], cols: usize) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::with_capacity(spans.len());
    let (mut row, mut col) = (0, 0);
    for &s in spans {
        let s = s.min(cols);
        if col + s > cols {
            row += 1;
            col = 0;
        }
        out.push((row, col, s));
        col += s;
    }
    out
}

/// Running offsets of consecutive extents separated by `gap`.
fn offsets(extents: &[f32], gap: f32) -> Vec<f32> {
    let mut at = 0.0;
    extents
        .iter()
        .map(|e| {
            let x = at;
            at += e + gap;
            x
        })
        .collect()
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for Grid<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(&mut self, tree: &mut iced::advanced::widget::Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        let total = limits.max().width;
        let widths = column_widths(&self.columns, total, self.gap_x);
        let xs = offsets(&widths, self.gap_x);
        let placed = place(&self.spans, widths.len());
        let gap_x = self.gap_x;
        let span_w = |c: usize, s: usize| widths[c..c + s].iter().sum::<f32>() + gap_x * (s as f32 - 1.0);
        let rows = placed.last().map_or(0, |p| p.0 + 1);
        let mut row_h = vec![0.0f32; rows];
        for ((cell, t), &(r, c, s)) in self.cells.iter_mut().zip(tree.children.iter_mut()).zip(&placed) {
            let w = span_w(c, s);
            let node = cell.as_widget_mut().layout(t, renderer, &layout::Limits::new(Size::new(w, 0.0), Size::new(w, f32::INFINITY)));
            row_h[r] = row_h[r].max(node.size().height);
        }
        if self.equal_rows {
            let m = row_h.iter().copied().fold(0.0, f32::max);
            row_h.iter_mut().for_each(|h| *h = m);
        }
        let ys = offsets(&row_h, self.gap_y);
        let mut nodes = Vec::with_capacity(self.cells.len());
        for ((cell, t), &(r, c, s)) in self.cells.iter_mut().zip(tree.children.iter_mut()).zip(&placed) {
            let (w, h) = (span_w(c, s), row_h[r]);
            let node = cell.as_widget_mut().layout(t, renderer, &layout::Limits::new(Size::new(w, h), Size::new(w, h)));
            nodes.push(node.move_to(Point::new(xs[c], ys[r])));
        }
        let height = if rows == 0 { 0.0 } else { ys[rows - 1] + row_h[rows - 1] };
        layout::Node::with_children(Size::new(total, height), nodes)
    }

    forward_cells!();
}

into_element!(Grid);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_columns_with_gap() {
        assert_eq!(column_widths(&Columns::Tracks(vec![Track::Fr(1.0); 2]), 816.0, 16.0), vec![400.0, 400.0]);
    }

    #[test]
    fn fixed_and_fr_tracks() {
        // grid-cols-[3rem_repeat(7,1fr)] at 400 px, no gap.
        let mut t = vec![Track::Px(48.0)];
        t.extend([Track::Fr(1.0); 7]);
        let w = column_widths(&Columns::Tracks(t), 400.0, 0.0);
        assert_eq!(w[0], 48.0);
        assert!((w[1] - 352.0 / 7.0).abs() < 1e-4);
    }

    #[test]
    fn auto_fill_columns() {
        // minmax(140px, 1fr) with gap 12 in 600 px: 4 columns.
        assert_eq!(column_widths(&Columns::AutoFill(140.0), 600.0, 12.0).len(), 4);
        assert_eq!(column_widths(&Columns::AutoFill(140.0), 100.0, 12.0).len(), 1);
    }

    #[test]
    fn spans_wrap_to_the_next_row() {
        assert_eq!(place(&[1, 1, 2, 1], 2), vec![(0, 0, 1), (0, 1, 1), (1, 0, 2), (2, 0, 1)]);
        assert_eq!(place(&[1, 2], 2), vec![(0, 0, 1), (1, 0, 2)]);
        assert_eq!(place(&[5], 2), vec![(0, 0, 2)]);
    }
}
