//! `layout/responsive`: Responsive (resize the window). An AutoGrid of six
//! boxes (min 140 px) and a three-column Masonry of six cards.

use bato_kit::iced::widget::column;
use bato_kit::layout::masonry;
use bato_kit::prelude::*;

use super::primitives::demo_box;
use crate::showcase::Effect;

#[derive(Debug, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Msg {}

pub fn update(_state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {}
}

pub fn view<'a>(_state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    let caption = |t: &str| label(t.to_string(), 12.0, 16.0).color(c(p.muted));
    let auto = (1..=6).fold(Grid::auto_fill(140.0).gap(12.0), |g, i| {
        g.push(demo_box(&on, format!("#{i}"), true))
    });
    let cards = [6.0, 12.0, 8.0, 16.0, 10.0, 7.0]
        .iter()
        .enumerate()
        .map(|(i, h)| {
            surface(label(format!("card {}", i + 1), 12.0, 16.0).color(c(p.subtext)))
                .width(Length::Fill)
                .height(h * 8.0)
                .padding(9)
                .fixed(
                    BoxStyle::default()
                        .bg(c(p.card))
                        .border(c(p.border))
                        .radius(6.0),
                )
                .into()
        })
        .collect();
    card(cx, "Responsive (resize the window)")
        .body(
            column![
                caption("AutoGrid"),
                auto,
                caption("Masonry"),
                masonry(3, 12.0, cards),
            ]
            .spacing(16),
        )
        .into()
}
