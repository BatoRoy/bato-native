//! Menu primitives (INF-4): the `MenuList` surface (`Menu.tsx`) shared by
//! ContextMenu, Dropdown and Select, its entries, and `nextEnabledIndex`.
//!
//! The list owns its transient state (the highlighted index, hover, press)
//! and its keyboard: while it is on screen, ↑/↓ move the highlight (wrapping,
//! skipping separators and disabled items, exactly `nextEnabledIndex`),
//! Home/End jump to the first/last enabled item, Enter picks the highlighted
//! one. Escape belongs to the overlay that shows the list
//! ([`crate::overlay::anchored`]). Like React, the pointer and the keyboard
//! share one highlight: entering a row highlights it, and arrows move on
//! from there; leaving a row keeps it. No typeahead (React has none).
//!
//! Picking an item publishes `on_close` (if set) and then the item's message,
//! React's `onClose(); item.onClick()`.
//!
//! ```ignore
//! let items = vec![
//!     action("Copy", Msg::Copy).icon(Ico::Copy).shortcut("⌘C"),
//!     action("Paste", Msg::Paste).icon(Ico::Clipboard).disabled(true),
//!     separator(),
//!     action("Delete", Msg::Delete).icon(Ico::Trash2).danger(),
//! ];
//! menu_list(&cx, items).on_close(Msg::CloseMenu)
//! ```

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::{
    Border, Color, Element, Event, Length, Point, Rectangle, Size, keyboard, mouse, touch, window,
};

use crate::anim::{ease_standard, with_opacity};
use crate::ctx::Ctx;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::style::{c, shadows};
use crate::surface::{BoxStyle, paint};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// A menu item (`MenuAction`).
#[derive(Debug, Clone)]
pub struct MenuAction<M> {
    pub label: String,
    pub icon: Option<Ico>,
    /// Display-only hint, e.g. `⌘C`.
    pub shortcut: Option<String>,
    pub danger: bool,
    pub disabled: bool,
    pub on_select: M,
    /// Submenu entries (data model only: the row shows a chevron; opening
    /// submenus comes with the components that need them).
    pub submenu: Vec<MenuEntry<M>>,
}

/// An item or a separator (`MenuEntry`).
#[derive(Debug, Clone)]
pub enum MenuEntry<M> {
    Action(MenuAction<M>),
    Separator,
}

impl<M> MenuEntry<M> {
    /// A selectable action (`isAction(e) && !e.disabled`).
    pub fn is_enabled(&self) -> bool {
        matches!(self, MenuEntry::Action(a) if !a.disabled)
    }
}

/// A menu item that publishes `on_select`.
pub fn action<M>(label: impl Into<String>, on_select: M) -> MenuAction<M> {
    MenuAction {
        label: label.into(),
        icon: None,
        shortcut: None,
        danger: false,
        disabled: false,
        on_select,
        submenu: Vec::new(),
    }
}

/// `{ type: 'separator' }`.
pub fn separator<M>() -> MenuEntry<M> {
    MenuEntry::Separator
}

impl<M> MenuAction<M> {
    pub fn icon(mut self, ico: Ico) -> Self {
        self.icon = Some(ico);
        self
    }
    pub fn shortcut(mut self, s: impl Into<String>) -> Self {
        self.shortcut = Some(s.into());
        self
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }
    pub fn submenu(mut self, entries: Vec<MenuEntry<M>>) -> Self {
        self.submenu = entries;
        self
    }
}

impl<M> From<MenuAction<M>> for MenuEntry<M> {
    fn from(a: MenuAction<M>) -> Self {
        MenuEntry::Action(a)
    }
}

