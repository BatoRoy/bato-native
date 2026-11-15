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
//! **Submenus** (BatoBrowse `Menu.tsx` `SubmenuRow`): an item with
//! [`MenuAction::submenu`] shows a chevron and flies a nested list out to its
//! right, top-aligned with the row (`-top-1`), flipped to the left when it
//! would cross the window's right edge (8 px margin) and shifted up when it
//! would cross the bottom. Unlike React, opening has hover intent: the row
//! must stay hovered [`OPEN_DELAY`], and hovering a sibling closes the open
//! fly-out only after [`CLOSE_DELAY`], so a diagonal path to it does not
//! close it. Keyboard: → or Enter on the row opens it with its first item
//! highlighted, ← or Escape closes it (back to the row), ↑/↓ move inside
//! it. The row's own message is never published.
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

use crate::anim::{ease_out, ease_standard, with_opacity};
use crate::ctx::Ctx;
use crate::icon::{Ico, Icon, icon};
use crate::label::label;
use crate::scale::snap_rect;
use crate::style::{c, shadows};
use crate::surface::{BoxStyle, paint};
use iced::advanced::overlay;
use iced::{Transformation, Vector};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// A menu item (`MenuAction`).
#[derive(Debug, Clone)]
pub struct MenuAction<M> {
    pub label: String,
    pub icon: Option<Icon>,
    /// Display-only hint, e.g. `⌘C`.
    pub shortcut: Option<String>,
    pub danger: bool,
    pub disabled: bool,
    pub on_select: M,
    /// Submenu entries: the row opens them in a fly-out (its own
    /// `on_select` is then never published).
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
    pub fn icon(mut self, ico: impl Into<Icon>) -> Self {
        self.icon = Some(ico.into());
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
/// The shortcut's line box: `.mono-text` (index.css, after the utilities)
/// sets `line-height: 1.6` over `text-xs`'s 16 px, so 12 × 1.6. Centred in
/// the 32 px row, that puts its baseline 1 px higher than a 16 px box would.
const SHORTCUT_LH: f32 = 19.2;
const SEP_MY: f32 = 4.0; // my-1
const MIN_W: f32 = 192.0; // min-w-[12rem]
const TRANSITION: Duration = Duration::from_millis(150);
/// Hover intent: how long a submenu row must stay hovered to open.
pub const OPEN_DELAY: Duration = Duration::from_millis(100);
/// Hover intent: how long the pointer may cross siblings on its way into an
/// open fly-out before it closes.
pub const CLOSE_DELAY: Duration = Duration::from_millis(250);
/// The fly-out's enter animation (`animate-scale-in`).
const SUB_ENTER: f32 = 0.120;
/// Window margin of the flip/shift (`window.innerWidth - 8`).
const EDGE: f32 = 8.0;

/// The panel's border-box width for rows of max-content width `natural`
/// (padding and border included): `min-w`, capped by the room. Like
/// Chromium's LayoutUnits it stays fractional, so a label is never cut by
/// a rounding; the box is painted pixel-snapped (`snap_rect`: both edges
/// rounded), which is how a content-sized menu comes out as wide as React's
/// instead of a pixel wider (a `ceil` here did that).
fn panel_width(natural: f32, min_width: f32, max_w: f32) -> f32 {
    natural.max(min_width).min(max_w.max(min_width.min(max_w)))
}

/// Where a fly-out of `size` opens next to the submenu `row` (absolute
/// coordinates) in a `window`: right of the row, its first item level with
/// the row (`-top-1`); flipped left when it would cross the right edge and
/// fits on the left; shifted up by its overflow past the bottom edge.
pub fn submenu_position(row: Rectangle, size: Size, window: Size) -> Point {
    // The row sits PAD inside its own list, which has a BORDER.
    let (left_edge, right_edge) = (row.x - PAD - BORDER, row.x + row.width + PAD + BORDER);
    let flip = right_edge + size.width > window.width - EDGE && left_edge - size.width > EDGE;
    let x = if flip {
        left_edge - size.width
    } else {
        right_edge
    };
    let top = row.y - PAD - BORDER;
    let overflow = top + size.height - (window.height - EDGE);
    let y = if overflow > 0.0 { top - overflow } else { top };
    Point::new(x, y.max(0.0))
}

/// What the hover-intent timer will do when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Open(usize),
    Close,
}

/// The hover-intent state machine of a list with submenus.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Intent {
    pub open: Option<usize>,
    pub pending: Option<(Pending, Instant)>,
}

