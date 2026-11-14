//! `feedback/accordion`: Accordion. Three items, one open at a time.

use bato_kit::feedback::{accordion, body_text, toggle};
use bato_kit::iced::widget::container;
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// The accordion's container, for the self-test.
pub const ITEMS: &str = "feedback/accordion/items";

const ITEMS_TEXT: [(&str, &str); 3] = [
    ("What is this?", "A collapsible section of content."),
    ("Can multiple open?", "Set the `multiple` prop to allow it."),
    (
        "Is it themed?",
        "Yes — everything uses the accent + theme tokens.",
    ),
];

#[derive(Debug, Default)]
pub struct State {
    pub open: Vec<usize>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Toggle(usize),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Toggle(i) => toggle(&mut state.open, i, false),
    }
    None
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let items = ITEMS_TEXT
        .iter()
        .map(|(t, b)| (t.to_string(), body_text(&on, *b)));
    card(cx, "Accordion")
        .body(container(accordion(&on, items, &state.open, Msg::Toggle)).id(ITEMS))
        .into()
}
