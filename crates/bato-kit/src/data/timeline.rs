//! `Timeline` (`data/Timeline.tsx`): items grouped into lanes (insertion
//! order, `Items` when unlaned) along a shared time axis with 7 ticks, a
//! red `now` line, and bars that are buttons (`hover:brightness-110`, 150 ms;
//! the `title` becomes a tooltip). The range defaults to the items' span.
//!
//! When `scrollable` (default) the content is at least `min_width` (720) wide
//! and scrolls horizontally, with the lane labels and the axis gutter
//! **sticky** at the left: they are drawn over the scrolled content at the
//! scroller's offset ([`Sticky`]), above the now line, which is above the
//! bars (React's z-20 / z-10).
//!
//! ```ignore
//! timeline(&cx, items).min_width(1100.0).scrollable(self.scroll)
//! timeline(&cx, items).on_item_click(|it| Msg::Open(it.id.clone()))
//! ```

pub use chrono;
use chrono::NaiveDateTime;
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::widget::{Space, column, row, stack};
use iced::{Color, Element, Event, Length, Point, Rectangle, Size, Vector, mouse};

use crate::El;
use crate::anim::fade;
use crate::ctx::Ctx;
use crate::focus::Ring;
use crate::label::label;
use crate::layout::{forward_cells, hline, into_element, rounded_clip};
use crate::overlay::tooltip;
use crate::scale::snap_rect;
use crate::scroller::scroll_area;
use crate::style::{c, ink_for};
use crate::surface::{BoxStyle, Status, surface};

/// The lane-label gutter (`w-32`, border included).
pub const GUTTER: f32 = 128.0;
/// A lane's minimum height (`min-h-[3rem]`, its `border-b` included).
pub const LANE_H: f32 = 48.0;
/// The axis row (`h-8`).
pub const AXIS_H: f32 = 32.0;

/// One bar (`TimelineItem`); times are local.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineItem {
    pub id: String,
    pub label: String,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub lane: Option<String>,
    /// `None`: the accent (with `accent_ink` text); else `inkOn(color)`.
    pub color: Option<bato_theme::Rgb>,
}

// ── Pure maths (unit-tested) ─────────────────────────────────────────────

fn ms(t: NaiveDateTime) -> f64 {
    t.and_utc().timestamp_millis() as f64
}

/// The axis: range start, span (ms, ≥ 1) and the 7 tick times.
#[derive(Debug, Clone, PartialEq)]
pub struct Axis {
    pub start: NaiveDateTime,
    pub span_ms: f64,
    pub ticks: Vec<NaiveDateTime>,
}

pub fn axis(
    items: &[TimelineItem],
    start: Option<NaiveDateTime>,
    end: Option<NaiveDateTime>,
) -> Axis {
    let rs = start
        .or_else(|| items.iter().map(|i| i.start).min())
        .unwrap_or_default();
    let re = end
        .or_else(|| items.iter().map(|i| i.end).max())
        .unwrap_or(rs);
    let span = (ms(re) - ms(rs)).max(1.0);
    let ticks = (0..=6)
        .map(|i| {
            let off = (i as f64 / 6.0) * span;
            rs + chrono::Duration::milliseconds(off as i64)
        })
        .collect();
    Axis {
        start: rs,
        span_ms: span,
        ticks,
    }
}

impl Axis {
    /// The position of `t` on the axis, 0..1 inside the range (`pct / 100`).
    pub fn frac(&self, t: NaiveDateTime) -> f64 {
        (ms(t) - ms(self.start)) / self.span_ms
    }
    /// A bar's `(left, width)` fractions: width at least 2 %.
    pub fn bar(&self, it: &TimelineItem) -> (f64, f64) {
        let l = self.frac(it.start);
        (l, (self.frac(it.end) - l).max(0.02))
    }
}

/// The items grouped by lane, lanes in first-seen order.
pub fn lanes(items: &[TimelineItem]) -> Vec<(String, Vec<&TimelineItem>)> {
    let mut out: Vec<(String, Vec<&TimelineItem>)> = Vec::new();
    for it in items {
        let k = it.lane.clone().unwrap_or_else(|| "Items".to_string());
        match out.iter_mut().find(|(l, _)| *l == k) {
            Some((_, v)) => v.push(it),
            None => out.push((k, vec![it])),
        }
    }
    out
}

