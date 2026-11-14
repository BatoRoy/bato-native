//! Interaction lab section: the modal family, the command palette and the
//! file dropzone (Phase 3, batch B3).
//!
//! The Showcase's Triggers card (ConfirmDialog danger, the Drawer, its
//! four-command palette) plus the drawer's four sides and the FileDropzone of
//! the Toggles card, built exactly as those cards will use them. The frame's
//! own palette (Ctrl K, the sidebar search box, Home's Commands) needs no
//! section: [`selftest`] drives it from `main.rs`.
//!
//! [`selftest`] is `BATO_SELFTEST=modals`.

use std::path::PathBuf;
use std::time::Duration;

use bato_kit::iced::keyboard::{Modifiers, key::Named};
use bato_kit::iced::widget::{column, row};
use bato_kit::iced::{Event, Point, Task, mouse, widget, window};
use bato_kit::inputs::file_dropzone;
use bato_kit::modal::{Modal, Side};
use bato_kit::overlay::palette::{self, Command, Outcome, Palette};
use bato_kit::overlay::script::{Step, Target};
use bato_kit::overlay::{confirm_dialog, drawer};
use bato_kit::prelude::*;
use bato_shell::dialogs::OpenFiles;
use bato_shell::iced_ext;

#[derive(Debug, Default)]
pub struct State {
    confirm: bool,
    drawer: Option<Side>,
    palette: Palette,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Confirm(bool),
    Confirmed,
    Drawer(Option<Side>),
    OpenPalette,
    Palette(palette::Msg),
    Run(&'static str),
    Browse,
    Picked(Vec<PathBuf>),
    Dropped(Vec<PathBuf>),
}

impl Msg {
    /// The toast this message shows (the Showcase's `toast(…)` calls).
    pub fn toast(&self) -> Option<(String, Tone)> {
        Some(match self {
            Msg::Confirmed => ("Deleted".into(), Tone::Error),
            Msg::Run("home") => ("Home".into(), Tone::Info),
            Msg::Run("settings") => ("Settings".into(), Tone::Info),
            Msg::Run("new") => ("New file".into(), Tone::Success),
            Msg::Run("add") => ("Added".into(), Tone::Success),
            Msg::Picked(p) if !p.is_empty() => {
                (format!("{} path(s) selected", p.len()), Tone::Success)
            }
            Msg::Dropped(f) => (format!("{} file(s) selected", f.len()), Tone::Success),
            _ => return None,
        })
    }
}

/// The Triggers card's palette commands.
fn commands() -> Vec<Command<Msg>> {
    vec![
        Command::new("home", "Go to Home", Msg::Run("home"))
            .icon(Ico::Home)
            .group("Navigation"),
        Command::new("settings", "Open Settings", Msg::Run("settings"))
            .icon(Ico::Settings)
            .group("Navigation"),
        Command::new("new", "New File", Msg::Run("new"))
            .icon(Ico::FileText)
            .group("Actions")
            .hint("⌘N"),
        Command::new("add", "Add Item", Msg::Run("add"))
            .icon(Ico::Plus)
            .group("Actions"),
    ]
}

pub fn update(state: &mut State, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Confirm(v) => state.confirm = v,
        Msg::Confirmed => state.confirm = false,
        Msg::Drawer(s) => state.drawer = s,
        Msg::OpenPalette => state.palette.open(),
        Msg::Palette(m) => {
            if let Some(Outcome::Run(run)) = state.palette.update(m, &commands()) {
                return update(state, run);
            }
        }
        Msg::Browse => {
            let o = OpenFiles {
                multiple: true,
                ..OpenFiles::default()
            };
            return iced_ext::open_files(o).map(|r| Msg::Picked(r.unwrap_or_default()));
        }
        Msg::Run(_) | Msg::Picked(_) | Msg::Dropped(_) => {}
    }
    Task::none()
}

fn id(s: &'static str) -> widget::Id {
    widget::Id::from(s)
}
const CONFIRM: &str = "md-confirm";
const DRAWER: &str = "md-drawer";
const LEFT: &str = "md-left";
const TOP: &str = "md-top";
const BOTTOM: &str = "md-bottom";
const PALETTE: &str = "md-palette";
const ZONE: &str = "md-zone";

fn target<'a>(name: &'static str, el: impl Into<El<'a, Msg>>) -> El<'a, Msg> {
    container(el).id(id(name)).into()
}

