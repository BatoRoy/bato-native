//! `DataTable` (`data/DataTable.tsx`): a client-side table with sortable
//! columns (asc → desc → none), an optional filter across every accessor
//! (resets the page), row selection with a select-all header checkbox
//! (indeterminate when some rows of the page are selected), row hover,
//! per-row actions (labeled tone buttons and icon buttons revealed on row
//! hover), an empty state and built-in pagination.
//!
//! The sort, filter text and page are React state inside the component; in
//! iced they live in the app as a [`TableState`] that [`TableMsg`]s update.
//! Selection is controlled, as in React (`selected` + `on_select`).
//!
//! The table itself is a [`TableGrid`]: Chromium's auto table layout
//! (max-content column widths, fixed `width` columns, the excess shared in
//! proportion to max-content), rows centred vertically, a row hover fill
//! (`text@.06`, 150 ms) and the `group-hover` link for the row's children
//! ([`reveal`], [`group_progress`]).
//!
//! ```ignore
//! data_table(&cx, &people, columns, &self.table, Msg::Table)
//!     .row_key(|p| p.id.clone())
//!     .filterable(true)
//!     .selectable(&self.selected, Msg::Selected)
//!     .page_size(6)
//!     .row_actions(|p| vec![RowAction::new(Ico::Eye, Msg::View(p.id)).label("View").tone(ActionTone::Accent)])
//! ```

use std::cell::Cell;
use std::cmp::Ordering;
use std::rc::Rc;
use std::time::Instant;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{self, Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::widget::{Space, column, row};
use iced::{
    Alignment, Color, Element, Event, Length, Padding, Point, Rectangle, Size, mouse, window,
};

use super::pagination::pagination;
use crate::El;
use crate::anim::{Tween, ease_standard, fade, opacity, with_opacity};
use crate::checkbox::checkbox;
use crate::ctx::Ctx;
use crate::empty::empty_state;
use crate::field::text_control;
use crate::focus::Ring;
use crate::icon::{Ico, Icon, icon};
use crate::label::{Align, label};
use crate::layout::rounded_clip;
use crate::overlay::tooltip;
use crate::style::{c, lerp};
use crate::surface::{BoxStyle, Status, surface};

const FADE: std::time::Duration = std::time::Duration::from_millis(150);

// ── Values, sorting, filtering, paging (unit-tested) ────────────────────

/// What an accessor returns (`string | number`).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Num(f64),
}

