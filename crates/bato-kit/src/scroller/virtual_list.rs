//! Virtualization (the Scroll lab's virtual rows, generalised for DataTable
//! and long lists): `count` rows, fixed or variable heights, of which only
//! those in the visible window (plus an overscan margin) exist as widgets.
//! The list is as tall as all its rows, so a [`Scroller`](super::Scroller)
//! around it scrolls exactly as over real content.
//!
//! ```ignore
//! virtual_list(10_000, Heights::Fixed(36.0), move |i| row(&items[i]).into())
//! virtual_list(n, Heights::Each(heights), |i| …).gap(8.0).overscan(600.0)
//! ```
//!
//! The rows are built during layout from the window the last update saw.
//! When scrolling moves the visible rows outside what is built, the list
//! asks iced for a new layout (`Shell::invalidate_layout`), which happens
//! before the frame is drawn, so no blank rows show. A row keeps its widget
//! state (hover transitions, inputs) while it stays built.

use std::collections::HashMap;
use std::ops::Range;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::{Element, Event, Length, Point, Rectangle, Size, mouse};

/// Row heights.
#[derive(Debug, Clone, PartialEq)]
pub enum Heights {
    /// Every row this tall.
    Fixed(f32),
    /// Row `i` is `heights[i]` tall (missing entries: the last one, or 0).
    Each(Vec<f32>),
}

/// Row positions: prefix sums, or arithmetic for fixed heights (no
/// allocation for a million rows).
#[derive(Debug, Clone, PartialEq)]
pub struct Extents {
    count: usize,
    gap: f32,
    fixed: Option<f32>,
    /// `tops[i]` = top of row i; `tops[count]` = total + gap.
    tops: Vec<f32>,
}

impl Extents {
    pub fn new(count: usize, heights: &Heights, gap: f32) -> Self {
        match heights {
            Heights::Fixed(h) => Self {
                count,
                gap,
                fixed: Some(*h),
                tops: Vec::new(),
            },
            Heights::Each(hs) => {
                let mut tops = Vec::with_capacity(count + 1);
                let mut y = 0.0;
                for i in 0..count {
                    tops.push(y);
                    y += hs.get(i).or(hs.last()).copied().unwrap_or(0.0) + gap;
                }
                tops.push(y);
                Self {
                    count,
                    gap,
                    fixed: None,
                    tops,
                }
            }
        }
    }

    pub fn count(&self) -> usize {
        self.count
    }

    /// Top of row `i` (`i == count`: the end).
    pub fn top(&self, i: usize) -> f32 {
        match self.fixed {
            Some(h) => i.min(self.count) as f32 * (h + self.gap),
            None => self.tops[i.min(self.count)],
        }
    }

    pub fn height(&self, i: usize) -> f32 {
        match self.fixed {
            Some(h) => h,
            None => self.tops[i + 1] - self.tops[i] - self.gap,
        }
    }

    /// The list's height (no gap after the last row).
    pub fn total(&self) -> f32 {
        if self.count == 0 {
            0.0
        } else {
            self.top(self.count) - self.gap
        }
    }

    /// The rows that intersect `[lo, hi)` (list coordinates).
    pub fn range(&self, lo: f32, hi: f32) -> Range<usize> {
        if self.count == 0 || hi <= lo || hi <= 0.0 || lo >= self.total() {
            return 0..0;
        }
        let first = self.row_at(lo.max(0.0));
        let mut last = self.row_at((hi - 0.001).min(self.total() - 0.001));
        if last < first {
            last = first;
        }
        first..(last + 1).min(self.count)
    }

    /// The row whose box (or the gap after it) contains `y`.
    fn row_at(&self, y: f32) -> usize {
        match self.fixed {
            Some(h) => ((y / (h + self.gap)).floor().max(0.0) as usize).min(self.count - 1),
            None => self.tops[..self.count]
                .partition_point(|t| *t <= y)
                .saturating_sub(1),
        }
    }
}

/// Is `inner` within `outer`?
fn covers(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    inner.is_empty() || (outer.start <= inner.start && inner.end <= outer.end)
}

type RowFn<'a, M> = Box<dyn Fn(usize) -> Element<'a, M, iced::Theme, iced::Renderer> + 'a>;

pub struct VirtualList<'a, Message> {
    extents: Extents,
    heights: Heights,
    row: RowFn<'a, Message>,
    overscan: f32,
    built: Vec<(usize, Element<'a, Message, iced::Theme, iced::Renderer>)>,
}

/// `count` rows; `row(i)` builds row `i` when it comes near the viewport.
pub fn virtual_list<'a, Message>(
    count: usize,
    heights: Heights,
    row: impl Fn(usize) -> Element<'a, Message, iced::Theme, iced::Renderer> + 'a,
) -> VirtualList<'a, Message> {
    VirtualList {
        extents: Extents::new(count, &heights, 0.0),
        heights,
        row: Box::new(row),
        overscan: 400.0,
        built: Vec::new(),
    }
}

impl<'a, Message> VirtualList<'a, Message> {
    /// Space between rows (`space-y-N`).
    pub fn gap(mut self, gap: f32) -> Self {
        self.extents = Extents::new(self.extents.count, &self.heights, gap);
        self
    }
    /// How far beyond the viewport rows are built (px, each side).
    pub fn overscan(mut self, px: f32) -> Self {
        self.overscan = px.max(0.0);
        self
    }
}

