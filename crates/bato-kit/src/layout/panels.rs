//! `PanelGroup` / `Panel` (N resizable panels, nestable) and
//! `ResizablePanels` (the older two-panel split).
//!
//! **PanelGroup** lays panels out at `sizes[i]` % of its length, like
//! `flex: 0 0 <size>%`, with a 1 px hairline (`bg-app-line`, hover
//! `lineStrong`, `accentBright` while dragged, `transition-colors`) after
//! every panel but the last. The hairlines take layout space, so the
//! panels overflow the group by `n − 1` px exactly as in React (inside a
//! nested group this shows the outer panel's scrollbar). A 9 px invisible
//! grab strip centred on each hairline sits over the neighbours and shows
//! the `col-resize` / `row-resize` cursor. Dragging moves the two
//! neighbours by the pointer delta in % of the group, neither going below
//! its `min` (default 5). Mouse and touch.
//!
//! Each panel is `overflow-auto` (a [`scroll_area`] on both axes) and
//! clipped to its box.
//!
//! The group is controlled: `on_resize` reports every move, `on_commit` the
//! final sizes on release (persist those, e.g. with [`crate::kv::UiState`]).
//! [`initial_sizes`] is React's starting distribution.
//!
//! **ResizablePanels**: the first panel is `size` % (clamped to
//! `min..=max`, default 15..=85) of the whole box, then a 4 px
//! `bg-app-border` bar (hover `accent`), then the second panel takes the
//! rest; dragging puts the bar under the pointer.
//!
//! Neither has keyboard resizing (React has none).
//!
//! ```ignore
//! panel_group(&cx, Direction::Horizontal, &state.sizes)
//!     .panel(sidebar, 15.0)
//!     .panel(editor, 5.0)
//!     .on_resize(Msg::Resize)
//!     .on_commit(Msg::Save)
//! ```

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::{Color, Element, Event, Length, Point, Rectangle, Size, Vector, mouse, touch, window};

use crate::anim::{ease_standard, fade};
use crate::ctx::Ctx;
use crate::cursor::Cursor;
use crate::scale::snap_rect;
use crate::scroller::scroll_area;
use crate::style::{c, lerp};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Horizontal,
    Vertical,
}

/// React's default panel minimum (%).
pub const MIN: f32 = 5.0;
/// The grab strip reaches this far past each side of the hairline.
pub const GRAB: f32 = 4.0;
const FADE: Duration = Duration::from_millis(150);

/// The starting sizes: `stored` when it has one size per panel, otherwise
/// each `defaults[i]`, the rest split equally among the panels without one.
pub fn initial_sizes(defaults: &[Option<f32>], stored: Option<&[f32]>) -> Vec<f32> {
    if let Some(s) = stored
        && s.len() == defaults.len()
    {
        return s.to_vec();
    }
    let used: f32 = defaults.iter().flatten().sum();
    let rest = defaults.iter().filter(|d| d.is_none()).count();
    let each = if rest > 0 {
        (100.0 - used) / rest as f32
    } else {
        0.0
    };
    defaults.iter().map(|d| d.unwrap_or(each)).collect()
}

/// The sizes after dragging handle `i` by `delta` % from `start`, clamped so
/// neither neighbour goes below its minimum.
pub fn drag_sizes(start: &[f32], i: usize, delta: f32, min_a: f32, min_b: f32) -> Vec<f32> {
    let mut d = delta.max(min_a - start[i]);
    d = d.min(start[i + 1] - min_b);
    let mut next = start.to_vec();
    next[i] = start[i] + d;
    next[i + 1] = start[i + 1] - d;
    next
}

/// `ResizablePanels`: the first panel's size for a pointer at `ratio`
/// (0..1 of the box), clamped to `min..=max` %.
pub fn bar_size(ratio: f32, min: f32, max: f32) -> f32 {
    (ratio * 100.0).max(min).min(max)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// PanelGroup: 1 px hairlines with overlaid grab strips.
    Hairline,
    /// ResizablePanels: one 4 px bar in the flow.
    Bar { min: f32, max: f32 },
}

