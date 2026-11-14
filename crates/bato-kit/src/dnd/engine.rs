//! The drag engine shared by the sortable widgets: the sensor state machine
//! ([`Session`]), the FLIP motion of the items ([`Motion`]), handle marks
//! ([`handle`], [`no_drag`]) and the drag preview overlay ([`Preview`]).

use std::any::Any;
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::widget::{self, Operation, Tree, Widget};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::{Color, Element, Event, Length, Point, Rectangle, Size, Vector, mouse};

use crate::anim::{cubic_bezier, with_opacity};
use crate::focus::{Ring, paint_ring};
use crate::scroller::chain;
use crate::surface::{BoxStyle, paint};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// dnd-kit's `PointerSensor` `activationConstraint: { distance: 4 }`: a
/// press becomes a drag once the pointer is more than this far (CSS px).
pub const ACTIVATION: f32 = 4.0;
/// `useSortable`'s `transition`: `transform 200ms ease`.
pub const SORT: Duration = Duration::from_millis(200);
/// `DragOverlay`'s drop animation: 250 ms `ease`.
pub const DROP: Duration = Duration::from_millis(250);

/// CSS `ease` = `cubic-bezier(0.25, 0.1, 0.25, 1)`.
pub fn ease(t: f32) -> f32 {
    cubic_bezier(0.25, 0.1, 0.25, 1.0, t)
}

/// What drives a drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pointer {
    Mouse,
    Finger(u64),
    Keyboard,
}

/// A press or a drag in progress. Positions are window coordinates, so
/// the pointer stays put while scrollers move the content under it.
#[derive(Debug, Clone, PartialEq)]
pub struct Grab<K> {
    pub key: K,
    pub pointer: Pointer,
    /// Where the press landed.
    pub start: Point,
    /// Where the pointer is now.
    pub at: Point,
    /// The press position minus the item's top-left.
    pub grab: Vector,
    /// The finger is held in the scrollers' touch latch.
    pub claimed: bool,
}

impl<K> Grab<K> {
    /// The preview's top-left (window coordinates).
    pub fn top_left(&self) -> Point {
        self.at - self.grab
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Phase<K> {
    #[default]
    Idle,
    /// Pressed, not yet past [`ACTIVATION`].
    Pending(Grab<K>),
    Dragging(Grab<K>),
}

/// What a sensor event did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Nothing,
    Started,
    Moved,
    Dropped,
    Cancelled,
}

/// The sensors (dnd-kit's `PointerSensor` / `KeyboardSensor`) for one
/// widget, plus the auto-scroll bookkeeping.
#[derive(Debug)]
pub struct Session<K> {
    pub phase: Phase<K>,
    /// The touch-latch identity (see [`chain::touch_claim`]).
    pub token: u64,
    /// dnd-kit's scroll intent: the directions the pointer has moved in
    /// during this drag (x−, x+, y−, y+). Auto-scroll goes only those ways.
    pub intent: [bool; 4],
    last: Point,
    tick: Option<Instant>,
    /// An auto-scroll step is due on the next frame.
    armed: bool,
}

impl<K> Default for Session<K> {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            token: chain::next_id(),
            intent: [false; 4],
            last: Point::ORIGIN,
            tick: None,
            armed: false,
        }
    }
}

impl<K: Clone + PartialEq> Session<K> {
    pub fn dragging(&self) -> Option<&Grab<K>> {
        match &self.phase {
            Phase::Dragging(g) => Some(g),
            _ => None,
        }
    }
    pub fn pressed(&self) -> Option<&Grab<K>> {
        match &self.phase {
            Phase::Pending(g) | Phase::Dragging(g) => Some(g),
            Phase::Idle => None,
        }
    }
    pub fn is_idle(&self) -> bool {
        self.phase == Phase::Idle
    }

