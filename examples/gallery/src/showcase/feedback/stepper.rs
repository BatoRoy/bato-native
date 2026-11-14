//! `feedback/stepper`: Stepper. Account / Profile / Done with clickable
//! steps, and Back / Next buttons (clamped to 0..=2).

use bato_kit::feedback::{Step, stepper};
use bato_kit::iced::widget::{column, container, row};
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// Self-test anchors.
pub const STEPS: &str = "feedback/stepper/steps";
pub const BACK: &str = "feedback/stepper/back";
pub const NEXT: &str = "feedback/stepper/next";

#[derive(Debug)]
pub struct State {
    pub step: usize,
}

impl Default for State {
    /// React's `useState(1)`.
    fn default() -> Self {
        Self { step: 1 }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Go(usize),
    Back,
    Next,
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    state.step = match msg {
        Msg::Go(i) => i,
        Msg::Back => state.step.saturating_sub(1),
        Msg::Next => (state.step + 1).min(2),
    };
    None
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let steps = stepper(
        &on,
        [Step::new("Account"), Step::new("Profile"), Step::new("Done")],
        state.step,
    )
    .on_step(Msg::Go);
    let buttons = row![
        container(button(&on).secondary().label("Back").on_press(Msg::Back)).id(BACK),
        container(button(&on).label("Next").on_press(Msg::Next)).id(NEXT),
    ]
    .spacing(8);
    card(cx, "Stepper")
        .body(column![container(steps).id(STEPS), vgap(16.0), buttons])
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_and_next_clamp() {
        let mut s = State::default();
        update(&mut s, Msg::Next);
        update(&mut s, Msg::Next);
        assert_eq!(s.step, 2);
        update(&mut s, Msg::Go(0));
        update(&mut s, Msg::Back);
        assert_eq!(s.step, 0);
    }
}
