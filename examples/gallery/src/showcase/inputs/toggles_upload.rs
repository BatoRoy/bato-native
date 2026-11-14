//! `inputs/toggles-upload`: Switch, Checkbox, indeterminate Checkbox,
//! RadioGroup, the native-input row (index.css theming), the disabled
//! states, and the FileDropzone (`space-y-4`).
//!
//! The FileDropzone belongs to batch B3: see [`file_dropzone_slot`].

use bato_kit::iced::widget::{column, row};
use bato_kit::inputs::toggles::{native_checkbox, native_radio, native_radio_group, radio_group};
use bato_kit::prelude::*;

use crate::showcase::Effect;

#[derive(Debug)]
pub struct State {
    sw: bool,
    cb: bool,
    radio: char,
    /// The native checkboxes: checked, unchecked, indeterminate.
    native: [bool; 3],
    /// `el.indeterminate = true` until the first click.
    native_mixed: bool,
    /// The native radio group: 0 = "radio on".
    native_radio: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            sw: true,
            cb: false,
            radio: 'a',
            native: [true, false, false],
            native_mixed: true,
            native_radio: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Sw(bool),
    Cb(bool),
    Radio(char),
    Native(usize, bool),
    NativeRadio(usize),
    /// The inert demo controls.
    Ignore,
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Sw(on) => state.sw = on,
        Msg::Cb(on) => state.cb = on,
        Msg::Radio(r) => state.radio = r,
        Msg::Native(i, on) => {
            state.native[i] = on;
            if i == 2 {
                state.native_mixed = false;
            }
        }
        Msg::NativeRadio(i) => state.native_radio = i,
        Msg::Ignore => {}
    }
    None
}

/// `Divider label=…`: `flex items-center gap-3`, 1 px `border` lines and a
/// 12 px `muted` label.
fn divider<'a>(cx: &Ctx, text: &str) -> El<'a, Msg> {
    let p = cx.p();
    // The 1 px lines centre at 7.5 px on the 16 px row: Chromium puts them
    // at 8.
    let line = || column![vgap(8.0), hline(c(p.border))].width(Length::Fill);
    row![
        line(),
        label(text.to_string(), 12.0, 16.0).color(c(p.muted)),
        line(),
    ]
    .spacing(12)
    .into()
}

// ── Slot: FileDropzone (batch B3) ───────────────────────────────────────
//
// B3 replaces the body of this function with the real control, e.g.
//     file_dropzone(cx).hint("PNG, JPG up to 10MB")
//         .on_files(|f| Msg::Files(f.len())).on_paths(|p| Msg::Paths(p.len()))
// whose callbacks toast "N file(s) selected" / "N path(s) selected"
// (`Effect::Toast(…, Tone::Success)`). Until then it is a neutral box of the
// same size: 2 px borders + py-8 + the 40 px icon + gap 8 + 20 px text +
// gap 8 + the 16 px hint = 160 px, full width.
fn file_dropzone_slot<'a>(cx: &Ctx) -> El<'a, Msg> {
    container(label("FileDropzone: batch B3", 12.0, 16.0).color(c(cx.p().muted)))
        .width(Length::Fill)
        .height(160)
        .center_x(Length::Fill)
        .center_y(160)
        .into()
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let options = |n: usize| {
        ['a', 'b', 'c']
            .into_iter()
            .take(n)
            .map(|v| (v, format!("Option {}", v.to_ascii_uppercase())))
            .collect::<Vec<_>>()
    };
    let native = row![
        native_checkbox(&on, state.native[0], |v| Msg::Native(0, v)).label("checked"),
        native_checkbox(&on, state.native[1], |v| Msg::Native(1, v)).label("unchecked"),
        native_checkbox(&on, state.native[2], |v| Msg::Native(2, v))
            .indeterminate(state.native_mixed)
            .label("indeterminate"),
        native_radio_group(
            vec![
                native_radio(&on, state.native_radio == 0, Msg::NativeRadio(0))
                    .label("radio on")
                    .into(),
                native_radio(&on, state.native_radio == 1, Msg::NativeRadio(1))
                    .label("radio off")
                    .into(),
            ],
            24.0,
            Msg::NativeRadio,
        ),
    ]
    .spacing(24)
    .align_y(Alignment::Center);
    let disabled = row![
        column![
            switch(&on, true, |_| Msg::Ignore).label("On, disabled").disabled(true),
            switch(&on, false, |_| Msg::Ignore).label("Off, disabled").disabled(true),
        ]
        .spacing(12),
        column![
            checkbox(&on, true, |_| Msg::Ignore).label("Checked, disabled").disabled(true),
            checkbox(&on, false, |_| Msg::Ignore).label("Unchecked, disabled").disabled(true),
        ]
        .spacing(12),
        radio_group(&on, 'b', options(2), |_| Msg::Ignore).disabled(true),
    ]
    .spacing(40);
    let body = column![
        container(switch(&on, state.sw, Msg::Sw).label("Enable notifications")).id("b1:switch"),
        container(checkbox(&on, state.cb, Msg::Cb).label("I agree to the terms")).id("b1:checkbox"),
        checkbox(&on, false, |_| Msg::Ignore)
            .indeterminate(true)
            .label("Indeterminate (partial selection)"),
        container(radio_group(&on, state.radio, options(3), Msg::Radio)).id("b1:radio"),
        divider(&on, "native inputs — themed by index.css"),
        container(native).id("b1:native"),
        divider(&on, "disabled"),
        disabled,
        file_dropzone_slot(&on),
    ]
    .spacing(16);
    card(cx, "Toggles & file upload").body(body).into()
}

use std::convert::Infallible;
use std::time::Duration;

use bato_kit::iced::keyboard::{Modifiers, key::Named};
use bato_kit::overlay::script::Step;

use super::text_fields::{click, key};

/// Labels toggle (switch, checkbox), Option C, the native indeterminate
/// and radio; ← roves the native radios with the focus ring.
pub(crate) fn selftest() -> Vec<Step<Infallible>> {
    vec![
        click(457.0, 237.0),
        click(437.0, 273.0),
        click(400.0, 401.0),
        click(647.0, 469.0),
        click(874.0, 469.0),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("clicked"),
        key(Named::ArrowLeft),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("native-radio-roved"),
        Step::Key(Named::Tab, Modifiers::SHIFT),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("shift-tab-ring"),
    ]
}