pub struct PanelGroup<'a, M> {
    cx: Ctx,
    dir: Direction,
    kind: Kind,
    sizes: Vec<f32>,
    panels: Vec<El<'a, M>>,
    mins: Vec<f32>,
    on_resize: Option<Box<dyn Fn(Vec<f32>) -> M + 'a>>,
    on_commit: Option<Box<dyn Fn(Vec<f32>) -> M + 'a>>,
}

/// A `PanelGroup` with these sizes (%); add one [`PanelGroup::panel`] per
/// size.
pub fn panel_group<'a, M: 'a>(cx: &Ctx, dir: Direction, sizes: &[f32]) -> PanelGroup<'a, M> {
    PanelGroup {
        cx: cx.clone(),
        dir,
        kind: Kind::Hairline,
        sizes: sizes.to_vec(),
        panels: Vec::new(),
        mins: Vec::new(),
        on_resize: None,
        on_commit: None,
    }
}

/// `ResizablePanels`: `first` at `size` % (clamped to `min..=max`), a 4 px
/// bar, `second` in the rest. `on_resize` / `on_commit` get
/// `[size, 100 − size]`.
pub fn resizable_panels<'a, M: 'a>(
    cx: &Ctx,
    dir: Direction,
    size: f32,
    min: f32,
    max: f32,
    first: impl Into<El<'a, M>>,
    second: impl Into<El<'a, M>>,
) -> PanelGroup<'a, M> {
    let s = size.max(min).min(max);
    let mut g = PanelGroup {
        cx: cx.clone(),
        dir,
        kind: Kind::Bar { min, max },
        sizes: vec![s, 100.0 - s],
        panels: Vec::new(),
        mins: Vec::new(),
        on_resize: None,
        on_commit: None,
    };
    g = g.panel(first, min).panel(second, 100.0 - max);
    g
}

impl<'a, M: 'a> PanelGroup<'a, M> {
    /// The next panel, at least `min` % (React's `min`, default [`MIN`]).
    pub fn panel(mut self, content: impl Into<El<'a, M>>, min: f32) -> Self {
        self.panels.push(
            scroll_area(&self.cx, content)
                .both()
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        );
        self.mins.push(min);
        self
    }
    /// Every size change while dragging.
    pub fn on_resize(mut self, f: impl Fn(Vec<f32>) -> M + 'a) -> Self {
        self.on_resize = Some(Box::new(f));
        self
    }
    /// The final sizes when a drag ends.
    pub fn on_commit(mut self, f: impl Fn(Vec<f32>) -> M + 'a) -> Self {
        self.on_commit = Some(Box::new(f));
        self
    }

    fn horizontal(&self) -> bool {
        self.dir == Direction::Horizontal
    }
    fn handle_px(&self) -> f32 {
        match self.kind {
            Kind::Hairline => 1.0,
            Kind::Bar { .. } => 4.0,
        }
    }
    /// Main-axis lengths of the panels for a group `total` long.
    fn lengths(&self, total: f32) -> Vec<f32> {
        let n = self.panels.len();
        match self.kind {
            Kind::Hairline => (0..n)
                .map(|i| total * self.sizes.get(i).copied().unwrap_or(0.0) / 100.0)
                .collect(),
            Kind::Bar { .. } => {
                let first = total * self.sizes.first().copied().unwrap_or(50.0) / 100.0;
                let mut v = vec![first];
                if n > 1 {
                    v.push((total - first - 4.0).max(0.0));
                }
                v
            }
        }
    }
    /// Handle rectangles (the visible line), from the panel lengths for a
    /// group `total` long (a panel's own node may round its size).
    fn handles(&self, layout: Layout<'_>, total: f32) -> Vec<Rectangle> {
        let t = self.handle_px();
        let b = layout.bounds();
        let horizontal = self.horizontal();
        let lens = self.lengths(total);
        let mut pos = 0.0;
        let mut out = Vec::new();
        for len in lens.iter().take(lens.len().saturating_sub(1)) {
            pos += len;
            out.push(if horizontal {
                Rectangle::new(Point::new(b.x + pos, b.y), Size::new(t, b.height))
            } else {
                Rectangle::new(Point::new(b.x, b.y + pos), Size::new(b.width, t))
            });
            pos += t;
        }
        out
    }
    /// The grab area of a handle.
    fn grab(&self, h: Rectangle) -> Rectangle {
        match self.kind {
            Kind::Bar { .. } => h,
            Kind::Hairline if self.horizontal() => Rectangle {
                x: h.x - GRAB,
                width: h.width + 2.0 * GRAB,
                ..h
            },
            Kind::Hairline => Rectangle {
                y: h.y - GRAB,
                height: h.height + 2.0 * GRAB,
                ..h
            },
        }
    }
    fn colors(&self) -> [Color; 3] {
        let p = self.cx.p();
        match self.kind {
            Kind::Hairline => [
                c(self.cx.line()),
                c(self.cx.line_strong()),
                c(p.accent_bright),
            ],
            Kind::Bar { .. } => [c(p.border), c(p.accent), c(p.accent)],
        }
    }
    fn cursor(&self) -> Cursor {
        if self.horizontal() {
            Cursor::ColResize
        } else {
            Cursor::RowResize
        }
    }
}

