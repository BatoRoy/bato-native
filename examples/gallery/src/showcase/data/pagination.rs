//! `data/pagination`: Pagination (standalone). 12 pages, starting on page 3,
//! one sibling.

use bato_kit::data::pagination;
use bato_kit::prelude::*;

use crate::showcase::Effect;

#[derive(Debug)]
pub struct State {
    pub page: usize,
}

impl Default for State {
    fn default() -> Self {
        Self { page: 3 }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Page(usize),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Page(p) => state.page = p,
    }
    None
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    card(cx, "Pagination (standalone)")
        .body(pagination(&cx.on_card(), state.page, 12, Msg::Page))
        .into()
}
