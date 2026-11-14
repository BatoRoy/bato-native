//! `feedback/atoms`: Feedback atoms. Five badges, a spinner, a progress bar
//! and a labelled ring at 64 %, and two skeleton bars.

use bato_kit::feedback::{circular_progress, progress, skeleton};
use bato_kit::iced::widget::{column, row};
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// React's `useState(64)`.
pub const PROGRESS: f32 = 64.0;

#[derive(Debug, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Msg {}

pub fn update(_state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {}
}

pub fn view<'a>(_state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    let badges = row![
        badge(&on, Tone::Success, "Success"),
        badge(&on, Tone::Error, "Error"),
        badge(&on, Tone::Warning, "Warning"),
        badge(&on, Tone::Info, "Info"),
        badge(&on, Tone::Neutral, "Neutral"),
    ]
    .spacing(8)
    .wrap();
    let bars = row![
        spinner(16.0).color(c(p.text)),
        progress(&on, PROGRESS),
        circular_progress(&on, PROGRESS).label(),
    ]
    .spacing(16)
    .align_y(Alignment::Center);
    let skeletons = column![
        skeleton(&on).fraction(0.75),
        skeleton(&on).fraction(0.5),
    ]
    .spacing(8);
    card(cx, "Feedback atoms")
        .body(column![badges, bars, skeletons].spacing(16))
        .into()
}