impl Intent {
    /// The pointer is over `row` (`None`: off the list, or over the open
    /// fly-out); `sub` says whether that row has a submenu.
    pub fn hover(&mut self, row: Option<usize>, sub: bool, now: Instant) {
        match row {
            // Into the fly-out (or away): keep what is open.
            None => self.pending = None,
            Some(i) if self.open == Some(i) => self.pending = None,
            Some(i) if sub => self.pending = Some((Pending::Open(i), now + OPEN_DELAY)),
            Some(_) if self.open.is_some() => {
                if !matches!(self.pending, Some((Pending::Close, _))) {
                    self.pending = Some((Pending::Close, now + CLOSE_DELAY));
                }
            }
            Some(_) => self.pending = None,
        }
    }

    /// Take a due timer (the caller opens or closes, [`Intent::open_now`],
    /// [`Intent::close_now`]).
    pub fn tick(&mut self, now: Instant) -> Option<Pending> {
        match self.pending {
            Some((p, at)) if now >= at => {
                self.pending = None;
                Some(p)
            }
            _ => None,
        }
    }

    /// When to wake up for the pending timer.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.map(|(_, at)| at)
    }

    pub fn open_now(&mut self, i: usize) {
        self.open = Some(i);
        self.pending = None;
    }

    pub fn close_now(&mut self) {
        self.open = None;
        self.pending = None;
    }
}

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
    /// Index into `subs` of this row's fly-out.
    sub: Option<usize>,
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
    /// The fly-outs of submenu rows (their trees follow `children`'s).
    subs: Vec<MenuList<'a, M>>,
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
    let mut subs: Vec<MenuList<'a, M>> = Vec::new();
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
                    children.push(
                        label(s.clone(), 12.0, SHORTCUT_LH)
                            .mono()
                            .color(c(p.muted))
                            .into(),
                    );
                    true
                } else {
                    false
                };
                let sub = (!a.submenu.is_empty()).then(|| {
                    subs.push(menu_list(cx, a.submenu));
                    subs.len() - 1
                });
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
                        sub,
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
        subs,
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
    /// Published before the item's message when an item is picked (in a
    /// fly-out too).
    pub fn on_close(mut self, m: M) -> Self
    where
        M: Clone,
    {
        self.subs = std::mem::take(&mut self.subs)
            .into_iter()
            .map(|s| s.on_close(m.clone()))
            .collect();
        self.on_close = Some(m);
        self
    }
    /// `min-w-[12rem]` by default; Select passes the trigger width.
    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }
}