#[derive(Debug, Clone)]
struct Drag {
    handle: usize,
    finger: Option<u64>,
    start_pos: f32,
    start: Vec<f32>,
    last: Vec<f32>,
}

#[derive(Debug, Clone, Copy)]
struct Fade {
    from: Color,
    to: Color,
    start: Option<Instant>,
}

#[derive(Debug, Default)]
struct State {
    /// The group's length at the last layout.
    total: f32,
    drag: Option<Drag>,
    hover: Option<usize>,
    fades: Vec<Fade>,
}

impl State {
    /// Aim each handle's colour at its state; true while any fades.
    fn retarget(&mut self, colors: [Color; 3], n: usize, now: Instant) -> bool {
        self.fades.resize(
            n,
            Fade {
                from: colors[0],
                to: colors[0],
                start: None,
            },
        );
        for (i, f) in self.fades.iter_mut().enumerate() {
            let want = if self.drag.as_ref().is_some_and(|d| d.handle == i) {
                colors[2]
            } else if self.hover == Some(i) {
                colors[1]
            } else {
                colors[0]
            };
            if f.to != want {
                f.from = fade_at(f, now);
                f.to = want;
                f.start = Some(now);
            }
        }
        self.fades
            .iter()
            .any(|f| f.start.is_some_and(|s| now < s + FADE))
    }
}