/// `filter: brightness(1.1)` on an sRGB colour.
pub fn brighten(c: Color, k: f32) -> Color {
    Color {
        r: (c.r * k).min(1.0),
        g: (c.g * k).min(1.0),
        b: (c.b * k).min(1.0),
        a: c.a,
    }
}

// ── Builder ─────────────────────────────────────────────────────────────

type OnItem<'a, M> = Box<dyn Fn(&TimelineItem) -> M + 'a>;

pub struct Timeline<'a, M> {
    cx: Ctx,
    items: Vec<TimelineItem>,
    start: Option<NaiveDateTime>,
    end: Option<NaiveDateTime>,
    on_item_click: Option<OnItem<'a, M>>,
    axis_format: String,
    scrollable: bool,
    min_width: f32,
    now: NaiveDateTime,
    id: Option<iced::widget::Id>,
}

pub fn timeline<'a, M: Clone + 'a>(cx: &Ctx, items: Vec<TimelineItem>) -> Timeline<'a, M> {
    Timeline {
        cx: cx.clone(),
        items,
        start: None,
        end: None,
        on_item_click: None,
        axis_format: "MMM d".to_string(),
        scrollable: true,
        min_width: 720.0,
        now: crate::datefmt::now(),
        id: None,
    }
}

impl<'a, M: Clone + 'a> Timeline<'a, M> {
    pub fn range(mut self, start: Option<NaiveDateTime>, end: Option<NaiveDateTime>) -> Self {
        self.start = start;
        self.end = end;
        self
    }
    pub fn on_item_click(mut self, f: impl Fn(&TimelineItem) -> M + 'a) -> Self {
        self.on_item_click = Some(Box::new(f));
        self
    }
    /// The tick label pattern (date-fns tokens, default `MMM d`).
    pub fn axis_format(mut self, f: impl Into<String>) -> Self {
        self.axis_format = f.into();
        self
    }
    pub fn scrollable(mut self, on: bool) -> Self {
        self.scrollable = on;
        self
    }
    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }
    /// The time of the now line (default: when the view was built).
    pub fn now(mut self, t: NaiveDateTime) -> Self {
        self.now = t;
        self
    }
    /// The horizontal scroller's id ([`crate::scroller::scroll_offset`]).
    pub fn id(mut self, id: impl Into<iced::widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl<'a, M: Clone + 'a> From<Timeline<'a, M>> for El<'a, M> {
    fn from(tl: Timeline<'a, M>) -> Self {
        let cx = tl.cx.on_card();
        let p = cx.p();
        let ax = axis(&tl.items, tl.start, tl.end);
        let border = c(p.border);
        let lanes = lanes(&tl.items);

        // Axis: ticks over the content area, the sticky gutter on top.
        let mut ticks = Track::new(AXIS_H);
        for (i, t) in ax.ticks.iter().enumerate() {
            let text = crate::datefmt::format(t, &tl.axis_format);
            ticks = ticks.push(
                Place::Center(i as f32 / (ax.ticks.len() - 1) as f32),
                label(text, 12.0, 16.0).color(c(p.muted)),
            );
        }
        let gutter = surface(Space::new().width(GUTTER - 1.0).height(AXIS_H))
            .fixed(BoxStyle::default().bg(c(p.surface)));
        let axis_row: El<'a, M> = surface(stack![
            row![Space::new().width(GUTTER), ticks],
            Sticky::new(row![gutter, vrule(border, AXIS_H)]),
        ])
        .width(Length::Fill)
        .fixed(BoxStyle::default().bg(c(p.surface)))
        .into();

        // Lanes: bars, then the now line, then the sticky labels.
        let n = lanes.len();
        let mut bars_col = column![];
        let mut labels_col = column![];
        for (li, (lane, its)) in lanes.iter().enumerate() {
            // border-box: a bordered lane is 47 + its 1 px line.
            let lane_h = if li + 1 < n { LANE_H - 1.0 } else { LANE_H };
            let mut track = Track::new(lane_h);
            for it in its {
                let (l, w) = ax.bar(it);
                let (bg, ink) = match it.color {
                    Some(col) => (c(col), c(ink_for(col))),
                    None => (c(p.accent), c(p.accent_ink)),
                };
                let title = format!(
                    "{} · {} → {}",
                    it.label,
                    crate::datefmt::format(&it.start, "PP"),
                    crate::datefmt::format(&it.end, "PP")
                );
                let bar = surface(
                    label(it.label.clone(), 12.0, 16.0)
                        .medium()
                        .ellipsis()
                        .width(Length::Fill),
                )
                .width(Length::Fill)
                .height(28)
                .padding([0.0, 8.0])
                .center_y()
                .style(move |st| {
                    let k = if st == Status::Hover { 1.1 } else { 1.0 };
                    BoxStyle::default()
                        .bg(brighten(bg, k))
                        .fg(brighten(ink, k))
                        .radius(6.0)
                })
                .focusable(true)
                .focus_ring(Ring::plain(&cx))
                .on_press_maybe(tl.on_item_click.as_ref().map(|f| f(it)));
                track = track.push(
                    Place::Span {
                        left: l as f32,
                        width: w as f32,
                        top: 8.0,
                        height: 28.0,
                    },
                    tooltip(&cx, bar, title),
                );
            }
            bars_col = bars_col.push(row![Space::new().width(GUTTER), track]);
            let cell = surface(
                label(lane.clone(), 14.0, 20.0)
                    .color(c(p.subtext))
                    .ellipsis(),
            )
            .width(GUTTER - 1.0)
            .height(lane_h)
            .padding([0.0, 12.0])
            .center_y()
            .fixed(BoxStyle::default().bg(c(p.bg)));
            labels_col = labels_col.push(row![cell, vrule(border, lane_h)]);
            if li + 1 < n {
                bars_col = bars_col.push(hline(border));
                labels_col = labels_col.push(container_w(GUTTER, hline(border)));
            }
        }
        let now = ax.frac(tl.now);
        let mut lanes_stack = stack![bars_col.width(Length::Fill)];
        if (0.0..=1.0).contains(&now) {
            lanes_stack = lanes_stack.push(NowLine {
                frac: now as f32,
                color: crate::style::rgba(p.red, 0.6),
            });
        }
        lanes_stack = lanes_stack.push(Sticky::new(labels_col));

        let content = column![axis_row, hline(border), lanes_stack].width(Length::Fill);
        let body: El<'a, M> = if tl.scrollable {
            let mut s = scroll_area(&cx, MinWidth::new(tl.min_width, content)).horizontal();
            if let Some(id) = tl.id {
                s = s.id(id);
            }
            s.into()
        } else {
            content.into()
        };
        rounded_clip(12.0, c(cx.over), body).border(border).into()
    }
}

