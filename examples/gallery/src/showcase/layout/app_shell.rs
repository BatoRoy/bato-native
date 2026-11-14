//! `layout/app-shell`: AppShell scaffold. Header, a 160 px nav sidebar,
//! footer, and a scrolling main region with an Action button (a toast).

use bato_kit::button::{Size, Variant, btn_label, button_style};
use bato_kit::iced::widget::{column, container, row};
use bato_kit::layout::{app_shell, rounded_clip};
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// The Action button, for the self-test.
pub const ACTION: &str = "layout/app-shell/action";

#[derive(Debug, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Msg {
    Action,
}

pub fn update(_state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Action => Some(Effect::Toast("AppShell action".into(), Tone::Info)),
    }
}

pub fn view<'a>(_state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    let header = container(label("Header", 14.0, 20.0).medium().color(c(p.text)))
        .padding([10, 16]);
    // `w-40 bg-app-bg border-r border-app-border p-3 text-sm text-app-muted space-y-2`
    let nav = |t: &str| label(t.to_string(), 14.0, 20.0).color(c(p.muted));
    let sidebar = row![
        container(column![nav("Nav item"), nav("Nav item")].spacing(8))
            .padding(12)
            .width(159)
            .height(Length::Fill),
        vline(c(p.border)),
    ];
    let sidebar = surface(sidebar)
        .height(Length::Fill)
        .fixed(BoxStyle::default().bg(c(p.bg)));
    let footer = container(label("Footer", 12.0, 16.0).color(c(p.muted))).padding([8, 16]);
    // `<Button variant="secondary" className="ml-1 !py-1 !px-2">`: h-9,
    // px 8 inside the 1 px border.
    let action = surface(btn_label("Action", Size::Md))
        .height(36)
        .padding([0, 9])
        .center_y()
        .on_press(Msg::Action)
        .style(button_style(&on, Variant::Secondary, false))
        .focus_ring(Ring::focus(&on));
    let main = container(
        row![
            label("Scrollable content region.", 14.0, 20.0).color(c(p.subtext)),
            // The space before the button, then `ml-1`.
            gap(8.0),
            container(action).id(ACTION),
        ]
        .align_y(Alignment::Center),
    )
    .padding(16);
    let shell = app_shell(&on, main)
        .header(header)
        .sidebar(sidebar)
        .footer(footer);
    card(cx, "AppShell scaffold")
        .body(
            rounded_clip(8.0, c(on.over), shell)
                .border(c(p.border))
                .height(224),
        )
        .into()
}
