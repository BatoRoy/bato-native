//! `data/charts`: Line chart · Area chart · Bar chart · Sparkline, a 2×2
//! grid. One "Visits" series of 14 points (`20 + round(30 sin(i/2) + 2i)`),
//! five weekday bars, and a 1,284 figure with a 120 × 36 area sparkline.

use bato_kit::data::charts::js_round;
use bato_kit::data::{Bar, Series, area_chart, bar_chart, line_chart, sparkline};
use bato_kit::iced::widget::row;
use bato_kit::prelude::*;

use crate::showcase::Effect;

#[derive(Debug, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Msg {}

pub fn update(_state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {}
}

/// `{ x: i, y: 20 + Math.round(30 * Math.sin(i / 2) + i * 2) }` for 14 i.
pub fn visits() -> Vec<(f64, f64)> {
    (0..14)
        .map(|i| {
            let i = i as f64;
            (i, 20.0 + js_round(30.0 * (i / 2.0).sin() + i * 2.0))
        })
        .collect()
}

pub fn view<'a>(_state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let series = || vec![Series::new("Visits", visits())];
    let bars = [
        ("Mon", 12.0),
        ("Tue", 19.0),
        ("Wed", 7.0),
        ("Thu", 22.0),
        ("Fri", 15.0),
    ]
    .into_iter()
    .map(|(l, v)| Bar::new(l, v))
    .collect();
    let ys: Vec<f64> = visits().into_iter().map(|p| p.1).collect();
    let spark = row![
        label("1,284", 24.0, 32.0).semibold().color(c(on.p().text)),
        sparkline(&on, &ys).area().size(120.0, 36.0),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    // `grid grid-cols-2 gap-6`: several cards under one id.
    Grid::columns(2)
        .gap(24.0)
        .push(card(cx, "Line chart").body(line_chart(&on, series())))
        .push(card(cx, "Area chart").body(area_chart(&on, series())))
        .push(card(cx, "Bar chart").body(bar_chart(&on, bars)))
        .push(card(cx, "Sparkline").body(spark))
        .into()
}
