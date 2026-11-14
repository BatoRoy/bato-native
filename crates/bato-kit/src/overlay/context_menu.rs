//! `ContextMenu` + `useContextMenu` (`ContextMenu.tsx`,
//! `hooks/useContextMenu.ts`): a right press anywhere in an area opens a
//! [`MenuList`] at the pointer, clamped to the window (8 px), never flipped.
//! Like React it is controlled: the app keeps a [`ContextMenuState`] (the
//! hook) and the area reports where the menu opens.
//!
//! Dismissal: a press anywhere outside the menu (the area included: a
//! right press elsewhere in it closes this menu and opens a new one at the
//! new point), Esc, scroll, resize, window blur. Keys: ↑ ↓ (wrap, skip
//! disabled and separators), Home End, Enter. Picking an item closes the
//! menu first (`onClose(); item.onClick()`). It floats above modals.
//!
//! ```ignore
//! context_menu(&cx, area, state.menu.position(), items(), Msg::Menu, Msg::MenuClose)
//! // update: Msg::Menu(p) => state.menu.open(p), Msg::MenuClose => state.menu.close()
//! ```
//!
//! [`MenuList`]: crate::menu::MenuList

use std::f32::consts::{FRAC_PI_2, PI};

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::{Border, Color, Element, Event, Length, Point, Rectangle, Size, Vector, mouse};

use super::anchored::{Anchored, context_area};
use crate::anim::fade;
use crate::ctx::Ctx;
use crate::menu::{MenuEntry, menu_list};

type El<'a, M> = iced::Element<'a, M, iced::Theme, iced::Renderer>;

/// `useContextMenu()`: open state and position (relative to the area).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ContextMenuState {
    at: Option<Point>,
}

impl ContextMenuState {
    /// `onContextMenu`: open at `p`.
    pub fn open(&mut self, p: Point) {
        self.at = Some(p);
    }
    /// `close()`.
    pub fn close(&mut self) {
        self.at = None;
    }
    pub fn is_open(&self) -> bool {
        self.at.is_some()
    }
    /// Where the menu opens while it is open.
    pub fn position(&self) -> Option<Point> {
        self.at
    }
}

/// `area` with a context menu of `items`: `on_open(point)` on a right press
/// (the point relative to the area), `on_close` when the menu goes away
/// (dismissed, or before an item's own message).
pub fn context_menu<'a, M: Clone + 'a>(
    cx: &Ctx,
    area: impl Into<El<'a, M>>,
    at: Option<Point>,
    items: Vec<MenuEntry<M>>,
    on_open: impl Fn(Point) -> M + 'a,
    on_close: M,
) -> Anchored<'a, M> {
    let close = on_close.clone();
    context_area(area, on_open).menu(at, menu_list(cx, items).on_close(on_close), move |_| {
        close.clone()
    })
}

// ── Dashed border (`border-2 border-dashed`) ────────────────────────────

/// Blink's `StrokeData::SelectBestDashGap`: the gap that fits a whole number
/// of dashes along a closed path of `length`, closest to `gap`.
pub fn best_dash_gap(length: f32, dash: f32, gap: f32) -> f32 {
    let min_n = (length / (dash + gap)).floor().max(1.0);
    let max_n = min_n + 1.0;
    let min_gap = (length - min_n * dash) / min_n;
    let max_gap = (length - max_n * dash) / max_n;
    if max_gap <= 0.0 || (min_gap - gap).abs() < (max_gap - gap).abs() {
        min_gap
    } else {
        max_gap
    }
}

/// One piece of the centreline of a rounded rectangle, clockwise from the
/// bottom of the left side (Skia's `addRRect` start).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Piece {
    /// From → to, axis-aligned.
    Line(Point, Point),
    /// Centre, radius, start angle, sweep (clockwise on screen = +).
    Arc(Point, f32, f32),
}

fn pieces(b: Rectangle, r: f32) -> [Piece; 8] {
    let (x0, y0, x1, y1) = (b.x, b.y, b.x + b.width, b.y + b.height);
    [
        Piece::Line(Point::new(x0, y1 - r), Point::new(x0, y0 + r)),
        Piece::Arc(Point::new(x0 + r, y0 + r), r, PI),
        Piece::Line(Point::new(x0 + r, y0), Point::new(x1 - r, y0)),
        Piece::Arc(Point::new(x1 - r, y0 + r), r, -FRAC_PI_2),
        Piece::Line(Point::new(x1, y0 + r), Point::new(x1, y1 - r)),
        Piece::Arc(Point::new(x1 - r, y1 - r), r, 0.0),
        Piece::Line(Point::new(x1 - r, y1), Point::new(x0 + r, y1)),
        Piece::Arc(Point::new(x0 + r, y1 - r), r, FRAC_PI_2),
    ]
}

fn piece_len(p: &Piece) -> f32 {
    match p {
        Piece::Line(a, b) => (b.x - a.x).abs() + (b.y - a.y).abs(),
        Piece::Arc(_, r, _) => FRAC_PI_2 * r,
    }
}

/// The dash intervals `[start, end)` along a closed path of `length`.
pub fn dash_intervals(length: f32, dash: f32, gap: f32) -> Vec<(f32, f32)> {
    let gap = best_dash_gap(length, dash, gap);
    let period = dash + gap;
    let mut v = Vec::new();
    let mut s = 0.0;
    while s < length - 1e-3 {
        v.push((s, (s + dash).min(length)));
        s += period;
    }
    v
}