impl Value {
    /// `String(v)`.
    pub fn to_js_string(&self) -> String {
        match self {
            Value::Text(s) => s.clone(),
            Value::Num(n) => super::charts::js_string(*n),
        }
    }
    /// `a < b` / `a > b` as JavaScript compares them (strings by UTF-16
    /// code units, numbers numerically, mixed as numbers).
    pub fn js_cmp(&self, o: &Value) -> Ordering {
        match (self, o) {
            (Value::Text(a), Value::Text(b)) => a.encode_utf16().cmp(b.encode_utf16()),
            _ => {
                let n = |v: &Value| match v {
                    Value::Num(n) => *n,
                    Value::Text(s) => s.trim().parse().unwrap_or(f64::NAN),
                };
                n(self).partial_cmp(&n(o)).unwrap_or(Ordering::Equal)
            }
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}
impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Num(n)
    }
}
impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::Num(n as f64)
    }
}
impl From<u32> for Value {
    fn from(n: u32) -> Self {
        Value::Num(n as f64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

/// The table's own state (React's `useState`s).
#[derive(Debug, Clone, PartialEq)]
pub struct TableState {
    pub sort: Option<(String, SortDir)>,
    pub query: String,
    /// 1-based; clamped to the page count when drawn.
    pub page: usize,
}

impl Default for TableState {
    fn default() -> Self {
        Self {
            sort: None,
            query: String::new(),
            page: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TableMsg {
    /// A sortable header was pressed.
    Sort(String),
    /// The filter text changed (back to page 1).
    Query(String),
    Page(usize),
}

impl TableState {
    pub fn update(&mut self, msg: TableMsg) {
        match msg {
            TableMsg::Sort(key) => {
                self.sort = match self.sort.take() {
                    Some((k, SortDir::Asc)) if k == key => Some((k, SortDir::Desc)),
                    Some((k, SortDir::Desc)) if k == key => None,
                    _ => Some((key, SortDir::Asc)),
                }
            }
            TableMsg::Query(q) => {
                self.query = q;
                self.page = 1;
            }
            TableMsg::Page(p) => self.page = p,
        }
    }
}

/// Indices of the rows whose accessor values contain `query`
/// (case-insensitive); all rows for a blank query.
pub fn filter_rows(values: &[Vec<Option<Value>>], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..values.len()).collect();
    }
    let q = query.to_lowercase();
    (0..values.len())
        .filter(|&i| {
            values[i]
                .iter()
                .flatten()
                .any(|v| v.to_js_string().to_lowercase().contains(&q))
        })
        .collect()
}

/// Stable-sort `idx` by `key(i)` in `dir`.
pub fn sort_rows(idx: &mut [usize], key: impl Fn(usize) -> Value, dir: SortDir) {
    idx.sort_by(|&a, &b| {
        let o = key(a).js_cmp(&key(b));
        if dir == SortDir::Asc { o } else { o.reverse() }
    });
}

/// `(page count, clamped page, row range)` for `n` rows.
pub fn paginate(n: usize, page: usize, size: usize) -> (usize, usize, std::ops::Range<usize>) {
    let size = size.max(1);
    let count = n.div_ceil(size);
    let page = page.min(count.max(1)).max(1);
    let lo = ((page - 1) * size).min(n);
    (count, page, lo..(lo + size).min(n))
}

/// The select-all toggle: unselect the page when all of it is selected,
/// else add the page's keys (keeping order, no duplicates).
pub fn toggle_all(selected: &[String], page_keys: &[String]) -> Vec<String> {
    let all = !page_keys.is_empty() && page_keys.iter().all(|k| selected.contains(k));
    if all {
        selected
            .iter()
            .filter(|k| !page_keys.contains(k))
            .cloned()
            .collect()
    } else {
        let mut out = selected.to_vec();
        for k in page_keys {
            if !out.contains(k) {
                out.push(k.clone());
            }
        }
        out
    }
}

/// One row's checkbox toggle.
pub fn toggle_one(selected: &[String], key: &str) -> Vec<String> {
    if selected.iter().any(|k| k == key) {
        selected.iter().filter(|k| *k != key).cloned().collect()
    } else {
        let mut out = selected.to_vec();
        out.push(key.to_string());
        out
    }
}

// ── Columns and actions ─────────────────────────────────────────────────

type Accessor<'a, T> = Box<dyn Fn(&T) -> Value + 'a>;
type Render<'a, T, M> = Box<dyn Fn(&T, &Ctx) -> El<'a, M> + 'a>;

/// `ColumnDef`.
pub struct Column<'a, T, M> {
    pub key: String,
    header: String,
    accessor: Option<Accessor<'a, T>>,
    render: Option<Render<'a, T, M>>,
    sortable: bool,
    align: Align,
    width: Option<f32>,
}

pub fn column_def<'a, T, M>(key: impl Into<String>, header: impl Into<String>) -> Column<'a, T, M> {
    Column {
        key: key.into(),
        header: header.into(),
        accessor: None,
        render: None,
        sortable: false,
        align: Align::Left,
        width: None,
    }
}

impl<'a, T, M> Column<'a, T, M> {
    /// The value used for sorting, filtering and the default cell text.
    pub fn accessor(mut self, f: impl Fn(&T) -> Value + 'a) -> Self {
        self.accessor = Some(Box::new(f));
        self
    }
    /// A custom cell (a Badge…); `cx` is the cell's backdrop.
    pub fn render(mut self, f: impl Fn(&T, &Ctx) -> El<'a, M> + 'a) -> Self {
        self.render = Some(Box::new(f));
        self
    }
    pub fn sortable(mut self) -> Self {
        self.sortable = true;
        self
    }
    pub fn align(mut self, a: Align) -> Self {
        self.align = a;
        self
    }
    /// A fixed column width (border-box px).
    pub fn width(mut self, px: f32) -> Self {
        self.width = Some(px);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActionTone {
    Accent,
    #[default]
    Default,
    Danger,
}

/// `RowAction`: with a `label` a small bordered button, else an icon button
/// revealed on row hover (unless `show_on_hover(false)`).
#[derive(Debug, Clone)]
pub struct RowAction<M> {
    pub icon: Icon,
    pub label: Option<String>,
    pub on_press: M,
    pub tone: ActionTone,
    pub show_on_hover: Option<bool>,
    pub title: Option<String>,
}

impl<M> RowAction<M> {
    pub fn new(icon: impl Into<Icon>, on_press: M) -> Self {
        Self {
            icon: icon.into(),
            label: None,
            on_press,
            tone: ActionTone::Default,
            show_on_hover: None,
            title: None,
        }
    }
    pub fn label(mut self, s: impl Into<String>) -> Self {
        self.label = Some(s.into());
        self
    }
    pub fn tone(mut self, t: ActionTone) -> Self {
        self.tone = t;
        self
    }
    pub fn show_on_hover(mut self, on: bool) -> Self {
        self.show_on_hover = Some(on);
        self
    }
    pub fn title(mut self, s: impl Into<String>) -> Self {
        self.title = Some(s.into());
        self
    }
}

/// An action button (`ActionButton`). `cx` is the row's resting backdrop;
/// translucent fills follow the row hover through [`group_progress`].
pub fn action_button<'a, M: Clone + 'a>(cx: &Ctx, a: RowAction<M>) -> El<'a, M> {
    let t = cx.t.clone();
    let rest = cx.over;
    let hovered = t.wash(0.06, rest);
    let tone = a.tone;
    let labeled = a.label.is_some();
    let reveal_on_hover = a.show_on_hover.unwrap_or(!labeled);
    // The row under the button, as the group hover has it now.
    let row_bg = move || {
        let k = group_progress();
        let (a, b) = (c(rest), c(hovered));
        let m = lerp(a, b, k);
        bato_theme::Rgb::new(
            (m.r * 255.0).round() as u8,
            (m.g * 255.0).round() as u8,
            (m.b * 255.0).round() as u8,
        )
    };
    let el: El<'a, M> = if let Some(text) = a.label {
        let content = row![icon(a.icon, 12.0), label(text, 12.0, 16.0).medium(),]
            .spacing(6)
            .align_y(Alignment::Center);
        surface(content)
            .padding(Padding::from([5.0, 11.0]))
            .on_press(a.on_press)
            .focus_ring(Ring::plain(cx))
            .style(move |st| {
                let p = &t.p;
                let row = row_bg();
                let h = st == Status::Hover;
                let (fg, fill, border) = match tone {
                    ActionTone::Accent => (
                        p.accent_bright,
                        bato_theme::blend(p.accent, if h { 0.2 } else { 0.1 }, row),
                        None,
                    ),
                    ActionTone::Default => (
                        p.subtext,
                        bato_theme::blend(p.text, if h { 0.08 } else { 0.04 }, row),
                        Some(p.line_strong),
                    ),
                    ActionTone::Danger => (
                        p.red,
                        bato_theme::blend(p.red, if h { 0.2 } else { 0.1 }, row),
                        None,
                    ),
                };
                let border = match border {
                    Some(ls) => ls.over(fill),
                    None => {
                        let base = if tone == ActionTone::Danger {
                            p.red
                        } else {
                            p.accent
                        };
                        bato_theme::blend(base, 0.2, fill)
                    }
                };
                BoxStyle::default()
                    .bg(c(fill))
                    .border(c(border))
                    .fg(c(fg))
                    .radius(8.0)
            })
            .into()
    } else {
        surface(icon(a.icon, 15.0))
            .padding(6)
            .on_press(a.on_press)
            .focus_ring(Ring::plain(cx))
            .style(move |st| {
                let p = &t.p;
                let h = st == Status::Hover;
                let row = row_bg();
                let (fg, fill) = match (tone, h) {
                    (ActionTone::Accent, false) => (p.accent_bright, None),
                    (ActionTone::Accent, true) => {
                        (p.accent_bright, Some(bato_theme::blend(p.accent, 0.1, row)))
                    }
                    (ActionTone::Default, false) => (p.muted, None),
                    (ActionTone::Default, true) => (p.text, None),
                    (ActionTone::Danger, false) => (p.muted, None),
                    (ActionTone::Danger, true) => (p.red, Some(bato_theme::blend(p.red, 0.1, row))),
                };
                let mut s = BoxStyle::default().fg(c(fg)).radius(8.0);
                if let Some(f) = fill {
                    s = s.bg(c(f));
                }
                s
            })
            .into()
    };
    let el = match a.title {
        Some(title) => tooltip(cx, el, title).into(),
        None => el,
    };
    if reveal_on_hover {
        reveal(el).into()
    } else {
        el
    }
}

// ── The builder ─────────────────────────────────────────────────────────

type OnMsg<'a, M> = Rc<dyn Fn(TableMsg) -> M + 'a>;
type RowFn<'a, T, R> = Box<dyn Fn(&T) -> R + 'a>;
type OnSelect<'a, M> = Box<dyn Fn(Vec<String>) -> M + 'a>;

pub struct DataTable<'a, T, M> {
    cx: Ctx,
    rows: &'a [T],
    columns: Vec<Column<'a, T, M>>,
    state: &'a TableState,
    on_msg: OnMsg<'a, M>,
    row_key: Option<RowFn<'a, T, String>>,
    filterable: bool,
    page_size: usize,
    selected: Option<(&'a [String], OnSelect<'a, M>)>,
    on_row_click: Option<RowFn<'a, T, M>>,
    row_actions: Option<RowFn<'a, T, Vec<RowAction<M>>>>,
    empty_message: String,
    filter_id: Option<widget::Id>,
}

pub fn data_table<'a, T, M: Clone + 'a>(
    cx: &Ctx,
    rows: &'a [T],
    columns: Vec<Column<'a, T, M>>,
    state: &'a TableState,
    on_msg: impl Fn(TableMsg) -> M + 'a,
) -> DataTable<'a, T, M> {
    DataTable {
        cx: cx.clone(),
        rows,
        columns,
        state,
        on_msg: Rc::new(on_msg),
        row_key: None,
        filterable: false,
        page_size: 10,
        selected: None,
        on_row_click: None,
        row_actions: None,
        empty_message: "No data".to_string(),
        filter_id: None,
    }
}

impl<'a, T, M: Clone + 'a> DataTable<'a, T, M> {
    pub fn row_key(mut self, f: impl Fn(&T) -> String + 'a) -> Self {
        self.row_key = Some(Box::new(f));
        self
    }
    /// The "Filter…" box above the table.
    pub fn filterable(mut self, on: bool) -> Self {
        self.filterable = on;
        self
    }
    pub fn page_size(mut self, n: usize) -> Self {
        self.page_size = n;
        self
    }
    /// Checkbox column (needs [`Self::row_key`]).
    pub fn selectable(
        mut self,
        selected: &'a [String],
        on_select: impl Fn(Vec<String>) -> M + 'a,
    ) -> Self {
        self.selected = Some((selected, Box::new(on_select)));
        self
    }
    pub fn on_row_click(mut self, f: impl Fn(&T) -> M + 'a) -> Self {
        self.on_row_click = Some(Box::new(f));
        self
    }
    pub fn row_actions(mut self, f: impl Fn(&T) -> Vec<RowAction<M>> + 'a) -> Self {
        self.row_actions = Some(Box::new(f));
        self
    }
    pub fn empty_message(mut self, s: impl Into<String>) -> Self {
        self.empty_message = s.into();
        self
    }
    /// The filter input's id (focus, self-tests).
    pub fn filter_id(mut self, id: impl Into<widget::Id>) -> Self {
        self.filter_id = Some(id.into());
        self
    }

