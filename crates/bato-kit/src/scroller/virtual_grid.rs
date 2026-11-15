//! A virtualized grid of tiles with variable row heights (BatoBrowse's file
//! grid, `UI-INVENTORY` §5.2.1): CSS
//! `grid-template-columns: repeat(auto-fill, minmax(min, 1fr))` with a gap,
//! rows as tall as their tallest tile, and only the rows near the viewport
//! built, on [`virtual_list`].
//!
//! The column count comes from the width the grid gets, so it is known only
//! at layout: [`VirtualGrid::on_layout`] reports it (for ↑/↓ = ±columns) when
//! it changes. Row heights come from `tile_height(index, tile_width)` (e.g.
//! the name's measured lines, [`crate::label::Label::shape`]) and are cached
//! until the width, the count or [`VirtualGrid::key`] changes.
//! [`VirtualGrid::cursor`] scrolls the keyboard cursor's tile into view
//! (`scrollIntoView({block: 'nearest'})`) when it moves.
//!
//! ```ignore
//! virtual_grid(entries.len(), 112.0, |i, w| 74.0 + name_height(&entries[i].name, w), |i| tile(&entries[i]))
//!     .gap(8.0)
//!     .key(listing_revision)
//!     .cursor(state.cursor)
//!     .on_layout(Msg::GridLayout)
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::widget::{Row, container};
use iced::{Element, Event, Length, Point, Rectangle, Size, mouse};

use super::virtual_list::{Heights, VirtualList, virtual_list};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// The grid's columns for a width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridLayout {
    pub columns: usize,
    /// Each tile's width (the `1fr` share).
    pub tile_width: f32,
    pub gap: f32,
}

impl GridLayout {
    /// `repeat(auto-fill, minmax(min, 1fr))` in `width` with `gap`: as many
    /// columns of at least `min` as fit (at least one), sharing the width.
    pub fn auto_fill(width: f32, min: f32, gap: f32) -> Self {
        let min = min.max(1.0);
        let columns = (((width + gap) / (min + gap)).floor() as usize).max(1);
        let tile_width = ((width - gap * (columns - 1) as f32) / columns as f32).max(0.0);
        Self {
            columns,
            tile_width,
            gap,
        }
    }

    /// Rows for `count` tiles.
    pub fn rows(&self, count: usize) -> usize {
        count.div_ceil(self.columns)
    }

    /// `(row, column)` of tile `index`.
    pub fn cell(&self, index: usize) -> (usize, usize) {
        (index / self.columns, index % self.columns)
    }

    /// The tiles of `row`.
    pub fn row_range(&self, row: usize, count: usize) -> std::ops::Range<usize> {
        let a = (row * self.columns).min(count);
        a..(a + self.columns).min(count)
    }

    /// Each row's height: its tallest tile's.
    pub fn row_heights(&self, count: usize, tile_height: impl Fn(usize, f32) -> f32) -> Vec<f32> {
        (0..self.rows(count))
            .map(|r| {
                self.row_range(r, count)
                    .map(|i| tile_height(i, self.tile_width))
                    .fold(0.0, f32::max)
            })
            .collect()
    }

    /// Tile `index`'s box relative to the grid's top-left, given the row
    /// heights (for scroll-into-view and marquee hit-tests).
    pub fn tile_rect(&self, index: usize, heights: &[f32]) -> Rectangle {
        let (r, c) = self.cell(index);
        let y: f32 = heights.iter().take(r).map(|h| h + self.gap).sum();
        Rectangle::new(
            Point::new(c as f32 * (self.tile_width + self.gap), y),
            Size::new(self.tile_width, heights.get(r).copied().unwrap_or(0.0)),
        )
    }

    /// The cursor after an arrow key in a grid of `count` tiles: ←/→ by
    /// one, ↑/↓ by a row (clamped, like a file manager: ↓ from the last
    /// full row lands on the last tile).
    pub fn step(&self, from: usize, count: usize, dx: i32, dy: i32) -> usize {
        if count == 0 {
            return 0;
        }
        let i = from as i64 + dx as i64 + dy as i64 * self.columns as i64;
        if dy > 0 && i >= count as i64 && self.cell(from).0 + 1 < self.rows(count) {
            return count - 1;
        }
        i.clamp(0, count as i64 - 1) as usize
    }
}

type TileFn<'a, M> = Rc<dyn Fn(usize) -> El<'a, M> + 'a>;
type HeightFn<'a> = Box<dyn Fn(usize, f32) -> f32 + 'a>;

pub struct VirtualGrid<'a, M> {
    count: usize,
    min: f32,
    gap: f32,
    overscan: f32,
    key: u64,
    tile: TileFn<'a, M>,
    tile_height: HeightFn<'a>,
    cursor: Option<usize>,
    on_layout: Option<Box<dyn Fn(GridLayout) -> M + 'a>>,
    /// Built at layout, once the width is known.
    inner: Option<VirtualList<'a, M>>,
}

