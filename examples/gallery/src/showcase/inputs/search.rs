//! `inputs/search`: Search (suggestions + autocomplete). A SearchInput over
//! twelve frameworks, then two labelled switches that turn the suggestions
//! dropdown and the inline autocomplete on and off (`ShowcasePage.tsx`).

use bato_kit::iced::widget::{column, row};
use bato_kit::inputs::search::search_input;
use bato_kit::prelude::*;
use crate::showcase::Effect;

pub const FRAMEWORKS: [&str; 12] = [
    "React", "Redux", "Recoil", "Remix", "Svelte", "Solid", "Vue", "Vite", "Vitest", "Angular",
    "Astro", "Next.js",
];

#[derive(Debug)]
pub struct State {
    pub value: String,
    pub suggestions: bool,
    pub auto: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            value: String::new(),
            suggestions: true,
            auto: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Value(String),
    Suggestions(bool),
    Auto(bool),
    /// Focus the field (self-test): it scrolls into view and opens.
    Focus,
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Value(v) => state.value = v,
        Msg::Suggestions(v) => state.suggestions = v,
        Msg::Auto(v) => state.auto = v,
        Msg::Focus => bato_kit::focus::request(bato_kit::focus::Target::Id(FIELD.into())),
    }
    None
}

/// The input's id (self-test).
pub const FIELD: &str = "search-input";

/// `<Switch checked onChange label />`: the switch and its 14 px `subtext`
/// label, 10 px apart; a click on the label toggles too.
pub fn labelled_switch<'a, M: Clone + 'a>(
    cx: &Ctx,
    on: bool,
    text: &'static str,
    f: impl Fn(bool) -> M + 'a,
) -> El<'a, M> {
    let p = cx.p();
    let msg = f(!on);
    surface(
        row![
            switch(cx, on, f),
            label(text, 14.0, 20.0).color(c(p.subtext))
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .focusable(false)
    .on_press(msg)
    .into()
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let input = search_input(&on, &state.value, Msg::Value)
        .suggestions(&FRAMEWORKS)
        .show_suggestions(state.suggestions)
        .auto_complete(state.auto)
        .placeholder("Search frameworks…")
        .id(FIELD);
    let switches = row![
        labelled_switch(
            &on,
            state.suggestions,
            "Suggestions dropdown",
            Msg::Suggestions
        ),
        labelled_switch(&on, state.auto, "Inline autocomplete", Msg::Auto),
    ]
    .spacing(16)
    .wrap();
    card(cx, "Search (suggestions + autocomplete)")
        .body(column![input, vgap(12.0), switches])
        .into()
}