    /// A press on item `key` at `at` (window), `grab` from its top-left.
    /// `claim`: hold a finger at once (a `touch-action: none` handle), so
    /// the scrollers around never take it.
    pub fn press(&mut self, key: K, pointer: Pointer, at: Point, grab: Vector, claim: bool) {
        let claimed = match pointer {
            Pointer::Finger(f) if claim => {
                chain::touch_claim(f, self.token);
                true
            }
            _ => false,
        };
        self.phase = Phase::Pending(Grab {
            key,
            pointer,
            start: at,
            at,
            grab,
            claimed,
        });
    }

    /// A keyboard pick-up (Space / Enter on a focused item).
    pub fn pick_up(&mut self, key: K, top_left: Point) {
        self.release_latch();
        self.phase = Phase::Dragging(Grab {
            key,
            pointer: Pointer::Keyboard,
            start: top_left,
            at: top_left,
            grab: Vector::ZERO,
            claimed: false,
        });
        self.reset_scroll();
    }

    /// The pointer moved to `at` (window).
    pub fn moved(&mut self, pointer: Pointer, at: Point) -> Signal {
        match &mut self.phase {
            Phase::Pending(g) if g.pointer == pointer => {
                g.at = at;
                if (at - g.start).x.hypot((at - g.start).y) > ACTIVATION {
                    let g = g.clone();
                    self.phase = Phase::Dragging(g);
                    self.reset_scroll();
                    self.last = at;
                    self.armed = true;
                    Signal::Started
                } else {
                    Signal::Nothing
                }
            }
            Phase::Dragging(g) if g.pointer == pointer => {
                g.at = at;
                let d = at - self.last;
                for (i, v) in [(0, -d.x), (1, d.x), (2, -d.y), (3, d.y)] {
                    if v > 0.0 {
                        self.intent[i] = true;
                    }
                }
                self.last = at;
                self.armed = true;
                Signal::Moved
            }
            _ => Signal::Nothing,
        }
    }

    /// The pointer was released.
    pub fn released(&mut self, pointer: Pointer) -> Signal {
        match &self.phase {
            Phase::Pending(g) if g.pointer == pointer => {
                self.end();
                Signal::Nothing
            }
            Phase::Dragging(g) if g.pointer == pointer => {
                self.end();
                Signal::Dropped
            }
            _ => Signal::Nothing,
        }
    }

    /// Esc, a lost finger, a resized window: drop nothing.
    pub fn cancel(&mut self) -> Signal {
        let was = matches!(self.phase, Phase::Dragging(_));
        if self.is_idle() {
            return Signal::Nothing;
        }
        self.end();
        if was {
            Signal::Cancelled
        } else {
            Signal::Nothing
        }
    }

    /// A keyboard drop.
    pub fn drop_keyboard(&mut self) -> Signal {
        match &self.phase {
            Phase::Dragging(g) if g.pointer == Pointer::Keyboard => {
                self.end();
                Signal::Dropped
            }
            _ => Signal::Nothing,
        }
    }

    fn release_latch(&mut self) {
        if let Phase::Pending(g) | Phase::Dragging(g) = &self.phase
            && let (Pointer::Finger(f), true) = (g.pointer, g.claimed)
        {
            chain::touch_release(f, self.token);
        }
    }

    fn end(&mut self) {
        self.release_latch();
        self.phase = Phase::Idle;
        self.reset_scroll();
    }

    fn reset_scroll(&mut self) {
        self.intent = [false; 4];
        self.tick = None;
        self.armed = false;
        chain::take_edge_scrolled();
    }

    /// On `RedrawRequested` during a pointer drag: post an auto-scroll step
    /// for the pointer at `at` (layout coordinates) when one is due (the
    /// pointer moved, or the last step scrolled). Returns whether to ask
    /// for another frame.
    pub fn auto_scroll(&mut self, at: Point, now: Instant) -> bool {
        let Some(g) = self.dragging() else {
            return false;
        };
        if g.pointer == Pointer::Keyboard {
            return false;
        }
        let scrolled = chain::take_edge_scrolled();
        if !(self.armed || scrolled) {
            self.tick = None;
            return false;
        }
        self.armed = false;
        let ms = self
            .tick
            .map_or(16.0, |t| {
                now.saturating_duration_since(t).as_secs_f32() * 1000.0
            })
            .clamp(1.0, 50.0);
        self.tick = Some(now);
        crate::scroller::edge_scroll(at, ms, self.intent);
        true
    }
}

