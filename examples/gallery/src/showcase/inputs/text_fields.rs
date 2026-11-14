//! `inputs/text-fields`: seven TextFields in a 2-column grid
//! (`grid grid-cols-2 gap-4`), as `InputsSection` builds them.

use bato_kit::inputs::text_field::text_field;
use bato_kit::prelude::*;

use crate::showcase::Effect;

#[derive(Debug, Default)]
pub struct State {
    search: String,
    email: String,
    pwd: String,
    reveal: bool,
    count: String,
    amount: String,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Search(String),
    Email(String),
    Pwd(String),
    Reveal(bool),
    Count(String),
    Amount(String),
    /// "With error" has `onChange={() => {}}`.
    Ignore,
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Search(s) => state.search = s,
        Msg::Email(s) => state.email = s,
        Msg::Pwd(s) => state.pwd = s,
        Msg::Reveal(on) => state.reveal = on,
        Msg::Count(s) => state.count = s,
        Msg::Amount(s) => state.amount = s,
        Msg::Ignore => {}
    }
    None
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let body = Grid::columns(2)
        .gap(16.0)
        .push(
            text_field(&on, &state.search, Msg::Search)
                .label("Search")
                .left_icon(Ico::Search)
                .placeholder("Search…")
                .clearable(Msg::Search(String::new()))
                .id("b1:search"),
        )
        .push(
            text_field(&on, &state.email, Msg::Email)
                .label("Email")
                .left_icon(Ico::Mail)
                .placeholder("you@example.com"),
        )
        .push(
            text_field(&on, &state.pwd, Msg::Pwd)
                .label("Password")
                .password(state.reveal, Msg::Reveal)
                .placeholder("••••••••")
                .id("b1:password"),
        )
        .push(
            text_field(&on, "bad value", |_| Msg::Ignore)
                .label("With error")
                .error(Some("That doesn't look right")),
        )
        .push(
            text_field(&on, &state.count, Msg::Count)
                .label("Char count")
                .max_len(20)
                .show_count(true)
                .placeholder("Max 20"),
        )
        .push(
            text_field(&on, &state.amount, Msg::Amount)
                .label("Prefix / suffix")
                .prefix("$")
                .suffix("USD")
                .placeholder("0.00"),
        )
        .push(
            text_field(&on, "you@example.com", |_| Msg::Ignore)
                .label("Disabled")
                .left_icon(Ico::Mail)
                .disabled(true),
        );
    card(cx, "Text fields").body(body).into()
}

// ── BATO_SELFTEST=inputs:<card> (batch B1) ──────────────────────────────

use std::convert::Infallible;
use std::time::Duration;

use bato_kit::iced::keyboard::{Key, Modifiers, key::Named};
use bato_kit::iced::{Point, mouse};
use bato_kit::overlay::script::{Step, Target, key_events};

/// A left click at a window point (the card scrolled to the top with
/// `BATO_VIEW=…&card=<card>`; points measured on the Electron reference).
pub(crate) fn click(x: f32, y: f32) -> Step<Infallible> {
    Step::Click(Target::At(Point::new(x, y)), mouse::Button::Left)
}

pub(crate) fn typed(s: &str) -> Vec<Step<Infallible>> {
    s.chars().map(|c| Step::Char(c, Modifiers::default())).collect()
}

pub(crate) fn key(k: Named) -> Step<Infallible> {
    Step::Key(k, Modifiers::default())
}

/// Hold a key for `ms` (OS auto-repeat is not simulated).
pub(crate) fn hold_key(k: Named, ms: u64) -> Vec<Step<Infallible>> {
    let [down, up] = key_events(Key::Named(k), Modifiers::default());
    vec![
        Step::Event(down),
        Step::Wait(Duration::from_millis(ms)),
        Step::Event(up),
    ]
}

/// Hold the left button at a point for `ms`.
pub(crate) fn hold(x: f32, y: f32, ms: u64) -> Vec<Step<Infallible>> {
    vec![
        Step::Press(Target::At(Point::new(x, y)), mouse::Button::Left),
        Step::Wait(Duration::from_millis(ms)),
        Step::Event(bato_kit::iced::Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left,
        ))),
    ]
}

/// The B1 self-tests, one per card (`inputs:text-fields`, …).
pub fn b1_selftest(card: &str) -> Option<Vec<Step<Infallible>>> {
    let steps = match card {
        "text-fields" => selftest(),
        "specialized" => super::specialized::selftest(),
        "sliders-color" => super::sliders_color::selftest(),
        "time-input" => super::time_input::selftest(),
        "toggles-upload" => super::toggles_upload::selftest(),
        "numpad" => super::specialized::numpad_selftest(),
        _ => return None,
    };
    Some(steps)
}

/// Search: type, the clear button appears; Password: type, reveal; back
/// in Search (halo); clear.
fn selftest() -> Vec<Step<Infallible>> {
    let mut s = vec![Step::Shot("rest"), click(500.0, 122.0)];
    s.extend(typed("hello"));
    s.push(click(500.0, 198.0));
    s.extend(typed("secret"));
    s.push(click(724.0, 198.0));
    s.push(click(500.0, 122.0));
    s.extend(typed("!"));
    s.push(click(500.0, 296.0));
    s.extend(typed("abcdefghijklmnopqrstuvwxyz"));
    s.push(click(500.0, 122.0));
    s.push(Step::Wait(Duration::from_millis(300)));
    s.push(Step::Shot("typed-revealed-limited"));
    s.push(click(724.0, 122.0));
    s.push(key(Named::Tab));
    s.push(Step::Wait(Duration::from_millis(300)));
    s.push(Step::Shot("cleared-tab"));
    s
}
