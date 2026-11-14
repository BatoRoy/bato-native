//! `feedback/navigation`: Navigation & controls. Breadcrumbs, a segmented
//! control (List / Grid / Board), an avatar and an avatar group of five
//! (four shown, `+1`).

use bato_kit::feedback::{AvatarSize, Crumb, avatar, avatar_group, breadcrumbs};
use bato_kit::iced::widget::{column, container, row};
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// The "Library" crumb's container, for the self-test.
pub const LIBRARY: &str = "feedback/navigation/library";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    List,
    Grid,
    Board,
}

#[derive(Debug, Default)]
pub struct State {
    pub seg: View,
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// A crumb with React's `onClick: () => {}`.
    Crumb,
    Seg(View),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Crumb => {}
        Msg::Seg(v) => state.seg = v,
    }
    None
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let crumbs = container(breadcrumbs(
        &on,
        [
            Crumb::new("Home").on_press(Msg::Crumb),
            Crumb::new("Library").on_press(Msg::Crumb),
            Crumb::new("Item"),
        ],
    ))
    .id(LIBRARY);
    let seg = segmented(
        &on,
        state.seg,
        [
            Segment::new(View::List, "List"),
            Segment::new(View::Grid, "Grid"),
            Segment::new(View::Board, "Board"),
        ],
        Msg::Seg,
    );
    let avatars = row![
        avatar(&on, "Ada Lovelace", AvatarSize::Md),
        avatar_group(
            &on,
            &["Ada L", "Bob K", "Cy R", "Dan P", "Eve M"],
            4,
            AvatarSize::Md
        ),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    card(cx, "Navigation & controls")
        .body(column![crumbs, seg, avatars].spacing(16))
        .into()
}
