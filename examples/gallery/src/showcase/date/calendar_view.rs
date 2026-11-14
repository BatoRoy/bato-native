//! `date/calendar-view`: Calendar view. Not ported yet.
//!
//! A card owns its `State`, `Msg`, `update` and `view`; the tab lists it once.

use bato_kit::{Ctx, El};

use crate::showcase::{Effect, stub};

#[derive(Debug, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Msg {}

pub fn update(_state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {}
}

pub fn view<'a>(_state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    stub(cx, "Calendar view")
}