/// `nextEnabledIndex(items, from, dir)`: the next selectable entry after
/// `from` (`-1` = none) in direction `dir` (±1), wrapping; `-1` if there is
/// none. A port, quirks included: ↑ from `-1` lands on the second-to-last
/// slot first, as in React.
pub fn next_enabled_index<M>(items: &[MenuEntry<M>], from: i32, dir: i32) -> i32 {
    let n = items.len() as i32;
    if n == 0 {
        return -1;
    }
    for step in 1..=n {
        // JS `%` keeps the dividend's sign; the only negative case is
        // -1 % 1 (= -0, which indexes item 0), so `rem_euclid` matches.
        let i = (from + dir * step + n * step).rem_euclid(n);
        if items[i as usize].is_enabled() {
            return i;
        }
    }
    -1
}

/// The first (`dir` = 1) or last (`dir` = -1) enabled entry (Home / End).
pub fn edge_enabled_index<M>(items: &[MenuEntry<M>], dir: i32) -> i32 {
    let n = items.len() as i32;
    let from = if dir > 0 { n - 1 } else { 0 };
    // From the opposite end, the next step wraps onto the edge itself.
    next_enabled_index(items, from, dir)
}

/// Measurements (`Menu.tsx` classes, CSS px).
const BORDER: f32 = 1.0;
const PAD: f32 = 4.0; // p-1
const ROW_H: f32 = 32.0; // h-8
const ROW_PX: f32 = 10.0; // px-2.5
const GAP: f32 = 10.0; // gap-2.5
const ICON: f32 = 15.0;
const SHORTCUT_PL: f32 = 16.0; // pl-4
const SEP_MY: f32 = 4.0; // my-1
const MIN_W: f32 = 192.0; // min-w-[12rem]
const TRANSITION: Duration = Duration::from_millis(150);

struct Row<M> {
    /// `None` for a separator.
    action: Option<RowAction<M>>,
}

struct RowAction<M> {
    msg: M,
    enabled: bool,
    danger: bool,
    has_icon: bool,
    has_trailing: bool,
    /// Index of this row's first child; the children are
    /// `[icon rest, icon active]?, label rest, label active, trailing?`.
    first: usize,
}

impl<M> RowAction<M> {
    fn count(&self) -> usize {
        2 + if self.has_icon { 2 } else { 0 } + usize::from(self.has_trailing)
    }
}

#[derive(Debug, Clone, Copy)]
struct Colors {
    panel: Color,
    border: Color,
    line: Color,
    hl: Color,
    hl_danger: Color,
}

/// The `MenuList` surface.
pub struct MenuList<'a, M> {
    rows: Vec<Row<M>>,
    entries: Vec<MenuEntry<()>>,
    children: Vec<El<'a, M>>,
    on_close: Option<M>,
    colors: Colors,
    shadows: Vec<crate::surface::Sh>,
    min_width: f32,
}

/// The menu surface for `entries`; `cx` is the context the menu floats in
/// (it draws on `bg-app-card`).
pub fn menu_list<'a, M: Clone + 'a>(cx: &Ctx, entries: Vec<MenuEntry<M>>) -> MenuList<'a, M> {
    let on = cx.on_card();
    let p = on.p();
    let mut rows = Vec::with_capacity(entries.len());
    let mut children: Vec<El<'a, M>> = Vec::new();
    let mut shape = Vec::with_capacity(entries.len());
    for e in entries {
        match e {
            MenuEntry::Separator => {
                rows.push(Row { action: None });
                shape.push(MenuEntry::Separator);
            }
            MenuEntry::Action(a) => {
                // Rest: danger → red, else subtext; disabled inherits the
                // body text colour (and the row fades to 40 %).
                let rest = if a.disabled {
                    p.text
                } else if a.danger {
                    p.red
                } else {
                    p.subtext
                };
                let active = if a.danger && !a.disabled {
                    p.red
                } else {
                    p.text
                };
                let first = children.len();
                if let Some(ico) = a.icon {
                    children.push(icon(ico, ICON).color(c(rest)).into());
                    children.push(icon(ico, ICON).color(c(active)).into());
                }
                let text = |col| label(a.label.clone(), 13.0, 20.0).ellipsis().color(c(col));
                children.push(text(rest).into());
                children.push(text(active).into());
                let has_trailing = if !a.submenu.is_empty() {
                    children.push(icon(Ico::ChevronRight, 14.0).color(c(p.muted)).into());
                    true
                } else if let Some(s) = &a.shortcut {
                    children.push(label(s.clone(), 12.0, 16.0).mono().color(c(p.muted)).into());
                    true
                } else {
                    false
                };
                shape.push(if a.disabled {
                    MenuEntry::Action(action(String::new(), ()).disabled(true))
                } else {
                    MenuEntry::Action(action(String::new(), ()))
                });
                rows.push(Row {
                    action: Some(RowAction {
                        msg: a.on_select,
                        enabled: !a.disabled,
                        danger: a.danger,
                        has_icon: a.icon.is_some(),
                        has_trailing,
                        first,
                    }),
                });
            }
        }
    }
    MenuList {
        rows,
        entries: shape,
        children,
        on_close: None,
        colors: Colors {
            panel: c(p.card),
            border: c(on.line_strong()),
            line: c(on.line()),
            hl: c(on.wash(0.07)),
            hl_danger: c(on.blend(p.red, 0.10)),
        },
        shadows: shadows(&p.shadow_lg),
        min_width: MIN_W,
    }
}