pub fn view<'a>(_state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    let triggers = row![
        target(
            CONFIRM,
            button(&on)
                .danger()
                .label("Confirm dialog")
                .on_press(Msg::Confirm(true))
        ),
        target(
            DRAWER,
            button(&on)
                .secondary()
                .label("Open drawer")
                .on_press(Msg::Drawer(Some(Side::Right)))
        ),
        target(
            PALETTE,
            button(&on)
                .secondary()
                .label("Command palette")
                .on_press(Msg::OpenPalette)
        ),
    ]
    .spacing(12);
    let sides = row![
        label("Drawer sides", 13.0, 19.5).color(c(p.subtext)),
        target(
            LEFT,
            button(&on)
                .secondary()
                .sm()
                .label("Left")
                .on_press(Msg::Drawer(Some(Side::Left)))
        ),
        target(
            TOP,
            button(&on)
                .secondary()
                .sm()
                .label("Top")
                .on_press(Msg::Drawer(Some(Side::Top)))
        ),
        target(
            BOTTOM,
            button(&on)
                .secondary()
                .sm()
                .label("Bottom")
                .on_press(Msg::Drawer(Some(Side::Bottom)))
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let zone = target(
        ZONE,
        file_dropzone(&on)
            .hint("PNG, JPG up to 10MB")
            .on_browse(Msg::Browse)
            .on_drop(Msg::Dropped),
    );
    card(cx, "Modal family, palette and file drop")
        .icon(Ico::Upload)
        .body(column![triggers, sides, zone].spacing(16))
        .into()
}

/// The open levels, bottom first.
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
    if let Some(side) = state.drawer {
        let body = label(
            "A slide-in panel. Use it for filters, details, or forms.",
            14.0,
            20.0,
        )
        .color(c(on.p().subtext))
        .wrap();
        v.push(
            drawer(cx, body, Msg::Drawer(None))
                .side(side)
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

/// `BATO_SELFTEST=modals`, this section's part (main.rs adds the frame's
/// palette around it).
pub fn selftest() -> Vec<Step<Msg>> {
    let none = Modifiers::default();
    let left = mouse::Button::Left;
    let click = |name: &'static str| Step::Click(Target::Center(id(name)), left);
    let file = |n: &str| PathBuf::from(format!("/tmp/bato-selftest/{n}.png"));
    vec![
        click(CONFIRM),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("confirm"),
        // Enter confirms wherever focus is: toast "Deleted".
        Step::Key(Named::Enter, none),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("confirm-enter"),
        click(DRAWER),
        Step::Wait(Duration::from_millis(400)),
        Step::Shot("drawer-right"),
        Step::Key(Named::Escape, none),
        click(LEFT),
        Step::Wait(Duration::from_millis(400)),
        Step::Shot("drawer-left"),
        Step::Click(Target::At(Point::new(900.0, 400.0)), left),
        click(BOTTOM),
        Step::Wait(Duration::from_millis(400)),
        Step::Shot("drawer-bottom"),
        Step::Key(Named::Escape, none),
        click(TOP),
        Step::Wait(Duration::from_millis(400)),
        Step::Shot("drawer-top"),
        Step::Key(Named::Escape, none),
        // The Showcase palette: all commands, then ↓ ↓ and a filter.
        click(PALETTE),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("palette"),
        Step::Key(Named::ArrowDown, none),
        Step::Key(Named::ArrowDown, none),
        Step::Wait(Duration::from_millis(200)),
        Step::Shot("palette-down2"),
        Step::Char('i', none),
        Step::Char('t', none),
        Step::Wait(Duration::from_millis(200)),
        Step::Shot("palette-filter-it"),
        Step::Char('z', none),
        Step::Wait(Duration::from_millis(200)),
        Step::Shot("palette-empty"),
        Step::Key(Named::Backspace, none),
        // Enter runs the active row ("Add Item"): toast "Added".
        Step::Key(Named::Enter, none),
        Step::Wait(Duration::from_millis(300)),
        // The dropzone: hover, files dragged over it, two dropped.
        Step::Move(Target::Center(id(ZONE))),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("zone-hover"),
        Step::Event(Event::Window(window::Event::FileHovered(file("a")))),
        Step::Event(Event::Window(window::Event::FileHovered(file("b")))),
        Step::Move(Target::Id(id(ZONE), bato_kit::iced::Vector::new(40.0, 40.0))),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("zone-drag-over"),
        Step::Event(Event::Window(window::Event::FileDropped(file("a")))),
        Step::Event(Event::Window(window::Event::FileDropped(file("b")))),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("zone-dropped"),
        // A drag that leaves without dropping clears the highlight.
        Step::Event(Event::Window(window::Event::FileHovered(file("c")))),
        Step::Event(Event::Window(window::Event::FilesHoveredLeft)),
        Step::Move(Target::At(Point::new(1200.0, 20.0))),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("zone-left"),
    ]
}
