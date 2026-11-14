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

/// `BATO_SELFTEST=dnd` (with `BATO_VIEW=view=examples&tab=data&card=sortable`):
/// SortableList pointer drag, drop, Esc cancel and keyboard sorting, then
/// the KanbanBoard: a cross-column drag and drop, a drag over an empty
/// column part cancelled with Esc, and edge auto-scroll. Window points:
/// the page is clamped at its end, the sortable card's top at y = 108.
pub fn selftest<M>() -> Vec<bato_kit::overlay::script::Step<M>> {
    use bato_kit::iced::keyboard::{Modifiers, key::Named};
    use bato_kit::iced::{Event, Point, mouse};
    use bato_kit::overlay::script::{Step, Target};
    use std::time::Duration;
    let left = mouse::Button::Left;
    let at = |x: f32, y: f32| Target::At(Point::new(x, y));
    let mv = |x: f32, y: f32| Step::Move(Target::At(Point::new(x, y)));
    let wait = |ms| Step::Wait(Duration::from_millis(ms));
    let release = || Step::Event(Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)));
    let key = |k| Step::Key(k, Modifiers::empty());
    // Row r (0-based) grip centre.
    let grip = |r: f32| (368.0, 188.0 + 54.0 * r);
    vec![
        Step::Shot("rest"),
        // Pointer drag: First item over Third item.
        Step::Press(at(grip(0.0).0, grip(0.0).1), left),
        mv(368.0, 190.0),
        mv(370.0, 230.0),
        mv(372.0, 292.0),
        wait(350),
        Step::Shot("sortable-drag"),
        release(),
        wait(350),
        Step::Shot("sortable-dropped"),
        // Esc mid-drag: everything slides back.
        Step::Press(at(grip(0.0).0, grip(0.0).1), left),
        mv(368.0, 200.0),
        mv(368.0, 300.0),
        wait(300),
        key(Named::Escape),
        release(),
        wait(400),
        Step::Shot("sortable-cancelled"),
        // Keyboard: focus row 0, Space, ↓ ↓, Enter.
        Step::Click(at(800.0, grip(0.0).1), left),
        key(Named::Space),
        key(Named::ArrowDown),
        key(Named::ArrowDown),
        wait(350),
        Step::Shot("sortable-kbd"),
        key(Named::Enter),
        wait(350),
        Step::Shot("sortable-kbd-dropped"),
        // Kanban: Research onto Build UI (In progress).
        Step::Press(at(475.0, 546.0), left),
        mv(478.0, 549.0),
        mv(600.0, 556.0),
        mv(747.0, 560.0),
        wait(350),
        Step::Shot("kanban-drag"),
        release(),
        wait(450),
        Step::Shot("kanban-dropped"),
        // Wireframes over Done's empty part, then Esc.
        Step::Press(at(475.0, 546.0), left),
        mv(480.0, 552.0),
        mv(800.0, 590.0),
        mv(1019.0, 610.0),
        wait(350),
        Step::Shot("kanban-over-column"),
        key(Named::Escape),
        release(),
        wait(450),
        Step::Shot("kanban-cancelled"),
        // Edge auto-scroll: drag a card up into the page's top band.
        Step::Press(at(1019.0, 546.0), left),
        mv(1019.0, 540.0),
        mv(1019.0, 300.0),
        mv(1019.0, 40.0),
        wait(600),
        Step::Shot("autoscroll"),
        key(Named::Escape),
        release(),
        wait(450),
        Step::Shot("end"),
    ]
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
