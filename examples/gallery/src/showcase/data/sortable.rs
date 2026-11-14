//! `data/sortable`: Sortable list (drag to reorder). Four rows with grips;
//! the order is the card's state (React's `useState` + `onReorder`).

use bato_kit::data::sortable_list;
use bato_kit::dnd::array_move;
use bato_kit::iced::widget::container;
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// The list, for the self-test.
pub const LIST: &str = "data/sortable/list";

#[derive(Debug)]
pub struct State {
    pub items: Vec<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            items: ["First item", "Second item", "Third item", "Fourth item"]
                .map(String::from)
                .to_vec(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Reorder(usize, usize),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Reorder(from, to) => array_move(&mut state.items, from, to),
    }
    None
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let text = c(on.p().text);
    let list = sortable_list(
        &on,
        state
            .items
            .iter()
            .map(|x| (x.clone(), label(x.clone(), 14.0, 20.0).color(text))),
    )
    .on_reorder(Msg::Reorder);
    card(cx, "Sortable list (drag to reorder)")
        .body(container(list).id(LIST))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reorder_moves_one_item() {
        let mut s = State::default();
        update(&mut s, Msg::Reorder(0, 2));
        assert_eq!(s.items, ["Second item", "Third item", "First item", "Fourth item"]);
    }
}