/// Paint a `width` px dashed border inside `outer` with corner `radius`
/// (Chromium: dashes 3×width, gaps ≈2×width fitted to the perimeter, one
/// path around the rounded rectangle).
pub fn paint_dashed(
    renderer: &mut iced::Renderer,
    outer: Rectangle,
    radius: f32,
    width: f32,
    color: Color,
) {
    let h = width / 2.0;
    let center = Rectangle {
        x: outer.x + h,
        y: outer.y + h,
        width: outer.width - width,
        height: outer.height - width,
    };
    let r = (radius - h).max(0.0);
    let ps = pieces(center, r);
    let total: f32 = ps.iter().map(piece_len).sum();
    let color = fade(color);
    for (a, b) in dash_intervals(total, 3.0 * width, 2.0 * width) {
        let mut at = 0.0;
        for p in &ps {
            let len = piece_len(p);
            let (s0, s1) = ((a - at).max(0.0), (b - at).min(len));
            at += len;
            if s1 <= s0 {
                continue;
            }
            match *p {
                Piece::Line(p0, p1) => {
                    // `signum(0.0)` is 1: axis-aligned lines need a true 0.
                    let sign = |d: f32| if d == 0.0 { 0.0 } else { d.signum() };
                    let dir = Vector::new(sign(p1.x - p0.x), sign(p1.y - p0.y));
                    let q0 = p0 + dir * s0;
                    let q1 = p0 + dir * s1;
                    let (lx, hx) = (q0.x.min(q1.x), q0.x.max(q1.x));
                    let (ly, hy) = (q0.y.min(q1.y), q0.y.max(q1.y));
                    renderer.fill_quad(
                        Quad {
                            bounds: Rectangle {
                                x: lx - if dir.x == 0.0 { h } else { 0.0 },
                                y: ly - if dir.y == 0.0 { h } else { 0.0 },
                                width: if dir.x == 0.0 { width } else { hx - lx },
                                height: if dir.y == 0.0 { width } else { hy - ly },
                            },
                            snap: false,
                            ..Quad::default()
                        },
                        color,
                    );
                }
                Piece::Arc(c, rr, start) => {
                    // The sub-arc's box, clipping the whole rounded ring.
                    let (t0, t1) = (start + s0 / rr.max(1e-3), start + s1 / rr.max(1e-3));
                    let mut lo = Point::new(f32::MAX, f32::MAX);
                    let mut hi = Point::new(f32::MIN, f32::MIN);
                    for t in [t0, t1] {
                        for rad in [rr - h, rr + h] {
                            let q = Point::new(c.x + rad * t.cos(), c.y + rad * t.sin());
                            lo = Point::new(lo.x.min(q.x), lo.y.min(q.y));
                            hi = Point::new(hi.x.max(q.x), hi.y.max(q.y));
                        }
                    }
                    let clip = Rectangle::new(lo, Size::new(hi.x - lo.x, hi.y - lo.y));
                    renderer.with_layer(clip, |renderer| {
                        renderer.fill_quad(
                            Quad {
                                bounds: outer,
                                border: Border {
                                    radius: radius.into(),
                                    width,
                                    color,
                                },
                                ..Quad::default()
                            },
                            Color::TRANSPARENT,
                        );
                    });
                }
            }
        }
    }
}

/// Content inside a dashed border (`border-2 border-dashed rounded-lg`):
/// the ContextMenu demo area, drop zones.
pub struct Dashed<'a, M> {
    content: El<'a, M>,
    color: Color,
    radius: f32,
    width: f32,
}

pub fn dashed<'a, M>(content: impl Into<El<'a, M>>, color: Color) -> Dashed<'a, M> {
    Dashed {
        content: content.into(),
        color,
        radius: 8.0,
        width: 2.0,
    }
}

impl<'a, M> Dashed<'a, M> {
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
    pub fn border_width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for Dashed<'a, M> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let n = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
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
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        )
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
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
        paint_dashed(
            renderer,
            layout.bounds(),
            self.radius,
            self.width,
            self.color,
        );
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: 'a> From<Dashed<'a, M>> for El<'a, M> {
    fn from(d: Dashed<'a, M>) -> Self {
        Element::new(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashes_match_chromium_on_the_showcase_area() {
        // The 806×96 area, border 2, radius 8: centreline perimeter
        // 2·790 + 2·80 + 2π·7.
        let b = Rectangle::new(Point::new(350.0, 224.0), Size::new(804.0, 94.0));
        let total: f32 = pieces(b, 7.0).iter().map(piece_len).sum();
        assert!((total - 1783.98).abs() < 0.01, "{total}");
        let gap = best_dash_gap(total, 6.0, 4.0);
        assert!((gap - 4.0224).abs() < 1e-3, "{gap}");
        let d = dash_intervals(total, 6.0, 4.0);
        assert_eq!(d.len(), 178);
        // Up the left side from y = 311: [305, 311), then [295, 301) …
        assert_eq!(d[0], (0.0, 6.0));
        assert!((d[1].0 - 10.0224).abs() < 1e-3);
        // The 10th dash starts 0.8 px before the top edge (x 357): 356.2.
        let top = 80.0 + FRAC_PI_2 * 7.0;
        assert!((357.0 + d[9].0 - top - 356.2).abs() < 0.01);
    }

    #[test]
    fn hook_state() {
        let mut m = ContextMenuState::default();
        assert!(!m.is_open());
        m.open(Point::new(12.0, 30.0));
        assert_eq!(m.position(), Some(Point::new(12.0, 30.0)));
        m.open(Point::new(40.0, 5.0));
        assert_eq!(m.position(), Some(Point::new(40.0, 5.0)));
        m.close();
        assert_eq!(m.position(), None);
    }
}
