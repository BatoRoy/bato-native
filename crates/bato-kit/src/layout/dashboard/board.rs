//! The grid box (`relative w-full rounded-xl`, `minHeight`) and its
//! absolutely placed widgets: placement with the 150 ms
//! `transition-[left,top,width,height]`, the card frame, the edit-mode
//! dashed outline and resize mark, and the pointer gestures (header drag,
//! corner resize) that publish whole-cell deltas.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, mouse, touch, window};

use super::grid::{DashboardItem, GAP, HEADER, Handle, Metrics, cells};
use super::{DashboardMsg, Frame};
use crate::anim::{Tween, ease_standard, fade};
use crate::cursor::Cursor;
use crate::overlay::context_menu::paint_dashed;
use crate::scale::snap_rect;
use crate::scroller::chain;
use crate::surface::{BoxStyle, paint};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// `duration-150` (Tailwind's default `cubic-bezier(.4, 0, .2, 1)`).
pub const MOVE: Duration = Duration::from_millis(150);
/// The cursor a widget behind another sees: far outside it, so hovers
/// end (a chart's crosshair, the × colour) as when the pointer leaves.
const AWAY: mouse::Cursor = mouse::Cursor::Available(Point::new(-1.0e6, -1.0e6));
/// The resize corner: `absolute bottom-0 right-0 w-4 h-4`.
pub const CORNER: f32 = 16.0;

pub(super) struct Board<'a, M> {
    pub frame: Frame,
    pub metrics_cols: i32,
    pub row_height: f32,
    pub items: Vec<DashboardItem>,
    pub cells: Vec<El<'a, M>>,
    pub edit: bool,
    pub active: Option<String>,
    pub on: fn(DashboardMsg) -> M,
}

/// One coordinate set of a CSS transition on `left/top/width/height`.
#[derive(Debug, Clone, Copy)]
struct Placed {
    x: Tween,
    y: Tween,
    w: Tween,
    h: Tween,
}

impl Placed {
    fn settled(r: Rectangle) -> Self {
        Self {
            x: Tween::settled(r.x),
            y: Tween::settled(r.y),
            w: Tween::settled(r.width),
            h: Tween::settled(r.height),
        }
    }
    fn target(&self) -> Rectangle {
        Rectangle::new(
            Point::new(self.x.to, self.y.to),
            Size::new(self.w.to, self.h.to),
        )
    }
    fn at(&self, now: Instant) -> Rectangle {
        Rectangle::new(
            Point::new(self.x.value(now), self.y.value(now)),
            Size::new(self.w.value(now), self.h.value(now)),
        )
    }
    /// Aim at `r`: animated from where it is now, or at once.
    fn aim(&mut self, r: Rectangle, animate: bool, now: Instant) {
        if self.target() == r {
            return;
        }
        if !animate {
            *self = Self::settled(r);
            return;
        }
        for (t, v) in [
            (&mut self.x, r.x),
            (&mut self.y, r.y),
            (&mut self.w, r.width),
            (&mut self.h, r.height),
        ] {
            if t.to != v || t.is_running(now) {
                t.retarget(v, now);
                t.duration = MOVE;
                t.easing = ease_standard;
            }
        }
    }
    fn running(&self, now: Instant) -> bool {
        [self.x, self.y, self.w, self.h]
            .iter()
            .any(|t| t.is_running(now))
    }
}

#[derive(Debug, Clone)]
struct Press {
    finger: Option<u64>,
    start: Point,
    last: (i32, i32),
    handle: Handle,
}

struct State {
    placed: Vec<(String, Placed)>,
    press: Option<Press>,
    /// The cell pitch of the last layout.
    cell: (f32, f32),
    token: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            placed: Vec::new(),
            press: None,
            cell: (0.0, 0.0),
            token: chain::next_id(),
        }
    }
}

impl State {
    fn running(&self, now: Instant) -> bool {
        self.placed.iter().any(|(_, p)| p.running(now))
    }
}

