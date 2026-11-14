//! `inputs/selects`: Selects. A 2-column grid: the Combobox and the
//! Multi-select over the fruit options, then "Quick select" with a Select
//! across both columns (`ShowcasePage.tsx`).

use bato_kit::iced::widget::{column, row};
use bato_kit::inputs::color_picker::color_picker;
use bato_kit::inputs::combobox::combobox;
use bato_kit::inputs::multiselect::multiselect;
use bato_kit::overlay::select::{SelectOption, opt, select};
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// `fruitOptions`.
pub fn fruits() -> Vec<SelectOption<&'static str>> {
    vec![
        opt("apple", "Apple"),
        opt("banana", "Banana"),
        opt("cherry", "Cherry"),
        opt("date", "Date"),
        opt("elderberry", "Elderberry"),
    ]
}

#[derive(Debug)]
pub struct State {
    pub fruit: Option<&'static str>,
    pub multi: Vec<&'static str>,
    /// `BATO_SELFTEST=pickers` only: a ColorPicker under the card body, so
    /// the batch can drive it before it lands in Sliders & color (B1's card).
    pub color: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            fruit: Some("apple"),
            multi: vec!["apple", "cherry"],
            color: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Fruit(&'static str),
    Multi(Vec<&'static str>),
    /// Self-test: show (or set) the extra ColorPicker.
    Color(String),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Fruit(f) => state.fruit = Some(f),
        Msg::Multi(v) => state.multi = v,
        Msg::Color(c) => state.color = Some(c),
    }
    None
}


pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    let quick = row![
        label("Quick select", 14.0, 20.0)
            .color(c(p.subtext))
            .width(96),
        select(&on, state.fruit.unwrap_or("apple"), fruits(), Msg::Fruit),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    let body = Grid::columns(2)
        .gap(16.0)
        .push(
            combobox(&on, state.fruit, fruits(), Msg::Fruit).label("Combobox"),
        )
        .push(
            multiselect(&on, &state.multi, fruits(), Msg::Multi).label("Multi-select"),
        )
        .push_span(quick, 2);
    let body: El<'a, Msg> = match &state.color {
        // Above the grid, so its popover opens downwards.
        Some(col) => column![
            color_picker(&on, col, Msg::Color).label("Color"),
            vgap(20.0),
            body,
        ]
        .into(),
        None => body.into(),
    };
    card(cx, "Selects").body(body).into()
}
