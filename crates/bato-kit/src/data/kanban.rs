//! `KanbanBoard`: columns of cards, dragged within and across columns
//! (@dnd-kit with `closestCorners` and a `DragOverlay`).
//!
//! Look: `flex gap-4 items-start overflow-x-auto pb-2` of `w-64` columns
//! (`bg-app-bg border border-app-border rounded-xl`), each a header
//! (`px-3 py-2.5 border-b`: the title `text-sm font-medium`, the count
//! `text-xs text-app-muted bg-app-card rounded-full px-2 py-0.5`) over a
//! card list (`p-2 space-y-2 min-h-[4rem]`, tinted `bg-app-accent/5` with
//! `transition-colors` while a card is over its empty part). A card is
//! `bg-app-card border border-app-border rounded-lg px-3 py-2.5` with its
//! title (`text-sm font-medium`) and optional description (`text-xs
//! text-app-muted mt-1`), `cursor-grab`.
//!
//! Drag (as React's handlers): past 4 px the card's slot shows a ghost at
//! `opacity-40` and a copy with `shadow-app-lg` follows the pointer above
//! everything (React also tilts it `rotate-2`; iced has no rotation, so it
//! stays straight). Over another column the card moves there at once
//! (`on_change`, React's `onDragOver`): before the card it is over, or at
//! the end. The cards of the column it is in slide out of the way (200 ms
//! `ease`); dropping on a card of that column reorders (`onDragEnd`). The
//! copy then flies to the card's slot (250 ms `ease`) while the slot is
//! hidden. Esc cancels (a move to another column stays, as in React).
//! Edge auto-scroll drives the board's own scroller and the page.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::border::Radius;
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{Space, column, container, row};
use iced::{
    Alignment, Border, Color, Element, Event, Length, Padding, Point, Rectangle, Size, Vector,
    mouse, touch, window,
};

use super::sortable::lifted;
use crate::anim::{ease_standard, with_opacity};
use crate::ctx::Ctx;
use crate::cursor::Cursor;
use crate::date::ua_ring;
use crate::dnd::{
    self, Axis, DROP, Motion, Phase, Pointer, Preview, PreviewStyle, Session, Signal, ease,
};
use crate::label::label;
use crate::scale::snap_rect;
use crate::scroller::{chain, scroll_area};
use crate::style::c;
use crate::surface::{BoxStyle, paint, surface};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KanbanCard {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
}

impl KanbanCard {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: None,
        }
    }
    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KanbanColumn {
    pub id: String,
    pub title: String,
    pub cards: Vec<KanbanCard>,
}

impl KanbanColumn {
    pub fn new(id: impl Into<String>, title: impl Into<String>, cards: Vec<KanbanCard>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            cards,
        }
    }
}

/// `w-64`.
pub const COLUMN: f32 = 256.0;
/// `gap-4`.
pub const COLUMN_GAP: f32 = 16.0;
/// `p-2`, `space-y-2`.
const PAD: f32 = 8.0;
/// `min-h-[4rem]`.
const MIN_BODY: f32 = 64.0;
/// `opacity-40` on the dragged card's slot.
const GHOST: f32 = 0.4;
/// `transition-colors`.
const TINT: Duration = Duration::from_millis(150);

/// What the dragged card is over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Over {
    Column(usize),
    Card(String),
}

/// The column holding `card`.
fn column_of(cols: &[KanbanColumn], card: &str) -> Option<usize> {
    cols.iter()
        .position(|c| c.cards.iter().any(|k| k.id == card))
}

/// React's `onDragOver`: over a card or column of another column, the
/// active card moves there (before the card, or at the end).
pub fn drag_over(cols: &[KanbanColumn], active: &str, over: &Over) -> Option<Vec<KanbanColumn>> {
    let from = column_of(cols, active)?;
    let to = match over {
        Over::Column(i) => *i,
        Over::Card(id) => column_of(cols, id)?,
    };
    if from == to || to >= cols.len() {
        return None;
    }
    let card = cols[from].cards.iter().find(|c| c.id == active)?.clone();
    let at = match over {
        Over::Card(id) => cols[to].cards.iter().position(|c| c.id == *id),
        Over::Column(_) => None,
    }
    .unwrap_or(cols[to].cards.len());
    let mut next = cols.to_vec();
    next[from].cards.retain(|c| c.id != active);
    next[to].cards.insert(at, card);
    Some(next)
}