impl<'a, M> MenuList<'a, M> {
    /// Published before the item's message when an item is picked.
    pub fn on_close(mut self, m: M) -> Self {
        self.on_close = Some(m);
        self
    }
    /// `min-w-[12rem]` by default; Select passes the trigger width.
    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }
}

#[derive(Debug, Default)]
struct State {
    active: Option<usize>,
    prev: Option<usize>,
    changed: Option<Instant>,
    t: f32,
    hovered: Option<usize>,
    pressed: Option<usize>,
    /// Row rectangles relative to the list's top-left.
    rects: Vec<Rectangle>,
}

impl State {
    fn set_active(&mut self, i: Option<usize>, now: Instant) {
        if i == self.active {
            return;
        }
        self.prev = self.active;
        self.active = i;
        self.changed = Some(now);
        self.t = 0.0;
    }

    /// Highlight amount of row `i` (0..1).
    fn highlight(&self, i: usize) -> f32 {
        let t = if self.changed.is_some() { self.t } else { 1.0 };
        if self.active == Some(i) {
            t
        } else if self.prev == Some(i) {
            1.0 - t
        } else {
            0.0
        }
    }

    fn row_at(&self, origin: Point, p: Point) -> Option<usize> {
        let local = Point::new(p.x - origin.x, p.y - origin.y);
        self.rects.iter().position(|r| r.contains(local))
    }
}