// ── Motion ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct Anim {
    from: Vector,
    to: Vector,
    start: Instant,
    dur: Duration,
}

impl Anim {
    fn value(&self, now: Instant) -> Vector {
        let t = if self.dur.is_zero() {
            1.0
        } else {
            now.saturating_duration_since(self.start).as_secs_f32() / self.dur.as_secs_f32()
        };
        if t >= 1.0 {
            self.to
        } else {
            let e = ease(t);
            self.from + (self.to - self.from) * e
        }
    }
    fn running(&self, now: Instant) -> bool {
        !self.dur.is_zero() && now < self.start + self.dur
    }
}

/// Where each item is drawn (top-left, relative to the widget), keyed so it
/// survives reorders and moves between lists: a CSS `transform` transition
/// plus dnd-kit's FLIP of items whose index changed mid-drag.
#[derive(Debug)]
pub struct Motion<K> {
    items: Vec<(K, Anim)>,
}

impl<K> Default for Motion<K> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<K: Clone + PartialEq> Motion<K> {
    /// Aim `key` at `to`: animated (200 ms `ease`) from where it is drawn now,
    /// or at once. A key seen for the first time starts there.
    pub fn aim(&mut self, key: &K, to: Vector, animate: bool, now: Instant) {
        match self.items.iter_mut().find(|(k, _)| k == key) {
            Some((_, a)) => {
                if a.to != to {
                    let from = a.value(now);
                    *a = Anim {
                        from: if animate { from } else { to },
                        to,
                        start: now,
                        dur: if animate { SORT } else { Duration::ZERO },
                    };
                }
            }
            None => self.items.push((
                key.clone(),
                Anim {
                    from: to,
                    to,
                    start: now,
                    dur: Duration::ZERO,
                },
            )),
        }
    }
    /// Put `key` at `from` and animate it to `to`.
    pub fn slide(&mut self, key: &K, from: Vector, to: Vector, now: Instant) {
        self.aim(key, from, false, now);
        self.aim(key, to, true, now);
    }
    /// Where `key` is drawn now.
    pub fn at(&self, key: &K, now: Instant) -> Option<Vector> {
        self.items
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, a)| a.value(now))
    }
    pub fn running(&self, now: Instant) -> bool {
        self.items.iter().any(|(_, a)| a.running(now))
    }
    /// Forget keys no longer shown.
    pub fn retain(&mut self, keep: impl Fn(&K) -> bool) {
        self.items.retain(|(k, _)| keep(k));
    }
}

// ── Marks ───────────────────────────────────────────────────────────────

/// What a [`Mark`] says about the area it wraps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    /// Drags start here only (dnd-kit's handle `listeners`). An item with
    /// no handle drags from anywhere.
    Handle,
    /// Presses here never start a drag (`onPointerDown` stopPropagation).
    NoDrag,
}

/// Wraps content and tells the sortable around it what the area is for.
pub struct Mark<'a, M> {
    content: El<'a, M>,
    kind: MarkKind,
}

/// A drag handle (the grip of a SortableList row).
pub fn handle<'a, M>(content: impl Into<El<'a, M>>) -> Mark<'a, M> {
    Mark {
        content: content.into(),
        kind: MarkKind::Handle,
    }
}