/// React's `onDragEnd`: dropped on a card of its own column, reorder.
pub fn drag_end(cols: &[KanbanColumn], active: &str, over: &Over) -> Option<Vec<KanbanColumn>> {
    let Over::Card(over) = over else {
        return None;
    };
    let col = column_of(cols, active)?;
    let cards = &cols[col].cards;
    let new = cards.iter().position(|c| c.id == *over)?;
    let old = cards.iter().position(|c| c.id == active)?;
    if old == new {
        return None;
    }
    let mut next = cols.to_vec();
    dnd::array_move(&mut next[col].cards, old, new);
    Some(next)
}

pub struct KanbanBoard<'a, M> {
    cx: Ctx,
    columns: Vec<KanbanColumn>,
    render: Option<Box<dyn Fn(&KanbanCard) -> El<'a, M> + 'a>>,
    on_change: Option<Box<dyn Fn(Vec<KanbanColumn>) -> M + 'a>>,
}

/// A KanbanBoard over `columns` (controlled: apply `on_change`).
pub fn kanban_board<'a, M>(cx: &Ctx, columns: &[KanbanColumn]) -> KanbanBoard<'a, M> {
    KanbanBoard {
        cx: cx.clone(),
        columns: columns.to_vec(),
        render: None,
        on_change: None,
    }
}

impl<'a, M: Clone + 'a> KanbanBoard<'a, M> {
    /// The new columns after a move (React's `onChange`).
    pub fn on_change(mut self, f: impl Fn(Vec<KanbanColumn>) -> M + 'a) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }
    /// Draw cards yourself (React's `renderCard`).
    pub fn render_card(mut self, f: impl Fn(&KanbanCard) -> El<'a, M> + 'a) -> Self {
        self.render = Some(Box::new(f));
        self
    }
}

/// The default card (`CardBody`).
pub fn card_body<'a, M: Clone + 'a>(cx: &Ctx, card: &KanbanCard) -> El<'a, M> {
    let p = cx.p();
    let mut col = column![
        label(card.title.clone(), 14.0, 20.0)
            .medium()
            .color(c(p.text))
    ];
    if let Some(d) = &card.description {
        col = col
            .push(Space::new().height(4))
            .push(label(d.clone(), 12.0, 16.0).color(c(p.muted)));
    }
    surface(col)
        .padding([11, 13])
        .width(Length::Fill)
        .fixed(super::sortable::row_style(cx))
        .cursor(Cursor::Grab)
        .focusable(true)
        .focus_ring(ua_ring())
        .into()
}

fn header<'a, M: Clone + 'a>(cx: &Ctx, col: &KanbanColumn) -> El<'a, M> {
    let p = cx.p();
    let count = surface(label(col.cards.len().to_string(), 12.0, 16.0).color(c(p.muted)))
        .padding([2, 8])
        .fixed(BoxStyle::default().bg(c(p.card)).radius(999.0));
    surface(
        row![
            label(col.title.clone(), 14.0, 20.0)
                .medium()
                .color(c(p.text)),
            Space::new().width(Length::Fill),
            count
        ]
        .align_y(Alignment::Center),
    )
    .padding([10, 12])
    .width(Length::Fill)
    .into()
}

impl<'a, M: Clone + 'a> From<KanbanBoard<'a, M>> for El<'a, M> {
    fn from(b: KanbanBoard<'a, M>) -> Self {
        let on = b.cx.on_bg();
        let headers = b.columns.iter().map(|col| header(&on, col)).collect();
        let cards = b
            .columns
            .iter()
            .flat_map(|col| col.cards.iter())
            .map(|card| match &b.render {
                Some(f) => f(card),
                None => card_body(&b.cx.on_card(), card),
            })
            .collect();
        let board = Board {
            cx: b.cx.clone(),
            columns: b.columns,
            headers,
            cards,
            on_change: b.on_change,
        };
        scroll_area(
            &b.cx,
            container(El::new(board)).padding(Padding::ZERO.bottom(PAD)),
        )
        .horizontal()
        .width(Length::Fill)
        .into()
    }
}

