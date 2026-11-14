//! `feedback/alerts`: Alerts. The four tones; the warning is closable (a
//! toast says so).

use bato_kit::feedback::alert;
use bato_kit::iced::widget::{column, container};
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// The close button's container, for the self-test.
pub const CLOSE: &str = "feedback/alerts/warning";

#[derive(Debug, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Msg {
    Close,
}

pub fn update(_state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Close => Some(Effect::Toast("Dismissed".into(), Tone::Info)),
    }
}

pub fn view<'a>(_state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    card(cx, "Alerts")
        .body(
            column![
                alert(&on, Tone::Info, "Heads up").body("This is an informational message."),
                alert(&on, Tone::Success, "Saved"),
                container(
                    alert(&on, Tone::Warning, "Careful")
                        .body("This action may have consequences.")
                        .on_close(Msg::Close)
                )
                .id(CLOSE),
                alert(&on, Tone::Error, "Failed to save"),
            ]
            .spacing(8),
        )
        .into()
}
