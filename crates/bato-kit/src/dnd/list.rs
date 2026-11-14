//! [`sortable`]: one sortable list (dnd-kit's `SortableContext` with the
//! vertical or horizontal list strategy, `closestCenter`, no `DragOverlay`).
//!
//! - **Pointer**: a press on an item (on its [`handle`](super::handle) if it
//!   has one, never on a [`no_drag`](super::no_drag) area) becomes a drag
//!   past 4 px. The item then follows the pointer in the overlay layer
//!   (clipped to its scroll viewport, as the transformed DOM node is) with
//!   the [`PreviewStyle`]; the others slide out of the way (200 ms `ease`).
//!   The release reorders at once (`on_reorder(from, to)`, React's
//!   `arrayMove`); Esc cancels and the items slide back.
//! - **Touch**: a handle holds the finger at once (`touch-action: none`); an
//!   item without one lets the page scroll take the finger past the 15 px
//!   slop, which cancels the drag (Chromium's `pointercancel`).
//! - **Keyboard** ([`List::keyboard`]): Space / Enter on a focused item picks
//!   it up, the arrows move it one slot (200 ms), Space / Enter drops, Esc
//!   cancels (dnd-kit's `KeyboardSensor` + `sortableKeyboardCoordinates`).
//! - **Auto-scroll**: near the edge of the scrollers around (innermost first).

use std::time::Instant;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::keyboard::{self, Key, key::Named};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, mouse, touch, window};

use super::engine::Motion;
use super::engine::{Phase, Pointer, Preview, PreviewStyle, Session, Signal, may_drag};
use super::sort::{Axis, Dir, closest_center, keyboard_target, strategy};
use crate::cursor::Cursor;
use crate::focus::{FocusState, Ring, paint_ring, plain};
use crate::scroller::chain;

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

pub struct List<'a, K, M> {
    axis: Axis,
    gap: f32,
    keys: Vec<K>,
    items: Vec<El<'a, M>>,
    width: Length,
    preview: PreviewStyle,
    clip: bool,
    press_cursor: Option<Cursor>,
    drag_cursor: Option<Cursor>,
    ring: Option<(Ring, f32)>,
    keyboard: bool,
    on_reorder: Option<Box<dyn Fn(usize, usize) -> M + 'a>>,
}

/// A sortable list running along `axis`; add items with [`List::item`].
pub fn sortable<'a, K, M>(axis: Axis) -> List<'a, K, M> {
    List {
        axis,
        gap: 0.0,
        keys: Vec::new(),
        items: Vec::new(),
        width: match axis {
            Axis::Vertical => Length::Fill,
            Axis::Horizontal => Length::Shrink,
        },
        preview: PreviewStyle {
            opacity: 1.0,
            ..PreviewStyle::default()
        },
        clip: true,
        press_cursor: None,
        drag_cursor: None,
        ring: None,
        keyboard: false,
        on_reorder: None,
    }
}

impl<'a, K: Clone + PartialEq + 'static, M: 'a> List<'a, K, M> {
    /// The next item, identified by `key` (React's `id`).
    pub fn item(mut self, key: K, el: impl Into<El<'a, M>>) -> Self {
        self.keys.push(key);
        self.items.push(el.into());
        self
    }
    /// Space between items (`space-y-2` = 8).
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }
    pub fn width(mut self, w: impl Into<Length>) -> Self {
        self.width = w.into();
        self
    }
    /// How the dragged item looks (`opacity-50 shadow-app-lg`).
    pub fn preview(mut self, p: PreviewStyle) -> Self {
        self.preview = p;
        self
    }
    /// Clip the dragged item to the scroll viewport it lives in (default).
    pub fn clip_preview(mut self, on: bool) -> Self {
        self.clip = on;
        self
    }
    /// The cursor while pressed (`active:cursor-grabbing`) and while
    /// dragging; `None` leaves it to the items.
    pub fn cursors(mut self, press: Option<Cursor>, drag: Option<Cursor>) -> Self {
        self.press_cursor = press;
        self.drag_cursor = drag;
        self
    }
    /// Each item is a Tab stop (`tabIndex=0`) with this `:focus-visible`
    /// ring around its box (corner `radius`).
    pub fn focusable(mut self, ring: Ring, radius: f32) -> Self {
        self.ring = Some((ring, radius));
        self
    }
    /// Keyboard sorting on focused items (needs [`List::focusable`]).
    pub fn keyboard(mut self, on: bool) -> Self {
        self.keyboard = on;
        self
    }
    /// A drop moved item `from` to index `to` (apply with
    /// [`array_move`](super::array_move)).
    pub fn on_reorder(mut self, f: impl Fn(usize, usize) -> M + 'a) -> Self {
        self.on_reorder = Some(Box::new(f));
        self
    }
}

