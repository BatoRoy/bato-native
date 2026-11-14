//! `date/calendar-time`: the 2-col row of the Calendar card and the Time
//! picker card (`grid grid-cols-2 gap-6`). The Time picker follows the
//! Pickers card's "12-hour clock" and "Show seconds" switches.
//!
//! A card owns its `State`, `Msg`, `update` and `view`; the tab lists it once.

use bato_kit::date::{CalendarMsg, CalendarState, calendar, time_picker};
use bato_kit::datefmt::{NaiveDate, NaiveDateTime};
use bato_kit::iced::widget::Id;
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// Self-test targets.
pub const CALENDAR: &str = "date:calendar";
pub const TIME: &str = "date:time";

#[derive(Debug)]
pub struct State {
    pub cal: Option<NaiveDate>,
    pub cal_state: CalendarState,
    pub time: NaiveDateTime,
}

impl Default for State {
    fn default() -> Self {
        let today = super::today();
        Self {
            cal: Some(today),
            cal_state: CalendarState::new(today),
            time: super::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Cal(CalendarMsg),
    Time(NaiveDateTime),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Cal(m) => {
            if let Some(d) = state.cal_state.update(m) {
                state.cal = Some(d);
            }
        }
        Msg::Time(t) => state.time = t,
    }
    None
}

pub fn view<'a>(state: &'a State, cx: &Ctx, hour12: bool, seconds: bool) -> El<'a, Msg> {
    let on = cx.on_card();
    let cal = calendar(&on, &state.cal_state, Msg::Cal)
        .single(state.cal)
        .today(super::today());
    let time = time_picker(&on, state.time, Msg::Time)
        .hour12(hour12)
        .show_seconds(seconds)
        .id("tp-card");
    Grid::columns(2)
        .gap(24.0)
        .push(card(cx, "Calendar").body(container(cal).id(Id::new(CALENDAR))))
        .push(card(cx, "Time picker").body(container(time).id(Id::new(TIME))))
        .into()
}
