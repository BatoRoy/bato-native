//! `layout/primitives`: Primitives. HStack, a 4-column Grid, a row with a
//! vertical Divider, Center and a 16:9 AspectRatio, a labelled Divider and
//! a small Container.

use bato_kit::iced::widget::{column, container};
use bato_kit::layout::{
    ContainerSize, aspect_ratio, center, divider_label, divider_v, hstack, sized_container,
};
use bato_kit::prelude::*;

use crate::showcase::Effect;

#[derive(Debug, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Msg {}

pub fn update(_state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {}
}

/// The demo box: `flex items-center justify-center rounded-md
/// bg-app-accent/15 text-app-accentBright text-sm py-3 px-4`.
pub fn demo_box<'a, M: Clone + 'a>(cx: &Ctx, text: impl Into<String>, fill: bool) -> El<'a, M> {
    let p = cx.p();
    let s = surface(label(text, 14.0, 20.0).color(c(p.accent_bright)))
        .padding([12, 16])
        .center()
        .fixed(BoxStyle::default().bg(c(cx.tint(0.15))).radius(6.0));
    if fill { s.width(Length::Fill).into() } else { s.into() }
}

/// `rounded-md bg-app-text/[0.06]` with a centred 12/16 subtext label.
fn tile<'a>(cx: &Ctx, text: &str, w: f32, h: Option<f32>) -> El<'a, Msg> {
    let p = cx.p();
    let inner = center(label(text.to_string(), 12.0, 16.0).color(c(p.subtext)));
    let s = surface(inner)
        .width(w)
        .fixed(BoxStyle::default().bg(c(cx.wash(0.06))).radius(6.0));
    match h {
        Some(h) => s.height(h).into(),
        None => s.height(Length::Fill).into(),
    }
}

pub fn view<'a>(_state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    let hs = hstack(
        2.0,
        vec![
            demo_box(&on, "H", false),
            demo_box(&on, "Stack", false),
            demo_box(&on, "row", false),
        ],
    );
    let g = (1..=4).fold(Grid::columns(4).gap(8.0), |g, i| {
        g.push(demo_box(&on, i.to_string(), true))
    });
    // AspectRatio fills its parent's width, so size the wrapper (w-28).
    let ratio = container(aspect_ratio(16.0 / 9.0, tile(&on, "16:9", 112.0, None))).width(112);
    let stack = hstack(
        3.0,
        vec![
            label("Divider", 14.0, 20.0).color(c(p.subtext)).into(),
            divider_v(&on),
            tile(&on, "Center", 80.0, Some(40.0)),
            ratio.into(),
        ],
    )
    .align_y(Alignment::Center);
    let contained = sized_container(
        ContainerSize::Sm,
        false,
        container(
            label("Container (max-width, centered)", 12.0, 16.0).color(c(p.muted)),
        )
        .center_x(Length::Fill),
    );
    card(cx, "Primitives")
        .body(
            column![hs, g, stack, divider_label(&on, "section"), contained].spacing(16),
        )
        .into()
}