struct State<K> {
    session: Session<K>,
    motion: Motion<K>,
    /// The keys and their slots (relative to the widget) at the last layout.
    keys: Vec<K>,
    slots: Vec<Rectangle>,
    focus: Vec<(K, FocusState)>,
    over: Option<usize>,
    /// The keyboard drag's translation of the active item.
    kb: Vector,
    /// The pointer, relative to the widget (layout coordinates).
    pointer: Point,
}

impl<K> Default for State<K> {
    fn default() -> Self {
        Self {
            session: Session::default(),
            motion: Motion::default(),
            keys: Vec::new(),
            slots: Vec::new(),
            focus: Vec::new(),
            over: None,
            kb: Vector::ZERO,
            pointer: Point::ORIGIN,
        }
    }
}

impl<K: Clone + PartialEq> State<K> {
    fn index(&self, key: &K) -> Option<usize> {
        self.keys.iter().position(|k| k == key)
    }
    /// The dragged item's index, while dragging.
    fn from(&self) -> Option<usize> {
        self.session.dragging().and_then(|g| self.index(&g.key))
    }
    fn focus_of(&mut self, key: &K) -> Option<&mut FocusState> {
        self.focus
            .iter_mut()
            .find(|(k, _)| k == key)
            .map(|(_, f)| f)
    }
    fn focused(&self) -> Option<usize> {
        self.focus
            .iter()
            .find(|(_, f)| f.is_focused())
            .and_then(|(k, _)| self.index(k))
    }
    /// The active item's collision rectangle (relative to the widget).
    fn collision(&self) -> Option<Rectangle> {
        let g = self.session.dragging()?;
        let slot = self.slots.get(self.index(&g.key)?)?;
        let at = match g.pointer {
            Pointer::Keyboard => slot.position() + self.kb,
            _ => self.pointer - g.grab,
        };
        Some(Rectangle::new(at, slot.size()))
    }
    /// Re-run collision detection; true when `over` changed.
    fn collide(&mut self) -> bool {
        let Some(c) = self.collision() else {
            return false;
        };
        let over = closest_center(c, self.slots.iter().enumerate())
            .first()
            .copied();
        let changed = over != self.over;
        self.over = over;
        changed
    }
    /// Aim every item at its slot plus its displacement.
    fn retarget(&mut self, axis: Axis, animate: bool, now: Instant) {
        let from = self.from();
        let pointer = self.session.dragging().map(|g| g.pointer);
        let follow = self.collision().map(|c| c.position());
        for (i, (k, slot)) in self.keys.iter().zip(&self.slots).enumerate() {
            let base = Vector::new(slot.x, slot.y);
            let (to, anim) = match (from, self.over) {
                (Some(f), _) if f == i => match pointer {
                    Some(Pointer::Keyboard) => (base + self.kb, animate),
                    _ => (follow.map_or(base, |p| Vector::new(p.x, p.y)), false),
                },
                (Some(f), Some(o)) => (base + strategy(&self.slots, f, o, i, axis), animate),
                _ => (base, animate),
            };
            self.motion.aim(k, to, anim, now);
        }
        let keys = &self.keys;
        self.motion.retain(|k| keys.contains(k));
    }
    /// Item `i`'s draw offset from its slot.
    fn offset(&self, i: usize, now: Instant) -> Vector {
        let (Some(k), Some(s)) = (self.keys.get(i), self.slots.get(i)) else {
            return Vector::ZERO;
        };
        self.motion
            .at(k, now)
            .map_or(Vector::ZERO, |v| v - Vector::new(s.x, s.y))
    }
}