impl<'a, M: 'a> Board<'a, M> {
    /// The draw order: the dragged widget last (`z-10`).
    fn order(&self) -> Vec<usize> {
        let mut o: Vec<usize> = (0..self.items.len())
            .filter(|&i| Some(&self.items[i].id) != self.active.as_ref())
            .collect();
        o.extend(
            (0..self.items.len()).filter(|&i| Some(&self.items[i].id) == self.active.as_ref()),
        );
        o
    }

    /// The topmost widget under `p` (layout coordinates).
    fn hit(&self, layout: Layout<'_>, p: Point) -> Option<usize> {
        let rects: Vec<Rectangle> = layout.children().map(|l| frame_of(l.bounds())).collect();
        self.order()
            .into_iter()
            .rev()
            .find(|&i| rects[i].contains(p))
    }

    /// Which gesture a press at `p` on widget `i` starts (edit mode).
    fn zone(&self, layout: Layout<'_>, i: usize, p: Point) -> Option<Handle> {
        let b = layout.children().nth(i)?.bounds();
        let corner = Rectangle::new(
            Point::new(b.x + b.width - CORNER, b.y + b.height - CORNER),
            Size::new(CORNER, CORNER),
        );
        let header = Rectangle::new(b.position(), Size::new(b.width, HEADER));
        if corner.contains(p) {
            Some(Handle::Resize)
        } else if header.contains(p) {
            Some(Handle::Move)
        } else {
            None
        }
    }
}

/// Start a gesture on widget `id` (`setActiveId`).
fn begin<M>(
    state: &mut State,
    shell: &mut Shell<'_, M>,
    on: fn(DashboardMsg) -> M,
    id: String,
    p: Point,
    finger: Option<u64>,
    handle: Handle,
) {
    if let Some(f) = finger {
        chain::touch_claim(f, state.token);
    }
    state.press = Some(Press {
        finger,
        start: p,
        last: (0, 0),
        handle,
    });
    shell.publish(on(DashboardMsg::Begin(id, handle)));
    shell.capture_event();
    shell.request_redraw();
}