fn fade_at(f: &Fade, now: Instant) -> Color {
    match f.start {
        Some(s) => {
            let t = (now.saturating_duration_since(s).as_secs_f32() / FADE.as_secs_f32()).min(1.0);
            lerp(f.from, f.to, ease_standard(t))
        }
        None => f.to,
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for PanelGroup<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }
    fn children(&self) -> Vec<Tree> {
        self.panels.iter().map(Tree::new).collect()
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.panels);
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let max = limits.max();
        let (total, cross) = if self.horizontal() {
            (max.width, max.height)
        } else {
            (max.height, max.width)
        };
        tree.state.downcast_mut::<State>().total = total;
        let lens = self.lengths(total);
        let gap = self.handle_px();
        let horizontal = self.horizontal();
        let mut pos = 0.0f32;
        let nodes: Vec<layout::Node> = self
            .panels
            .iter_mut()
            .zip(tree.children.iter_mut())
            .zip(&lens)
            .map(|((p, t), &len)| {
                let size = if horizontal {
                    Size::new(len, cross)
                } else {
                    Size::new(cross, len)
                };
                let n = p
                    .as_widget_mut()
                    .layout(t, renderer, &layout::Limits::new(size, size));
                let at = if horizontal {
                    Point::new(pos, 0.0)
                } else {
                    Point::new(0.0, pos)
                };
                pos += len + gap;
                n.move_to(at)
            })
            .collect();
        // Like flex items with fixed bases, the panels and hairlines may
        // overflow the group: report the overflowing length.
        let used = (pos - gap).max(0.0);
        let len = match self.kind {
            Kind::Hairline => used.max(total),
            Kind::Bar { .. } => total,
        };
        let size = if horizontal {
            Size::new(len, cross)
        } else {
            Size::new(cross, len)
        };
        layout::Node::with_children(size, nodes)
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
        let total = tree.state.downcast_ref::<State>().total;
        let handles = self.handles(layout, total);
        let bounds = layout.bounds();
        let horizontal = self.horizontal();
        let along = |p: Point| if horizontal { p.x } else { p.y };
        let origin = along(bounds.position());
        let visible = |p: Point| viewport.contains(p) && bounds.contains(p);
        let hit = |p: Point| {
            if !visible(p) {
                return None;
            }
            handles.iter().position(|h| self.grab(*h).contains(p))
        };
        let colors = self.colors();
        let n = handles.len();

        // The strips sit over the panels (z-10): they see presses first.
        let state = tree.state.downcast_mut::<State>();
        let mut changed = false;
        let press = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                cursor.position().and_then(|p| hit(p).map(|i| (i, p, None)))
            }
            Event::Touch(touch::Event::FingerPressed { id, position }) => {
                hit(*position).map(|i| (i, *position, Some(id.0)))
            }
            _ => None,
        };
        if let Some((i, p, finger)) = press
            && state.drag.is_none()
            && !shell.is_event_captured()
        {
            state.drag = Some(Drag {
                handle: i,
                finger,
                start_pos: along(p),
                start: self.sizes.clone(),
                last: self.sizes.clone(),
            });
            shell.capture_event();
            changed = true;
        }

        let moved = match event {
            Event::Mouse(mouse::Event::CursorMoved { position }) => Some((*position, None)),
            Event::Touch(touch::Event::FingerMoved { id, position }) => {
                Some((*position, Some(id.0)))
            }
            _ => None,
        };
        if let Some((p, finger)) = moved {
            if let Some(d) = state.drag.as_mut()
                && d.finger == finger
                && total > 0.0
            {
                let next = match self.kind {
                    Kind::Hairline => {
                        let delta = (along(p) - d.start_pos) / total * 100.0;
                        drag_sizes(
                            &d.start,
                            d.handle,
                            delta,
                            self.mins.get(d.handle).copied().unwrap_or(MIN),
                            self.mins.get(d.handle + 1).copied().unwrap_or(MIN),
                        )
                    }
                    Kind::Bar { min, max } => {
                        let s = bar_size((along(p) - origin) / total, min, max);
                        vec![s, 100.0 - s]
                    }
                };
                if next != d.last {
                    d.last = next.clone();
                    if let Some(f) = &self.on_resize {
                        shell.publish(f(next));
                    }
                }
                shell.capture_event();
            } else if finger.is_none() {
                let h = hit(p);
                if h != state.hover {
                    state.hover = h;
                    changed = true;
                }
            }
        }
        if matches!(event, Event::Mouse(mouse::Event::CursorLeft)) && state.hover.is_some() {
            state.hover = None;
            changed = true;
        }

        let released = match event {
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(None),
            Event::Touch(
                touch::Event::FingerLifted { id, .. } | touch::Event::FingerLost { id, .. },
            ) => Some(Some(id.0)),
            _ => None,
        };
        if let Some(finger) = released
            && state.drag.as_ref().is_some_and(|d| d.finger == finger)
        {
            let d = state.drag.take().unwrap();
            if d.last != d.start
                && let Some(f) = &self.on_commit
            {
                shell.publish(f(d.last));
            }
            // The pointer may have left the strip while dragging.
            state.hover = cursor.position().and_then(hit);
            shell.capture_event();
            changed = true;
        }

        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            if state.retarget(colors, n, *now) {
                shell.request_redraw();
            }
        } else if changed {
            state.retarget(colors, n, Instant::now());
            shell.request_redraw();
        }

        if state.drag.is_some() {
            // The panels see no pointer input while a handle is held.
            if matches!(event, Event::Mouse(_) | Event::Touch(_)) {
                return;
            }
        }
        for ((p, t), l) in self
            .panels
            .iter_mut()
            .zip(tree.children.iter_mut())
            .zip(layout.children())
        {
            p.as_widget_mut()
                .update(t, event, l, cursor, renderer, clipboard, shell, viewport);
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
        let over_strip = cursor.position().is_some_and(|p| {
            viewport.contains(p)
                && layout.bounds().contains(p)
                && self
                    .handles(layout, state.total)
                    .iter()
                    .any(|h| self.grab(*h).contains(p))
        });
        if state.drag.is_some() || over_strip {
            return self.cursor().into();
        }
        self.panels
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((p, t), l)| {
                p.as_widget()
                    .mouse_interaction(t, l, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default()
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        operation.traverse(&mut |operation| {
            for ((p, t), l) in self
                .panels
                .iter_mut()
                .zip(tree.children.iter_mut())
                .zip(layout.children())
            {
                p.as_widget_mut().operate(t, l, renderer, operation);
            }
        });
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
        let dragging = state.drag.is_some();
        for ((p, t), l) in self
            .panels
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
        {
            let b = l.bounds();
            let Some(clip) = b.intersection(viewport) else {
                continue;
            };
            // While a handle is held the panels show no hover.
            let cur = if dragging {
                mouse::Cursor::Unavailable
            } else {
                cursor
            };
            renderer.with_layer(clip, |renderer| {
                p.as_widget().draw(t, renderer, theme, style, l, cur, &clip);
            });
        }
        let now = Instant::now();
        let rest = self.colors()[0];
        for (i, h) in self.handles(layout, state.total).into_iter().enumerate() {
            let color = state.fades.get(i).map_or(rest, |f| fade_at(f, now));
            renderer.fill_quad(
                Quad {
                    bounds: snap_rect(h),
                    ..Quad::default()
                },
                fade(color),
            );
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        iced::advanced::overlay::from_children(
            &mut self.panels,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: 'a> From<PanelGroup<'a, M>> for El<'a, M> {
    fn from(g: PanelGroup<'a, M>) -> Self {
        Element::new(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_sizes_like_react() {
        assert_eq!(initial_sizes(&[Some(28.0), None], None), vec![28.0, 72.0]);
        assert_eq!(initial_sizes(&[None, Some(32.0)], None), vec![68.0, 32.0]);
        assert_eq!(initial_sizes(&[None, None, None], None)[0], 100.0 / 3.0);
        assert_eq!(
            initial_sizes(&[Some(28.0), None], Some(&[40.0, 60.0])),
            vec![40.0, 60.0],
            "stored sizes win"
        );
        assert_eq!(
            initial_sizes(&[Some(28.0), None], Some(&[10.0, 20.0, 70.0])),
            vec![28.0, 72.0],
            "a stored array of the wrong length is ignored"
        );
    }

    #[test]
    fn drag_clamps_to_both_minimums() {
        let s = [28.0, 72.0];
        assert_eq!(drag_sizes(&s, 0, 10.0, 15.0, 5.0), vec![38.0, 62.0]);
        assert_eq!(drag_sizes(&s, 0, -20.0, 15.0, 5.0), vec![15.0, 85.0]);
        assert_eq!(drag_sizes(&s, 0, 90.0, 15.0, 5.0), vec![95.0, 5.0]);
        let three = [30.0, 40.0, 30.0];
        assert_eq!(drag_sizes(&three, 1, 5.0, 5.0, 5.0), vec![30.0, 45.0, 25.0]);
    }

    #[test]
    fn bar_follows_the_pointer_within_bounds() {
        assert_eq!(bar_size(0.5, 15.0, 85.0), 50.0);
        assert_eq!(bar_size(0.02, 15.0, 85.0), 15.0);
        assert_eq!(bar_size(1.2, 15.0, 85.0), 85.0);
    }

    #[test]
    fn handle_colours_fade() {
        let colors = [Color::BLACK, Color::WHITE, Color::from_rgb(1.0, 0.0, 0.0)];
        let mut s = State::default();
        let t0 = Instant::now();
        assert!(!s.retarget(colors, 2, t0), "at rest nothing moves");
        s.hover = Some(1);
        assert!(s.retarget(colors, 2, t0));
        assert_eq!(fade_at(&s.fades[1], t0 + FADE), Color::WHITE);
        assert_eq!(fade_at(&s.fades[0], t0 + FADE), Color::BLACK);
    }
}