struct Board<'a, M> {
    cx: Ctx,
    columns: Vec<KanbanColumn>,
    headers: Vec<El<'a, M>>,
    /// Every card, column by column.
    cards: Vec<El<'a, M>>,
    on_change: Option<Box<dyn Fn(Vec<KanbanColumn>) -> M + 'a>>,
}

/// One column's geometry, relative to the board.
#[derive(Debug, Clone, Default)]
struct Col {
    rect: Rectangle,
    /// The header's bottom border.
    rule: Rectangle,
    /// The card list (the droppable).
    body: Rectangle,
    /// `(card id, slot)`.
    cards: Vec<(String, Rectangle)>,
}

#[derive(Debug, Clone, Copy)]
struct Tint {
    from: f32,
    to: f32,
    start: Instant,
}

impl Tint {
    fn value(&self, now: Instant) -> f32 {
        let t =
            (now.saturating_duration_since(self.start).as_secs_f32() / TINT.as_secs_f32()).min(1.0);
        self.from + (self.to - self.from) * ease_standard(t)
    }
    fn running(&self, now: Instant) -> bool {
        now < self.start + TINT
    }
}

/// The copy flying back to its slot after a drop (window top-left).
#[derive(Debug, Clone)]
struct Landing {
    key: String,
    from: Point,
    start: Instant,
}

#[derive(Default)]
struct State {
    session: Session<String>,
    motion: Motion<String>,
    cols: Vec<Col>,
    over: Option<Over>,
    tints: Vec<Tint>,
    landing: Option<Landing>,
    /// The pointer relative to the board.
    pointer: Point,
}

impl State {
    fn slot(&self, key: &str) -> Option<(usize, usize, Rectangle)> {
        self.cols.iter().enumerate().find_map(|(ci, c)| {
            c.cards
                .iter()
                .position(|(k, _)| k == key)
                .map(|ri| (ci, ri, c.cards[ri].1))
        })
    }
    fn collision(&self) -> Option<Rectangle> {
        let g = self.session.dragging()?;
        let (_, _, slot) = self.slot(&g.key)?;
        Some(Rectangle::new(self.pointer - g.grab, slot.size()))
    }
    /// `closestCorners` over the column bodies and every card.
    fn collide(&mut self) -> bool {
        let Some(c) = self.collision() else {
            return false;
        };
        let mut all: Vec<(Over, Rectangle)> = Vec::new();
        for (i, col) in self.cols.iter().enumerate() {
            all.push((Over::Column(i), col.body));
            for (k, r) in &col.cards {
                all.push((Over::Card(k.clone()), *r));
            }
        }
        let best = dnd::closest_corners(c, all.iter().map(|(_, r)| r).enumerate())
            .first()
            .map(|i| all[*i].0.clone());
        let changed = best != self.over;
        self.over = best;
        changed
    }
    fn retarget(&mut self, animate: bool, now: Instant) {
        let active = self.session.dragging().map(|g| g.key.clone());
        let over_card = match &self.over {
            Some(Over::Card(k)) => Some(k.clone()),
            _ => None,
        };
        for col in &self.cols {
            let rects: Vec<Rectangle> = col.cards.iter().map(|(_, r)| *r).collect();
            let ai = active
                .as_ref()
                .and_then(|a| col.cards.iter().position(|(k, _)| k == a));
            let oi = over_card
                .as_ref()
                .and_then(|o| col.cards.iter().position(|(k, _)| k == o));
            for (i, (k, r)) in col.cards.iter().enumerate() {
                let mut to = Vector::new(r.x, r.y);
                if let (Some(a), Some(o)) = (ai, oi) {
                    to = to + dnd::strategy(&rects, a, o, i, Axis::Vertical);
                }
                self.motion.aim(k, to, animate, now);
            }
        }
        let cols = &self.cols;
        self.motion
            .retain(|k| cols.iter().any(|c| c.cards.iter().any(|(x, _)| x == k)));
    }
    fn offset(&self, key: &str, slot: Rectangle, now: Instant) -> Vector {
        self.motion
            .at(&key.to_string(), now)
            .map_or(Vector::ZERO, |v| v - Vector::new(slot.x, slot.y))
    }
    /// Aim each column's tint at whether the card is over its empty part.
    fn tint(&mut self, now: Instant) -> bool {
        let n = self.cols.len();
        self.tints.resize(
            n,
            Tint {
                from: 0.0,
                to: 0.0,
                start: now,
            },
        );
        let dragging = self.session.dragging().is_some();
        for (i, t) in self.tints.iter_mut().enumerate() {
            let want = if dragging && self.over == Some(Over::Column(i)) {
                1.0
            } else {
                0.0
            };
            if t.to != want {
                *t = Tint {
                    from: t.value(now),
                    to: want,
                    start: now,
                };
            }
        }
        self.tints.iter().any(|t| t.running(now))
    }
}