    fn key(&self, i: usize) -> String {
        self.row_key
            .as_ref()
            .map_or_else(|| i.to_string(), |f| f(&self.rows[i]))
    }

    /// The visible rows: filtered, sorted, then the page; plus the totals.
    fn visible(&self) -> (Vec<usize>, usize, usize, usize) {
        let values: Vec<Vec<Option<Value>>> = self
            .rows
            .iter()
            .map(|r| {
                self.columns
                    .iter()
                    .map(|c| c.accessor.as_ref().map(|f| f(r)))
                    .collect()
            })
            .collect();
        let mut idx = filter_rows(&values, &self.state.query);
        if let Some((key, dir)) = &self.state.sort
            && let Some(ci) = self
                .columns
                .iter()
                .position(|c| &c.key == key && c.accessor.is_some())
        {
            sort_rows(
                &mut idx,
                |i| values[i][ci].clone().unwrap_or(Value::Num(f64::NAN)),
                *dir,
            );
        }
        let total = idx.len();
        let (count, page, range) = paginate(total, self.state.page, self.page_size);
        (idx[range].to_vec(), total, count, page)
    }
}

impl<'a, T, M: Clone + 'a> From<DataTable<'a, T, M>> for El<'a, M> {
    fn from(dt: DataTable<'a, T, M>) -> Self {
        let cx = dt.cx.on_card();
        let p = cx.p().clone();
        let (page_rows, total, count, page) = dt.visible();
        let on_msg = dt.on_msg.clone();

        let mut specs: Vec<ColSpec> = Vec::new();
        let mut header: Vec<El<'a, M>> = Vec::new();
        let page_keys: Vec<String> = page_rows.iter().map(|&i| dt.key(i)).collect();

        if let Some((selected, on_select)) = &dt.selected {
            specs.push(ColSpec {
                width: Some(40.0),
                pad_x: 12.0,
                align: Align::Left,
                stop: true,
            });
            let all = !page_keys.is_empty() && page_keys.iter().all(|k| selected.contains(k));
            let some = !all && page_keys.iter().any(|k| selected.contains(k));
            let msg = on_select(toggle_all(selected, &page_keys));
            header.push(
                checkbox(&cx.on(p.surface), all, move |_| msg.clone())
                    .indeterminate(some)
                    .into(),
            );
        }
        for col in &dt.columns {
            specs.push(ColSpec {
                width: col.width,
                pad_x: 16.0,
                align: col.align,
                stop: false,
            });
            // `uppercase` on the th; preflight resets `text-transform` on a
            // `<button>`, so sortable headers keep their case.
            let shown = if col.sortable {
                col.header.clone()
            } else {
                col.header.to_uppercase()
            };
            let text = label(shown, 12.0, 16.0).medium().tracking(0.05);
            header.push(if col.sortable {
                let active = dt.state.sort.as_ref().filter(|(k, _)| *k == col.key);
                let chevron: El<'a, M> = match active {
                    Some((_, SortDir::Asc)) => icon(Ico::ChevronUp, 13.0).into(),
                    Some((_, SortDir::Desc)) => icon(Ico::ChevronDown, 13.0).into(),
                    None => opacity(0.4, icon(Ico::ChevronsUpDown, 13.0)).into(),
                };
                let is_active = active.is_some();
                let t = cx.t.clone();
                surface(row![text, chevron].spacing(4).align_y(Alignment::Center))
                    .on_press(on_msg(TableMsg::Sort(col.key.clone())))
                    .focus_ring(Ring::plain(&cx))
                    .style(move |st| {
                        let p = &t.p;
                        let fg = if is_active {
                            p.accent_bright
                        } else if st == Status::Hover {
                            p.text
                        } else {
                            p.muted
                        };
                        BoxStyle::default().fg(c(fg))
                    })
                    .into()
            } else {
                text.color(c(p.muted)).into()
            });
        }
        if dt.row_actions.is_some() {
            specs.push(ColSpec {
                width: None,
                pad_x: 16.0,
                align: Align::Right,
                stop: true,
            });
            header.push(Space::new().into());
        }

