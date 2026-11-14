//! `RailTip` (§4.9): the collapsed-sidebar tooltip. A card-coloured bubble
//! (radius 6, lineStrong border, shadow lg, 12/16 text) 8 px to the right of
//! the anchor after 400 ms of hover. Built on iced's `tooltip` (no fade-in);
//! moves onto the Phase 2 overlay layer with the general Tooltip.

use std::time::Duration;

use iced::Padding;
use iced::widget::tooltip;

use crate::El;
use crate::ctx::Ctx;
use crate::label::label;
use crate::style::{c, shadows};
use crate::surface::{BoxStyle, surface};

pub fn rail_tip<'a, M: Clone + 'a>(cx: &Ctx, anchor: impl Into<El<'a, M>>, text: &str) -> El<'a, M> {
    let p = cx.p();
    let st = BoxStyle {
        bg: Some(c(p.card)),
        border: Some(c(cx.t.line_strong_on(p.card))),
        radius: 6.0,
        shadows: shadows(&p.shadow_lg),
        ..BoxStyle::default()
    };
    let bubble = surface(label(text.to_string(), 12.0, 16.0).color(c(p.text))).padding(Padding::from([5.0, 9.0])).fixed(st);
    tooltip(anchor, bubble, tooltip::Position::Right).gap(8).padding(0).delay(Duration::from_millis(400)).into()
}
