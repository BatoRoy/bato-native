//! Interaction lab: the Phase 2 infrastructure (overlays and menus, focus,
//! scroll areas and fields), each section in its own file so the three
//! agents building them never edit the same file.

pub mod focus;
pub mod overlay;
pub mod scroll;

use bato_kit::iced::widget::column;
use bato_kit::iced::{Subscription, Task};
use bato_kit::prelude::*;

#[derive(Debug, Default)]
pub struct State {
    pub overlay: overlay::State,
    pub focus: focus::State,
    pub scroll: scroll::State,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Overlay(overlay::Msg),
    Focus(focus::Msg),
    Scroll(scroll::Msg),
}

pub fn update(state: &mut State, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Overlay(m) => overlay::update(&mut state.overlay, m).map(Msg::Overlay),
        Msg::Focus(m) => focus::update(&mut state.focus, m).map(Msg::Focus),
        Msg::Scroll(m) => scroll::update(&mut state.scroll, m).map(Msg::Scroll),
    }
}

pub fn subscription(state: &State) -> Subscription<Msg> {
    overlay::subscription(&state.overlay).map(Msg::Overlay)
}

/// The overlay-stack levels the lab has open, for `Frame::view_with`.
pub fn levels<'a>(state: &'a State, cx: &Ctx) -> Vec<bato_kit::modal::Modal<'a, Msg>> {
    overlay::levels(&state.overlay, cx)
        .into_iter()
        .map(|l| l.map(Msg::Overlay))
        .collect()
}

/// `BATO_SELFTEST=<name>` scripts.
pub fn selftest(name: &str) -> Option<Vec<bato_kit::overlay::script::Step<Msg>>> {
    match name {
        "overlay" => Some(
            overlay::selftest()
                .into_iter()
                .map(|s| s.map(Msg::Overlay))
                .collect(),
        ),
        _ => None,
    }
}

pub fn view<'a>(state: &'a State, cx: &Ctx, wide: bool) -> El<'a, Msg> {
    let p = cx.p();
    page(
        column![
            label("Interaction lab", 30.0, 36.0)
                .semibold()
                .tracking(-0.025)
                .color(c(p.text)),
            vgap(4.0),
            label("Overlays, focus and scroll areas: the shared infrastructure every component builds on.", 14.0, 20.0)
                .color(c(p.subtext)),
            vgap(32.0),
            overlay::view(&state.overlay, cx).map(Msg::Overlay),
            vgap(24.0),
            focus::view(&state.focus, cx).map(Msg::Focus),
            vgap(24.0),
            scroll::view(&state.scroll, cx).map(Msg::Scroll),
        ],
        wide,
        [48.0, 32.0],
    )
}
