//! `overlays/triggers`: Triggers. A wrapping row: the "Actions" Dropdown
//! (Edit, Delete), a Tooltip on "Hover me", the ConfirmDialog, Drawer and
//! CommandPalette buttons (their levels: [`levels`]) and the "Popover"
//! button with its anchored panel (`ShowcasePage.tsx`).
//!
//! Also [`selftest`]: `BATO_SELFTEST=pickers`, batch B2's scripted run over
//! the Selects, Sliders & color, Search and Overlays cards.

use std::time::Duration;

use bato_kit::Tone;
use bato_kit::iced::keyboard::{Modifiers, key::Named};
use bato_kit::iced::widget::{container, row};
use bato_kit::iced::{Point, Vector, mouse, widget};
use bato_kit::modal::Modal;
use bato_kit::overlay::palette::{self, Command, Outcome, Palette};
use bato_kit::overlay::script::{Step, Target};
use bato_kit::overlay::{confirm_dialog, drawer, dropdown, popover, popover_panel, tooltip};
use bato_kit::prelude::*;

use crate::showcase::{self, Effect, Tab};

#[derive(Debug, Default)]
pub struct State {
    pub pop: bool,
    pub confirm: bool,
    pub drawer: bool,
    pub palette: Palette,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Pop(bool),
    Toast(&'static str, Tone),
    /// A button without an action (React's `<Button>` with no `onClick`).
    Noop,
    Confirm(bool),
    Confirmed,
    Drawer(bool),
    OpenPalette,
    Palette(palette::Msg),
}

/// The palette's commands (`ShowcasePage.tsx`).
fn commands() -> Vec<Command<Msg>> {
    vec![
        Command::new("home", "Go to Home", Msg::Toast("Home", Tone::Info))
            .icon(Ico::Home)
            .group("Navigation"),
        Command::new(
            "settings",
            "Open Settings",
            Msg::Toast("Settings", Tone::Info),
        )
        .icon(Ico::Settings)
        .group("Navigation"),
        Command::new("new", "New File", Msg::Toast("New file", Tone::Success))
            .icon(Ico::FileText)
            .group("Actions")
            .hint("⌘N"),
        Command::new("add", "Add Item", Msg::Toast("Added", Tone::Success))
            .icon(Ico::Plus)
            .group("Actions"),
    ]
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Pop(v) => state.pop = v,
        Msg::Toast(text, tone) => return Some(Effect::Toast(text.into(), tone)),
        Msg::Noop => {}
        Msg::Confirm(v) => state.confirm = v,
        Msg::Confirmed => {
            state.confirm = false;
            return Some(Effect::Toast("Deleted".into(), Tone::Error));
        }
        Msg::Drawer(v) => state.drawer = v,
        Msg::OpenPalette => state.palette.open(),
        Msg::Palette(m) => {
            if let Some(Outcome::Run(run)) = state.palette.update(m, &commands()) {
                return update(state, run);
            }
        }
    }
    None
}

/// The open ConfirmDialog, Drawer and CommandPalette, bottom first.
pub fn levels<'a>(state: &'a State, cx: &Ctx) -> Vec<Modal<'a, Msg>> {
    let on = cx.on_card();
    let mut v = Vec::new();
    if state.confirm {
        v.push(
            confirm_dialog(
                cx,
                "Delete item?",
                "This cannot be undone.",
                Msg::Confirmed,
                Msg::Confirm(false),
            )
            .confirm_label("Delete")
            .danger(true)
            .into_level(),
        );
    }
    if state.drawer {
        let body = label(
            "A slide-in panel. Use it for filters, details, or forms.",
            14.0,
            20.0,
        )
        .color(c(on.p().subtext))
        .wrap();
        v.push(
            drawer(cx, body, Msg::Drawer(false))
                .title("Drawer")
                .into_level(),
        );
    }
    if state.palette.is_open() {
        v.push(
            state
                .palette
                .level(cx, &commands(), "Type a command…", Msg::Palette),
        );
    }
    v
}

