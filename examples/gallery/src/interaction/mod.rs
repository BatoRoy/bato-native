//! Interaction lab: the Phase 2 infrastructure (overlays and menus, focus,
//! scroll areas and fields), each section in its own file so the three
//! agents building them never edit the same file.

pub mod focus;
pub mod gaps;
pub mod overlay;
pub mod scroll;

use bato_kit::iced::Task;
use bato_kit::iced::widget::column;
use bato_kit::prelude::*;

#[derive(Debug, Default)]
pub struct State {
    pub overlay: overlay::State,
    pub focus: focus::State,
    pub scroll: scroll::State,
    pub gaps: gaps::State,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Overlay(overlay::Msg),
    Focus(focus::Msg),
    Scroll(scroll::Msg),
    Gaps(gaps::Msg),
}

impl Msg {
    /// An update status the gaps section asks the frame to show.
    pub fn gaps_status(&self) -> Option<&bato_kit::frame::UpdateStatus> {
        match self {
            Msg::Gaps(m) => gaps::simulated(m),
            _ => None,
        }
    }
}

pub fn update(state: &mut State, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Overlay(m) => overlay::update(&mut state.overlay, m).map(Msg::Overlay),
        Msg::Focus(m) => focus::update(&mut state.focus, m).map(Msg::Focus),
        Msg::Scroll(m) => scroll::update(&mut state.scroll, m).map(Msg::Scroll),
        Msg::Gaps(m) => gaps::update(&mut state.gaps, m).map(Msg::Gaps),
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
            vgap(24.0),
            gaps::view(&state.gaps, cx).map(Msg::Gaps),
        ],
        wide,
        [48.0, 32.0],
    )
}
