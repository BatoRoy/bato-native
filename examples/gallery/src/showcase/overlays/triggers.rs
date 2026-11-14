//! `overlays/triggers`: Triggers. A wrapping row: the "Actions" Dropdown
//! (Edit, Delete), a Tooltip on "Hover me", the ConfirmDialog / Drawer /
//! CommandPalette buttons (batch B3's slots) and the "Popover" button with
//! its anchored panel (`ShowcasePage.tsx`).
//!
//! Also [`selftest`]: `BATO_SELFTEST=pickers`, batch B2's scripted run over
//! the Selects, Sliders & color, Search and Overlays cards.

use std::time::Duration;

use bato_kit::Tone;
use bato_kit::iced::keyboard::{Modifiers, key::Named};
use bato_kit::iced::widget::{container, row};
use bato_kit::iced::{Point, Vector, mouse, widget};
use bato_kit::overlay::script::{Step, Target};
use bato_kit::overlay::{dropdown, popover, popover_panel, tooltip};
use bato_kit::prelude::*;

use crate::showcase::{self, Effect, Tab};

#[derive(Debug, Default)]
pub struct State {
    pub pop: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Pop(bool),
    Toast(&'static str, Tone),
    /// A button without an action (React's `<Button>` with no `onClick`).
    Noop,
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Pop(v) => state.pop = v,
        Msg::Toast(text, tone) => return Some(Effect::Toast(text.into(), tone)),
        Msg::Noop => {}
    }
    None
}

/// Self-test targets.
pub const ACTIONS: &str = "trig-actions";
pub const HOVER: &str = "trig-hover";
pub const POPOVER: &str = "trig-popover";

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    let actions = dropdown(
        &on,
        "Actions",
        vec![
            action("Edit", Msg::Toast("Edit", Tone::Info))
                .icon(Ico::Pencil)
                .into(),
            action("Delete", Msg::Toast("Deleted", Tone::Error))
                .icon(Ico::Trash2)
                .danger()
                .into(),
        ],
    );
    let hover = tooltip(
        cx,
        button(&on).secondary().label("Hover me").on_press(Msg::Noop),
        "A helpful hint",
    );
    let pop = popover(
        container(
            button(&on)
                .secondary()
                .label("Popover")
                .on_press(Msg::Pop(!state.pop)),
        )
        .id(POPOVER),
        state.pop.then(|| {
            popover_panel(
                cx,
                label(
                    "Anchored popover content. Dismisses on outside-click or Escape.",
                    14.0,
                    20.0,
                )
                .color(c(p.subtext))
                .wrap(),
            )
            .padding(13)
            .width(192)
            .into()
        }),
    )
    .on_dismiss(|_| Msg::Pop(false));
    let body = row![
        container(actions).id(ACTIONS),
        container(hover).id(HOVER),
        // ── B3 slot: ConfirmDialog ("Delete item?", danger) ──
        button(&on)
            .danger()
            .label("Confirm dialog")
            .on_press(Msg::Noop),
        // ── B3 slot: Drawer ──
        button(&on)
            .secondary()
            .label("Open drawer")
            .on_press(Msg::Noop),
        // ── B3 slot: CommandPalette ──
        button(&on)
            .secondary()
            .label("Command palette")
            .on_press(Msg::Noop),
        pop,
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .wrap();
    card(cx, "Triggers").body(body).into()
}

// ── BATO_SELFTEST=pickers ───────────────────────────────────────────────

fn id(s: &'static str) -> widget::Id {
    widget::Id::from(s)
}

