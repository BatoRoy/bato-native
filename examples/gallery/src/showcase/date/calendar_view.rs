//! `date/calendar-view`: Calendar view, 448 px tall (`h-[28rem]`), with five
//! events around today (Standup, Design review, Workshop, Retro, Demo).
//!
//! A card owns its `State`, `Msg`, `update` and `view`; the tab lists it once.

use bato_kit::date::View;
use bato_kit::date::{CalendarEvent, CalendarViewMsg, CalendarViewState, calendar_view};
use bato_kit::datefmt::{Days, Timelike};
use bato_kit::iced::widget::Id;
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// Self-test target.
pub const VIEW: &str = "date:view";

#[derive(Debug)]
pub struct State {
    pub view: CalendarViewState,
    pub events: Vec<CalendarEvent>,
}

/// `ev(d, h, dur, title)`: today + `d` days at `h`:00 for `dur` hours.
fn ev(d: i64, h: u32, dur: u32, title: &str) -> CalendarEvent {
    let today = super::today();
    let day = if d >= 0 {
        today + Days::new(d as u64)
    } else {
        today - Days::new(d.unsigned_abs())
    };
    let start = day.and_hms_opt(h, 0, 0).expect("valid hour");
    let end = start.with_hour(h + dur).expect("same day");
    CalendarEvent {
        id: format!("{d}-{h}-{title}"),
        title: title.into(),
        start,
        end: Some(end),
        color: None,
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            view: CalendarViewState {
                view: View::Month,
                cursor: super::today(),
            },
            events: vec![
                ev(0, 10, 1, "Standup"),
                ev(0, 14, 2, "Design review"),
                ev(2, 9, 3, "Workshop"),
                ev(-1, 16, 1, "Retro"),
                ev(4, 11, 2, "Demo"),
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    View(CalendarViewMsg),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    let Msg::View(m) = msg;
    state.view.update(&m);
    None
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let body =
        container(calendar_view(&on, &state.view, &state.events, Msg::View).today(super::today()))
            .height(448)
            .width(Length::Fill)
            .id(Id::new(VIEW));
    card(cx, "Calendar view").body(body).into()
}
