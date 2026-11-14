//! `data/timeline`: Timeline. Five items in three lanes around today
//! (Design: Mockups, Polish; Build: Components, Integration; QA: Test plan),
//! 1100 px wide and scrolling horizontally while the card's "Horizontal
//! scroll" Switch is on.

use bato_kit::data::{TimelineItem, timeline};
use bato_kit::prelude::*;
use bato_kit::data::timeline::chrono::{Duration, NaiveDateTime};

use crate::showcase::Effect;

/// The timeline's horizontal scroller, for the self-test.
pub const SCROLLER: &str = "data/timeline/scroller";

#[derive(Debug)]
pub struct State {
    pub scroll: bool,
    /// `new Date()` at mount: the items are placed around it.
    pub today: NaiveDateTime,
}

impl Default for State {
    fn default() -> Self {
        Self {
            scroll: true,
            today: bato_kit::datefmt::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Scroll(bool),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Scroll(on) => state.scroll = on,
    }
    None
}

/// `tl(offset, len, lane, label)` of the Showcase.
pub fn items(today: NaiveDateTime) -> Vec<TimelineItem> {
    let tl = |offset: i64, len: i64, lane: &str, label: &str| {
        let start = today + Duration::days(offset);
        TimelineItem {
            id: format!("{lane}-{label}"),
            label: label.into(),
            start,
            end: start + Duration::days(len),
            lane: Some(lane.into()),
            color: None,
        }
    };
    vec![
        tl(-2, 4, "Design", "Mockups"),
        tl(2, 5, "Design", "Polish"),
        tl(0, 6, "Build", "Components"),
        tl(6, 4, "Build", "Integration"),
        tl(1, 3, "QA", "Test plan"),
    ]
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let sw = switch(&on, state.scroll, Msg::Scroll).label("Horizontal scroll");
    card(cx, "Timeline")
        .action(sw)
        .body(
            timeline(&on, items(state.today))
                .scrollable(state.scroll)
                .min_width(1100.0)
                .now(bato_kit::datefmt::now())
                .id(SCROLLER),
        )
        .into()
}