fn shifted(cursor: mouse::Cursor, by: Vector) -> mouse::Cursor {
    match cursor {
        mouse::Cursor::Available(p) => mouse::Cursor::Available(p - by),
        mouse::Cursor::Levitating(p) => mouse::Cursor::Levitating(p - by),
        c => c,
    }
}

impl<'a, K: Clone + PartialEq + 'static, M: 'a> List<'a, K, M> {
    fn finish(&self, state: &mut State<K>, signal: Signal, shell: &mut Shell<'_, M>) {
        let now = Instant::now();
        match signal {
            Signal::Started => {
                state.over = state.from();
                state.kb = Vector::ZERO;
                state.collide();
                state.retarget(self.axis, true, now);
            }
            Signal::Moved => {
                let changed = state.collide();
                state.retarget(self.axis, changed, now);
            }
            _ => {}
        }
        shell.request_redraw();
    }

    /// End of a drag: reorder on a drop, slide back on a cancel.
    fn ended(
        &self,
        state: &mut State<K>,
        from: Option<usize>,
        signal: Signal,
        shell: &mut Shell<'_, M>,
    ) {
        let now = Instant::now();
        let over = state.over.take();
        match signal {
            Signal::Dropped => {
                if let (Some(f), Some(o), Some(cb)) = (from, over, &self.on_reorder)
                    && f != o
                {
                    shell.publish(cb(f, o));
                }
                state.retarget(self.axis, false, now);
            }
            Signal::Cancelled => {
                // dnd-kit animates back only what was displaced.
                state.retarget(self.axis, from.is_some() && from != over, now);
            }
            _ => {}
        }
        state.kb = Vector::ZERO;
        shell.request_redraw();
    }
}

