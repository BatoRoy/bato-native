//! Interaction lab section: Focus and keyboard (Phase 2). Owned by one Phase 2 agent;
//! replace this stub with demos of the infrastructure it builds.

use bato_kit::iced::Task;
use bato_kit::prelude::*;

#[derive(Debug, Default)]
pub struct State {}

#[derive(Debug, Clone)]
pub enum Msg {}

pub fn update(_state: &mut State, msg: Msg) -> Task<Msg> {
    match msg {}
}

pub fn view<'a>(_state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    card(cx, "Focus and keyboard")
        .body(
            label(
                "Tab order, focus rings on every control, the dialog focus trap.",
                14.0,
                20.0,
            )
            .color(c(p.subtext))
            .wrap(),
        )
        .into()
}
