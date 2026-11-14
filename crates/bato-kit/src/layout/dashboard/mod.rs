//! `Dashboard` (`layout/Dashboard.tsx`): an editable widget grid.
//!
//! Widgets live on a `cols`-column grid (`rowHeight` px rows, 10 px gaps)
//! as [`DashboardItem`]s `{x, y, w, h}` in cells. The toolbar has the
//! Snap / Free [`segmented`] control, and in edit mode the "Add widget"
//! [`dropdown`] (aligned right) before the Edit / Done button. In edit mode
//! the grid shows a dashed outline; each widget drags by its header
//! (`cursor-move`) and resizes from its bottom-right 16 px corner
//! (`cursor-se-resize`), cell by cell, live; the active widget has an
//! accent border, `shadow-app-lg` and sits on top. On release the layout is
//! committed: compacted upwards in Snap mode ([`compact`]), as dropped in
//! Free mode (overlap allowed). Widgets move to new places with a 150 ms
//! transition (not the dragged one). The header's × removes a widget; "Add
//! widget" appends one at `x 0` under the lowest. Every commit is persisted
//! in the kit's [`UiState`] when the dashboard has a store (React's
//! `storageKey`), and reported by [`Dashboard::update`] (`onChange`).
//!
//! React has no keyboard moving or resizing: the toolbar controls and the ×
//! buttons are Tab stops, nothing more. Mouse and touch drive gestures (a
//! finger on a header or corner is held from the press, so the page does
//! not scroll under it).
//!
//! ```ignore
//! // state
//! dash: Dashboard::new(defaults()).row_height(56.0).store(kv, "demo-dashboard"),
//! // update
//! Msg::Dash(m) => { state.dash.update(m); }
//! // view
//! dashboard(&cx, &state.dash, &types, |item| body(item), Msg::Dash)
//! ```

mod board;
pub mod grid;

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use iced::widget::{column, container, row};
use iced::{Alignment, Color, Element, Length, Padding};

pub use grid::{
    COLS, DashboardItem, GAP, Handle, Metrics, ROW_HEIGHT, apply, cells, clamp, collides, compact,
    content_height, pixel_height, rows,
};

use crate::button::{Variant, button};
use crate::ctx::Ctx;
use crate::icon::{Ico, icon};
use crate::kv::UiState;
use crate::label::label;
use crate::layout::hline;
use crate::menu::action;
use crate::overlay::dropdown::dropdown;
use crate::scroller::scroll_area;
use crate::segmented::{Segment, segmented};
use crate::style::{c, shadows};
use crate::surface::{BoxStyle, Sh, Status, surface};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// Snap: compact upwards on release. Free: leave widgets where dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Snap,
    Free,
}

/// `DashboardWidgetType` minus `render` (given to [`dashboard`] once).
#[derive(Debug, Clone)]
pub struct WidgetType {
    pub kind: String,
    pub label: String,
    pub icon: Option<Ico>,
    /// `defaultSize` (w, h) in cells.
    pub size: (i32, i32),
}