/// Batch B2's scripted run: start it with
/// `BATO_VIEW=view=examples&tab=inputs BATO_SELFTEST=pickers` (the page at
/// the top: script targets are found in unscrolled layout coordinates, so
/// the Search card, further down, is aimed at by window position after
/// scrolling it to the top).
pub fn selftest() -> Vec<Step<showcase::Msg>> {
    use crate::showcase::inputs::{self, search, selects};
    use crate::showcase::overlays;
    let none = Modifiers::default();
    let left = mouse::Button::Left;
    let wait = |ms| Step::Wait(Duration::from_millis(ms));
    let click = |name: &'static str| Step::Click(Target::Center(id(name)), left);
    // The control box of a field: under its 16 px label and 6 px gap.
    let field_box =
        |name: &'static str| Step::Click(Target::Id(id(name), Vector::new(300.0, 41.0)), left);
    let search_msg = |m| Step::Message(showcase::Msg::Inputs(inputs::Msg::Search(m)));
    let mut v = vec![
        Step::Shot("selects"),
        // ColorPicker (shown in this card for the test only): open, drag in
        // the saturation/value area, a hue key, a preset.
        Step::Message(showcase::Msg::Inputs(inputs::Msg::Selects(
            selects::Msg::Color("#7c3aed".into()),
        ))),
        wait(300),
        field_box(selects::COLOR),
        wait(300),
        Step::Shot("color-open"),
        // At 1280×800 the 289 px panel flips above the box (6 px gap); the
        // area starts 13 px into it.
        Step::Press(
            Target::Id(id(selects::COLOR), Vector::new(60.0, 22.0 - 6.0 - 289.0 + 13.0 + 30.0)),
            left,
        ),
        Step::Move(Target::Id(
            id(selects::COLOR),
            Vector::new(150.0, 22.0 - 6.0 - 289.0 + 13.0 + 60.0),
        )),
        Step::Event(bato_kit::iced::Event::Mouse(mouse::Event::ButtonReleased(left))),
        wait(300),
        Step::Shot("color-dragged"),
        Step::Key(Named::Escape, none),
        // Combobox: open (search autofocused), filter, ↓, Enter.
        field_box(selects::COMBOBOX),
        wait(300),
        Step::Shot("combobox-open"),
        Step::Char('e', none),
        Step::Char('r', none),
        Step::Key(Named::ArrowDown, none),
        wait(200),
        Step::Shot("combobox-filter"),
        Step::Key(Named::Enter, none),
        wait(300),
        Step::Shot("combobox-picked"),
        // Multi-select: open (the 221 px panel flips above), toggle Banana
        // (row 2 under the 52 px search section).
        field_box(selects::MULTI),
        wait(300),
        Step::Click(
            Target::Id(
                id(selects::MULTI),
                Vector::new(300.0, 22.0 - 6.0 - 221.0 + 52.0 + 4.0 + 32.0 + 16.0),
            ),
            left,
        ),
        wait(300),
        Step::Shot("multiselect-open"),
        Step::Key(Named::Escape, none),
        // Quick select: open, ↓ ↓ Enter (Elderberry → Banana… wraps).
        click(selects::QUICK),
        Step::Key(Named::ArrowDown, none),
        wait(300),
        Step::Shot("select-open"),
        Step::Key(Named::Escape, none),
        // Search: suggestions on focus, filter, autocomplete with Tab.
        search_msg(search::Msg::Focus),
        wait(300),
        Step::Shot("search-suggestions"),
        Step::Char('r', none),
        Step::Char('e', none),
        Step::Key(Named::ArrowDown, none),
        wait(200),
        Step::Shot("search-filtered"),
        search_msg(search::Msg::Auto(true)),
        wait(300),
        Step::Shot("search-ghost"),
        Step::Key(Named::Tab, none),
        wait(300),
        Step::Shot("search-accepted"),
        // Overlays tab.
        Step::Message(showcase::Msg::Tab(Tab::Overlays)),
        wait(500),
        Step::Shot("overlays"),
        Step::Click(
            Target::Id(id(overlays::context_menu::AREA), Vector::new(140.0, 30.0)),
            mouse::Button::Right,
        ),
        Step::Key(Named::ArrowDown, none),
        Step::Key(Named::ArrowDown, none),
        Step::Key(Named::ArrowDown, none),
        wait(300),
        Step::Shot("context-menu"),
        // A second right press elsewhere in the area moves the menu.
        Step::Click(
            Target::Id(id(overlays::context_menu::AREA), Vector::new(600.0, 60.0)),
            mouse::Button::Right,
        ),
        wait(300),
        Step::Shot("context-menu-moved"),
        Step::Key(Named::Escape, none),
        click(ACTIONS),
        Step::Key(Named::ArrowDown, none),
        Step::Key(Named::ArrowDown, none),
        wait(300),
        Step::Shot("dropdown"),
        Step::Key(Named::Escape, none),
        Step::Move(Target::Center(id(HOVER))),
        wait(700),
        Step::Shot("tooltip"),
        click(POPOVER),
        wait(300),
        Step::Shot("popover"),
        Step::Move(Target::At(Point::new(700.0, 700.0))),
        Step::Key(Named::Escape, none),
        wait(300),
        Step::Shot("end"),
    ];
    v.shrink_to_fit();
    v
}