/// `count` tiles at least `min` px wide; `tile_height(i, tile_width)` sizes
/// a tile, `tile(i)` builds it (only when its row nears the viewport). Tiles
/// stretch to their row's height (`align-items: stretch`).
pub fn virtual_grid<'a, M: 'a>(
    count: usize,
    min: f32,
    tile_height: impl Fn(usize, f32) -> f32 + 'a,
    tile: impl Fn(usize) -> El<'a, M> + 'a,
) -> VirtualGrid<'a, M> {
    VirtualGrid {
        count,
        min,
        gap: 0.0,
        overscan: 400.0,
        key: 0,
        tile: Rc::new(tile),
        tile_height: Box::new(tile_height),
        cursor: None,
        on_layout: None,
        inner: None,
    }
}

impl<'a, M: 'a> VirtualGrid<'a, M> {
    /// Row and column gap (`gap-N`).
    pub fn gap(mut self, px: f32) -> Self {
        self.gap = px.max(0.0);
        self
    }
    /// Rows built beyond the viewport (px, each side).
    pub fn overscan(mut self, px: f32) -> Self {
        self.overscan = px.max(0.0);
        self
    }
    /// Re-measure the rows when this changes (the listing, the name size,
    /// the tile size); the width and count are watched already.
    pub fn key(mut self, key: u64) -> Self {
        self.key = key;
        self
    }
    /// The keyboard cursor: scrolled into view (nearest) when it changes.
    pub fn cursor(mut self, index: Option<usize>) -> Self {
        self.cursor = index;
        self
    }
    /// Told the column layout when it changes (first layout included).
    pub fn on_layout(mut self, f: impl Fn(GridLayout) -> M + 'a) -> Self {
        self.on_layout = Some(Box::new(f));
        self
    }
}

/// `(count, width bits, key)`: what the row heights were measured for.
type MeasureKey = (usize, u32, u64);

#[derive(Default)]
struct State {
    measured: Option<(MeasureKey, GridLayout, Rc<Vec<f32>>)>,
    reported: Option<GridLayout>,
    /// The cursor last scrolled to (`None` until the first update: the
    /// cursor a grid mounts with is not scrolled to).
    revealed: Option<Option<usize>>,
    inner: Option<Tree>,
}

