//! `inputs/sliders-color`: Slider, RangeSlider, VolumeControl and the
//! ColorPicker (`space-y-5`; each slider under a 12 px `subtext` caption
//! with `mb-2`).
//!
//! The ColorPicker belongs to batch B2: see [`color_picker_slot`].

use bato_kit::iced::widget::column;
use bato_kit::inputs::slider::{Volume, VolumeEvent, range_slider, slider, volume_control};
use bato_kit::prelude::*;

use crate::showcase::Effect;

#[derive(Debug)]
pub struct State {
    vol: f64,
    range: (f64, f64),
    media: Volume,
    media_vol: f64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            vol: 60.0,
            range: (20.0, 80.0),
            media: Volume::default(),
            media_vol: 40.0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Vol(f64),
    Range((f64, f64)),
    Media(VolumeEvent),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Vol(v) => state.vol = v,
        Msg::Range(r) => state.range = r,
        Msg::Media(ev) => state.media.update(ev, &mut state.media_vol, 0.0, 100.0),
    }
    None
}

fn caption<'a>(cx: &Ctx, text: String, control: El<'a, Msg>) -> El<'a, Msg> {
    column![
        label(text, 12.0, 16.0).color(c(cx.p().subtext)),
        vgap(8.0),
        control
    ]
    .into()
}

// ── Slot: ColorPicker (batch B2) ────────────────────────────────────────
//
// B2 replaces the body of this function with the real control, e.g.
//     color_picker(cx, &state.color, Msg::Color).label("Color")
// with `color: String` = "#7c3aed" added to `State` (React's
// `useState('#7c3aed')`). Until then it is a neutral box of the same size:
// the Field label (16) + gap (6) + the 38 px trigger = 60 px, full width.
fn color_picker_slot<'a>(cx: &Ctx) -> El<'a, Msg> {
    container(
        label("ColorPicker: batch B2", 12.0, 16.0).color(c(cx.p().muted)),
    )
    .width(Length::Fill)
    .height(60)
    .center_y(60)
    .into()
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let (lo, hi) = state.range;
    let body = column![
        caption(
            &on,
            format!("Volume: {}", state.vol),
            container(slider(&on, state.vol, Msg::Vol)).id("b1:slider").into(),
        ),
        caption(
            &on,
            format!("Range: {lo}–{hi}"),
            container(range_slider(&on, state.range, Msg::Range))
                .id("b1:range")
                .into(),
        ),
        caption(
            &on,
            "Volume control".into(),
            container(
                volume_control(&on, &state.media, state.media_vol, Msg::Media).show_value(true),
            )
            .id("b1:volume")
            .into(),
        ),
        color_picker_slot(&on),
    ]
    .spacing(20);
    card(cx, "Sliders & color").body(body).into()
}

use std::convert::Infallible;
use std::time::Duration;

use bato_kit::iced::{Event, Point, mouse};
use bato_kit::overlay::script::{Step, Target};

use super::text_fields::click;

/// Slider: click at 25 %, drag to ~50 %; Range: drag the low thumb past
/// the high one (stops at it); Volume: mute.
pub(crate) fn selftest() -> Vec<Step<Infallible>> {
    let left = mouse::Button::Left;
    let at = |x: f32, y: f32| Target::At(Point::new(x, y));
    let release = Step::Event(Event::Mouse(mouse::Event::ButtonReleased(left)));
    vec![
        click(548.5, 115.0),
        Step::Move(at(830.0, 115.0)),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("slider-25-hover"),
        Step::Press(at(548.5, 115.0), left),
        Step::Move(at(650.0, 115.0)),
        Step::Move(at(750.0, 117.0)),
        release.clone(),
        Step::Press(at(508.0, 179.0), left),
        Step::Move(at(900.0, 179.0)),
        Step::Move(at(1100.0, 179.0)),
        release,
        click(361.0, 248.0),
        Step::Move(at(1200.0, 400.0)),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("dragged-muted"),
        click(361.0, 248.0),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("unmuted"),
    ]
}
