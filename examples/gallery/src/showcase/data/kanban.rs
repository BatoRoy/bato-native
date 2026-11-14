//! `data/kanban`: Kanban board (drag cards). Three columns (To do, In
//! progress, Done); the board is the card's state (React's `onChange`).

use bato_kit::data::{KanbanCard, KanbanColumn, kanban_board};
use bato_kit::prelude::*;

use crate::showcase::Effect;

#[derive(Debug)]
pub struct State {
    pub board: Vec<KanbanColumn>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            board: vec![
                KanbanColumn::new(
                    "todo",
                    "To do",
                    vec![
                        KanbanCard::new("c1", "Research"),
                        KanbanCard::new("c2", "Wireframes"),
                    ],
                ),
                KanbanColumn::new(
                    "doing",
                    "In progress",
                    vec![KanbanCard::new("c3", "Build UI").description("Component library")],
                ),
                KanbanColumn::new("done", "Done", vec![KanbanCard::new("c4", "Setup repo")]),
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Board(Vec<KanbanColumn>),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Board(b) => state.board = b,
    }
    None
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    card(cx, "Kanban board (drag cards)")
        .body(kanban_board(&cx.on_card(), &state.board).on_change(Msg::Board))
        .into()
}
