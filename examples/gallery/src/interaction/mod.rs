//! Interaction lab: the Phase 2 infrastructure (overlays and menus, focus,
//! scroll areas and fields), each section in its own file so the three
//! agents building them never edit the same file.

pub mod focus;
pub mod gaps;
pub mod modals;
pub mod overlay;
pub mod ports;
pub mod scroll;

use bato_kit::iced::widget::column;
use bato_kit::iced::{Subscription, Task};
use bato_kit::prelude::*;

#[derive(Debug, Default)]
pub struct State {
    pub overlay: overlay::State,
    pub focus: focus::State,
    pub scroll: scroll::State,
    pub gaps: gaps::State,
    pub modals: modals::State,
    pub ports: ports::State,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Overlay(overlay::Msg),
    Focus(focus::Msg),
    Scroll(scroll::Msg),
    Gaps(gaps::Msg),
    Modals(modals::Msg),
    Ports(ports::Msg),
}

impl Msg {
    /// An update status the gaps section asks the frame to show.
    pub fn gaps_status(&self) -> Option<&bato_kit::frame::UpdateStatus> {
        match self {
            Msg::Gaps(m) => gaps::simulated(m),
            _ => None,
        }
    }

    /// A toast the modals section asks the frame to show.
    pub fn toast(&self) -> Option<(String, Tone)> {
        match self {
            Msg::Modals(m) => m.toast(),
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
        Msg::Modals(m) => modals::update(&mut state.modals, m).map(Msg::Modals),
        Msg::Ports(m) => ports::update(&mut state.ports, m).map(Msg::Ports),
    }
}

pub fn subscription(state: &State) -> Subscription<Msg> {
    Subscription::batch([
        overlay::subscription(&state.overlay).map(Msg::Overlay),
        ports::subscription(&state.ports).map(Msg::Ports),
    ])
}

/// The overlay-stack levels the lab has open, for `Frame::view_with`.
pub fn levels<'a>(state: &'a State, cx: &Ctx) -> Vec<bato_kit::modal::Modal<'a, Msg>> {
    overlay::levels(&state.overlay, cx)
        .into_iter()
        .map(|l| l.map(Msg::Overlay))
        .chain(
            modals::levels(&state.modals, cx)
                .into_iter()
                .map(|l| l.map(Msg::Modals)),
        )
        .chain(
            ports::levels(&state.ports, cx)
                .into_iter()
                .map(|l| l.map(Msg::Ports)),
        )
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
        n if n.starts_with("inputs:") => crate::showcase::inputs::text_fields::b1_selftest(&n[7..])
            .map(|v| v.into_iter().map(|s| s.map(|n| match n {})).collect()),
        "boundary" => Some(
            modals::boundary_selftest()
                .into_iter()
                .map(|s| s.map(Msg::Modals))
                .collect(),
        ),
        n if n.starts_with("kit11") => {
            ports::selftest(n).map(|v| v.into_iter().map(|s| s.map(Msg::Ports)).collect())
        }
        "modals" => Some(
            modals::selftest()
                .into_iter()
                .map(|s| s.map(Msg::Modals))
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
            // First: its self-test aims at containers by id, which the
            // script finds in layout (unscrolled) coordinates.
            modals::view(&state.modals, cx).map(Msg::Modals),
            vgap(24.0),
            overlay::view(&state.overlay, cx).map(Msg::Overlay),
            vgap(24.0),
            focus::view(&state.focus, cx).map(Msg::Focus),
            vgap(24.0),
            scroll::view(&state.scroll, cx).map(Msg::Scroll),
            vgap(24.0),
            gaps::view(&state.gaps, cx).map(Msg::Gaps),
            vgap(24.0),
            ports::view(&state.ports, cx).map(Msg::Ports),
        ],
        wide,
        [48.0, 32.0],
    )
}