#[derive(Default)]
struct State {
    /// The rows to build at the next layout.
    want: Option<Range<usize>>,
    trees: HashMap<usize, Tree>,
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for VirtualList<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(self.extents.total()))
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
        let w = limits.max().width;
        let state = tree.state.downcast_mut::<State>();
        let want = state
            .want
            .clone()
            .unwrap_or_else(|| self.extents.range(0.0, 1200.0 + self.overscan));
        let want = want.start.min(self.extents.count)..want.end.min(self.extents.count);
        self.built = want.clone().map(|i| (i, (self.row)(i))).collect();
        state.trees.retain(|i, _| want.contains(i));
        let mut nodes = Vec::with_capacity(self.built.len());
        for (i, el) in &mut self.built {
            let t = state
                .trees
                .entry(*i)
                .or_insert_with(|| Tree::new(el.as_widget()));
            t.diff(el.as_widget());
            let h = self.extents.height(*i);
            let l = layout::Limits::new(Size::new(w, h), Size::new(w, h));
            nodes.push(
                el.as_widget_mut()
                    .layout(t, renderer, &l)
                    .move_to(Point::new(0.0, self.extents.top(*i))),
            );
        }
        layout::Node::with_children(Size::new(w, self.extents.total()), nodes)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let b = layout.bounds();
        let lo = viewport.y - b.y;
        let need = self.extents.range(lo, lo + viewport.height);
        let built = self
            .built
            .first()
            .map_or(0..0, |f| f.0..f.0 + self.built.len());
        let state = tree.state.downcast_mut::<State>();
        if !covers(&built, &need) {
            let o = self.overscan;
            state.want = Some(self.extents.range(lo - o, lo + viewport.height + o));
            shell.invalidate_layout();
            shell.request_redraw();
        }
        for ((i, el), l) in self.built.iter_mut().zip(layout.children()) {
            if let Some(t) = state.trees.get_mut(i) {
                el.as_widget_mut()
                    .update(t, event, l, cursor, renderer, clipboard, shell, viewport);
            }
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
        self.built
            .iter()
            .zip(layout.children())
            .filter(|(_, l)| cursor.is_over(l.bounds()))
            .find_map(|((i, el), l)| {
                state.trees.get(i).map(|t| {
                    el.as_widget()
                        .mouse_interaction(t, l, cursor, viewport, renderer)
                })
            })
            .unwrap_or_default()
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
        for ((i, el), l) in self.built.iter().zip(layout.children()) {
            if !l.bounds().intersects(viewport) {
                continue;
            }
            if let Some(t) = state.trees.get(i) {
                el.as_widget()
                    .draw(t, renderer, theme, style, l, cursor, viewport);
            }
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
        operation.traverse(&mut |op| {
            for ((i, el), l) in self.built.iter_mut().zip(layout.children()) {
                if let Some(t) = state.trees.get_mut(i) {
                    el.as_widget_mut().operate(t, l, renderer, op);
                }
            }
        });
    }
}

impl<'a, Message: 'a> From<VirtualList<'a, Message>>
    for Element<'a, Message, iced::Theme, iced::Renderer>
{
    fn from(v: VirtualList<'a, Message>) -> Self {
        Element::new(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_rows_are_arithmetic() {
        let e = Extents::new(1000, &Heights::Fixed(56.0), 0.0);
        assert_eq!(e.total(), 56_000.0);
        assert_eq!(e.top(10), 560.0);
        assert_eq!(e.range(0.0, 800.0), 0..15);
        assert_eq!(e.range(560.0, 616.0), 10..11, "exactly one row");
        assert_eq!(e.range(55_990.0, 57_000.0), 999..1000);
        assert_eq!(e.range(-500.0, -1.0), 0..0, "above the list");
        assert_eq!(e.range(60_000.0, 61_000.0), 0..0, "below the list");
    }

    #[test]
    fn variable_rows_and_gaps() {
        let e = Extents::new(4, &Heights::Each(vec![10.0, 30.0, 20.0, 40.0]), 8.0);
        // Tops: 0, 18, 56, 84; total 124.
        assert_eq!((e.top(1), e.top(2), e.top(3)), (18.0, 56.0, 84.0));
        assert_eq!(e.total(), 124.0);
        assert_eq!(e.height(1), 30.0);
        assert_eq!(e.range(0.0, 18.0), 0..1, "the gap after row 0 is row 0's");
        assert_eq!(e.range(19.0, 57.0), 1..3);
        assert_eq!(e.range(100.0, 500.0), 3..4);
        let g = Extents::new(3, &Heights::Fixed(10.0), 2.0);
        assert_eq!(g.total(), 34.0);
        assert_eq!(g.range(11.0, 12.5), 0..2);
        // Missing heights repeat the last one.
        let m = Extents::new(3, &Heights::Each(vec![5.0]), 0.0);
        assert_eq!(m.total(), 15.0);
    }

    #[test]
    fn rebuild_only_when_the_view_leaves_the_built_rows() {
        assert!(covers(&(10..40), &(12..30)));
        assert!(!covers(&(10..40), &(35..45)));
        assert!(covers(&(0..0), &(5..5)), "nothing visible");
        let e = Extents::new(0, &Heights::Fixed(10.0), 0.0);
        assert_eq!(e.range(0.0, 100.0), 0..0);
        assert_eq!(e.total(), 0.0);
    }
}