impl WidgetType {
    pub fn new(kind: impl Into<String>, label: impl Into<String>, w: i32, h: i32) -> Self {
        Self {
            kind: kind.into(),
            label: label.into(),
            icon: None,
            size: (w, h),
        }
    }
    pub fn icon(mut self, ico: Ico) -> Self {
        self.icon = Some(ico);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DashboardMsg {
    Mode(Mode),
    ToggleEdit,
    /// Add a widget of this kind and default size.
    Add {
        kind: String,
        w: i32,
        h: i32,
    },
    Remove(String),
    /// A press on a widget's header or corner (`startGesture`).
    Begin(String, Handle),
    /// The pointer is this many cells from the press point.
    Step(i32, i32),
    /// Released: commit.
    End,
}

#[derive(Debug, Clone)]
struct Gesture {
    id: String,
    handle: Handle,
    /// The layout when the gesture began (React's closure `items`).
    snapshot: Vec<DashboardItem>,
    latest: Vec<DashboardItem>,
}

/// The dashboard's state (React's `useState`s): the layout, edit and mode.
#[derive(Debug, Clone)]
pub struct Dashboard {
    pub items: Vec<DashboardItem>,
    pub edit: bool,
    pub mode: Mode,
    pub cols: i32,
    pub row_height: f32,
    gesture: Option<Gesture>,
    store: Option<(UiState, String)>,
}

static UID: AtomicU32 = AtomicU32::new(0);

/// `nextId()`: `w` + the time in ms in base 36 + a counter.
pub fn next_id() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    format!("w{}{}", base36(ms), UID.fetch_add(1, Ordering::Relaxed))
}

fn base36(mut n: u64) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(D[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

impl Dashboard {
    /// `defaultItems`, 12 columns, 64 px rows, Snap, not editing.
    pub fn new(defaults: Vec<DashboardItem>) -> Self {
        Self {
            items: defaults,
            edit: false,
            mode: Mode::Snap,
            cols: COLS,
            row_height: ROW_HEIGHT,
            gesture: None,
            store: None,
        }
    }
    pub fn cols(mut self, n: i32) -> Self {
        self.cols = n;
        self
    }
    pub fn row_height(mut self, px: f32) -> Self {
        self.row_height = px;
        self
    }
    /// `storageKey`: start from the layout stored at `key` if there is one,
    /// and write every commit there.
    pub fn store(mut self, kv: UiState, key: impl Into<String>) -> Self {
        let key = key.into();
        if let Some(items) = kv.get::<Vec<DashboardItem>>(&key) {
            self.items = items;
        }
        self.store = Some((kv, key));
        self
    }

    /// The widget being dragged or resized (`activeId`).
    pub fn active(&self) -> Option<&str> {
        self.gesture.as_ref().map(|g| g.id.as_str())
    }

    /// Apply a message. Returns the new layout when it was committed
    /// (React's `onChange`).
    pub fn update(&mut self, msg: DashboardMsg) -> Option<Vec<DashboardItem>> {
        match msg {
            DashboardMsg::Mode(m) => self.mode = m,
            DashboardMsg::ToggleEdit => self.edit = !self.edit,
            DashboardMsg::Add { kind, w, h } => {
                let y = rows(&self.items);
                let mut next = self.items.clone();
                next.push(DashboardItem::new(next_id(), kind, 0, y, w, h));
                return Some(self.commit(next));
            }
            DashboardMsg::Remove(id) => {
                let next = self
                    .items
                    .iter()
                    .filter(|it| it.id != id)
                    .cloned()
                    .collect();
                return Some(self.commit(next));
            }
            DashboardMsg::Begin(id, handle) => {
                if !self.edit || !self.items.iter().any(|it| it.id == id) {
                    return None;
                }
                self.gesture = Some(Gesture {
                    id,
                    handle,
                    snapshot: self.items.clone(),
                    latest: self.items.clone(),
                });
            }
            DashboardMsg::Step(dx, dy) => {
                let cols = self.cols;
                if let Some(g) = self.gesture.as_mut()
                    && let Some(start) = g.snapshot.iter().find(|it| it.id == g.id)
                {
                    let moved = apply(start, g.handle, dx, dy, cols);
                    g.latest = g
                        .snapshot
                        .iter()
                        .map(|it| {
                            if it.id == g.id {
                                moved.clone()
                            } else {
                                it.clone()
                            }
                        })
                        .collect();
                    self.items = g.latest.clone();
                }
            }
            DashboardMsg::End => {
                let g = self.gesture.take()?;
                let next = match self.mode {
                    Mode::Snap => compact(&g.latest),
                    Mode::Free => g.latest,
                };
                return Some(self.commit(next));
            }
        }
        None
    }

    fn commit(&mut self, next: Vec<DashboardItem>) -> Vec<DashboardItem> {
        self.items = next;
        if let Some((kv, key)) = self.store.as_mut()
            && let Err(e) = kv.set(key, &self.items)
        {
            eprintln!("ui-state: {e}");
        }
        self.items.clone()
    }
}

/// The colours the grid paints itself.
#[derive(Debug, Clone)]
struct Frame {
    card: Color,
    border: Color,
    accent: Color,
    muted: Color,
    edit_bg: Color,
    shadow: Vec<Sh>,
}

/// The header's ×: `text-app-muted hover:text-app-red`, a 13 px icon.
fn remove_button<'a>(cx: &Ctx, id: &str) -> El<'a, DashboardMsg> {
    let p = cx.p();
    let (muted, red) = (c(p.muted), c(p.red));
    surface(icon(Ico::X, 13.0))
        .style(move |s| BoxStyle::default().fg(if s == Status::Hover { red } else { muted }))
        .transition_ms(0)
        .on_press(DashboardMsg::Remove(id.to_string()))
        .focus_ring(crate::date::ua_ring())
        .into()
}

/// The dashboard on `cx`'s backdrop: toolbar, then the grid. `render`
/// draws a widget's body (inside `p-3`, on the card); `on` wraps the
/// dashboard's messages.
pub fn dashboard<'a, M: 'a>(
    cx: &Ctx,
    state: &'a Dashboard,
    types: &[WidgetType],
    render: impl Fn(&DashboardItem) -> El<'a, M>,
    on: fn(DashboardMsg) -> M,
) -> El<'a, M> {
    let p = cx.p();
    let on_card = cx.on_card();

    // Toolbar: `flex items-center justify-between mb-3`.
    let mode = segmented(
        cx,
        state.mode,
        [
            Segment::new(Mode::Snap, "Snap"),
            Segment::new(Mode::Free, "Free"),
        ],
        DashboardMsg::Mode,
    )
    .sm();
    let mut right = row![].spacing(8).align_y(Alignment::Center);
    if state.edit {
        let items = types
            .iter()
            .map(|t| {
                let a = action(
                    t.label.clone(),
                    DashboardMsg::Add {
                        kind: t.kind.clone(),
                        w: t.size.0,
                        h: t.size.1,
                    },
                );
                match t.icon {
                    Some(i) => a.icon(i).into(),
                    None => a.into(),
                }
            })
            .collect();
        right = right.push(
            dropdown(cx, "Add widget", items)
                .icon(Ico::Plus)
                .align_right(true),
        );
    }
    let (ico, text) = if state.edit {
        (Ico::Check, "Done")
    } else {
        (Ico::Pencil, "Edit")
    };
    right = right.push(
        button(cx)
            .variant(if state.edit {
                Variant::Primary
            } else {
                Variant::Ghost
            })
            .icon_sized(ico, 14.0)
            .label(text)
            .on_press(DashboardMsg::ToggleEdit),
    );
    let toolbar: El<'a, DashboardMsg> =
        row![mode, iced::widget::Space::new().width(Length::Fill), right]
            .align_y(Alignment::Center)
            .into();

    // The widgets.
    let cells = state
        .items
        .iter()
        .map(|it| {
            let ty = types.iter().find(|t| t.kind == it.kind);
            let title = ty.map_or(it.kind.clone(), |t| t.label.clone());
            let mut head = row![
                label(title, 12.0, 16.0)
                    .medium()
                    .color(c(p.subtext))
                    .ellipsis()
                    .width(Length::Fill)
            ]
            .align_y(Alignment::Center);
            if state.edit {
                head = head.push(remove_button(&on_card, &it.id));
            }
            let head: El<'a, M> = Element::from(
                container(head)
                    .padding(Padding::from([6.0, 12.0]))
                    .height(28)
                    .width(Length::Fill),
            )
            .map(on);
            let body = scroll_area(
                &on_card,
                container(render(it)).padding(12).width(Length::Fill),
            )
            .both()
            .height(Length::Fill);
            column![head, hline::<DashboardMsg>(c(p.border)).map(on), body].into()
        })
        .collect();

    let frame = Frame {
        card: c(p.card),
        border: c(p.border),
        accent: c(p.accent),
        muted: c(p.muted),
        edit_bg: c(cx.blend(p.bg, 0.3)),
        shadow: shadows(&p.shadow_lg),
    };
    let grid = board::Board {
        frame,
        metrics_cols: state.cols,
        row_height: state.row_height,
        items: state.items.clone(),
        cells,
        edit: state.edit,
        active: state.active().map(str::to_string),
        on,
    };
    column![toolbar.map(on), crate::layout::vgap(12.0), El::from(grid)]
        .width(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn showcase() -> Vec<DashboardItem> {
        vec![
            DashboardItem::new("d1", "stat", 0, 0, 3, 2),
            DashboardItem::new("d2", "chart", 3, 0, 6, 4),
            DashboardItem::new("d3", "list", 9, 0, 3, 4),
            DashboardItem::new("d4", "stat", 0, 2, 3, 2),
        ]
    }

    #[test]
    fn gestures_need_edit_mode() {
        let mut d = Dashboard::new(showcase());
        d.update(DashboardMsg::Begin("d1".into(), Handle::Move));
        assert_eq!(d.active(), None);
        d.update(DashboardMsg::ToggleEdit);
        d.update(DashboardMsg::Begin("d1".into(), Handle::Move));
        assert_eq!(d.active(), Some("d1"));
        d.update(DashboardMsg::Begin("nope".into(), Handle::Move));
        assert_eq!(d.active(), Some("d1"), "an unknown id starts nothing");
    }

    #[test]
    fn steps_apply_to_the_snapshot_then_snap_compacts() {
        let mut d = Dashboard::new(showcase());
        d.update(DashboardMsg::ToggleEdit);
        d.update(DashboardMsg::Begin("d4".into(), Handle::Move));
        d.update(DashboardMsg::Step(1, 1));
        d.update(DashboardMsg::Step(1, 3));
        // Live: from the start position, not cumulative; no compaction.
        let d4 = d.items.iter().find(|i| i.id == "d4").unwrap();
        assert_eq!((d4.x, d4.y), (1, 5));
        let out = d.update(DashboardMsg::End).unwrap();
        assert_eq!(d.active(), None);
        let d4 = out.iter().find(|i| i.id == "d4").unwrap();
        // x 1..4 reaches into the chart's column 3: it floats up to row 4.
        assert_eq!((d4.x, d4.y), (1, 4), "floated up under the chart");
        assert_eq!(d.update(DashboardMsg::End), None, "no gesture, no commit");
    }

    #[test]
    fn free_mode_keeps_the_drop() {
        let mut d = Dashboard::new(showcase());
        d.update(DashboardMsg::ToggleEdit);
        d.update(DashboardMsg::Mode(Mode::Free));
        d.update(DashboardMsg::Begin("d1".into(), Handle::Move));
        d.update(DashboardMsg::Step(2, 5));
        let out = d.update(DashboardMsg::End).unwrap();
        let d1 = out.iter().find(|i| i.id == "d1").unwrap();
        assert_eq!((d1.x, d1.y), (2, 5));
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn resize_then_compact() {
        let mut d = Dashboard::new(showcase());
        d.update(DashboardMsg::ToggleEdit);
        d.update(DashboardMsg::Begin("d1".into(), Handle::Resize));
        d.update(DashboardMsg::Step(1, 1));
        let out = d.update(DashboardMsg::End).unwrap();
        // d1 is 4 × 3 now and over the chart's first column; d1 sorts
        // first ((0,0) before (3,0)), so the chart goes under it.
        let get = |id: &str| out.iter().find(|i| i.id == id).unwrap().clone();
        assert_eq!((get("d1").w, get("d1").h), (4, 3));
        assert_eq!(get("d2").y, 3, "the chart goes under the wider stat");
        assert_eq!(get("d4").y, 3);
    }

    #[test]
    fn add_and_remove_commit_without_compacting() {
        let mut d = Dashboard::new(showcase());
        let out = d
            .update(DashboardMsg::Add {
                kind: "list".into(),
                w: 3,
                h: 4,
            })
            .unwrap();
        let new = out.last().unwrap();
        assert_eq!((new.x, new.y, new.w, new.h), (0, 4, 3, 4));
        assert!(new.id.starts_with('w'));
        let out = d.update(DashboardMsg::Remove("d1".into())).unwrap();
        assert_eq!(out.len(), 4);
        assert_eq!(
            out.iter().find(|i| i.id == "d4").unwrap().y,
            2,
            "nothing floats up after a remove"
        );
        let a = next_id();
        assert_ne!(a, next_id());
    }

    #[test]
    fn layout_persists_on_commit() {
        let dir = std::env::temp_dir().join(format!("bato-kit-dash-{}", std::process::id()));
        let file = dir.join("ui-state.json");
        let _ = std::fs::remove_dir_all(&dir);
        let mut d = Dashboard::new(showcase()).store(UiState::at(&file), "demo-dashboard");
        d.update(DashboardMsg::ToggleEdit);
        assert!(!file.exists(), "edit mode is not persisted");
        d.update(DashboardMsg::Remove("d2".into()));
        let again = Dashboard::new(showcase()).store(UiState::at(&file), "demo-dashboard");
        assert_eq!(again.items.len(), 3);
        assert!(!again.edit);
        // A corrupt value falls back to the defaults.
        let mut kv = UiState::at(&file);
        kv.set("demo-dashboard", "nope").unwrap();
        let bad = Dashboard::new(showcase()).store(UiState::at(&file), "demo-dashboard");
        assert_eq!(bad.items, showcase());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn base36_like_js() {
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
        assert_eq!(base36(1_763_100_000_000), "mhyg7uo0");
    }
}