/// An area that never starts a drag (an editor tab's close button).
pub fn no_drag<'a, M>(content: impl Into<El<'a, M>>) -> Mark<'a, M> {
    Mark {
        content: content.into(),
        kind: MarkKind::NoDrag,
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for Mark<'a, M> {
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
        operation: &mut dyn Operation,
    ) {
        operation.custom(None, layout.bounds(), &mut self.kind);
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

impl<'a, M: 'a> From<Mark<'a, M>> for El<'a, M> {
    fn from(m: Mark<'a, M>) -> Self {
        Element::new(m)
    }
}

/// Collects the [`Mark`]s inside an item.
#[derive(Default)]
struct Marks(Vec<(MarkKind, Rectangle)>);

impl Operation for Marks {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
    fn custom(&mut self, _id: Option<&widget::Id>, bounds: Rectangle, state: &mut dyn Any) {
        if let Some(k) = state.downcast_ref::<MarkKind>() {
            self.0.push((*k, bounds));
        }
    }
}

/// May a press at `p` (layout coordinates, item drawn at its layout
/// position) start dragging this item?
pub fn may_drag<M>(
    item: &mut El<'_, M>,
    tree: &mut Tree,
    layout: Layout<'_>,
    renderer: &iced::Renderer,
    p: Point,
) -> (bool, bool) {
    let mut marks = Marks::default();
    item.as_widget_mut()
        .operate(tree, layout, renderer, &mut marks);
    if marks
        .0
        .iter()
        .any(|(k, b)| *k == MarkKind::NoDrag && b.contains(p))
    {
        return (false, false);
    }
    let handles: Vec<&Rectangle> = marks
        .0
        .iter()
        .filter(|(k, _)| *k == MarkKind::Handle)
        .map(|(_, b)| b)
        .collect();
    if handles.is_empty() {
        (true, false)
    } else {
        (handles.iter().any(|b| b.contains(p)), true)
    }
}

// ── Preview ─────────────────────────────────────────────────────────────

/// How the dragged item is drawn in the overlay layer.
#[derive(Debug, Clone, Default)]
pub struct PreviewStyle {
    /// Group opacity (`opacity-50`).
    pub opacity: f32,
    /// Painted at the item's box before it (a shadow), and after it (the
    /// shadow's inset top highlight).
    pub under: Option<BoxStyle>,
    pub over: Option<BoxStyle>,
}

/// The dragged item, drawn again on top of everything at `at` (window
/// coordinates): an item's element with its own tree and layout.
pub struct Preview<'b, 'a, M> {
    pub content: &'b El<'a, M>,
    pub tree: &'b Tree,
    pub layout: Layout<'b>,
    pub at: Point,
    pub style: PreviewStyle,
    /// The `:focus-visible` ring around it, with the item's corner radius.
    pub ring: Option<(Ring, f32)>,
    /// Window clip (the scroll viewport the item lives in), or none (a
    /// `position: fixed` DragOverlay).
    pub clip: Option<Rectangle>,
}

impl<'a, M> overlay::Overlay<M, iced::Theme, iced::Renderer> for Preview<'_, 'a, M> {
    fn layout(&mut self, _renderer: &iced::Renderer, _bounds: Size) -> layout::Node {
        layout::Node::new(self.layout.bounds().size()).move_to(self.at)
    }
    fn draw(
        &self,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        _layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let b = self.layout.bounds();
        let shift = self.at - b.position();
        let cursor = match cursor {
            mouse::Cursor::Available(p) => mouse::Cursor::Available(p - shift),
            c => c,
        };
        let style = renderer::Style {
            text_color: Color {
                a: style.text_color.a * self.style.opacity,
                ..style.text_color
            },
        };
        let all = Rectangle::new(Point::new(-1e6, -1e6), Size::new(2e6, 2e6));
        let clip = self.clip.unwrap_or(all);
        renderer.with_layer(clip, |r| {
            r.with_translation(shift, |r| {
                with_opacity(self.style.opacity, || {
                    if let Some(s) = &self.style.under {
                        paint(r, crate::scale::snap_rect(b), s);
                    }
                    self.content.as_widget().draw(
                        self.tree,
                        r,
                        theme,
                        &style,
                        self.layout,
                        cursor,
                        &all,
                    );
                    if let Some(s) = &self.style.over {
                        paint(r, crate::scale::snap_rect(b), s);
                    }
                    if let Some((ring, radius)) = &self.ring {
                        paint_ring(r, b, *radius, ring);
                    }
                });
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activates_past_four_px_only() {
        let mut s = Session::<u8>::default();
        s.press(
            1,
            Pointer::Mouse,
            Point::new(10.0, 10.0),
            Vector::ZERO,
            false,
        );
        assert_eq!(
            s.moved(Pointer::Mouse, Point::new(14.0, 10.0)),
            Signal::Nothing
        );
        assert!(s.dragging().is_none(), "exactly 4 px is not enough");
        // Another pointer is ignored.
        assert_eq!(
            s.moved(Pointer::Finger(3), Point::new(40.0, 10.0)),
            Signal::Nothing
        );
        assert_eq!(
            s.moved(Pointer::Mouse, Point::new(13.0, 13.0)),
            Signal::Started
        );
        assert_eq!(s.dragging().unwrap().top_left(), Point::new(13.0, 13.0));
        assert_eq!(
            s.moved(Pointer::Mouse, Point::new(13.0, 30.0)),
            Signal::Moved
        );
        assert_eq!(s.intent, [false, false, false, true]);
        assert_eq!(s.released(Pointer::Mouse), Signal::Dropped);
        assert!(s.is_idle());
    }

    #[test]
    fn a_click_is_not_a_drag_and_escape_cancels() {
        let mut s = Session::<u8>::default();
        s.press(1, Pointer::Mouse, Point::ORIGIN, Vector::ZERO, false);
        assert_eq!(s.released(Pointer::Mouse), Signal::Nothing);
        s.press(1, Pointer::Mouse, Point::ORIGIN, Vector::ZERO, false);
        s.moved(Pointer::Mouse, Point::new(0.0, 20.0));
        assert_eq!(s.cancel(), Signal::Cancelled);
        assert_eq!(s.released(Pointer::Mouse), Signal::Nothing);
        assert_eq!(s.cancel(), Signal::Nothing);
    }

    #[test]
    fn keyboard_drag_drops_with_the_keyboard_only() {
        let mut s = Session::<u8>::default();
        s.pick_up(2, Point::new(0.0, 54.0));
        assert_eq!(s.released(Pointer::Mouse), Signal::Nothing);
        assert_eq!(s.drop_keyboard(), Signal::Dropped);
        assert!(s.is_idle());
    }

    #[test]
    fn a_handle_holds_the_finger() {
        let mut s = Session::<u8>::default();
        s.press(1, Pointer::Finger(7), Point::ORIGIN, Vector::ZERO, true);
        assert_eq!(chain::touch_holder(7), Some(s.token));
        s.moved(Pointer::Finger(7), Point::new(0.0, 9.0));
        assert_eq!(s.released(Pointer::Finger(7)), Signal::Dropped);
        assert_eq!(chain::touch_holder(7), None);
    }

    #[test]
    fn motion_animates_while_sorting_and_snaps_otherwise() {
        let t0 = Instant::now();
        let mut m = Motion::<u8>::default();
        m.aim(&1, Vector::new(0.0, 54.0), true, t0);
        assert_eq!(
            m.at(&1, t0),
            Some(Vector::new(0.0, 54.0)),
            "first sight: no motion"
        );
        m.aim(&1, Vector::ZERO, true, t0);
        assert!(m.running(t0 + Duration::from_millis(100)));
        let mid = m.at(&1, t0 + Duration::from_millis(100)).unwrap().y;
        assert!(mid > 0.0 && mid < 27.0, "ease is fast early: {mid}");
        assert_eq!(m.at(&1, t0 + SORT), Some(Vector::ZERO));
        m.aim(&1, Vector::new(0.0, 108.0), false, t0 + SORT);
        assert!(!m.running(t0 + SORT));
        assert_eq!(m.at(&1, t0 + SORT), Some(Vector::new(0.0, 108.0)));
    }
}
