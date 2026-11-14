//! `inputs/time-input`: the TimeInput, its value in mono, three Switches
//! and the key hint.

use bato_kit::iced::widget::{column, row};
use bato_kit::inputs::time_input::{TimeValue, time_input};
use bato_kit::prelude::*;

use crate::showcase::Effect;

#[derive(Debug)]
pub struct State {
    time: TimeValue,
    seconds: bool,
    cap: bool,
    wrap: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            time: TimeValue::new(9, 41, 0),
            seconds: true,
            cap: true,
            wrap: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Time(TimeValue),
    Seconds(bool),
    Cap(bool),
    Wrap(bool),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Time(t) => state.time = t,
        Msg::Seconds(on) => state.seconds = on,
        Msg::Cap(on) => state.cap = on,
        Msg::Wrap(on) => state.wrap = on,
    }
    None
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    let value = rich(
        vec![
            Span::new("Value: "),
            Span::new(state.time.hms())
                .mono()
                .size(12.0)
                .color(c(p.text)),
        ],
        14.0,
        20.0,
    )
    .color(c(p.subtext));
    // `flex flex-wrap items-end gap-6`.
    let top = row![
        container(
            time_input(&on, state.time, Msg::Time)
                .label("Duration")
                .show_seconds(state.seconds)
                .cap_hours(state.cap)
                .wrap(state.wrap),
        )
        .id("b1:time"),
        // The 12 px mono span makes the line box a pixel taller than 20.
        container(value).padding(Padding::default().bottom(1))
    ]
    .spacing(24)
    .align_y(Alignment::End);
    let switches = row![
        switch(&on, state.seconds, Msg::Seconds).label("Show seconds"),
        switch(&on, !state.cap, |v| Msg::Cap(!v)).label("Allow 99 hours"),
        switch(&on, state.wrap, Msg::Wrap).label("Wrap at ends"),
    ]
    .spacing(16);
    let body = column![
        top,
        vgap(16.0),
        switches,
        vgap(12.0),
        label(
            "←/→ switch boxes · ↑/↓ step (hold to repeat) · 59m + 1 carries to the hour.",
            12.0,
            16.0
        )
        .color(c(p.muted)),
    ];
    card(cx, "Time input (typeable)").body(body).into()
}

use std::convert::Infallible;
use std::time::Duration;

use bato_kit::iced::keyboard::key::Named;
use bato_kit::overlay::script::Step;

use super::text_fields::{click, hold, hold_key, key, typed};

/// Hours: click (selected), type 1 5 (→ minutes), ↑, → (seconds), hold ↑,
/// hold the up chevron; "Allow 99 hours" by its label.
pub(crate) fn selftest() -> Vec<Step<Infallible>> {
    let mut s = vec![click(373.0, 122.0), Step::Wait(Duration::from_millis(300))];
    s.push(Step::Shot("hours-selected"));
    s.extend(typed("15"));
    s.push(key(Named::ArrowUp));
    s.push(Step::Wait(Duration::from_millis(300)));
    s.push(Step::Shot("typed-15-minutes-up"));
    s.push(key(Named::ArrowRight));
    s.extend(hold_key(Named::ArrowUp, 1000));
    s.extend(hold(476.0, 115.0, 800));
    s.push(click(600.0, 167.0));
    s.push(Step::Wait(Duration::from_millis(400)));
    s.push(Step::Shot("held-99h"));
    s
}