#[derive(Debug)]
struct State {
    /// Arrow keys drive this list (a fly-out only once the keyboard or the
    /// pointer went into it).
    keys: bool,
    intent: Intent,
    /// A fly-out's enter animation (this list as a fly-out).
    shown: Option<Instant>,
    appear: f32,
    active: Option<usize>,
    prev: Option<usize>,
    changed: Option<Instant>,
    t: f32,
    hovered: Option<usize>,
    pressed: Option<usize>,
    /// Row rectangles relative to the list's top-left.
    rects: Vec<Rectangle>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            keys: true,
            intent: Intent::default(),
            shown: None,
            appear: 1.0,
            active: None,
            prev: None,
            changed: None,
            t: 0.0,
            hovered: None,
            pressed: None,
            rects: Vec::new(),
        }
    }
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
            Some(a) if a.enabled && a.sub.is_some() => false,
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

    fn sub_of(&self, i: usize) -> Option<usize> {
        self.rows
            .get(i)
            .and_then(|r| r.action.as_ref())
            .filter(|a| a.enabled)
            .and_then(|a| a.sub)
    }

    /// Open row `i`'s fly-out now; `keys` hands it the keyboard with its
    /// first item highlighted.
    fn open_sub(&self, tree: &mut Tree, i: usize, keys: bool, now: Instant) {
        let Some(j) = self.sub_of(i) else { return };
        tree.state.downcast_mut::<State>().intent.open_now(i);
        let sub = &mut tree.children[self.children.len() + j];
        let first = edge_enabled_index(&self.subs[j].entries, 1);
        let s = sub.state.downcast_mut::<State>();
        let rects = std::mem::take(&mut s.rects);
        *s = State {
            keys,
            shown: None,
            appear: 0.0,
            rects,
            ..State::default()
        };
        if keys && first >= 0 {
            s.set_active(Some(first as usize), now);
        }
    }

    fn close_sub(&self, tree: &mut Tree) {
        let st = tree.state.downcast_mut::<State>();
        let Some(i) = st.intent.open else { return };
        st.intent.close_now();
        if let Some(j) = self.sub_of(i) {
            // A nested fly-out closes with it.
            let sub = &mut tree.children[self.children.len() + j];
            self.subs[j].close_sub(sub);
        }
    }

    /// Whether the open fly-out (or one inside it) has the keyboard.
    fn sub_has_keys(&self, tree: &Tree) -> bool {
        let st = tree.state.downcast_ref::<State>();
        st.intent
            .open
            .and_then(|i| self.sub_of(i))
            .is_some_and(|j| {
                let sub = &tree.children[self.children.len() + j];
                sub.state.downcast_ref::<State>().keys || self.subs[j].sub_has_keys(sub)
            })
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
        let mut v: Vec<Tree> = self.children.iter().map(Tree::new).collect();
        v.extend(
            self.subs
                .iter()
                .map(|s| Tree::new(s as &dyn Widget<M, iced::Theme, iced::Renderer>)),
        );
        v
    }
    fn diff(&self, tree: &mut Tree) {
        let all: Vec<&dyn Widget<M, iced::Theme, iced::Renderer>> = self
            .children
            .iter()
            .map(|e| e.as_widget())
            .chain(
                self.subs
                    .iter()
                    .map(|s| s as &dyn Widget<M, iced::Theme, iced::Renderer>),
            )
            .collect();
        tree.diff_children(&all);
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
        let width = panel_width(natural + 2.0 * inset, self.min_width, max_w);
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
        let now = Instant::now();
        let is_action = |i: usize| self.rows.get(i).is_some_and(|r| r.action.is_some());
        // A pointer release / tap on row `a`: pick it, or open its fly-out.
        let activate = |this: &Self, tree: &mut Tree, a: usize, shell: &mut Shell<'_, M>| {
            if this.sub_of(a).is_some() {
                if tree.state.downcast_ref::<State>().intent.open != Some(a) {
                    this.close_sub(tree);
                    this.open_sub(tree, a, false, now);
                    shell.invalidate_layout();
                    shell.request_redraw();
                }
            } else {
                this.pick(a, shell);
            }
        };
        match event {
            Event::Window(window::Event::RedrawRequested(at)) => {
                let st = tree.state.downcast_mut::<State>();
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
                // Hover intent.
                match st.intent.tick(*at) {
                    Some(Pending::Open(i)) => {
                        self.close_sub(tree);
                        self.open_sub(tree, i, false, now);
                        shell.invalidate_layout();
                        shell.request_redraw();
                    }
                    Some(Pending::Close) => {
                        self.close_sub(tree);
                        shell.invalidate_layout();
                        shell.request_redraw();
                    }
                    None => {}
                }
                if let Some(d) = tree.state.downcast_ref::<State>().intent.deadline() {
                    shell.request_redraw_at(d);
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let st = tree.state.downcast_mut::<State>();
                let row = cursor
                    .position()
                    .and_then(|p| st.row_at(origin, p))
                    .filter(|i| is_action(*i));
                if row != st.hovered {
                    st.hovered = row;
                    // mouseenter on a row highlights it; leaving keeps it.
                    if let Some(i) = row {
                        st.set_active(Some(i), now);
                        st.keys = true;
                        shell.request_redraw();
                    }
                    let open = st.intent.open;
                    let sub = row.is_some_and(|i| self.sub_of(i).is_some());
                    st.intent.hover(row, sub, now);
                    if let Some(d) = st.intent.deadline() {
                        shell.request_redraw_at(d);
                    }
                    // Back on this list: the open fly-out loses the keys.
                    if let (Some(i), Some(o)) = (row, open)
                        && i != o
                        && let Some(j) = self.sub_of(o)
                    {
                        let n = self.children.len();
                        tree.children[n + j].state.downcast_mut::<State>().keys = false;
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(p) = cursor.position_over(layout.bounds()) {
                    let st = tree.state.downcast_mut::<State>();
                    st.pressed = st.row_at(origin, p).filter(|i| is_action(*i));
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let st = tree.state.downcast_mut::<State>();
                let pressed = st.pressed.take();
                let over = cursor
                    .position()
                    .and_then(|p| st.row_at(origin, p))
                    .filter(|i| is_action(*i));
                if let (Some(a), Some(b)) = (pressed, over)
                    && a == b
                {
                    activate(self, tree, a, shell);
                    shell.capture_event();
                }
            }
            Event::Touch(touch::Event::FingerPressed { position, .. }) => {
                if layout.bounds().contains(*position) {
                    let st = tree.state.downcast_mut::<State>();
                    st.pressed = st.row_at(origin, *position).filter(|i| is_action(*i));
                    shell.capture_event();
                }
            }
            Event::Touch(touch::Event::FingerLifted { position, .. }) => {
                let st = tree.state.downcast_mut::<State>();
                let pressed = st.pressed.take();
                if let Some(a) = pressed
                    && st.row_at(origin, *position) == Some(a)
                {
                    activate(self, tree, a, shell);
                    shell.capture_event();
                }
            }
            Event::Touch(touch::Event::FingerLost { .. }) => {
                tree.state.downcast_mut::<State>().pressed = None;
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(named),
                ..
            }) => {
                use keyboard::key::Named;
                if !tree.state.downcast_ref::<State>().keys {
                    // A fly-out the keyboard is not in: its parent moves.
                    return;
                }
                // The open fly-out had the arrows first (nested overlays see
                // events first); what reaches here closes it.
                if self.sub_has_keys(tree) {
                    if matches!(named, Named::ArrowLeft | Named::Escape) {
                        self.close_sub(tree);
                        shell.invalidate_layout();
                        shell.request_redraw();
                        shell.capture_event();
                    }
                    return;
                }
                let st = tree.state.downcast_mut::<State>();
                let from = st.active.map_or(-1, |i| i as i32);
                let next = match named {
                    Named::ArrowDown => Some(next_enabled_index(&self.entries, from, 1)),
                    Named::ArrowUp => Some(next_enabled_index(&self.entries, from, -1)),
                    Named::Home => Some(edge_enabled_index(&self.entries, 1)),
                    Named::End => Some(edge_enabled_index(&self.entries, -1)),
                    Named::Enter | Named::ArrowRight => {
                        if let Some(i) = st.active
                            && self.sub_of(i).is_some()
                        {
                            self.close_sub(tree);
                            self.open_sub(tree, i, true, now);
                            shell.invalidate_layout();
                            shell.request_redraw();
                            shell.capture_event();
                        } else if *named == Named::Enter
                            && let Some(i) = st.active
                            && self.pick(i, shell)
                        {
                            shell.capture_event();
                        }
                        None
                    }
                    _ => None,
                };
                if let Some(n) = next {
                    let st = tree.state.downcast_mut::<State>();
                    let to = (n >= 0).then_some(n as usize);
                    st.set_active(to, now);
                    let open = st.intent.open;
                    if open.is_some() && open != to {
                        self.close_sub(tree);
                        shell.invalidate_layout();
                    }
                    shell.request_redraw();
                    // preventDefault: the page under the menu doesn't scroll.
                    shell.capture_event();
                }
            }
            _ => {}
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
        let st = tree.state.downcast_ref::<State>();
        let i = st.intent.open?;
        let j = self.sub_of(i)?;
        let r = *st.rects.get(i)?;
        let b = layout.bounds();
        let row = Rectangle {
            x: b.x + r.x + translation.x,
            y: b.y + r.y + translation.y,
            ..r
        };
        let n = self.children.len();
        Some(overlay::Element::new(Box::new(Flyout {
            list: &mut self.subs[j],
            tree: &mut tree.children[n + j],
            row,
        })))
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
        paint(renderer, snap_rect(b), &self.style());
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
            let rect = snap_rect(Rectangle {
                x: b.x + rect.x,
                y: b.y + rect.y,
                ..rect
            });
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

/// An open submenu: the nested list as an overlay next to its row.
struct Flyout<'a, 'b, M> {
    list: &'b mut MenuList<'a, M>,
    tree: &'b mut Tree,
    /// The submenu row, absolute.
    row: Rectangle,
}

impl<'a, M: Clone + 'a> overlay::Overlay<M, iced::Theme, iced::Renderer> for Flyout<'a, '_, M> {
    fn layout(&mut self, renderer: &iced::Renderer, bounds: Size) -> layout::Node {
        let limits = layout::Limits::new(Size::ZERO, bounds);
        let node = self.list.layout(self.tree, renderer, &limits);
        let at = submenu_position(self.row, node.size(), bounds);
        node.move_to(at)
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
    ) {
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            let st = self.tree.state.downcast_mut::<State>();
            let t0 = *st.shown.get_or_insert(*now);
            let f = now.saturating_duration_since(t0).as_secs_f32() / SUB_ENTER;
            st.appear = ease_out(f.min(1.0));
            if f < 1.0 {
                shell.request_redraw();
            }
        }
        let b = layout.bounds();
        self.list.update(
            self.tree, event, layout, cursor, renderer, clipboard, shell, &b,
        );
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.list
            .mouse_interaction(self.tree, layout, cursor, &layout.bounds(), renderer)
    }

    fn draw(
        &self,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let p = self.tree.state.downcast_ref::<State>().appear;
        if p <= 0.0 {
            return;
        }
        let b = layout.bounds();
        let draw = |renderer: &mut iced::Renderer| {
            with_opacity(p, || {
                self.list
                    .draw(self.tree, renderer, theme, style, layout, cursor, &b);
            });
        };
        if p < 1.0 {
            // Scale from the corner next to the row.
            let ox = if b.x < self.row.x { b.x + b.width } else { b.x };
            let k = 0.96 + 0.04 * p;
            let tf = Transformation::translate(ox, b.y)
                * Transformation::scale(k)
                * Transformation::translate(-ox, -b.y);
            renderer.with_transformation(tf, draw);
        } else {
            draw(renderer);
        }
    }

    fn overlay<'c>(
        &'c mut self,
        layout: Layout<'c>,
        renderer: &iced::Renderer,
    ) -> Option<overlay::Element<'c, M, iced::Theme, iced::Renderer>> {
        let b = layout.bounds();
        self.list
            .overlay(self.tree, layout, renderer, &b, Vector::ZERO)
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
    fn content_sized_width_stays_fractional() {
        // Room to spare: max-content wins over min-w and is not rounded up.
        assert_eq!(panel_width(205.3, MIN_W, 1280.0), 205.3);
        // Painted with both edges rounded: 205 px at x = 100, like Chromium
        // (a ceil made it 206).
        let r = snap_rect(Rectangle::new(
            Point::new(100.0, 0.0),
            Size::new(205.3, 10.0),
        ));
        assert_eq!(r.width, 205.0);
        assert_eq!(panel_width(150.0, MIN_W, 1280.0), MIN_W);
        // No room: the window width caps it, min-w included.
        assert_eq!(panel_width(400.0, MIN_W, 300.0), 300.0);
        assert_eq!(panel_width(400.0, MIN_W, 150.0), 150.0);
    }

    #[test]
    fn shortcut_baseline_matches_chromium() {
        use crate::label::{Fam, blink_baseline};
        // `.mono-text` 12/19.2 centred in the 32 px row: Blink's baseline
        // sits 19 px below the row top (6.4 + 12 + floor(1.6), rounded).
        let top = (ROW_H - SHORTCUT_LH) / 2.0;
        assert_eq!(
            (top + blink_baseline(Fam::Mono, 12.0, SHORTCUT_LH, 1.0)).round(),
            19.0
        );
        // With text-xs's own 16 px line box it was 20: the pixel too low.
        let top16 = (ROW_H - 16.0) / 2.0;
        assert_eq!(
            (top16 + blink_baseline(Fam::Mono, 12.0, 16.0, 1.0)).round(),
            20.0
        );
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

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rectangle {
        Rectangle::new(Point::new(x, y), Size::new(w, h))
    }

    #[test]
    fn flyout_opens_right_flips_left_and_shifts_up() {
        let win = Size::new(1000.0, 600.0);
        let size = Size::new(200.0, 120.0);
        // A row at x 105..287 inside a list at 100..292 (border 1 + pad 4).
        let row = r(105.0, 205.0, 182.0, 32.0);
        assert_eq!(
            submenu_position(row, size, win),
            Point::new(292.0, 200.0),
            "right of the list, first item level with the row"
        );
        // Near the right edge: flips to the list's left.
        let row = r(705.0, 205.0, 182.0, 32.0);
        assert_eq!(submenu_position(row, size, win), Point::new(500.0, 200.0));
        // No room on either side: stays right (React: left only if it fits).
        let narrow = Size::new(400.0, 600.0);
        let row = r(105.0, 205.0, 182.0, 32.0);
        assert_eq!(submenu_position(row, size, narrow).x, 292.0);
        // Near the bottom: shifted up by the overflow (8 px margin).
        let row = r(105.0, 525.0, 182.0, 32.0);
        assert_eq!(submenu_position(row, size, win).y, 600.0 - 8.0 - 120.0);
    }

    #[test]
    fn hover_intent_opens_after_a_delay_and_survives_a_diagonal() {
        let t0 = Instant::now();
        let mut i = Intent::default();
        // Hover a submenu row: opens only after OPEN_DELAY.
        i.hover(Some(2), true, t0);
        assert_eq!(i.tick(t0 + OPEN_DELAY / 2), None);
        assert_eq!(i.tick(t0 + OPEN_DELAY), Some(Pending::Open(2)));
        i.open_now(2);
        // Crossing a sibling on the way into the fly-out does not close it.
        let t1 = t0 + Duration::from_millis(400);
        i.hover(Some(3), false, t1);
        assert_eq!(i.deadline(), Some(t1 + CLOSE_DELAY));
        i.hover(None, false, t1 + Duration::from_millis(50));
        assert_eq!(i.tick(t1 + CLOSE_DELAY), None);
        assert_eq!(i.open, Some(2));
        // Resting on a sibling closes it; another submenu row replaces it.
        i.hover(Some(3), false, t1);
        assert_eq!(i.tick(t1 + CLOSE_DELAY), Some(Pending::Close));
        i.close_now();
        i.hover(Some(4), true, t1);
        i.hover(Some(5), false, t1 + Duration::from_millis(10));
        assert_eq!(i.tick(t1 + OPEN_DELAY * 2), None, "left before the delay");
        // Back on the open row: nothing pending.
        i.open_now(4);
        i.hover(Some(4), true, t1);
        assert_eq!(i.pending, None);
    }

    #[test]
    fn submenu_rows_build_flyouts_and_never_pick() {
        let items: Vec<MenuEntry<u8>> = vec![
            action("Open", 0).into(),
            action("Open with", 9)
                .submenu(vec![action("Editor", 1).into(), action("Viewer", 2).into()])
                .into(),
            action("Off", 8)
                .disabled(true)
                .submenu(vec![action("X", 3).into()])
                .into(),
        ];
        let cx = Ctx::from_palette(bato_theme::palette(
            bato_theme::ThemeId::Dark,
            bato_theme::Rgb::new(0x8b, 0x5c, 0xf6),
        ));
        let m = menu_list(&cx, items).on_close(7);
        assert_eq!(m.subs.len(), 2);
        assert_eq!(m.sub_of(1), Some(0));
        assert_eq!(m.sub_of(2), None, "a disabled row does not open");
        assert_eq!(m.sub_of(0), None);
        assert_eq!(m.subs[0].on_close, Some(7), "on_close reaches the fly-out");
        assert_eq!(m.subs[0].entries.len(), 2);
    }
}