impl<'a, M: 'a> VirtualGrid<'a, M> {
    fn build(&mut self, state: &mut State, width: f32) -> (GridLayout, Rc<Vec<f32>>) {
        let k = (self.count, width.to_bits(), self.key);
        let (grid, heights) = match &state.measured {
            Some((key, g, h)) if *key == k => (*g, h.clone()),
            _ => {
                let g = GridLayout::auto_fill(width, self.min, self.gap);
                let h = Rc::new(g.row_heights(self.count, &self.tile_height));
                state.measured = Some((k, g, h.clone()));
                (g, h)
            }
        };
        let tile = self.tile.clone();
        let count = self.count;
        let hs = heights.clone();
        let row = move |r: usize| -> El<'a, M> {
            let h = hs.get(r).copied().unwrap_or(0.0);
            Row::with_children(
                grid.row_range(r, count)
                    .map(|i| container(tile(i)).width(grid.tile_width).height(h).into()),
            )
            .spacing(grid.gap)
            .into()
        };
        self.inner = Some(
            virtual_list(grid.rows(count), Heights::Each((*heights).clone()), row)
                .gap(grid.gap)
                .overscan(self.overscan),
        );
        (grid, heights)
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for VirtualGrid<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let width = limits.max().width;
        let state = tree.state.downcast_mut::<State>();
        self.build(state, width);
        let inner = self.inner.as_mut().expect("built");
        let t = state.inner.get_or_insert_with(|| {
            Tree::new(&*inner as &dyn Widget<M, iced::Theme, iced::Renderer>)
        });
        t.diff(&*inner as &dyn Widget<M, iced::Theme, iced::Renderer>);
        let n = inner.layout(t, renderer, limits);
        layout::Node::with_children(n.size(), vec![n])
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let Some((_, grid, heights)) = state.measured.clone() else {
            return;
        };
        if state.reported != Some(grid) {
            state.reported = Some(grid);
            if let Some(f) = &self.on_layout {
                shell.publish(f(grid));
            }
        }
        if state.revealed.is_none() {
            state.revealed = Some(self.cursor);
        }
        if Some(self.cursor) != state.revealed {
            state.revealed = Some(self.cursor);
            if let Some(i) = self.cursor.filter(|i| *i < self.count) {
                let b = layout.bounds();
                let r = grid.tile_rect(i, &heights);
                super::reveal(
                    Rectangle {
                        x: b.x + r.x,
                        y: b.y + r.y,
                        ..r
                    },
                    super::Reveal {
                        smooth: false,
                        ..super::Reveal::default()
                    },
                );
                shell.request_redraw();
            }
        }
        if let (Some(inner), Some(t)) = (self.inner.as_mut(), state.inner.as_mut()) {
            inner.update(
                t,
                event,
                layout.children().next().unwrap(),
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        match (&self.inner, &state.inner) {
            (Some(inner), Some(t)) => inner.mouse_interaction(
                t,
                layout.children().next().unwrap(),
                cursor,
                viewport,
                renderer,
            ),
            _ => mouse::Interaction::None,
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        if let (Some(inner), Some(t)) = (&self.inner, &state.inner) {
            inner.draw(
                t,
                renderer,
                theme,
                style,
                layout.children().next().unwrap(),
                cursor,
                viewport,
            );
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();
        if let (Some(inner), Some(t)) = (self.inner.as_mut(), state.inner.as_mut()) {
            inner.operate(t, layout.children().next().unwrap(), renderer, operation);
        }
    }
}

impl<'a, M: 'a> From<VirtualGrid<'a, M>> for El<'a, M> {
    fn from(g: VirtualGrid<'a, M>) -> Self {
        Element::new(g)
    }
}

/// Measure helper: a closure that caches `measure(i, width)` per tile for
/// the last width it saw (resizing re-measures once per width).
pub fn cached_heights<'a>(
    measure: impl Fn(usize, f32) -> f32 + 'a,
) -> impl Fn(usize, f32) -> f32 + 'a {
    let cache: RefCell<(u32, Vec<Option<f32>>)> = RefCell::new((0, Vec::new()));
    move |i, w| {
        let mut c = cache.borrow_mut();
        if c.0 != w.to_bits() {
            *c = (w.to_bits(), Vec::new());
        }
        if c.1.len() <= i {
            c.1.resize(i + 1, None);
        }
        *c.1[i].get_or_insert_with(|| measure(i, w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_fill_columns_like_css() {
        // minmax(112px, 1fr), gap 8, 600 px: (600+8)/(120) = 5.06 → 5.
        let g = GridLayout::auto_fill(600.0, 112.0, 8.0);
        assert_eq!(g.columns, 5);
        assert_eq!(g.tile_width, (600.0 - 32.0) / 5.0);
        // Narrower than one tile: still one column, shrunk.
        let g = GridLayout::auto_fill(80.0, 112.0, 8.0);
        assert_eq!((g.columns, g.tile_width), (1, 80.0));
        // Exactly n tiles.
        assert_eq!(GridLayout::auto_fill(352.0, 112.0, 8.0).columns, 3);
        assert_eq!(GridLayout::auto_fill(351.0, 112.0, 8.0).columns, 2);
    }

    #[test]
    fn rows_take_their_tallest_tile() {
        let g = GridLayout::auto_fill(352.0, 112.0, 8.0); // 3 columns
        assert_eq!(g.rows(7), 3);
        assert_eq!(g.row_range(2, 7), 6..7);
        let h = g.row_heights(7, |i, w| {
            assert_eq!(w, g.tile_width);
            if i == 4 { 130.0 } else { 100.0 }
        });
        assert_eq!(h, [100.0, 130.0, 100.0]);
        let r = g.tile_rect(5, &h);
        assert_eq!(r.y, 108.0);
        assert_eq!(r.height, 130.0);
        assert_eq!(r.x, 2.0 * (g.tile_width + 8.0));
        assert_eq!(g.tile_rect(6, &h).y, 108.0 + 138.0);
    }

    #[test]
    fn arrow_keys_step_by_tile_and_row() {
        let g = GridLayout::auto_fill(352.0, 112.0, 8.0); // 3 columns, 7 tiles
        assert_eq!(g.step(0, 7, 1, 0), 1);
        assert_eq!(g.step(0, 7, -1, 0), 0, "clamped at the start");
        assert_eq!(g.step(1, 7, 0, 1), 4);
        assert_eq!(
            g.step(4, 7, 0, 1),
            6,
            "↓ into a short last row: the last tile"
        );
        assert_eq!(g.step(6, 7, 0, 1), 6);
        assert_eq!(g.step(6, 7, 0, -1), 3);
        assert_eq!(g.step(0, 0, 0, 1), 0);
    }

    #[test]
    fn heights_are_cached_per_width() {
        let calls = std::cell::Cell::new(0);
        let f = cached_heights(|i, w| {
            calls.set(calls.get() + 1);
            i as f32 + w
        });
        assert_eq!(f(2, 10.0), 12.0);
        assert_eq!(f(2, 10.0), 12.0);
        assert_eq!(calls.get(), 1);
        assert_eq!(f(2, 20.0), 22.0);
        assert_eq!(calls.get(), 2);
    }
}
