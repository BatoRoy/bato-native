//! `dashboard/dashboard`: the Dashboard tab (`DashboardSection`). A line of
//! help text, then the Dashboard: 12 columns, 56 px rows, three widget
//! types (Stat with a 160 × 32 area Sparkline, Chart with a 150 px
//! LineChart, List), four default widgets. The layout persists in the
//! gallery's UI-state store under `demo-dashboard` (React's
//! `storageKey('demo-dashboard')`).
//!
//! The data is the fixed "Visits" series of the Charts card
//! (`20 + round(30 sin(i/2) + 2i)`), so nothing depends on the clock.

use bato_kit::data::{Series, line_chart, sparkline};
use bato_kit::iced::widget::{column, row};
use bato_kit::kv::UiState;
use bato_kit::layout::dashboard::{
    Dashboard, DashboardItem, DashboardMsg, WidgetType, content_height, dashboard,
};
use bato_kit::layout::rounded_clip;
use bato_kit::prelude::*;

use crate::showcase::Effect;
use crate::showcase::data::charts::visits;
use crate::showcase::layout::resizable::SLUG;

/// The UI-state key.
pub const KEY: &str = "demo-dashboard";
/// The dashboard's container, for the self-test.
pub const BOX: &str = "dashboard/dashboard/box";

pub fn defaults() -> Vec<DashboardItem> {
    vec![
        DashboardItem::new("d1", "stat", 0, 0, 3, 2),
        DashboardItem::new("d2", "chart", 3, 0, 6, 4),
        DashboardItem::new("d3", "list", 9, 0, 3, 4),
        DashboardItem::new("d4", "stat", 0, 2, 3, 2),
    ]
}

pub fn types() -> Vec<WidgetType> {
    vec![
        WidgetType::new("stat", "Stat", 3, 2).icon(Ico::Activity),
        WidgetType::new("chart", "Chart", 6, 4).icon(Ico::BarChart3),
        WidgetType::new("list", "List", 3, 4).icon(Ico::ListChecks),
    ]
}

#[derive(Debug)]
pub struct State {
    pub dash: Dashboard,
    types: Vec<WidgetType>,
}

impl State {
    pub fn with_store(kv: UiState) -> Self {
        Self {
            dash: Dashboard::new(defaults()).row_height(56.0).store(kv, KEY),
            types: types(),
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::with_store(UiState::load(SLUG))
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Dash(DashboardMsg),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Dash(m) => {
            state.dash.update(m);
        }
    }
    None
}

/// `stat('Active users', '1,284', …)`: `h-full flex flex-col
/// justify-between`. The sparkline `<svg>` is a flex item with `overflow:
/// hidden`, so its minimum height is 0: when the body is shorter than the
/// figure, the label and 32 px, it shrinks (clipping its drawing) and
/// otherwise sits at the bottom.
fn stat<'a>(cx: &Ctx, item: &DashboardItem, row_height: f32) -> El<'a, Msg> {
    let p = cx.p();
    let room = content_height(item.h, row_height).max(0.0) - 48.0;
    let ys: Vec<f64> = visits().into_iter().map(|p| p.1).collect();
    let spark = rounded_clip(
        0.0,
        c(cx.over),
        sparkline(cx, &ys).area().size(160.0, 32.0),
    )
    .width(160)
    .height(room.clamp(0.0, 32.0));
    column![
        label("1,284", 24.0, 32.0).semibold().color(c(p.text)),
        label("Active users", 12.0, 16.0).color(c(p.muted)),
        vgap((room - 32.0).max(0.0)),
        spark,
    ]
    .into()
}

/// `ul space-y-1.5 text-sm text-app-subtext`, a 6 px accent dot per item.
fn list<'a>(cx: &Ctx) -> El<'a, Msg> {
    let p = cx.p();
    let rows = ["Review PR", "Ship release", "Update docs", "Reply to issues"].map(|t| {
        row![
            surface(Space::new().width(6).height(6))
                .fixed(BoxStyle::default().bg(c(p.accent)).radius(3.0)),
            label(t, 14.0, 20.0).color(c(p.subtext)),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    });
    column(rows).spacing(6).into()
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let p = cx.p();
    let (sub, text) = (c(p.subtext), c(p.text));
    let strong = |s: &str| Span::new(s).color(text).weight(500);
    let help = rich(
        vec![
            Span::new("Toggle ").color(sub),
            strong("Edit"),
            Span::new(" to drag, resize, add and remove widgets. Switch ").color(sub),
            strong("Snap"),
            Span::new("/").color(sub),
            strong("Free"),
            Span::new(" placement. Layout persists.").color(sub),
        ],
        14.0,
        20.0,
    )
    .wrap();
    let on = cx.on_card();
    let rh = state.dash.row_height;
    let board = dashboard(
        cx,
        &state.dash,
        &state.types,
        move |item| match item.kind.as_str() {
            "stat" => stat(&on, item, rh),
            "chart" => line_chart(&on, vec![Series::new("Visits", visits())])
                .height(150.0)
                .into(),
            "list" => list(&on),
            _ => Space::new().into(),
        },
        Msg::Dash,
    );
    // `space-y-3`.
    column![help, container(board).id(BOX)].spacing(12).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bato_kit::layout::dashboard::Handle;

    #[test]
    fn layout_persists_across_restarts() {
        let dir = std::env::temp_dir().join(format!("gallery-dash-{}", std::process::id()));
        let file = dir.join("ui-state.json");
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = State::with_store(UiState::at(&file));
        assert_eq!(s.dash.items, defaults());
        update(&mut s, Msg::Dash(DashboardMsg::ToggleEdit));
        update(&mut s, Msg::Dash(DashboardMsg::Begin("d2".into(), Handle::Move)));
        update(&mut s, Msg::Dash(DashboardMsg::Step(0, 2)));
        assert!(!file.exists(), "live steps are not written");
        update(&mut s, Msg::Dash(DashboardMsg::End));
        let again = State::with_store(UiState::at(&file));
        assert_eq!(again.dash.items, s.dash.items);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stat_sparkline_shrinks_in_two_rows() {
        // 2 rows: 67 px of content → 19 px of sparkline, as in Chromium.
        assert_eq!(content_height(2, 56.0) - 48.0, 19.0);
    }
}
