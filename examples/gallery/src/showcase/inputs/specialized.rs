//! `inputs/specialized`: NumberInput, MaskedInput, CurrencyInput, the
//! verification code (OtpInput under a plain `<label>`), TagsInput
//! (`col-span-2`) and the autosizing TextArea, in `grid grid-cols-2 gap-4`.

use bato_kit::iced::widget::{column, text_editor};
use bato_kit::inputs::number_input::number_input;
use bato_kit::inputs::otp_input::otp_input;
use bato_kit::inputs::tags_input::{TagsEdit, TagsEvent, tags_input};
use bato_kit::inputs::text_area::{self, text_area};
use bato_kit::inputs::text_field::{CurrencyEdit, CurrencyEvent, currency_input, masked_input};
use bato_kit::prelude::*;

use crate::showcase::Effect;

#[derive(Debug)]
pub struct State {
    num: f64,
    phone: String,
    price: Option<f64>,
    price_edit: CurrencyEdit,
    otp: String,
    tags: Vec<String>,
    tags_edit: TagsEdit,
    note: text_editor::Content,
}

impl Default for State {
    fn default() -> Self {
        Self {
            num: 3.0,
            phone: String::new(),
            price: Some(1999.0),
            price_edit: CurrencyEdit::new("USD"),
            otp: String::new(),
            tags: vec!["react".into(), "electron".into()],
            tags_edit: TagsEdit::default(),
            note: text_editor::Content::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Num(f64),
    Phone(String),
    Price(CurrencyEvent),
    Otp(String),
    Tags(TagsEvent),
    Note(text_editor::Action),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Num(n) => state.num = n,
        Msg::Phone(s) => state.phone = s,
        Msg::Price(ev) => {
            if let Some(v) = state.price_edit.update(ev, state.price) {
                state.price = v;
            }
        }
        Msg::Otp(s) => state.otp = s,
        Msg::Tags(ev) => state.tags_edit.update(ev, &mut state.tags),
        Msg::Note(a) => text_area::apply(&mut state.note, a, None),
    }
    None
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    // `<div><label class="text-xs font-medium">…</label><div class="mt-1">`:
    // the label sits on the 24 px line of the 16 px body font.
    let otp = column![
        container(
            label("Verification code", 12.0, 16.0)
                .medium()
                .color(c(p.subtext))
        )
        .height(24)
        .padding(Padding::default().top(5.5)),
        vgap(4.0),
        container(otp_input(&on, &state.otp, Msg::Otp).length(6)).id("b1:otp"),
    ];
    let body = Grid::columns(2)
        .gap(16.0)
        .push(container(
            number_input(&on, state.num, Msg::Num)
                .min(0.0)
                .max(10.0)
                .label("Quantity"),
        ).id("b1:qty"))
        .push(
            masked_input(&on, "(###) ###-####", &state.phone, Msg::Phone)
                .label("Phone")
                .placeholder("(555) 123-4567")
                .id("b1:phone"),
        )
        .push(
            currency_input(&on, &state.price_edit, state.price, Msg::Price)
                .label("Price")
                .id("b1:price"),
        )
        .push(otp)
        .push_span(
            container(
                tags_input(&on, &state.tags_edit, &state.tags, Msg::Tags).label("Tags"),
            )
            .id("b1:tags"),
            2,
        )
        .push(container(
            text_area(&on, &state.note, Msg::Note)
                .label("Notes (autosize)")
                .autosize(true)
                .placeholder("Type a lot…"),
        ).id("b1:notes"));
    card(cx, "Specialized inputs").body(body).into()
}

use std::convert::Infallible;
use std::time::Duration;

use bato_kit::iced::keyboard::key::Named;
use bato_kit::overlay::script::Step;

use super::text_fields::{click, hold, key, typed};

/// Quantity: +, then hold + to the max; Phone masks; Price shows the raw
/// number while focused; OTP advances; Tags add and remove; Notes grow.
pub(crate) fn selftest() -> Vec<Step<Infallible>> {
    let mut s = vec![click(727.0, 122.0)];
    s.extend(hold(727.0, 122.0, 1200));
    s.push(click(955.0, 122.0));
    s.extend(typed("5551234567"));
    s.push(click(544.0, 198.0));
    s.push(Step::Wait(Duration::from_millis(300)));
    s.push(Step::Shot("qty-phone-price"));
    s.push(click(778.0, 207.0));
    s.extend(typed("123"));
    s.push(click(800.0, 286.0));
    s.extend(typed("vue"));
    s.push(key(Named::Enter));
    s.extend(typed("svelte,"));
    s.push(key(Named::Backspace));
    s.push(click(544.0, 383.0));
    for (i, line) in ["one", "two", "three", "four", "five"].iter().enumerate() {
        if i > 0 {
            s.push(key(Named::Enter));
        }
        s.extend(typed(line));
    }
    s.push(click(800.0, 286.0));
    s.extend(typed("draft"));
    s.push(Step::Wait(Duration::from_millis(300)));
    s.push(Step::Shot("otp-tags-notes"));
    // Blur commits the draft.
    s.push(click(1100.0, 60.0));
    s.push(key(Named::ArrowUp));
    s.push(Step::Wait(Duration::from_millis(300)));
    s.push(Step::Shot("tags-committed"));
    s
}