impl<'a, M: Clone + 'a> MenuList<'a, M> {
    fn pick(&self, i: usize, shell: &mut Shell<'_, M>) -> bool {
        match self.rows.get(i).and_then(|r| r.action.as_ref()) {
            Some(a) if a.enabled => {
                if let Some(m) = &self.on_close {
                    shell.publish(m.clone());
                }
                shell.publish(a.msg.clone());
                true
            }
            _ => false,
        }
    }

    fn style(&self) -> BoxStyle {
        BoxStyle {
            bg: Some(self.colors.panel),
            border: Some(self.colors.border),
            radius: 12.0,
            shadows: self.shadows.clone(),
            ..BoxStyle::default()
        }
    }
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for MenuList<'a, M> {
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
        self.children.iter().map(Tree::new).collect()
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.children);
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let max_w = limits.max().width;
        let inset = BORDER + PAD;
        let free = layout::Limits::new(Size::ZERO, Size::new(f32::INFINITY, f32::INFINITY));
        // Natural (max-content) width of every row.
        let mut natural: f32 = 0.0;
        for r in &self.rows {
            let Some(a) = &r.action else { continue };
            let lab = a.first + if a.has_icon { 2 } else { 0 };
            let lw = self.children[lab]
                .as_widget_mut()
                .layout(&mut tree.children[lab], renderer, &free)
                .size()
                .width;
            let tw = if a.has_trailing {
                let ti = lab + 2;
                GAP + SHORTCUT_PL
                    + self.children[ti]
                        .as_widget_mut()
                        .layout(&mut tree.children[ti], renderer, &free)
                        .size()
                        .width
            } else {
                0.0
            };
            let iw = if a.has_icon { ICON + GAP } else { 0.0 };
            natural = natural.max(ROW_PX + iw + lw + tw + ROW_PX);
        }
        let width = (natural + 2.0 * inset)
            .max(self.min_width)
            .min(max_w.max(self.min_width.min(max_w)))
            .ceil();
        let inner_w = width - 2.0 * inset;

        let mut nodes: Vec<layout::Node> = Vec::with_capacity(self.children.len());
        let mut rects = Vec::with_capacity(self.rows.len());
        let mut y = inset;
        for r in &self.rows {
            let Some(a) = &r.action else {
                y += SEP_MY;
                rects.push(Rectangle::new(
                    Point::new(BORDER, y),
                    Size::new(width - 2.0 * BORDER, 1.0),
                ));
                y += 1.0 + SEP_MY;
                continue;
            };
            let row = Rectangle::new(Point::new(inset, y), Size::new(inner_w, ROW_H));
            rects.push(row);
            let mut x = row.x + ROW_PX;
            let mut idx = a.first;
            if a.has_icon {
                for _ in 0..2 {
                    let n = self.children[idx].as_widget_mut().layout(
                        &mut tree.children[idx],
                        renderer,
                        &free,
                    );
                    let h = n.size().height;
                    nodes.push(n.move_to(Point::new(x, y + (ROW_H - h) / 2.0)));
                    idx += 1;
                }
                x += ICON + GAP;
            }
            let trailing_w = if a.has_trailing {
                let ti = idx + 2;
                let n = self.children[ti].as_widget_mut().layout(
                    &mut tree.children[ti],
                    renderer,
                    &free,
                );
                n.size().width
            } else {
                0.0
            };
            let label_max = (row.x + row.width
                - ROW_PX
                - if a.has_trailing {
                    GAP + SHORTCUT_PL + trailing_w
                } else {
                    0.0
                }
                - x)
                .max(0.0);
            let lim = layout::Limits::new(Size::ZERO, Size::new(label_max, f32::INFINITY));
            for _ in 0..2 {
                let n = self.children[idx].as_widget_mut().layout(
                    &mut tree.children[idx],
                    renderer,
                    &lim,
                );
                let h = n.size().height;
                nodes.push(n.move_to(Point::new(x, y + (ROW_H - h) / 2.0)));
                idx += 1;
            }
            if a.has_trailing {
                let n = self.children[idx].as_widget_mut().layout(
                    &mut tree.children[idx],
                    renderer,
                    &free,
                );
                let s = n.size();
                nodes.push(n.move_to(Point::new(
                    row.x + row.width - ROW_PX - s.width,
                    y + (ROW_H - s.height) / 2.0,
                )));
            }
            y += ROW_H;
        }
        let height = y + inset;
        tree.state.downcast_mut::<State>().rects = rects;
        layout::Node::with_children(Size::new(width, height), nodes)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        let origin = layout.bounds().position();
        let st = tree.state.downcast_mut::<State>();
        let now = Instant::now();
        let is_action = |i: usize| self.rows.get(i).is_some_and(|r| r.action.is_some());
        match event {
            Event::Window(window::Event::RedrawRequested(at)) => {
                if let Some(t0) = st.changed {
                    let f =
                        at.saturating_duration_since(t0).as_secs_f32() / TRANSITION.as_secs_f32();
                    st.t = ease_standard(f.min(1.0));
                    if f >= 1.0 {
                        st.changed = None;
                        st.prev = None;
                    } else {
                        shell.request_redraw();
                    }
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let row = cursor
                    .position()
                    .and_then(|p| st.row_at(origin, p))
                    .filter(|i| is_action(*i));
                if row != st.hovered {
                    st.hovered = row;
                    // mouseenter on a row highlights it; leaving keeps it.
                    if let Some(i) = row {
                        st.set_active(Some(i), now);
                        shell.request_redraw();
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(p) = cursor.position_over(layout.bounds()) {
                    st.pressed = st.row_at(origin, p).filter(|i| is_action(*i));
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let pressed = st.pressed.take();
                let over = cursor
                    .position()
                    .and_then(|p| st.row_at(origin, p))
                    .filter(|i| is_action(*i));
                if let (Some(a), Some(b)) = (pressed, over)
                    && a == b
                {
                    self.pick(a, shell);
                    shell.capture_event();
                }
            }
            Event::Touch(touch::Event::FingerPressed { position, .. }) => {
                if layout.bounds().contains(*position) {
                    st.pressed = st.row_at(origin, *position).filter(|i| is_action(*i));
                    shell.capture_event();
                }
            }
            Event::Touch(touch::Event::FingerLifted { position, .. }) => {
                let pressed = st.pressed.take();
                if let Some(a) = pressed
                    && st.row_at(origin, *position) == Some(a)
                {
                    self.pick(a, shell);
                    shell.capture_event();
                }
            }
            Event::Touch(touch::Event::FingerLost { .. }) => st.pressed = None,
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(named),
                ..
            }) => {
                use keyboard::key::Named;
                let from = st.active.map_or(-1, |i| i as i32);
                let next = match named {
                    Named::ArrowDown => Some(next_enabled_index(&self.entries, from, 1)),
                    Named::ArrowUp => Some(next_enabled_index(&self.entries, from, -1)),
                    Named::Home => Some(edge_enabled_index(&self.entries, 1)),
                    Named::End => Some(edge_enabled_index(&self.entries, -1)),
                    Named::Enter => {
                        if let Some(i) = st.active
                            && self.pick(i, shell)
                        {
                            shell.capture_event();
                        }
                        None
                    }
                    _ => None,
                };
                if let Some(n) = next {
                    st.set_active((n >= 0).then_some(n as usize), now);
                    shell.request_redraw();
                    // preventDefault: the page under the menu doesn't scroll.
                    shell.capture_event();
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let st = tree.state.downcast_ref::<State>();
        let Some(p) = cursor.position_over(layout.bounds()) else {
            return mouse::Interaction::None;
        };
        match st
            .row_at(layout.bounds().position(), p)
            .and_then(|i| self.rows[i].action.as_ref())
        {
            Some(a) if a.enabled => mouse::Interaction::Pointer,
            Some(_) => mouse::Interaction::NotAllowed,
            None => mouse::Interaction::Idle,
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
        let st = tree.state.downcast_ref::<State>();
        let b = layout.bounds();
        paint(renderer, b, &self.style());
        let kids: Vec<Layout<'_>> = layout.children().collect();
        let draw_child = |renderer: &mut iced::Renderer, i: usize| {
            self.children[i].as_widget().draw(
                &tree.children[i],
                renderer,
                theme,
                style,
                kids[i],
                cursor,
                viewport,
            );
        };
        let mut node = 0usize;
        for (i, row) in self.rows.iter().enumerate() {
            let rect = st.rects.get(i).copied().unwrap_or_default();
            let rect = Rectangle {
                x: b.x + rect.x,
                y: b.y + rect.y,
                ..rect
            };
            let Some(a) = &row.action else {
                renderer.fill_quad(
                    Quad {
                        bounds: rect,
                        ..Quad::default()
                    },
                    crate::anim::fade(self.colors.line),
                );
                continue;
            };
            let k = if a.enabled { st.highlight(i) } else { 0.0 };
            if k > 0.0 {
                let hl = if a.danger {
                    self.colors.hl_danger
                } else {
                    self.colors.hl
                };
                renderer.fill_quad(
                    Quad {
                        bounds: rect,
                        border: Border {
                            radius: 6.0.into(),
                            ..Border::default()
                        },
                        ..Quad::default()
                    },
                    crate::anim::fade(Color { a: hl.a * k, ..hl }),
                );
            }
            debug_assert_eq!(node, a.first);
            let n = a.count();
            let row_alpha = if a.enabled { 1.0 } else { 0.4 };
            with_opacity(row_alpha, || {
                // Children: [icon rest, icon active]?, label rest, label active, trailing?
                let mut j = a.first;
                let pairs = if a.has_icon { 2 } else { 1 };
                for _ in 0..pairs {
                    if k < 1.0 {
                        with_opacity(1.0 - k, || draw_child(renderer, j));
                    }
                    if k > 0.0 {
                        with_opacity(k, || draw_child(renderer, j + 1));
                    }
                    j += 2;
                }
                if a.has_trailing {
                    draw_child(renderer, j);
                }
            });
            node += n;
        }
    }
}

impl<'a, M: Clone + 'a> From<MenuList<'a, M>> for El<'a, M> {
    fn from(m: MenuList<'a, M>) -> Self {
        Element::new(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<MenuEntry<u8>> {
        // The Showcase context menu: Copy, Cut, Paste (disabled), —, Delete.
        vec![
            action("Copy", 0).into(),
            action("Cut", 1).into(),
            action("Paste", 2).disabled(true).into(),
            separator(),
            action("Delete", 4).danger().into(),
        ]
    }

    #[test]
    fn next_enabled_index_matches_react() {
        let it = items();
        // ↓ from nothing: the first item.
        assert_eq!(next_enabled_index(&it, -1, 1), 0);
        assert_eq!(next_enabled_index(&it, 0, 1), 1);
        // Skips the disabled item and the separator.
        assert_eq!(next_enabled_index(&it, 1, 1), 4);
        // Wraps.
        assert_eq!(next_enabled_index(&it, 4, 1), 0);
        assert_eq!(next_enabled_index(&it, 0, -1), 4);
        assert_eq!(next_enabled_index(&it, 4, -1), 1);
        // ↑ from nothing starts at n-2 (React's arithmetic): 3 is a
        // separator, 2 disabled, so Cut.
        assert_eq!(next_enabled_index(&it, -1, -1), 1);
        let two: Vec<MenuEntry<u8>> = vec![action("Edit", 0).into(), action("Delete", 1).into()];
        assert_eq!(next_enabled_index(&two, -1, -1), 0);
        let one: Vec<MenuEntry<u8>> = vec![action("Only", 0).into()];
        assert_eq!(next_enabled_index(&one, -1, -1), 0);
        assert_eq!(next_enabled_index(&one, 0, 1), 0);
        let none: Vec<MenuEntry<u8>> = vec![separator(), action("Off", 0).disabled(true).into()];
        assert_eq!(next_enabled_index(&none, -1, 1), -1);
        assert_eq!(next_enabled_index::<u8>(&[], -1, 1), -1);
    }

    #[test]
    fn home_and_end() {
        let it = items();
        assert_eq!(edge_enabled_index(&it, 1), 0);
        assert_eq!(edge_enabled_index(&it, -1), 4);
        let tail_off: Vec<MenuEntry<u8>> = vec![
            separator(),
            action("A", 0).into(),
            action("B", 1).into(),
            action("C", 2).disabled(true).into(),
        ];
        assert_eq!(edge_enabled_index(&tail_off, 1), 1);
        assert_eq!(edge_enabled_index(&tail_off, -1), 2);
    }

    #[test]
    fn highlight_crossfades_between_rows() {
        let mut s = State::default();
        let t0 = Instant::now();
        s.set_active(Some(0), t0);
        s.t = 0.25;
        assert_eq!(s.highlight(0), 0.25);
        s.set_active(Some(1), t0);
        s.t = 0.5;
        assert_eq!((s.highlight(0), s.highlight(1)), (0.5, 0.5));
        s.changed = None;
        assert_eq!((s.highlight(0), s.highlight(1)), (0.0, 1.0));
    }
}