        let mut grid = TableGrid::new(specs, header, &cx);
        for (n, &i) in page_rows.iter().enumerate() {
            let r = &dt.rows[i];
            let mut cells: Vec<El<'a, M>> = Vec::new();
            if let Some((selected, on_select)) = &dt.selected {
                let key = &page_keys[n];
                let msg = on_select(toggle_one(selected, key));
                cells.push(checkbox(&cx, selected.contains(key), move |_| msg.clone()).into());
            }
            for col in &dt.columns {
                cells.push(match (&col.render, &col.accessor) {
                    (Some(f), _) => f(r, &cx),
                    (None, Some(a)) => label(a(r).to_js_string(), 14.0, 20.0)
                        .color(c(p.text))
                        .into(),
                    (None, None) => Space::new().into(),
                });
            }
            if let Some(actions) = &dt.row_actions {
                let buttons: Vec<El<'a, M>> = actions(r)
                    .into_iter()
                    .map(|a| action_button(&cx, a))
                    .collect();
                cells.push(row(buttons).spacing(4).align_y(Alignment::Center).into());
            }
            grid = grid.row(cells, dt.on_row_click.as_ref().map(|f| f(r)));
        }

        let mut table = column![grid];
        if page_rows.is_empty() {
            table = table.push(empty_state(&cx, dt.empty_message.clone()));
        }
        let wrapper = rounded_clip(12.0, c(cx.over), table).border(c(p.border));