fn vrule<'a, M: Clone + 'a>(color: Color, h: f32) -> El<'a, M> {
    surface(Space::new().width(1).height(h))
        .fixed(BoxStyle::default().bg(color))
        .into()
}

fn container_w<'a, M: Clone + 'a>(w: f32, el: impl Into<El<'a, M>>) -> El<'a, M> {
    iced::widget::container(el).width(w).into()
}

// ── Track: percentage placement ─────────────────────────────────────────

/// Where a [`Track`] child goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    /// `absolute; left: l%; width: w%; top; height` (fractions of the width).
    Span {
        left: f32,
        width: f32,
        top: f32,
        height: f32,
    },
    /// `absolute; left: at%; top: 50%; translate(-50%, -50%)`.
    Center(f32),
}

/// A fixed-height box placing children at fractions of its width.
pub struct Track<'a, Message> {
    cells: Vec<Element<'a, Message, iced::Theme, iced::Renderer>>,
    places: Vec<Place>,
    height: f32,
}

impl<'a, Message> Track<'a, Message> {
    pub fn new(height: f32) -> Self {
        Self {
            cells: Vec::new(),
            places: Vec::new(),
            height,
        }
    }
    pub fn push(
        mut self,
        place: Place,
        child: impl Into<Element<'a, Message, iced::Theme, iced::Renderer>>,
    ) -> Self {
        self.cells.push(child.into());
        self.places.push(place);
        self
    }
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for Track<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(self.height))
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let w = limits.max().width;
        let h = self.height;
        let nodes = self
            .cells
            .iter_mut()
            .zip(tree.children.iter_mut())
            .zip(&self.places)
            .map(|((cell, t), place)| match *place {
                Place::Span {
                    left,
                    width,
                    top,
                    height,
                } => {
                    let size = Size::new(width * w, height);
                    cell.as_widget_mut()
                        .layout(t, renderer, &layout::Limits::new(size, size))
                        .move_to(Point::new(left * w, top))
                }
                Place::Center(at) => {
                    let n = cell.as_widget_mut().layout(
                        t,
                        renderer,
                        &layout::Limits::new(Size::ZERO, Size::new(f32::INFINITY, h)),
                    );
                    let s = n.size();
                    n.move_to(Point::new(at * w - s.width / 2.0, (h - s.height) / 2.0))
                }
            })
            .collect();
        layout::Node::with_children(Size::new(w, h), nodes)
    }

    forward_cells!();
}