impl<'a, K: Clone + PartialEq + 'static, M: 'a> Widget<M, iced::Theme, iced::Renderer>
    for List<'a, K, M>
{
    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Shrink)
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<K>>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::<K>::default())
    }
    fn children(&self) -> Vec<Tree> {
        self.items.iter().map(Tree::new).collect()
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.items);
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(self.width);
        let max = limits.max();
        let mut nodes = Vec::with_capacity(self.items.len());
        let size = match self.axis {
            Axis::Vertical => {
                let w = max.width;
                let mut y = 0.0;
                for (item, t) in self.items.iter_mut().zip(tree.children.iter_mut()) {
                    let l = layout::Limits::new(Size::new(w, 0.0), Size::new(w, f32::INFINITY));
                    let n = item.as_widget_mut().layout(t, renderer, &l);
                    let h = n.size().height;
                    nodes.push(n.move_to(Point::new(0.0, y)));
                    y += h + self.gap;
                }
                Size::new(w, (y - self.gap).max(0.0))
            }
            Axis::Horizontal => {
                // `items-stretch`: measure, then lay out at the tallest.
                let loose = layout::Limits::new(Size::ZERO, Size::new(f32::INFINITY, max.height));
                let cross = self
                    .items
                    .iter_mut()
                    .zip(tree.children.iter_mut())
                    .map(|(i, t)| i.as_widget_mut().layout(t, renderer, &loose).size().height)
                    .fold(0.0, f32::max);
                let mut x = 0.0;
                for (item, t) in self.items.iter_mut().zip(tree.children.iter_mut()) {
                    let l =
                        layout::Limits::new(Size::new(0.0, cross), Size::new(f32::INFINITY, cross));
                    let n = item.as_widget_mut().layout(t, renderer, &l);
                    let w = n.size().width;
                    nodes.push(n.move_to(Point::new(x, 0.0)));
                    x += w + self.gap;
                }
                Size::new((x - self.gap).max(0.0), cross)
            }
        };
        let state = tree.state.downcast_mut::<State<K>>();
        state.keys = self.keys.clone();
        state.slots = nodes.iter().map(|n| n.bounds()).collect();
        for k in &self.keys {
            if state.focus_of(k).is_none() {
                state.focus.push((k.clone(), FocusState::default()));
            }
        }
        let keys = &self.keys;
        state.focus.retain(|(k, _)| keys.contains(k));
        let sorting = state.session.dragging().is_some();
        if state
            .session
            .pressed()
            .is_some_and(|g| !keys.contains(&g.key))
        {
            state.session.cancel();
        }
        state.collide();
        state.retarget(self.axis, sorting, Instant::now());
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
        let now = Instant::now();
        let bounds = layout.bounds();
        let origin = Vector::new(bounds.x, bounds.y);
        let shift = {
            let (x, y) = chain::content_shift();
            Vector::new(x, y)
        };
        let captured = shell.is_event_captured();
        let Tree {
            state, children, ..
        } = tree;
        let state = state.downcast_mut::<State<K>>();
        let focusable = self.ring.is_some();

        // ── Focus (each item is a Tab stop) ─────────────────────────────
        if focusable {
            let mut changed = false;
            for (i, k) in self.keys.iter().enumerate() {
                let b = state.slots.get(i).copied().unwrap_or_default() + origin;
                let inside = match event {
                    Event::Touch(touch::Event::FingerPressed { position, .. }) => {
                        b.contains(*position) && viewport.contains(*position)
                    }
                    _ => cursor.is_over(b) && cursor.is_over(*viewport),
                };
                if let Some(f) = state.focus_of(k) {
                    changed |= f.pointer(event, inside, true);
                    f.reveal(b, shell);
                }
            }
            if changed {
                shell.request_redraw();
            }
        }

        // ── Pointer sensor ──────────────────────────────────────────────
        let press = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                cursor.position().map(|p| (p, Pointer::Mouse))
            }
            Event::Touch(touch::Event::FingerPressed { id, position }) => {
                Some((*position, Pointer::Finger(id.0)))
            }
            _ => None,
        };
        if let Some((p, pointer)) = press
            && !captured
            && state.session.is_idle()
            && bounds.contains(p)
            && viewport.contains(p)
        {
            // Topmost (last drawn) first.
            let hit = (0..self.items.len()).rev().find(|&i| {
                let b = state.slots.get(i).copied().unwrap_or_default() + origin;
                (b + state.offset(i, now)).contains(p)
            });
            if let Some(i) = hit {
                let off = state.offset(i, now);
                let item_layout = layout.children().nth(i).unwrap();
                let (ok, handle) = may_drag(
                    &mut self.items[i],
                    &mut children[i],
                    item_layout,
                    renderer,
                    p - off,
                );
                if ok {
                    let top_left = item_layout.bounds().position() + off;
                    state.session.press(
                        self.keys[i].clone(),
                        pointer,
                        p - shift,
                        p - top_left,
                        handle,
                    );
                    state.pointer = p - origin;
                    shell.request_redraw();
                }
            }
        }

        let ending_from = state.from();
        let mut signal = Signal::Nothing;
        match event {
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                signal = state.session.moved(Pointer::Mouse, *position);
            }
            Event::Touch(touch::Event::FingerMoved { id, position }) => {
                signal = state
                    .session
                    .moved(Pointer::Finger(id.0), *position - shift);
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                signal = state.session.released(Pointer::Mouse);
            }
            Event::Touch(touch::Event::FingerLifted { id, .. }) => {
                signal = state.session.released(Pointer::Finger(id.0));
            }
            Event::Touch(touch::Event::FingerLost { id, .. }) => {
                let mine = state
                    .session
                    .pressed()
                    .filter(|g| g.pointer == Pointer::Finger(id.0))
                    .map(|g| g.claimed);
                // A scroller stepping aside for us only cancels presses.
                if let Some(claimed) = mine
                    && !(claimed && chain::is_synthetic_lost())
                {
                    signal = state.session.cancel();
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Named(Named::Escape),
                ..
            }) if !state.session.is_idle() => {
                signal = state.session.cancel();
                shell.capture_event();
            }
            Event::Window(window::Event::Resized(_)) => {
                signal = state.session.cancel();
            }
            _ => {}
        }
        if let Some(g) = state.session.pressed()
            && g.pointer != Pointer::Keyboard
        {
            state.pointer = g.at + shift - origin;
        }
        match signal {
            Signal::Started | Signal::Moved => {
                self.finish(state, signal, shell);
                shell.capture_event();
            }
            Signal::Dropped | Signal::Cancelled => {
                self.ended(state, ending_from, signal, shell);
                shell.capture_event();
            }
            Signal::Nothing => {}
        }

        // ── Keyboard sensor ─────────────────────────────────────────────
        if self.keyboard
            && let Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Named(named),
                modifiers,
                repeat,
                ..
            }) = event
            && plain(*modifiers)
            && !shell.is_event_captured()
        {
            let kbd = state
                .session
                .dragging()
                .is_some_and(|g| g.pointer == Pointer::Keyboard);
            match named {
                Named::Space | Named::Enter if !*repeat => {
                    if kbd {
                        let from = state.from();
                        let s = state.session.drop_keyboard();
                        self.ended(state, from, s, shell);
                        shell.capture_event();
                    } else if state.session.is_idle()
                        && let Some(i) = state.focused()
                    {
                        let at = state.slots[i].position() + origin - shift;
                        state
                            .session
                            .pick_up(self.keys[i].clone(), Point::new(at.x, at.y));
                        self.finish(state, Signal::Started, shell);
                        shell.capture_event();
                    }
                }
                Named::ArrowUp | Named::ArrowDown | Named::ArrowLeft | Named::ArrowRight if kbd => {
                    let dir = match named {
                        Named::ArrowUp => Dir::Up,
                        Named::ArrowDown => Dir::Down,
                        Named::ArrowLeft => Dir::Left,
                        _ => Dir::Right,
                    };
                    if let (Some(f), Some(c)) = (state.from(), state.collision())
                        && let Some((_, at)) = keyboard_target(&state.slots, f, c, state.over, dir)
                    {
                        let slot = state.slots[f].position();
                        state.kb = at - slot;
                        state.collide();
                        state.retarget(self.axis, true, now);
                        let rect = Rectangle::new(at, state.slots[f].size()) + origin;
                        crate::scroller::scroll_into_view(rect, crate::scroller::Align::Nearest);
                        shell.request_redraw();
                    }
                    shell.capture_event();
                }
                _ => {}
            }
        }
        // Focus leaving the item being sorted by keyboard cancels.
        if let Some(g) = state.session.dragging()
            && g.pointer == Pointer::Keyboard
            && state.focused() != state.from()
        {
            let from = state.from();
            let s = state.session.cancel();
            self.ended(state, from, s, shell);
        }

        // ── Frames: auto-scroll, collisions under a scrolling page, motion
        if let Event::Window(window::Event::RedrawRequested(at)) = event {
            if let Some(g) = state.session.dragging()
                && g.pointer != Pointer::Keyboard
            {
                let content = g.at + shift;
                state.pointer = content - origin;
                if state.collide() {
                    state.retarget(self.axis, true, *at);
                } else {
                    state.retarget(self.axis, false, *at);
                }
                if state.session.auto_scroll(content, *at) {
                    shell.request_redraw();
                }
            }
            if state.motion.running(*at) {
                shell.request_redraw();
            }
        }

        // ── The items ───────────────────────────────────────────────────
        let dragging = state.session.dragging().map(|g| (g.pointer, g.key.clone()));
        let ended = matches!(signal, Signal::Dropped | Signal::Cancelled);
        for (i, ((item, t), l)) in self
            .items
            .iter_mut()
            .zip(children.iter_mut())
            .zip(layout.children())
            .enumerate()
        {
            let off = state.offset(i, now);
            let active = dragging.as_ref().is_some_and(|(_, k)| *k == self.keys[i]);
            // While a pointer drags, the items see no pointer but the dragged
            // one under it; the release that ends a drag clicks nothing.
            let pointer_event = matches!(event, Event::Mouse(_) | Event::Touch(_));
            let c = if (ended || dragging.is_some()) && pointer_event && !active {
                mouse::Cursor::Unavailable
            } else if active && ended {
                mouse::Cursor::Unavailable
            } else {
                shifted(cursor, off)
            };
            let ev;
            let event = match event {
                Event::Touch(touch::Event::FingerLifted { id, position }) if ended => {
                    ev = Event::Touch(touch::Event::FingerLost {
                        id: *id,
                        position: *position,
                    });
                    &ev
                }
                _ => event,
            };
            let vp = *viewport - off;
            item.as_widget_mut()
                .update(t, event, l, c, renderer, clipboard, shell, &vp);
        }
        // A drag starting cancels the item's press (no click on release).
        if signal == Signal::Started
            && let Some(g) = state.session.dragging()
            && let Pointer::Finger(f) = g.pointer
            && let Some(i) = state.from()
        {
            let lost = Event::Touch(touch::Event::FingerLost {
                id: touch::Finger(f),
                position: g.at + shift,
            });
            let l = layout.children().nth(i).unwrap();
            self.items[i].as_widget_mut().update(
                &mut children[i],
                &lost,
                l,
                mouse::Cursor::Unavailable,
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
        let state = tree.state.downcast_ref::<State<K>>();
        match &state.session.phase {
            Phase::Pending(g) if g.pointer == Pointer::Mouse => {
                if let Some(c) = self.press_cursor {
                    return c.into();
                }
            }
            Phase::Dragging(g) if g.pointer == Pointer::Mouse => {
                if let Some(c) = self.drag_cursor.or(self.press_cursor) {
                    return c.into();
                }
            }
            _ => {}
        }
        let now = Instant::now();
        self.items
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .enumerate()
            .map(|(i, ((item, t), l))| {
                let off = state.offset(i, now);
                item.as_widget().mouse_interaction(
                    t,
                    l,
                    shifted(cursor, off),
                    &(*viewport - off),
                    renderer,
                )
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
        let focusable = self.ring.is_some();
        let Tree {
            state, children, ..
        } = tree;
        let state = state.downcast_mut::<State<K>>();
        let keys = &self.keys;
        let items = &mut self.items;
        operation.traverse(&mut |op| {
            for (i, ((item, t), l)) in items
                .iter_mut()
                .zip(children.iter_mut())
                .zip(layout.children())
                .enumerate()
            {
                if focusable && let Some(f) = state.focus_of(&keys[i]) {
                    f.report(op, None, l.bounds(), true, true);
                }
                item.as_widget_mut().operate(t, l, renderer, op);
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
        let state = tree.state.downcast_ref::<State<K>>();
        let now = Instant::now();
        let from = state.from();
        for (i, ((item, t), l)) in self
            .items
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .enumerate()
        {
            if Some(i) == from {
                continue; // drawn in the overlay
            }
            let off = state.offset(i, now);
            renderer.with_translation(off, |r| {
                item.as_widget().draw(
                    t,
                    r,
                    theme,
                    style,
                    l,
                    shifted(cursor, off),
                    &(*viewport - off),
                );
                if let Some((ring, radius)) = &self.ring
                    && state
                        .focus
                        .iter()
                        .any(|(k, f)| *k == self.keys[i] && f.ring_visible())
                {
                    paint_ring(r, l.bounds(), *radius, ring);
                }
            });
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        let preview = {
            let state = tree.state.downcast_ref::<State<K>>();
            state.from().and_then(|i| {
                let g = state.session.dragging()?;
                let l = layout.children().nth(i)?;
                let at = match g.pointer {
                    Pointer::Keyboard => {
                        l.bounds().position() + state.offset(i, Instant::now()) + translation
                    }
                    _ => g.top_left(),
                };
                let ring = self.ring.filter(|_| {
                    state
                        .focus
                        .iter()
                        .any(|(k, f)| *k == self.keys[i] && f.ring_visible())
                });
                Some((i, at, ring))
            })
        };
        let Some((i, at, ring)) = preview else {
            return overlay::from_children(
                &mut self.items,
                tree,
                layout,
                renderer,
                viewport,
                translation,
            );
        };
        Some(overlay::Element::new(Box::new(Preview {
            content: &self.items[i],
            tree: &tree.children[i],
            layout: layout.children().nth(i)?,
            at,
            style: self.preview.clone(),
            ring,
            clip: self.clip.then_some(*viewport + translation),
        })))
    }
}

impl<'a, K: Clone + PartialEq + 'static, M: 'a> From<List<'a, K, M>> for El<'a, M> {
    fn from(l: List<'a, K, M>) -> Self {
        Element::new(l)
    }
}
