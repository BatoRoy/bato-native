//! `overlays/context-menu`: Context menu. A dashed area ("Right-click
//! here"); a right press anywhere in it opens the menu at the pointer: Copy
//! (⌘C), Cut, Paste (disabled), a separator, Delete (danger). Each item
//! toasts like React's.

use bato_kit::Tone;
use bato_kit::iced::Point;
use bato_kit::iced::widget::container;
use bato_kit::overlay::context_menu::{ContextMenuState, context_menu, dashed};
use bato_kit::prelude::*;

use crate::showcase::Effect;

#[derive(Debug, Default)]
pub struct State {
    pub menu: ContextMenuState,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Open(Point),
    Close,
    Toast(&'static str, Tone),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Open(p) => state.menu.open(p),
        Msg::Close => state.menu.close(),
        Msg::Toast(text, tone) => return Some(Effect::Toast(text.into(), tone)),
    }
    None
}

/// Self-test target.
pub const AREA: &str = "context-area";

fn items() -> Vec<MenuEntry<Msg>> {
    vec![
        action("Copy", Msg::Toast("Copied", Tone::Success))
            .icon(Ico::Copy)
            .shortcut("⌘C")
            .into(),
        action("Cut", Msg::Toast("Cut", Tone::Info))
            .icon(Ico::Scissors)
            .into(),
        action("Paste", Msg::Close)
            .icon(Ico::Clipboard)
            .disabled(true)
            .into(),
        separator(),
        action("Delete", Msg::Toast("Deleted", Tone::Error))
            .icon(Ico::Trash2)
            .danger()
            .into(),
    ]
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    let area = dashed(
        container(label("Right-click here", 14.0, 20.0).color(c(p.muted)))
            .center_x(Length::Fill)
            .center_y(96),
        c(p.border),
    );
    let menu = context_menu(
        cx,
        container(area).id(AREA),
        state.menu.position(),
        items(),
        Msg::Open,
        Msg::Close,
    );
    card(cx, "Context menu").body(menu).into()
}