        let mut out = column![].spacing(12);
        if dt.filterable {
            let on = on_msg.clone();
            let mut input = text_control(&cx, "Filter…", &dt.state.query, move |s| {
                on(TableMsg::Query(s))
            })
            .left(icon(Ico::Search, 14.0).color(c(p.muted)))
            .width(320);
            if let Some(id) = dt.filter_id {
                input = input.id(id);
            }
            out = out.push(input);
        }
        out = out.push(wrapper);
        if total > 0 {
            let rows_text = format!("{total} {}", if total == 1 { "row" } else { "rows" });
            let mut footer = row![
                label(rows_text, 12.0, 16.0).color(c(p.muted)),
                Space::new().width(Length::Fill),
            ]
            .align_y(Alignment::Center);
            if count > 1 {
                let on = on_msg.clone();
                footer = footer.push(pagination(&cx, page, count, move |n| on(TableMsg::Page(n))));
            }
            out = out.push(footer);
        }
        out.into()
    }
}

// ── Group hover ─────────────────────────────────────────────────────────

thread_local! {
    /// The row being updated / drawn: (pointer over it, hover fill 0..1).
    static GROUP: Cell<(bool, f32)> = const { Cell::new((false, 0.0)) };
}

/// Whether the pointer is over the row being updated (`group-hover`).
pub fn group_hovered() -> bool {
    GROUP.with(|g| g.get().0)
}

/// The row hover transition (0..1) of the row being drawn.
pub fn group_progress() -> f32 {
    GROUP.with(|g| g.get().1)
}

fn with_group<R>(g: (bool, f32), f: impl FnOnce() -> R) -> R {
    let prev = GROUP.with(|c| c.replace(g));
    let r = f();
    GROUP.with(|c| c.set(prev));
    r
}

/// `opacity-0 group-hover:opacity-100 focus:opacity-100 transition-all`:
/// the content shows while the enclosing [`TableGrid`] row is hovered or
/// while focus is inside it, fading over 150 ms. Still interactive while
/// hidden, like CSS.
pub struct Reveal<'a, M> {
    content: El<'a, M>,
}

pub fn reveal<'a, M>(content: impl Into<El<'a, M>>) -> Reveal<'a, M> {
    Reveal {
        content: content.into(),
    }
}

struct RevealState {
    t: Tween,
    focused: bool,
}

/// Finds whether any focusable inside is focused.
struct FocusProbe(bool);

impl Operation for FocusProbe {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
    fn focusable(
        &mut self,
        _id: Option<&widget::Id>,
        _bounds: Rectangle,
        state: &mut dyn widget::operation::Focusable,
    ) {
        self.0 |= state.is_focused();
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for Reveal<'a, M> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<RevealState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(RevealState {
            t: Tween::settled(0.0),
            focused: false,
        })
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
        let child = layout.children().next().unwrap();
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            child,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        let mut probe = FocusProbe(false);
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], child, renderer, &mut probe);
        let st = tree.state.downcast_mut::<RevealState>();
        st.focused = probe.0;
        let target = if group_hovered() || st.focused {
            1.0
        } else {
            0.0
        };
        let now = Instant::now();
        if st.t.to != target {
            st.t.retarget(target, now);
            st.t.duration = FADE;
            st.t.easing = ease_standard;
            shell.request_redraw();
        } else if let Event::Window(window::Event::RedrawRequested(at)) = event
            && st.t.is_running(*at)
        {
            shell.request_redraw();
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
        let a = tree
            .state
            .downcast_ref::<RevealState>()
            .t
            .value(Instant::now());
        if a <= 0.0 {
            return;
        }
        with_opacity(a, || {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout.children().next().unwrap(),
                cursor,
                viewport,
            );
        });
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: 'a> From<Reveal<'a, M>> for El<'a, M> {
    fn from(r: Reveal<'a, M>) -> Self {
        Element::new(r)
    }
}

