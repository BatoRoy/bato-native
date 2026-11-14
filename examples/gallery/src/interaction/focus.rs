//! Interaction lab section: Focus and keyboard (Phase 2, INF-2).
//!
//! - **Tab order**: a row of every focusable control kind in document order;
//!   Tab / Shift+Tab walk it, Enter / Space activate, rings show only after
//!   keyboard input.
//! - **Programmatic focus**: focus a field by id (`focus::focus`), blur.
//! - **Roving** (opt-in): one Tab stop, arrows move the selection.
//! - **Focus scope**: a panel that traps Tab, focuses its first control when
//!   it opens and gives focus back to its opener when it closes (what a Modal
//!   level gets).
//!
//! `BATO_SELFTEST=focus:focus=focus-primary,tab*2` (see `bato_kit::focus`)
//! drives it from the keyboard path; ids: `focus-primary`, `focus-name`,
//! `focus-open`.

use bato_kit::focus::{self, focus_scope};
use bato_kit::iced::Task;
use bato_kit::iced::widget::{column, row};
use bato_kit::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    List,
    Grid,
    Board,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Density {
    Compact,
    #[default]
    Cozy,
    Roomy,
}

#[derive(Debug)]
pub struct State {
    last: String,
    agree: bool,
    notify: bool,
    view: View,
    density: Density,
    name: String,
    scope_open: bool,
    scope_name: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            last: "Nothing yet".into(),
            agree: false,
            notify: true,
            view: View::List,
            density: Density::Cozy,
            name: String::new(),
            scope_open: false,
            scope_name: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Pressed(&'static str),
    Agree(bool),
    Notify(bool),
    View(View),
    Density(Density),
    Name(String),
    FocusName,
    Blur,
    OpenScope,
    CloseScope,
    ScopeName(String),
}

pub fn update(state: &mut State, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Pressed(what) => state.last = format!("{what} pressed"),
        Msg::Agree(v) => {
            state.agree = v;
            state.last = format!("Agree → {v}");
        }
        Msg::Notify(v) => {
            state.notify = v;
            state.last = format!("Switch → {v}");
        }
        Msg::View(v) => {
            state.view = v;
            state.last = format!("View → {v:?}");
        }
        Msg::Density(d) => {
            state.density = d;
            state.last = format!("Density → {d:?}");
        }
        Msg::Name(n) => state.name = n,
        Msg::FocusName => return focus::focus("focus-name"),
        Msg::Blur => return focus::blur(),
        Msg::OpenScope => state.scope_open = true,
        Msg::CloseScope => {
            state.scope_open = false;
            state.last = "Scope closed".into();
        }
        Msg::ScopeName(n) => state.scope_name = n,
    }
    Task::none()
}

fn caption<'a>(cx: &Ctx, text: &'a str) -> El<'a, Msg> {
    label(text, 12.0, 16.0)
        .medium()
        .color(c(cx.p().muted))
        .into()
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();

    let controls = row![
        button(&on)
            .label("Primary")
            .id("focus-primary")
            .on_press(Msg::Pressed("Primary")),
        button(&on)
            .secondary()
            .label("Secondary")
            .on_press(Msg::Pressed("Secondary")),
        button(&on)
            .ghost()
            .label("Ghost")
            .on_press(Msg::Pressed("Ghost")),
        button(&on).secondary().label("Disabled").disabled(true),
        switch(&on, state.notify, Msg::Notify),
        checkbox(&on, state.agree, Msg::Agree).label("Agree"),
        segmented(
            &on,
            state.view,
            [
                Segment::new(View::List, "List"),
                Segment::new(View::Grid, "Grid"),
                Segment::new(View::Board, "Board"),
            ],
            Msg::View,
        ),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let program = row![
        container(input(&on, "Type something…", &state.name, Msg::Name).id("focus-name"))
            .width(260),
        button(&on)
            .secondary()
            .sm()
            .label("Focus the field")
            .on_press(Msg::FocusName),
        button(&on).ghost().sm().label("Blur").on_press(Msg::Blur),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let rove = row![
        segmented(
            &on,
            state.density,
            [
                Segment::new(Density::Compact, "Compact"),
                Segment::new(Density::Cozy, "Cozy"),
                Segment::new(Density::Roomy, "Roomy"),
            ],
            Msg::Density,
        )
        .roving(),
        label("One Tab stop; ← → move the selection.", 13.0, 19.5).color(c(p.subtext)),
    ]
    .spacing(16)
    .align_y(Alignment::Center);

    let opener = button(&on)
        .secondary()
        .label(if state.scope_open {
            "Scope is open"
        } else {
            "Open focus scope"
        })
        .id("focus-open")
        .on_press(if state.scope_open {
            Msg::CloseScope
        } else {
            Msg::OpenScope
        });

    let mut body = column![
        label(
            "Tab and Shift+Tab walk the controls in document order; Enter and Space activate. \
             Rings show only after keyboard input, never after a click.",
            14.0,
            20.0
        )
        .color(c(p.subtext))
        .wrap(),
        vgap(16.0),
        caption(&on, "TAB ORDER"),
        vgap(8.0),
        controls,
        vgap(20.0),
        caption(&on, "PROGRAMMATIC FOCUS"),
        vgap(8.0),
        program,
        vgap(20.0),
        caption(&on, "ROVING (OPT-IN)"),
        vgap(8.0),
        rove,
        vgap(20.0),
        caption(&on, "FOCUS SCOPE"),
        vgap(8.0),
        opener,
    ];
    if state.scope_open {
        let inner = on.on(on.wash(0.03));
        let panel = surface(
            column![
                label("Focus scope", 14.0, 20.0).semibold().color(c(p.text)),
                vgap(2.0),
                label(
                    "Tab wraps inside; Close gives focus back to the opener.",
                    13.0,
                    19.5
                )
                .color(c(p.subtext)),
                vgap(12.0),
                input(&inner, "Name", &state.scope_name, Msg::ScopeName),
                vgap(12.0),
                row![
                    button(&inner).label("Save").on_press(Msg::Pressed("Save")),
                    button(&inner)
                        .secondary()
                        .label("Close")
                        .on_press(Msg::CloseScope),
                ]
                .spacing(8),
            ]
            .width(Length::Fill),
        )
        .padding(17)
        .width(Length::Fill)
        .fixed(
            BoxStyle::default()
                .bg(c(inner.over))
                .border(c(inner.line()))
                .radius(12.0),
        );
        body = body.push(vgap(12.0)).push(focus_scope(panel));
    }
    body = body.push(vgap(16.0)).push(
        label(format!("Last action: {}", state.last), 13.0, 19.5)
            .mono()
            .color(c(p.muted)),
    );

    card(cx, "Focus and keyboard").body(body).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_update_the_readout() {
        let mut s = State::default();
        let _ = update(&mut s, Msg::Pressed("Primary"));
        assert_eq!(s.last, "Primary pressed");
        let _ = update(&mut s, Msg::OpenScope);
        assert!(s.scope_open);
        let _ = update(&mut s, Msg::CloseScope);
        assert!(!s.scope_open);
        let _ = update(&mut s, Msg::Density(Density::Roomy));
        assert_eq!(s.density, Density::Roomy);
    }
}
