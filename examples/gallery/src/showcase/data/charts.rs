//! `data/charts`: Line chart · Area chart · Bar chart · Sparkline. Not ported yet.
//!
//! A card owns its `State`, `Msg`, `update` and `view`; the tab lists it once.

use bato_kit::prelude::*;

use crate::showcase::{Effect, stub};

#[derive(Debug, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Msg {}

pub fn update(_state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {}
}

pub fn view<'a>(_state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    // `grid grid-cols-2 gap-6`: several cards under one id.
    Grid::columns(2)
        .gap(24.0)
        .push(stub(cx, "Line chart"))
        .push(stub(cx, "Area chart"))
        .push(stub(cx, "Bar chart"))
        .push(stub(cx, "Sparkline"))
        .into()
}