impl<'a, M: Clone + 'a> Board<'a, M> {
    fn children(&self) -> impl Iterator<Item = &El<'a, M>> {
        self.headers.iter().chain(&self.cards)
    }
    /// The index of a card's element among the children.
    fn child_of(&self, key: &str) -> Option<usize> {
        self.columns
            .iter()
            .flat_map(|c| c.cards.iter())
            .position(|c| c.id == key)
            .map(|i| self.headers.len() + i)
    }
    fn publish(&self, cols: Option<Vec<KanbanColumn>>, shell: &mut Shell<'_, M>) {
        if let (Some(cols), Some(f)) = (cols, &self.on_change) {
            shell.publish(f(cols));
        }
    }
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for Board<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }
    fn children(&self) -> Vec<Tree> {
        self.children().map(Tree::new).collect()
    }
    fn diff(&self, tree: &mut Tree) {
        let all: Vec<&El<'a, M>> = self.children().collect();
        tree.diff_children(&all);
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        let inner = COLUMN - 2.0;
        let card_w = inner - 2.0 * PAD;
        let mut nodes = Vec::new();
        let mut cols = Vec::new();
        let (htrees, ctrees) = tree.children.split_at_mut(self.headers.len());
        let mut ctrees = ctrees.iter_mut();
        let mut cards = self.cards.iter_mut();
        let mut x = 0.0;
        let mut height: f32 = 0.0;
        let mut card_nodes = Vec::new();
        for ((col, h), ht) in self.columns.iter().zip(self.headers.iter_mut()).zip(htrees) {
            let fixed = layout::Limits::new(Size::new(inner, 0.0), Size::new(inner, f32::INFINITY));
            let hn = h.as_widget_mut().layout(ht, renderer, &fixed);
            let hh = hn.size().height;
            nodes.push(hn.move_to(Point::new(x + 1.0, 1.0)));
            let top = 1.0 + hh + 1.0;
            let mut y = top + PAD;
            let mut slots = Vec::new();
            for card in &col.cards {
                let (Some(el), Some(t)) = (cards.next(), ctrees.next()) else {
                    break;
                };
                let l =
                    layout::Limits::new(Size::new(card_w, 0.0), Size::new(card_w, f32::INFINITY));
                let n = el.as_widget_mut().layout(t, renderer, &l);
                let at = Point::new(x + 1.0 + PAD, y);
                slots.push((card.id.clone(), Rectangle::new(at, n.size())));
                y += n.size().height + PAD;
                card_nodes.push(n.move_to(at));
            }
            let body_h = (y - top).max(MIN_BODY);
            let rect = Rectangle::new(Point::new(x, 0.0), Size::new(COLUMN, top + body_h + 1.0));
            height = height.max(rect.height);
            cols.push(Col {
                rect,
                rule: Rectangle::new(Point::new(x + 1.0, 1.0 + hh), Size::new(inner, 1.0)),
                body: Rectangle::new(Point::new(x + 1.0, top), Size::new(inner, body_h)),
                cards: slots,
            });
            x += COLUMN + COLUMN_GAP;
        }
        nodes.extend(card_nodes);
        let width = (x - COLUMN_GAP).max(0.0);
        let state = tree.state.downcast_mut::<State>();
        state.cols = cols;
        let now = Instant::now();
        let sorting = state.session.dragging().is_some();
        if let Some(g) = state.session.pressed()
            && state.slot(&g.key).is_none()
        {
            state.session.cancel();
        }
        state.collide();
        state.retarget(sorting, now);
        state.tint(now);
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
        let now = Instant::now();
        let bounds = layout.bounds();
        let origin = Vector::new(bounds.x, bounds.y);
        let shift = {
            let (x, y) = chain::content_shift();
            Vector::new(x, y)
        };
        let captured = shell.is_event_captured();
        let nh = self.headers.len();

        // ── Press ───────────────────────────────────────────────────────
        let press = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                cursor.position().map(|p| (p, Pointer::Mouse))
            }
            Event::Touch(touch::Event::FingerPressed { id, position }) => {
                Some((*position, Pointer::Finger(id.0)))
            }
            _ => None,
        };
        {
            let state = tree.state.downcast_mut::<State>();
            if let Some((p, pointer)) = press
                && !captured
                && state.landing.is_none()
                && state.session.is_idle()
                && viewport.contains(p)
            {
                let hit = state
                    .cols
                    .iter()
                    .flat_map(|c| c.cards.iter())
                    .find(|(k, r)| (*r + origin + state.offset(k, *r, now)).contains(p));
                if let Some((k, r)) = hit.cloned() {
                    let top_left = r.position() + origin + state.offset(&k, r, now);
                    state
                        .session
                        .press(k, pointer, p - shift, p - top_left, false);
                    state.pointer = p - origin;
                }
            }
        }

        // ── Sensor ──────────────────────────────────────────────────────
        let state = tree.state.downcast_mut::<State>();
        let active = state.session.pressed().map(|g| g.key.clone());
        let before = state.session.dragging().cloned();
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
            Event::Touch(touch::Event::FingerLost { id, .. })
                if state
                    .session
                    .pressed()
                    .is_some_and(|g| g.pointer == Pointer::Finger(id.0)) =>
            {
                signal = state.session.cancel();
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
        if let Some(g) = state.session.dragging() {
            state.pointer = g.at + shift - origin;
        }
        match signal {
            Signal::Started => {
                state.over = None;
                state.collide();
                state.retarget(true, now);
                state.tint(now);
                shell.capture_event();
                shell.request_redraw();
            }
            Signal::Moved => {
                if state.collide() {
                    state.retarget(true, now);
                    state.tint(now);
                    if let (Some(k), Some(o)) = (&active, &state.over) {
                        self.publish(drag_over(&self.columns, k, o), shell);
                    }
                }
                shell.capture_event();
                shell.request_redraw();
            }
            Signal::Dropped | Signal::Cancelled => {
                let over = state.over.take();
                if let (Some(k), Some(o), Signal::Dropped) = (&active, &over, signal) {
                    self.publish(drag_end(&self.columns, k, o), shell);
                }
                // The copy flies from where it was let go to the slot.
                state.landing = before.map(|g| Landing {
                    from: g.top_left(),
                    key: g.key,
                    start: now,
                });
                state.retarget(signal == Signal::Cancelled, now);
                state.tint(now);
                shell.capture_event();
                shell.request_redraw();
            }
            Signal::Nothing => {}
        }

        // ── Frames ──────────────────────────────────────────────────────
        if let Event::Window(window::Event::RedrawRequested(at)) = event {
            if let Some(g) = state.session.dragging() {
                let content = g.at + shift;
                state.pointer = content - origin;
                if state.collide() {
                    state.retarget(true, *at);
                    if let (Some(k), Some(o)) = (&active, &state.over) {
                        self.publish(drag_over(&self.columns, k, o), shell);
                    }
                }
                if state.session.auto_scroll(content, *at) {
                    shell.request_redraw();
                }
            }
            if let Some(l) = &state.landing
                && now >= l.start + DROP
            {
                state.landing = None;
            }
            if state.motion.running(*at) || state.tint(*at) || state.landing.is_some() {
                shell.request_redraw();
            }
        }

        // ── Children ────────────────────────────────────────────────────
        let dragging = state.session.dragging().map(|g| g.key.clone());
        let ended = matches!(signal, Signal::Dropped | Signal::Cancelled);
        let offsets: Vec<Vector> = state
            .cols
            .iter()
            .flat_map(|c| c.cards.iter())
            .map(|(k, r)| state.offset(k, *r, now))
            .collect();
        let pointer_event = matches!(event, Event::Mouse(_) | Event::Touch(_));
        let mut children: Vec<&mut El<'a, M>> = self
            .headers
            .iter_mut()
            .chain(self.cards.iter_mut())
            .collect();
        for (i, ((el, t), l)) in children
            .iter_mut()
            .zip(tree.children.iter_mut())
            .zip(layout.children())
            .enumerate()
        {
            let off = if i >= nh {
                offsets.get(i - nh).copied().unwrap_or(Vector::ZERO)
            } else {
                Vector::ZERO
            };
            let c = if (ended || dragging.is_some()) && pointer_event {
                mouse::Cursor::Unavailable
            } else {
                match cursor {
                    mouse::Cursor::Available(p) => mouse::Cursor::Available(p - off),
                    c => c,
                }
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
            el.as_widget_mut().update(
                t,
                event,
                l,
                c,
                renderer,
                clipboard,
                shell,
                &(*viewport - off),
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
        match &state.session.phase {
            // `active:cursor-grabbing` while pressed; over the DragOverlay
            // copy (`cursor-grab`) while dragging.
            Phase::Pending(g) if g.pointer == Pointer::Mouse => return Cursor::Grabbing.into(),
            Phase::Dragging(g) if g.pointer == Pointer::Mouse => return Cursor::Grab.into(),
            _ => {}
        }
        self.children()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((el, t), l)| {
                el.as_widget()
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
        // DOM order: each column's header, then its cards.
        let nh = self.headers.len();
        let counts: Vec<usize> = self.columns.iter().map(|c| c.cards.len()).collect();
        let layouts: Vec<Layout<'_>> = layout.children().collect();
        let mut trees: Vec<&mut Tree> = tree.children.iter_mut().collect();
        let mut els: Vec<&mut El<'a, M>> = self
            .headers
            .iter_mut()
            .chain(self.cards.iter_mut())
            .collect();
        let mut order = Vec::new();
        let mut next = nh;
        for (ci, n) in counts.iter().enumerate() {
            order.push(ci);
            order.extend(next..next + n);
            next += n;
        }
        operation.traverse(&mut |op| {
            for &i in &order {
                if let (Some(el), Some(t), Some(l)) =
                    (els.get_mut(i), trees.get_mut(i), layouts.get(i))
                {
                    el.as_widget_mut().operate(t, *l, renderer, op);
                }
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
        let now = Instant::now();
        let b = layout.bounds();
        let origin = Vector::new(b.x, b.y);
        let p = self.cx.p();
        let on = self.cx.on_bg();
        let tint = c(on.tint(0.05));
        for (i, col) in state.cols.iter().enumerate() {
            paint(
                renderer,
                snap_rect(col.rect + origin),
                &BoxStyle::default()
                    .bg(c(p.bg))
                    .border(c(p.border))
                    .radius(12.0),
            );
            let t = state.tints.get(i).map_or(0.0, |t| t.value(now));
            if t > 0.0 {
                renderer.fill_quad(
                    Quad {
                        bounds: snap_rect(col.body + origin),
                        border: Border {
                            radius: Radius {
                                top_left: 0.0,
                                top_right: 0.0,
                                bottom_right: 11.0,
                                bottom_left: 11.0,
                            },
                            ..Border::default()
                        },
                        ..Quad::default()
                    },
                    Color { a: t, ..tint },
                );
            }
            renderer.fill_quad(
                Quad {
                    bounds: snap_rect(col.rule + origin),
                    ..Quad::default()
                },
                c(p.border),
            );
        }
        let nh = self.headers.len();
        let dragging = state.session.dragging().map(|g| g.key.as_str());
        let landing = state.landing.as_ref().map(|l| l.key.as_str());
        let keys: Vec<(&String, Rectangle)> = state
            .cols
            .iter()
            .flat_map(|c| c.cards.iter().map(|(k, r)| (k, *r)))
            .collect();
        for (i, ((el, t), l)) in self
            .children()
            .zip(&tree.children)
            .zip(layout.children())
            .enumerate()
        {
            if i < nh {
                el.as_widget()
                    .draw(t, renderer, theme, style, l, cursor, viewport);
                continue;
            }
            let Some((k, r)) = keys.get(i - nh) else {
                continue;
            };
            if landing == Some(k.as_str()) {
                continue; // the copy is landing on it
            }
            let off = state.offset(k, *r, now);
            let alpha = if dragging == Some(k.as_str()) {
                GHOST
            } else {
                1.0
            };
            renderer.with_translation(off, |r| {
                with_opacity(alpha, || {
                    let style = renderer::Style {
                        text_color: Color {
                            a: style.text_color.a * alpha,
                            ..style.text_color
                        },
                    };
                    el.as_widget()
                        .draw(t, r, theme, &style, l, cursor, &(*viewport - off));
                });
            });
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        _renderer: &iced::Renderer,
        _viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        let state = tree.state.downcast_ref::<State>();
        let now = Instant::now();
        let (key, at) = if let Some(g) = state.session.dragging() {
            (g.key.clone(), g.top_left())
        } else if let Some(l) = &state.landing {
            let (_, _, slot) = state.slot(&l.key)?;
            let to =
                slot.position() + Vector::new(layout.bounds().x, layout.bounds().y) + translation;
            let t = (now.saturating_duration_since(l.start).as_secs_f32() / DROP.as_secs_f32())
                .min(1.0);
            let e = ease(t);
            (l.key.clone(), l.from + (to - l.from) * e)
        } else {
            return None;
        };
        let i = self.child_of(&key)?;
        let style: PreviewStyle = lifted(&self.cx.on_card(), 8.0, 1.0);
        Some(overlay::Element::new(Box::new(Preview {
            content: if i < self.headers.len() {
                &self.headers[i]
            } else {
                &self.cards[i - self.headers.len()]
            },
            tree: &tree.children[i],
            layout: layout.children().nth(i)?,
            at,
            style,
            ring: None,
            clip: None,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Vec<KanbanColumn> {
        vec![
            KanbanColumn::new(
                "todo",
                "To do",
                vec![
                    KanbanCard::new("c1", "Research"),
                    KanbanCard::new("c2", "Wireframes"),
                ],
            ),
            KanbanColumn::new(
                "doing",
                "In progress",
                vec![KanbanCard::new("c3", "Build UI").description("Component library")],
            ),
            KanbanColumn::new("done", "Done", vec![KanbanCard::new("c4", "Setup repo")]),
        ]
    }

    #[test]
    fn over_another_column_moves_the_card() {
        let b = board();
        // Over a card: before it.
        let n = drag_over(&b, "c1", &Over::Card("c3".into())).unwrap();
        assert_eq!(n[0].cards.len(), 1);
        assert_eq!(n[1].cards[0].id, "c1");
        assert_eq!(n[1].cards[1].id, "c3");
        // Over a column's empty part: at the end.
        let n = drag_over(&b, "c1", &Over::Column(2)).unwrap();
        assert_eq!(n[2].cards.last().unwrap().id, "c1");
        // Same column: nothing.
        assert!(drag_over(&b, "c1", &Over::Card("c2".into())).is_none());
        assert!(drag_over(&b, "c1", &Over::Column(0)).is_none());
    }

    #[test]
    fn dropping_on_a_card_of_its_column_reorders() {
        let b = board();
        let n = drag_end(&b, "c1", &Over::Card("c2".into())).unwrap();
        assert_eq!(n[0].cards[0].id, "c2");
        assert_eq!(n[0].cards[1].id, "c1");
        assert!(drag_end(&b, "c1", &Over::Card("c1".into())).is_none());
        assert!(drag_end(&b, "c1", &Over::Column(0)).is_none());
        assert!(drag_end(&b, "c1", &Over::Card("c3".into())).is_none());
    }
}