// ── TableGrid ───────────────────────────────────────────────────────────

/// One column of a [`TableGrid`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColSpec {
    /// A fixed border-box width; `None` = auto.
    pub width: Option<f32>,
    /// The cell's horizontal padding (`px-3` = 12, `px-4` = 16).
    pub pad_x: f32,
    pub align: Align,
    /// Clicks in this column don't reach the row (`stopPropagation`).
    pub stop: bool,
}

/// The cells' vertical padding (`py-3`).
pub const PAD_Y: f32 = 12.0;

/// Column widths for a table `avail` wide (Chromium's auto layout): fixed
/// columns keep their width; auto columns get their max-content width plus
/// a share of the excess proportional to it.
pub fn column_widths(specs: &[ColSpec], max_content: &[f32], avail: f32) -> Vec<f32> {
    let fixed: f32 = specs.iter().filter_map(|s| s.width).sum();
    let auto_sum: f32 = specs
        .iter()
        .zip(max_content)
        .filter(|(s, _)| s.width.is_none())
        .map(|(_, m)| *m)
        .sum();
    let excess = (avail - fixed - auto_sum).max(0.0);
    specs
        .iter()
        .zip(max_content)
        .map(|(s, m)| match s.width {
            Some(w) => w,
            None if auto_sum > 0.0 => m + excess * m / auto_sum,
            None => *m,
        })
        .collect()
}

struct GridRow<M> {
    on_press: Option<M>,
}

/// The table body: a header row on `surface`, then body rows with a hover
/// fill, `border` lines between rows, cells laid out by [`column_widths`].
pub struct TableGrid<'a, M> {
    specs: Vec<ColSpec>,
    cells: Vec<El<'a, M>>,
    rows: Vec<GridRow<M>>,
    header_bg: Color,
    line: Color,
    hover: Color,
}

impl<'a, M: Clone + 'a> TableGrid<'a, M> {
    pub fn new(specs: Vec<ColSpec>, header: Vec<El<'a, M>>, cx: &Ctx) -> Self {
        assert_eq!(specs.len(), header.len());
        let p = cx.p();
        Self {
            specs,
            cells: header,
            rows: vec![GridRow { on_press: None }],
            header_bg: c(p.surface),
            line: c(p.border),
            hover: c(cx.wash(0.06)),
        }
    }
    pub fn row(mut self, cells: Vec<El<'a, M>>, on_press: Option<M>) -> Self {
        assert_eq!(cells.len(), self.specs.len());
        self.cells.extend(cells);
        self.rows.push(GridRow { on_press });
        self
    }
}

#[derive(Default)]
struct GridState {
    /// Per row: (y, height without the bottom line).
    rows: Vec<(f32, f32)>,
    /// Column x positions and widths.
    cols: Vec<(f32, f32)>,
    hovered: Option<usize>,
    fades: Vec<Tween>,
    pressed: Option<usize>,
}

