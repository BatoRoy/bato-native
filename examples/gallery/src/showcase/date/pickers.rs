//! `date/pickers`: Pickers. DatePicker (clearable) and DateRangePicker side
//! by side, DateTimePicker across both columns, then the "12-hour clock" and
//! "Show seconds" switches (they also drive the Time picker card).
//!
//! A card owns its `State`, `Msg`, `update` and `view`; the tab lists it once.

use bato_kit::date::{
    DateRange, PickerMsg, PickerState, date_picker, date_range_picker, date_time_picker,
};
use bato_kit::datefmt::{NaiveDate, NaiveDateTime};
use bato_kit::iced::widget::{Id, column, row};
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// Self-test targets.
pub const DATE: &str = "date:date";
pub const RANGE: &str = "date:range";
pub const DATETIME: &str = "date:datetime";
pub const HOUR12: &str = "date:hour12";
pub const SECONDS: &str = "date:seconds";

#[derive(Debug)]
pub struct State {
    pub date: Option<NaiveDate>,
    pub date_p: PickerState,
    pub range: DateRange,
    pub range_p: PickerState,
    pub dt: Option<NaiveDateTime>,
    pub dt_p: PickerState,
    pub hour12: bool,
    pub seconds: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            date: Some(super::today()),
            date_p: PickerState::default(),
            range: DateRange::EMPTY,
            range_p: PickerState::default(),
            dt: Some(super::now()),
            dt_p: PickerState::default(),
            hour12: false,
            seconds: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Date(PickerMsg),
    Range(PickerMsg),
    DateTime(PickerMsg),
    Hour12(bool),
    Seconds(bool),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Date(m) => state.date_p.update_date(m, &mut state.date),
        Msg::Range(m) => state.range_p.update_range(m, &mut state.range),
        Msg::DateTime(m) => state.dt_p.update_datetime(m, &mut state.dt),
        Msg::Hour12(v) => state.hour12 = v,
        Msg::Seconds(v) => state.seconds = v,
    }
    None
}

/// `Switch` with a label: the whole `<label>` toggles it and focuses the
/// switch.
fn labelled_switch<'a>(
    cx: &Ctx,
    id: &'static str,
    on: bool,
    text: &str,
    f: fn(bool) -> Msg,
) -> El<'a, Msg> {
    container(
        surface(
            row![
                switch(cx, on, f),
                label(text, 14.0, 20.0).color(c(cx.p().subtext))
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .on_press(f(!on))
        .focusable(false)
        .labels_child(),
    )
    .id(Id::new(id))
    .into()
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let date = date_picker(&on, &state.date_p, state.date, Msg::Date)
        .label("Date")
        .clearable(true)
        .id("pk-date");
    let range = date_range_picker(&on, &state.range_p, state.range, Msg::Range)
        .label("Date range")
        .id("pk-range");
    let dt = date_time_picker(&on, &state.dt_p, state.dt, Msg::DateTime)
        .label("Date & time")
        .hour12(state.hour12)
        .show_seconds(state.seconds)
        .id("pk-dt");
    let fields = Grid::columns(2)
        .gap(16.0)
        .push(container(date).id(Id::new(DATE)))
        .push(container(range).id(Id::new(RANGE)))
        .push_span(container(dt).id(Id::new(DATETIME)), 2);
    let switches = row![
        labelled_switch(&on, HOUR12, state.hour12, "12-hour clock", Msg::Hour12),
        labelled_switch(&on, SECONDS, state.seconds, "Show seconds", Msg::Seconds),
    ]
    .spacing(16)
    .align_y(Alignment::Center);
    card(cx, "Pickers")
        .body(column![fields, vgap(16.0), switches])
        .into()
}
