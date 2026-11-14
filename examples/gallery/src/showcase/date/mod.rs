//! The **Date** tab: its cards in page order (docs/CARDS.md). One file
//! per card; a batch edits only its own card files and its line here.
//!
//! Written out instead of `tab_cards!` because two cards share state: the
//! Pickers card's "12-hour clock" / "Show seconds" switches also drive the
//! Time picker card (React's `DateSection` state).
//!
//! `BATO_SELFTEST=date` ([`selftest`]) drives every component through the
//! real update path; start it with `BATO_VIEW=view=examples&tab=date`.

pub mod calendar_time;
pub mod calendar_view;
pub mod pickers;

use std::time::Duration;

use bato_kit::iced::keyboard::Modifiers;
use bato_kit::iced::keyboard::key::Named;
use bato_kit::iced::widget::Id;
use bato_kit::iced::{Vector, mouse};
use bato_kit::overlay::script::{Step, Target};
use bato_kit::scroller::{Align, scroll_to};
use bato_kit::{Ctx, El};

use crate::showcase::{Effect, Tab, card_id};

/// Now: `BATO_NOW=2026-11-14T08:23:00` pins the clock (comparing against a
/// stored Electron reference), else the local time, in whole seconds.
pub fn now() -> bato_kit::datefmt::NaiveDateTime {
    std::env::var("BATO_NOW")
        .ok()
        .and_then(|s| {
            bato_kit::datefmt::NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S").ok()
        })
        .unwrap_or_else(|| bato_kit::date::math::trunc_seconds(bato_kit::datefmt::now()))
}

/// Today, by [`now`].
pub fn today() -> bato_kit::datefmt::NaiveDate {
    now().date()
}

#[derive(Debug, Default)]
pub struct State {
    pub pickers: pickers::State,
    pub calendar_time: calendar_time::State,
    pub calendar_view: calendar_view::State,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Pickers(pickers::Msg),
    CalendarTime(calendar_time::Msg),
    CalendarView(calendar_view::Msg),
    /// Self-test: scroll a card to the top.
    ScrollTo(&'static str),
}

/// Card ids in page order.
pub const IDS: &[&str] = &["pickers", "calendar-time", "calendar-view"];

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Pickers(m) => pickers::update(&mut state.pickers, m),
        Msg::CalendarTime(m) => calendar_time::update(&mut state.calendar_time, m),
        Msg::CalendarView(m) => calendar_view::update(&mut state.calendar_view, m),
        Msg::ScrollTo(card) => {
            scroll_to(card_id(Tab::Date, card), Align::Start);
            None
        }
    }
}

/// `(card id, view)` in page order.
pub fn cards<'a>(state: &'a State, cx: &Ctx) -> Vec<(&'static str, El<'a, Msg>)> {
    let p = &state.pickers;
    vec![
        ("pickers", pickers::view(p, cx).map(Msg::Pickers)),
        (
            "calendar-time",
            calendar_time::view(&state.calendar_time, cx, p.hour12, p.seconds)
                .map(Msg::CalendarTime),
        ),
        (
            "calendar-view",
            calendar_view::view(&state.calendar_view, cx).map(Msg::CalendarView),
        ),
    ]
}

/// `BATO_SELFTEST=date`: popovers, range hover, the 12-hour / seconds
/// DateTimePicker, recentring time columns, Tab through the Calendar, and
/// the CalendarView's month, week and hover states.
pub fn selftest(name: &str) -> Option<Vec<Step<Msg>>> {
    if name != "date" {
        return None;
    }
    let left = mouse::Button::Left;
    let none = Modifiers::empty();
    let id = |s: &str| Id::from(s.to_string());
    let click = |name: &str| Step::Click(Target::Center(id(name)), left);
    let at = |name: &str, x: f32, y: f32| Target::Id(id(name), Vector::new(x, y));
    // A day of a picker's popover Calendar: `p-3` + 1, header 24 + 12,
    // weekdays 24 + 4, 256 / 7 wide columns, 34 px rows.
    let day = |panel: &str, col: f32, row: f32| {
        at(panel, 256.0 / 7.0 * (col + 0.5), 64.0 + 34.0 * row + 16.0)
    };
    let settle = Step::Wait(Duration::from_millis(350));
    Some(vec![
        Step::Shot("rest"),
        // DatePicker: open, hover a day, Escape.
        click(pickers::DATE),
        settle.clone(),
        Step::Move(day("pk-date:panel", 2.0, 2.0)),
        settle.clone(),
        Step::Shot("date-open"),
        Step::Key(Named::Escape, none),
        // DateRangePicker: start, hover preview, end (closes).
        click(pickers::RANGE),
        settle.clone(),
        Step::Click(day("pk-range:panel", 1.0, 1.0), left),
        Step::Move(day("pk-range:panel", 4.0, 2.0)),
        settle.clone(),
        Step::Shot("range-hover"),
        Step::Click(day("pk-range:panel", 4.0, 2.0), left),
        settle.clone(),
        Step::Shot("range-done"),
        // 12-hour clock + seconds, then the DateTimePicker.
        click(pickers::HOUR12),
        click(pickers::SECONDS),
        click(pickers::DATETIME),
        settle.clone(),
        Step::Shot("datetime-open"),
        // The hour one below the centre: picked, then centred.
        Step::Click(at("pk-dt:panel", 294.0 + 22.0, 24.0 + 80.0 + 30.0), left),
        settle.clone(),
        Step::Shot("datetime-hour"),
        Step::Key(Named::Escape, none),
        // Calendar card: next month, then keyboard focus on a day.
        Step::Click(at(calendar_time::CALENDAR, 244.0, 12.0), left),
        Step::Click(
            at(
                calendar_time::CALENDAR,
                256.0 / 7.0 * 2.5,
                64.0 + 34.0 + 16.0,
            ),
            left,
        ),
        Step::Key(Named::Tab, none),
        settle.clone(),
        Step::Shot("calendar-next-focus"),
        // Time picker card: a minute below the centre.
        Step::Click(
            at(calendar_time::TIME, 52.0 + 22.0, 24.0 + 80.0 + 30.0),
            left,
        ),
        settle.clone(),
        Step::Shot("time-pick"),
        // CalendarView: month, week, an event hovered.
        Step::Message(Msg::ScrollTo("calendar-view")),
        Step::Wait(Duration::from_millis(900)),
        Step::Move(at(calendar_view::VIEW, 100.0, 120.0)),
        settle.clone(),
        Step::Shot("month-hover"),
        Step::Click(at(calendar_view::VIEW, 776.0, 16.0), left),
        settle.clone(),
        Step::Shot("week"),
        Step::Move(at(calendar_view::VIEW, 400.0, 250.0)),
        Step::Event(bato_kit::iced::Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: -3.0 },
        })),
        Step::Wait(Duration::from_millis(600)),
        Step::Shot("week-scrolled"),
    ])
}