/// The widget's border box around its content box (the 1 px border).
fn frame_of(content: Rectangle) -> Rectangle {
    content.expand(1.0)
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for Board<'a, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }
    fn children(&self) -> Vec<Tree> {
        self.cells.iter().map(Tree::new).collect()
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.cells);
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let width = limits.max().width;
        let m = Metrics {
            cols: self.metrics_cols,
            row_height: self.row_height,
            width,
        };
        let now = Instant::now();
        let state = tree.state.downcast_mut::<State>();
        state.cell = (m.cell_w(), m.cell_h());
        state
            .placed
            .retain(|(k, _)| self.items.iter().any(|it| &it.id == k));
        let mut rects = Vec::with_capacity(self.items.len());
        for it in &self.items {
            let target = m.rect(it);
            let animate = self.active.as_ref() != Some(&it.id);
            let r = match state.placed.iter_mut().find(|(k, _)| k == &it.id) {
                Some((_, p)) => {
                    p.aim(target, animate, now);
                    p.at(now)
                }
                None => {
                    state.placed.push((it.id.clone(), Placed::settled(target)));
                    target
                }
            };
            rects.push(r);
        }
        let height = m.height(&self.items);
        let nodes = self
            .cells
            .iter_mut()
            .zip(tree.children.iter_mut())
            .zip(rects)
            .map(|((cell, t), r)| {
                // The content box: inside the 1 px border (fractional, like
                // Chromium's layout; only the painted frame is snapped).
                let size = Size::new((r.width - 2.0).max(0.0), (r.height - 2.0).max(0.0));
                cell.as_widget_mut()
                    .layout(t, renderer, &layout::Limits::new(size, size))
                    .move_to(Point::new(r.x + 1.0, r.y + 1.0))
            })
            .collect();
        layout::Node::with_children(Size::new(width, height), nodes)
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
        let on = self.on;
        let visible = |p: Point| viewport.contains(p) && layout.bounds().contains(p);

        // A press on the corner starts a resize before the content sees it
        // (the corner sits over the body).
        let press = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                cursor.position().map(|p| (p, None))
            }
            Event::Touch(touch::Event::FingerPressed { id, position }) => {
                Some((*position, Some(id.0)))
            }
            _ => None,
        };
        let top = cursor
            .position()
            .filter(|p| visible(*p))
            .and_then(|p| self.hit(layout, p));
        if self.edit
            && let Some((p, finger)) = press
            && visible(p)
            && tree.state.downcast_ref::<State>().press.is_none()
            && tree.state.downcast_ref::<State>().cell.0 > GAP
            && !shell.is_event_captured()
            && let Some(i) = self.hit(layout, p)
            && self.zone(layout, i, p) == Some(Handle::Resize)
        {
            let id = self.items[i].id.clone();
            begin(
                tree.state.downcast_mut::<State>(),
                shell,
                on,
                id,
                p,
                finger,
                Handle::Resize,
            );
            return;
        }

        // Gesture in progress: whole-cell deltas from the press point.
        let moved = match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => cursor.position().map(|p| (p, None)),
            Event::Touch(touch::Event::FingerMoved { id, position }) => {
                Some((*position, Some(id.0)))
            }
            _ => None,
        };
        let state = tree.state.downcast_mut::<State>();
        if let Some((p, finger)) = moved
            && let Some(pr) = state.press.as_mut()
            && pr.finger == finger
        {
            let d = (
                cells(p.x - pr.start.x, state.cell.0),
                cells(p.y - pr.start.y, state.cell.1),
            );
            if d != pr.last {
                pr.last = d;
                shell.publish(on(DashboardMsg::Step(d.0, d.1)));
            }
            if finger.is_some() {
                shell.capture_event();
            }
        }
        let released = match event {
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(None),
            Event::Touch(touch::Event::FingerLifted { id, .. }) => Some(Some(id.0)),
            Event::Touch(touch::Event::FingerLost { id, .. }) if !chain::is_synthetic_lost() => {
                Some(Some(id.0))
            }
            _ => None,
        };
        if let Some(finger) = released
            && state.press.as_ref().is_some_and(|p| p.finger == finger)
        {
            state.press = None;
            if let Some(f) = finger {
                chain::touch_release(f, state.token);
            }
            shell.publish(on(DashboardMsg::End));
            shell.capture_event();
        }

        if let Event::Window(window::Event::RedrawRequested(now)) = event
            && state.running(*now)
        {
            shell.invalidate_layout();
            shell.request_redraw();
        }

        // The content. Mouse input reaches only the topmost widget under
        // the cursor (the others are behind it).
        let pointer = !matches!(event, Event::Touch(_) | Event::Keyboard(_));
        for (i, ((cell, t), l)) in self
            .cells
            .iter_mut()
            .zip(tree.children.iter_mut())
            .zip(layout.children())
            .enumerate()
        {
            let cur = if pointer && top != Some(i) {
                AWAY
            } else {
                cursor
            };
            cell.as_widget_mut()
                .update(t, event, l, cur, renderer, clipboard, shell, viewport);
        }

        // A header press the content did not take (the remove button
        // does) starts a move.
        if self.edit
            && let Some((p, finger)) = press
            && visible(p)
            && !shell.is_event_captured()
            && tree.state.downcast_ref::<State>().press.is_none()
            && tree.state.downcast_ref::<State>().cell.0 > GAP
            && let Some(i) = self.hit(layout, p)
            && self.zone(layout, i, p) == Some(Handle::Move)
        {
            let id = self.items[i].id.clone();
            begin(
                tree.state.downcast_mut::<State>(),
                shell,
                on,
                id,
                p,
                finger,
                Handle::Move,
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
        let gesture_cursor = |h: Handle| -> mouse::Interaction {
            match h {
                Handle::Move => Cursor::Move.into(),
                Handle::Resize => Cursor::NwseResize.into(),
            }
        };
        if let Some(pr) = &state.press {
            return gesture_cursor(pr.handle);
        }
        let Some(p) = cursor.position().filter(|p| viewport.contains(*p)) else {
            return mouse::Interaction::None;
        };
        let Some(i) = self.hit(layout, p) else {
            return mouse::Interaction::None;
        };
        let zone = if self.edit {
            self.zone(layout, i, p)
        } else {
            None
        };
        if zone == Some(Handle::Resize) {
            return gesture_cursor(Handle::Resize);
        }
        let (Some(cell), Some(t), Some(l)) = (
            self.cells.get(i),
            tree.children.get(i),
            layout.children().nth(i),
        ) else {
            return mouse::Interaction::None;
        };
        let inner = cell
            .as_widget()
            .mouse_interaction(t, l, cursor, viewport, renderer);
        match zone {
            Some(Handle::Move)
                if matches!(inner, mouse::Interaction::None | mouse::Interaction::Idle) =>
            {
                gesture_cursor(Handle::Move)
            }
            _ => inner,
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        operation.traverse(&mut |operation| {
            for ((cell, t), l) in self
                .cells
                .iter_mut()
                .zip(tree.children.iter_mut())
                .zip(layout.children())
            {
                cell.as_widget_mut().operate(t, l, renderer, operation);
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
        let f = &self.frame;
        let bounds = layout.bounds();
        if self.edit {
            // `bg-app-bg/30` and `outline-1 outline-dashed` round `rounded-xl`.
            paint(
                renderer,
                bounds,
                &BoxStyle::default().bg(f.edit_bg).radius(12.0),
            );
            paint_dashed(renderer, bounds.expand(1.0), 13.0, 1.0, f.border);
        }
        let top = cursor.position().and_then(|p| {
            (viewport.contains(p) && bounds.contains(p))
                .then(|| self.hit(layout, p))
                .flatten()
        });
        let children: Vec<Layout<'_>> = layout.children().collect();
        for i in self.order() {
            let l = children[i];
            let content = l.bounds();
            let outer = snap_rect(frame_of(content));
            if outer.intersection(viewport).is_none() {
                continue;
            }
            let dragging = self.active.as_ref() == Some(&self.items[i].id);
            let mut s = BoxStyle::default()
                .bg(f.card)
                .border(if dragging { f.accent } else { f.border })
                .radius(8.0);
            if dragging {
                s = s.shadows(f.shadow.clone());
            }
            // Each widget is its own stack of layers (frame, content,
            // corner mark), so a widget on top covers the text and icons
            // of the one behind it: iced draws a layer's text after all of
            // its quads.
            renderer.with_layer(*viewport, |renderer| paint(renderer, outer, &s));
            let cur = if top == Some(i) { cursor } else { AWAY };
            if let Some(clip) = content.intersection(viewport) {
                renderer.with_layer(clip, |renderer| {
                    self.cells[i].as_widget().draw(
                        &tree.children[i],
                        renderer,
                        theme,
                        style,
                        l,
                        cur,
                        &clip,
                    );
                });
            }
            if self.edit
                && let Some(clip) = content.intersection(viewport)
            {
                // `absolute bottom-1 right-1 w-2 h-2 border-r-2 border-b-2`.
                let (r, b) = (
                    content.x + content.width - 4.0,
                    content.y + content.height - 4.0,
                );
                let color = fade(f.muted);
                renderer.with_layer(clip, |renderer| {
                    for q in [
                        Rectangle::new(Point::new(r - 2.0, b - 8.0), Size::new(2.0, 8.0)),
                        Rectangle::new(Point::new(r - 8.0, b - 2.0), Size::new(8.0, 2.0)),
                    ] {
                        renderer.fill_quad(
                            Quad {
                                bounds: q,
                                ..Quad::default()
                            },
                            color,
                        );
                    }
                });
            }
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
            &mut self.cells,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: 'a> From<Board<'a, M>> for El<'a, M> {
    fn from(b: Board<'a, M>) -> Self {
        Element::new(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_animates_except_when_dragged() {
        let t0 = Instant::now();
        let a = Rectangle::new(Point::new(0.0, 0.0), Size::new(100.0, 50.0));
        let b = Rectangle::new(Point::new(80.0, 66.0), Size::new(100.0, 50.0));
        let mut p = Placed::settled(a);
        assert!(!p.running(t0));
        p.aim(b, true, t0);
        assert!(p.running(t0));
        let mid = p.at(t0 + MOVE / 2);
        assert!(mid.x > 0.0 && mid.x < 80.0 && mid.y > 0.0 && mid.y < 66.0);
        assert_eq!(p.at(t0 + MOVE), b);
        assert!(!p.running(t0 + MOVE));
        p.aim(a, false, t0);
        assert_eq!(p.at(t0), a, "the dragged widget jumps");
    }
}