into_element!(Track);

// ── MinWidth ────────────────────────────────────────────────────────────

/// `min-width: N` inside a horizontal scroller: as wide as offered
/// (the scroller's minimum), but at least `min`.
pub struct MinWidth<'a, Message> {
    cells: Vec<Element<'a, Message, iced::Theme, iced::Renderer>>,
    min: f32,
}

impl<'a, Message> MinWidth<'a, Message> {
    pub fn new(
        min: f32,
        content: impl Into<Element<'a, Message, iced::Theme, iced::Renderer>>,
    ) -> Self {
        Self {
            cells: vec![content.into()],
            min,
        }
    }
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for MinWidth<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let w = limits.min().width.max(self.min);
        let max_h = limits.max().height;
        let n = self.cells[0].as_widget_mut().layout(
            &mut tree.children[0],
            renderer,
            &layout::Limits::new(Size::new(w, 0.0), Size::new(w, max_h)),
        );
        // Scrollable overflow: what sticks out right (the last tick label,
        // centred on 100 %) widens the scroll range, as in Chromium.
        let right = overflow_right(&n, 0.0).max(w);
        layout::Node::with_children(Size::new(right, n.size().height), vec![n])
    }

    forward_cells!();
}

into_element!(MinWidth);

/// The rightmost edge of a layout tree, relative to its parent.
fn overflow_right(n: &layout::Node, x: f32) -> f32 {
    let b = n.bounds();
    let left = x + b.x;
    n.children()
        .iter()
        .map(|c| overflow_right(c, left))
        .fold(left + b.width, f32::max)
}

// ── Sticky ──────────────────────────────────────────────────────────────

/// `position: sticky; left: 0` inside a horizontal scroller: the content is
/// drawn shifted right by however far the scroller's visible area starts
/// past it, so it stays at the scroller's left edge. It is not interactive,
/// but it hides what it covers from the pointer (layers below see a
/// levitated cursor in a `stack`).
pub struct Sticky<'a, Message> {
    content: Element<'a, Message, iced::Theme, iced::Renderer>,
}

impl<'a, Message> Sticky<'a, Message> {
    pub fn new(content: impl Into<Element<'a, Message, iced::Theme, iced::Renderer>>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

/// The sticky shift for content at `bounds` seen through `viewport`.
pub fn sticky_dx(bounds: Rectangle, viewport: &Rectangle) -> f32 {
    (viewport.x - bounds.x).max(0.0)
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for Sticky<'a, Message> {
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
        _tree: &mut Tree,
        _event: &Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        _shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
    }
    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let b = layout.bounds();
        let shifted = b + Vector::new(sticky_dx(b, viewport), 0.0);
        if cursor.is_over(shifted) {
            mouse::Interaction::Idle
        } else {
            mouse::Interaction::None
        }
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let dx = sticky_dx(layout.bounds(), viewport);
        renderer.with_translation(Vector::new(dx, 0.0), |r| {
            self.content.as_widget().draw(
                &tree.children[0],
                r,
                theme,
                style,
                layout.children().next().unwrap(),
                mouse::Cursor::Unavailable,
                &(*viewport - Vector::new(dx, 0.0)),
            );
        });
    }
}

