//! `layout/scrollable`: Scrollable. 14 rows in a 150 px box with the edge
//! fade (`<Scrollable maxHeight={150} fade className="rounded-lg border">`).

use bato_kit::iced::widget::{column, container};
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// The scroll box, for the self-test.
pub const AREA: &str = "layout/scrollable/area";

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
    let rows = (1..=14).map(|i| {
        El::from(
            surface(label(format!("Scrollable row {i}"), 14.0, 20.0).color(c(p.subtext)))
                .padding([4, 8])
                .width(Length::Fill)
                .fixed(BoxStyle::default().bg(c(p.bg)).radius(4.0)),
        )
    });
    let area = scroll_area(&on, container(column(rows).spacing(8)).padding(12))
        .max_height(150.0)
        .faded()
        .border(c(p.border), 8.0);
    card(cx, "Scrollable")
        .body(container(area).id(AREA))
        .into()
}