impl GridState {
    fn row_at(&self, p: Point, b: Rectangle) -> Option<usize> {
        if !b.contains(p) {
            return None;
        }
        let y = p.y - b.y;
        self.rows
            .iter()
            .position(|(ry, h)| y >= *ry && y < ry + h + 1.0)
    }
    fn col_at(&self, x: f32) -> Option<usize> {
        self.cols.iter().position(|(cx, w)| x >= *cx && x < cx + w)
    }
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for TableGrid<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<GridState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(GridState::default())
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
        let avail = limits.max().width;
        let ncol = self.specs.len();
        let loose = layout::Limits::new(Size::ZERO, Size::new(f32::INFINITY, f32::INFINITY));
        // Max-content widths.
        let mut maxw = vec![0.0f32; ncol];
        for (i, (cell, t)) in self
            .cells
            .iter_mut()
            .zip(tree.children.iter_mut())
            .enumerate()
        {
            let col = i % ncol;
            let w = cell
                .as_widget_mut()
                .layout(t, renderer, &loose)
                .size()
                .width;
            maxw[col] = maxw[col].max(w + 2.0 * self.specs[col].pad_x);
        }
        let widths = column_widths(&self.specs, &maxw, avail);
        let mut xs = Vec::with_capacity(ncol);
        let mut x = 0.0;
        for w in &widths {
            xs.push((x, *w));
            x += w;
        }
        // Lay out at the final widths, then place.
        let mut nodes: Vec<layout::Node> = Vec::with_capacity(self.cells.len());
        for (i, (cell, t)) in self
            .cells
            .iter_mut()
            .zip(tree.children.iter_mut())
            .enumerate()
        {
            let col = i % ncol;
            let inner = (widths[col] - 2.0 * self.specs[col].pad_x).max(0.0);
            let l = layout::Limits::new(Size::ZERO, Size::new(inner, f32::INFINITY));
            nodes.push(cell.as_widget_mut().layout(t, renderer, &l));
        }
        let nrows = self.rows.len();
        let mut rows = Vec::with_capacity(nrows);
        let mut y = 0.0;
        for r in 0..nrows {
            let h = nodes[r * ncol..(r + 1) * ncol]
                .iter()
                .map(|n| n.size().height)
                .fold(0.0, f32::max)
                + 2.0 * PAD_Y;
            for col in 0..ncol {
                let n = &mut nodes[r * ncol + col];
                let s = n.size();
                let spec = &self.specs[col];
                let (cx, cw) = xs[col];
                let inner = cw - 2.0 * spec.pad_x;
                let dx = match spec.align {
                    Align::Left => 0.0,
                    Align::Center => (inner - s.width) / 2.0,
                    Align::Right => inner - s.width,
                };
                n.move_to_mut(Point::new(cx + spec.pad_x + dx, y + (h - s.height) / 2.0));
            }
            rows.push((y, h));
            y += h + if r + 1 < nrows { 1.0 } else { 0.0 };
        }
        let st = tree.state.downcast_mut::<GridState>();
        st.rows = rows;
        st.cols = xs;
        st.fades.resize(nrows, Tween::settled(0.0));
        if st.hovered.is_some_and(|h| h >= nrows) {
            st.hovered = None;
        }
        layout::Node::with_children(Size::new(avail, y), nodes)
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
        let b = layout.bounds();
        let ncol = self.specs.len();
        // Row hover first, so the cells see this event's group state.
        {
            let st = tree.state.downcast_mut::<GridState>();
            let over = cursor
                .position()
                .filter(|p| viewport.contains(*p))
                .and_then(|p| st.row_at(p, b))
                .filter(|r| *r > 0);
            if over != st.hovered
                && matches!(
                    event,
                    Event::Mouse(_) | Event::Window(window::Event::RedrawRequested(_))
                )
            {
                let now = Instant::now();
                for (r, t) in st.fades.iter_mut().enumerate() {
                    let to = if Some(r) == over { 1.0 } else { 0.0 };
                    if t.to != to {
                        t.retarget(to, now);
                        t.duration = FADE;
                        t.easing = ease_standard;
                    }
                }
                st.hovered = over;
                shell.request_redraw();
            }
        }
        let hovered = tree.state.downcast_ref::<GridState>().hovered;
        let mut children = tree.children.iter_mut();
        let mut lays = layout.children();
        for (r, _) in self.rows.iter().enumerate() {
            with_group((hovered == Some(r), 0.0), || {
                for cell in &mut self.cells[r * ncol..(r + 1) * ncol] {
                    let (t, l) = (children.next().unwrap(), lays.next().unwrap());
                    cell.as_widget_mut()
                        .update(t, event, l, cursor, renderer, clipboard, shell, viewport);
                }
            });
        }
        let st = tree.state.downcast_mut::<GridState>();
        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                if st.fades.iter().any(|t| t.is_running(*now)) {
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if !shell.is_event_captured() =>
            {
                st.pressed = cursor.position().and_then(|p| {
                    let r = st.row_at(p, b)?;
                    let col = st.col_at(p.x - b.x)?;
                    (self.rows[r].on_press.is_some() && !self.specs[col].stop).then_some(r)
                });
                if st.pressed.is_some() {
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if let Some(r) = st.pressed.take()
                    && cursor.position().and_then(|p| st.row_at(p, b)) == Some(r)
                    && let Some(m) = self.rows[r].on_press.clone()
                {
                    shell.publish(m);
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
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let inner = self
            .cells
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((c, t), l)| {
                c.as_widget()
                    .mouse_interaction(t, l, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default();
        if inner != mouse::Interaction::None {
            return inner;
        }
        let st = tree.state.downcast_ref::<GridState>();
        match cursor
            .position()
            .and_then(|p| st.row_at(p, layout.bounds()))
        {
            Some(r) if self.rows[r].on_press.is_some() => mouse::Interaction::Pointer,
            _ => mouse::Interaction::None,
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
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
        let st = tree.state.downcast_ref::<GridState>();
        let b = layout.bounds();
        let ncol = self.specs.len();
        let now = Instant::now();
        let quad = |r: &mut iced::Renderer, y: f32, h: f32, color: Color| {
            r.fill_quad(
                Quad {
                    bounds: Rectangle {
                        x: b.x,
                        y: b.y + y,
                        width: b.width,
                        height: h,
                    },
                    ..Quad::default()
                },
                fade(color),
            );
        };
        let mut lays = layout.children();
        let mut trees = tree.children.iter();
        for (r, &(y, h)) in st.rows.iter().enumerate() {
            let k = st.fades.get(r).map_or(0.0, |t| t.value(now));
            if r == 0 {
                quad(renderer, y, h, self.header_bg);
            } else if k > 0.0 {
                quad(renderer, y, h, Color { a: k, ..self.hover });
            }
            if r + 1 < st.rows.len() {
                quad(renderer, y + h, 1.0, self.line);
            }
            with_group((st.hovered == Some(r), k), || {
                for cell in &self.cells[r * ncol..(r + 1) * ncol] {
                    let (t, l) = (trees.next().unwrap(), lays.next().unwrap());
                    cell.as_widget()
                        .draw(t, renderer, theme, style, l, cursor, viewport);
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
        translation: iced::Vector,
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

impl<'a, M: Clone + 'a> From<TableGrid<'a, M>> for El<'a, M> {
    fn from(g: TableGrid<'a, M>) -> Self {
        Element::new(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_cycles_asc_desc_none_and_query_resets_the_page() {
        let mut s = TableState {
            page: 3,
            ..TableState::default()
        };
        s.update(TableMsg::Sort("name".into()));
        assert_eq!(s.sort, Some(("name".into(), SortDir::Asc)));
        s.update(TableMsg::Sort("name".into()));
        assert_eq!(s.sort, Some(("name".into(), SortDir::Desc)));
        s.update(TableMsg::Sort("name".into()));
        assert_eq!(s.sort, None);
        s.update(TableMsg::Sort("name".into()));
        s.update(TableMsg::Sort("age".into()));
        assert_eq!(s.sort, Some(("age".into(), SortDir::Asc)));
        s.update(TableMsg::Query("ad".into()));
        assert_eq!((s.page, s.query.as_str()), (1, "ad"));
    }

    #[test]
    fn filter_is_case_insensitive_across_accessors() {
        let v = |n: &str, a: f64| vec![Some(Value::from(n)), Some(Value::Num(a)), None];
        let rows = vec![
            v("Ada Lovelace 1", 24.0),
            v("Alan Turing 2", 25.0),
            v("Ken T 7", 30.0),
        ];
        assert_eq!(filter_rows(&rows, "  "), [0, 1, 2]);
        assert_eq!(filter_rows(&rows, "a"), [0, 1]);
        assert_eq!(filter_rows(&rows, "TURING"), [1]);
        assert_eq!(filter_rows(&rows, "30"), [2]);
        // The query is not trimmed for matching, only for the blank test.
        assert_eq!(filter_rows(&rows, "g "), [1]);
    }

    #[test]
    fn sorting_is_stable_and_js_ordered() {
        let vals = [
            Value::from("b"),
            Value::from("B"),
            Value::from("a"),
            Value::from("b"),
        ];
        let mut idx = vec![0, 1, 2, 3];
        sort_rows(&mut idx, |i| vals[i].clone(), SortDir::Asc);
        assert_eq!(idx, [1, 2, 0, 3]); // "B" < "a" < "b" (code units), stable
        sort_rows(&mut idx, |i| vals[i].clone(), SortDir::Desc);
        assert_eq!(idx, [0, 3, 2, 1]);
        let nums = [Value::Num(10.0), Value::Num(9.0)];
        let mut idx = vec![0, 1];
        sort_rows(&mut idx, |i| nums[i].clone(), SortDir::Asc);
        assert_eq!(idx, [1, 0]);
    }

    #[test]
    fn pages_clamp() {
        assert_eq!(paginate(23, 1, 6), (4, 1, 0..6));
        assert_eq!(paginate(23, 4, 6), (4, 4, 18..23));
        assert_eq!(paginate(6, 4, 6), (1, 1, 0..6));
        assert_eq!(paginate(0, 2, 6), (0, 1, 0..0));
    }

    #[test]
    fn selection_toggles() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(toggle_all(&s(&["9"]), &s(&["1", "2"])), s(&["9", "1", "2"]));
        assert_eq!(toggle_all(&s(&["1", "9", "2"]), &s(&["1", "2"])), s(&["9"]));
        assert_eq!(toggle_all(&s(&["1"]), &s(&["1", "2"])), s(&["1", "2"]));
        assert_eq!(toggle_one(&s(&["1", "2"]), "1"), s(&["2"]));
        assert_eq!(toggle_one(&s(&["2"]), "1"), s(&["2", "1"]));
    }

    #[test]
    fn auto_layout_shares_the_excess_by_max_content() {
        let spec = |w| ColSpec {
            width: w,
            pad_x: 16.0,
            align: Align::Left,
            stop: false,
        };
        let w = column_widths(
            &[spec(Some(40.0)), spec(None), spec(None)],
            &[40.0, 100.0, 300.0],
            840.0,
        );
        assert_eq!(w, [40.0, 200.0, 600.0]);
        // No excess: max-content.
        let w = column_widths(&[spec(None), spec(None)], &[100.0, 300.0], 300.0);
        assert_eq!(w, [100.0, 300.0]);
    }

    #[test]
    fn js_values() {
        assert_eq!(Value::Num(24.0).to_js_string(), "24");
        assert_eq!(Value::from("x").js_cmp(&Value::from("y")), Ordering::Less);
    }
}