/// Self-test targets.
pub const ACTIONS: &str = "trig-actions";
pub const HOVER: &str = "trig-hover";
pub const POPOVER: &str = "trig-popover";
pub const CONFIRM: &str = "trig-confirm";
pub const DRAWER: &str = "trig-drawer";
pub const PALETTE: &str = "trig-palette";

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
        container(
            button(&on)
                .danger()
                .label("Confirm dialog")
                .on_press(Msg::Confirm(true))
        )
        .id(CONFIRM),
        container(
            button(&on)
                .secondary()
                .label("Open drawer")
                .on_press(Msg::Drawer(true))
        )
        .id(DRAWER),
        container(
            button(&on)
                .secondary()
                .label("Command palette")
                .on_press(Msg::OpenPalette)
        )
        .id(PALETTE),
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
/// `BATO_VIEW=view=examples&tab=inputs&card=selects BATO_SELFTEST=pickers`.
/// The card then sits 24 px under the top of the page (React's anchor), so
/// targets are window positions (script ids resolve in unscrolled layout
/// coordinates); the first step shows a ColorPicker at the top of the card
/// body (test only), which moves the rest 80 px down.
pub fn selftest() -> Vec<Step<showcase::Msg>> {
    use crate::showcase::inputs::{self, search, selects};
    use crate::showcase::overlays;
    let none = Modifiers::default();
    let left = mouse::Button::Left;
    let wait = |ms| Step::Wait(Duration::from_millis(ms));
    let at = |x: f32, y: f32| Target::At(Point::new(x, y));
    let click_at = |x: f32, y: f32| Step::Click(Target::At(Point::new(x, y)), left);
    let click = |name: &'static str| Step::Click(Target::Center(id(name)), left);
    let search_msg = |m| Step::Message(showcase::Msg::Inputs(inputs::Msg::Search(m)));
    let mut v = vec![
        Step::Shot("selects"),
        // ColorPicker: open (the box at y 103–141), drag in the
        // saturation/value area (the panel 6 px under the box, the area
        // 13 px into it), then a hue key and a preset.
        Step::Message(showcase::Msg::Inputs(inputs::Msg::Selects(
            selects::Msg::Color("#7c3aed".into()),
        ))),
        wait(300),
        click_at(750.0, 122.0),
        wait(300),
        Step::Shot("color-open"),
        Step::Press(at(400.0, 190.0), left),
        Step::Move(at(480.0, 220.0)),
        Step::Event(bato_kit::iced::Event::Mouse(mouse::Event::ButtonReleased(left))),
        wait(300),
        Step::Shot("color-dragged"),
        Step::Key(Named::Escape, none),
        // Combobox (box y 183–221): open (search autofocused), filter, ↓,
        // Enter.
        click_at(544.0, 202.0),
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
        // Multi-select: open, toggle Banana (row 2 under the 52 px search
        // section of the panel at y 227).
        click_at(1000.0, 202.0),
        wait(300),
        click_at(1000.0, 331.0),
        wait(300),
        Step::Shot("multiselect-open"),
        Step::Key(Named::Escape, none),
        // Quick select: open, ↓.
        click_at(498.0, 256.0),
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
        // The caret went to the end of the completion: `!` lands after it.
        Step::Char('!', none),
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
        // The modal family (B3's components) from this card.
        click(CONFIRM),
        wait(400),
        Step::Shot("confirm"),
        Step::Key(Named::Escape, none),
        click(DRAWER),
        wait(500),
        Step::Shot("drawer"),
        Step::Key(Named::Escape, none),
        click(PALETTE),
        wait(400),
        Step::Shot("palette"),
        Step::Key(Named::Escape, none),
        wait(400),
        Step::Shot("end"),
    ];
    v.shrink_to_fit();
    v
}