into_element!(Sticky);

// ── Now line ────────────────────────────────────────────────────────────

/// The `w-px bg-app-red/60` line at `frac` of the area right of the gutter.
struct NowLine {
    frac: f32,
    color: Color,
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for NowLine {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }
    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.max())
    }
    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let b = layout.bounds();
        let x = b.x + GUTTER + self.frac * (b.width - GUTTER);
        renderer.fill_quad(
            Quad {
                bounds: snap_rect(Rectangle {
                    x,
                    y: b.y,
                    width: 1.0,
                    height: b.height,
                }),
                ..Quad::default()
            },
            fade(self.color),
        );
    }
}

impl<'a, Message: 'a> From<NowLine> for Element<'a, Message, iced::Theme, iced::Renderer> {
    fn from(w: NowLine) -> Self {
        Element::new(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, d)
            .unwrap()
            .and_hms_opt(h, 30, 0)
            .unwrap()
    }

    fn item(lane: &str, label: &str, s: u32, e: u32) -> TimelineItem {
        TimelineItem {
            id: format!("{lane}-{label}"),
            label: label.into(),
            start: at(s, 10),
            end: at(e, 10),
            lane: Some(lane.into()),
            color: None,
        }
    }

    /// The Showcase's items around Nov 14.
    fn showcase() -> Vec<TimelineItem> {
        vec![
            item("Design", "Mockups", 12, 16),
            item("Design", "Polish", 16, 21),
            item("Build", "Components", 14, 20),
            item("Build", "Integration", 20, 24),
            item("QA", "Test plan", 15, 18),
        ]
    }

    #[test]
    fn axis_spans_the_items_with_seven_ticks() {
        let items = showcase();
        let ax = axis(&items, None, None);
        assert_eq!(ax.start, at(12, 10));
        assert_eq!(ax.span_ms, 12.0 * 86_400_000.0);
        let labels: Vec<String> = ax
            .ticks
            .iter()
            .map(|t| crate::datefmt::format(t, "MMM d"))
            .collect();
        assert_eq!(
            labels,
            [
                "Nov 12", "Nov 14", "Nov 16", "Nov 18", "Nov 20", "Nov 22", "Nov 24"
            ]
        );
        // The now line at the "Nov 14" tick.
        assert!((ax.frac(at(14, 10)) - 1.0 / 6.0).abs() < 1e-9);
        let (l, w) = ax.bar(&items[2]);
        assert!((l - 2.0 / 12.0).abs() < 1e-9 && (w - 0.5).abs() < 1e-9);
    }

    #[test]
    fn bars_are_at_least_two_percent() {
        let ax = axis(&showcase(), None, None);
        let it = item("x", "dot", 14, 14);
        assert_eq!(ax.bar(&it).1, 0.02);
    }

    #[test]
    fn lanes_keep_first_seen_order() {
        let mut items = showcase();
        items.push(TimelineItem {
            lane: None,
            ..item("", "loose", 14, 15)
        });
        let names: Vec<String> = lanes(&items).into_iter().map(|(l, _)| l).collect();
        assert_eq!(names, ["Design", "Build", "QA", "Items"]);
        assert_eq!(lanes(&items)[1].1.len(), 2);
    }

    #[test]
    fn sticky_shift_follows_the_scroll() {
        let b = Rectangle::new(Point::new(100.0, 0.0), Size::new(1100.0, 50.0));
        let vp = |x| Rectangle::new(Point::new(x, 0.0), Size::new(800.0, 50.0));
        assert_eq!(sticky_dx(b, &vp(100.0)), 0.0);
        assert_eq!(sticky_dx(b, &vp(140.0)), 40.0);
        assert_eq!(sticky_dx(b, &vp(0.0)), 0.0);
    }

    #[test]
    fn brightness_scales_and_clamps() {
        let c = brighten(Color::from_rgb(0.5, 0.95, 0.0), 1.1);
        assert!((c.r - 0.55).abs() < 1e-6 && c.g == 1.0 && c.b == 0.0);
    }
}
