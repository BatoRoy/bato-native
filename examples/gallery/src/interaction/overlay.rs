//! Interaction lab section: overlays, menus and hotkeys (Phase 2, INF-3…6).
//!
//! A popover, a popover that prefers the right side and flips near the
//! window edge, a dropdown menu with keyboard navigation, a right-click
//! context area, a dialog that opens a nested dialog and a drawer (the
//! overlay stack), a menu inside a dialog (floating layer over a level), and
//! the hotkey registry (`Ctrl K` opens a palette level, `?` counts).
//!
//! [`selftest`] is the `BATO_SELFTEST=overlay` script: it drives every state
//! through real input events and asks for a screenshot of each.

use std::time::Duration;

use bato_kit::floating::{Options, Placement};
use bato_kit::hotkeys::{Hotkeys, KeyPress};
use bato_kit::iced::keyboard::{Modifiers, key::Named};
use bato_kit::iced::widget::{column, container, row};
use bato_kit::iced::{Point, Subscription, Task, Vector, mouse, widget};
use bato_kit::menu::MenuEntry;
use bato_kit::modal::{Kind, Modal, Side, drawer_panel, level, modal_panel};
use bato_kit::overlay::script::{Step, Target};
use bato_kit::overlay::{Dismiss, Position};
use bato_kit::prelude::*;

#[derive(Debug)]
pub struct State {
    pop: bool,
    flip: bool,
    dropdown: bool,
    menu_at: Option<Point>,
    dialog: bool,
    dialog_menu: bool,
    nested: bool,
    inner_drawer: bool,
    drawer: bool,
    palette: bool,
    last: Option<&'static str>,
    hotkey: Option<(String, u32)>,
    keys: Hotkeys<Msg>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            pop: false,
            flip: false,
            dropdown: false,
            menu_at: None,
            dialog: false,
            dialog_menu: false,
            nested: false,
            inner_drawer: false,
            drawer: false,
            palette: false,
            last: None,
            hotkey: None,
            keys: Hotkeys::new()
                .bind("mod+k", Msg::Hotkey("mod+k"))
                .bind("shift+?", Msg::Hotkey("shift+?"))
                // In the palette's scope, Ctrl K closes it again.
                .bind_in("palette", "mod+k", Msg::Hotkey("mod+k (palette scope)"))
                .in_inputs(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Pop(bool),
    Flip(bool),
    Dropdown(bool),
    Menu(Option<Point>),
    Picked(&'static str),
    Dialog(bool),
    DialogMenu(bool),
    Nested(bool),
    InnerDrawer(bool),
    Drawer(bool),
    Palette(bool),
    Key(KeyPress),
    Hotkey(&'static str),
}

pub fn update(state: &mut State, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Pop(v) => state.pop = v,
        Msg::Flip(v) => state.flip = v,
        Msg::Dropdown(v) => state.dropdown = v,
        Msg::Menu(at) => state.menu_at = at,
        Msg::Picked(what) => state.last = Some(what),
        Msg::Dialog(v) => {
            state.dialog = v;
            if !v {
                state.nested = false;
                state.inner_drawer = false;
                state.dialog_menu = false;
            }
        }
        Msg::DialogMenu(v) => state.dialog_menu = v,
        Msg::Nested(v) => {
            state.nested = v;
            if !v {
                state.inner_drawer = false;
            }
        }
        Msg::InnerDrawer(v) => state.inner_drawer = v,
        Msg::Drawer(v) => state.drawer = v,
        Msg::Palette(v) => {
            state.palette = v;
            if v {
                state.keys.enter("palette");
            } else {
                state.keys.leave("palette");
            }
        }
        Msg::Key(press) => {
            if let Some(m) = state.keys.resolve(&press) {
                return update(state, m);
            }
        }
        Msg::Hotkey(combo) => {
            let n = match &state.hotkey {
                Some((c, n)) if c == combo => n + 1,
                _ => 1,
            };
            state.hotkey = Some((combo.to_string(), n));
            match combo {
                "mod+k" => return update(state, Msg::Palette(true)),
                "mod+k (palette scope)" => return update(state, Msg::Palette(false)),
                _ => {}
            }
        }
    }
    Task::none()
}

pub fn subscription(state: &State) -> Subscription<Msg> {
    state.keys.subscription().map(Msg::Key)
}

// ── Ids the self-test aims at ───────────────────────────────────────────

fn id(s: &'static str) -> widget::Id {
    widget::Id::from(s)
}
const POP: &str = "ov-pop";
const FLIP: &str = "ov-flip";
const DROPDOWN: &str = "ov-dropdown";
const MENU: &str = "ov-menu";
const AREA: &str = "ov-area";
const CTX_MENU: &str = "ov-context-menu";
const DIALOG: &str = "ov-dialog";
const DRAWER: &str = "ov-drawer";
const NESTED: &str = "ov-nested";
const INNER_DRAWER: &str = "ov-inner-drawer";
const DIALOG_MORE: &str = "ov-dialog-more";
const BLANK: &str = "ov-blank";

fn target<'a>(name: &'static str, el: impl Into<El<'a, Msg>>) -> El<'a, Msg> {
    container(el).id(id(name)).into()
}

/// The styled popover panel (`bg-app-card border-app-lineStrong
/// rounded-xl shadow-app-lg`) with `p-3 w-48` text.
fn popover_panel<'a>(cx: &Ctx, text: &'static str) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    surface(label(text, 14.0, 20.0).color(c(p.subtext)).wrap())
        .width(192)
        .padding(13)
        .fixed(BoxStyle {
            bg: Some(c(p.card)),
            border: Some(c(on.line_strong())),
            radius: 12.0,
            shadows: bato_kit::style::shadows(&p.shadow_lg),
            ..BoxStyle::default()
        })
        .into()
}

/// The Dropdown trigger (`Dropdown.tsx`): bordered, `bg-app-bg/50`, subtext
/// turning text on hover, chevron flipped while open.
fn dropdown_trigger<'a>(cx: &Ctx, text: &'static str, open: bool, msg: Msg) -> El<'a, Msg> {
    let p = cx.p().clone();
    let fill = c(cx.recessed());
    surface(
        row![
            label(text, 14.0, 20.0),
            icon(Ico::ChevronDown, 14.0).rotation(if open { std::f32::consts::PI } else { 0.0 }),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([8.0, 13.0]))
    .style(move |s| {
        let hover = s == Status::Hover;
        BoxStyle::default()
            .bg(fill)
            .border(c(if hover { p.accent_bright } else { p.control }))
            .radius(8.0)
            .fg(c(if hover { p.text } else { p.subtext }))
    })
    .on_press(msg)
    .into()
}

fn actions() -> Vec<MenuEntry<Msg>> {
    vec![
        action("Edit", Msg::Picked("Edit")).icon(Ico::Pencil).into(),
        action("Duplicate", Msg::Picked("Duplicate"))
            .icon(Ico::Copy)
            .shortcut("⌘D")
            .into(),
        action("Archive", Msg::Picked("Archive"))
            .icon(Ico::Download)
            .disabled(true)
            .into(),
        separator(),
        action("Delete", Msg::Picked("Delete"))
            .icon(Ico::Trash2)
            .danger()
            .into(),
    ]
}

fn context_items() -> Vec<MenuEntry<Msg>> {
    // The Showcase's context menu.
    vec![
        action("Copy", Msg::Picked("Copy"))
            .icon(Ico::Copy)
            .shortcut("⌘C")
            .into(),
        action("Cut", Msg::Picked("Cut")).icon(Ico::Scissors).into(),
        action("Paste", Msg::Picked("Paste"))
            .icon(Ico::Clipboard)
            .disabled(true)
            .into(),
        separator(),
        action("Delete", Msg::Picked("Delete"))
            .icon(Ico::Trash2)
            .danger()
            .into(),
    ]
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();

    let pop = bato_kit::overlay::anchored(
        target(
            POP,
            button(&on)
                .secondary()
                .label("Popover")
                .on_press(Msg::Pop(!state.pop)),
        ),
        state.pop.then(|| {
            popover_panel(
                cx,
                "Anchored popover content. Dismisses on outside-click or Escape.",
            )
        }),
    )
    .on_dismiss(|_| Msg::Pop(false));

    let dropdown = bato_kit::overlay::anchored(
        target(
            DROPDOWN,
            dropdown_trigger(
                &on,
                "Actions",
                state.dropdown,
                Msg::Dropdown(!state.dropdown),
            ),
        ),
        state.dropdown.then(|| {
            menu_list(cx, actions())
                .on_close(Msg::Dropdown(false))
                .into()
        }),
    )
    .position(Position::Below {
        align_right: false,
        gap: 4.0,
    })
    .dismiss(Dismiss::DROPDOWN)
    .id(id(MENU))
    .on_dismiss(|_| Msg::Dropdown(false));

    let flip = bato_kit::overlay::anchored(
        target(
            FLIP,
            button(&on)
                .secondary()
                .icon(Ico::ChevronRight)
                .label("Prefers right")
                .on_press(Msg::Flip(!state.flip)),
        ),
        state.flip.then(|| {
            popover_panel(
                cx,
                "Placed on the right unless the window is too narrow there: then it flips left.",
            )
        }),
    )
    .float(Options::default().placement(Placement::Right))
    .on_dismiss(|_| Msg::Flip(false));

    let triggers = row![
        pop,
        dropdown,
        target(
            DIALOG,
            button(&on)
                .secondary()
                .label("Open dialog")
                .on_press(Msg::Dialog(true))
        ),
        target(
            DRAWER,
            button(&on)
                .secondary()
                .label("Open drawer")
                .on_press(Msg::Drawer(true))
        ),
        Space::new().width(Length::Fill),
        flip,
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let area_box = surface(label("Right-click here", 14.0, 20.0).color(c(p.muted)))
        .width(Length::Fill)
        .height(96)
        .center()
        .fixed(
            BoxStyle::default()
                .border(c(p.border))
                .radius(8.0)
                .bg(c(p.card)),
        );
    let area = bato_kit::overlay::context_area(target(AREA, area_box), |p| Msg::Menu(Some(p)))
        .menu(
            state.menu_at,
            menu_list(cx, context_items()).on_close(Msg::Menu(None)),
            |_| Msg::Menu(None),
        )
        .id(id(CTX_MENU));

    let count = |combo: &str| match &state.hotkey {
        Some((c, n)) if c == combo => format!("{n}×"),
        _ => "—".into(),
    };
    let hotkeys = row![
        kbd(&on, bato_kit::frame::MOD_K),
        label("command palette level", 13.0, 19.5).color(c(p.subtext)),
        gap(16.0),
        kbd(&on, "?"),
        label(format!("pressed {}", count("shift+?")), 13.0, 19.5).color(c(p.subtext)),
        Space::new().width(Length::Fill),
        label(
            match &state.hotkey {
                Some((combo, n)) => format!("Last hotkey: {combo} ({n}×)"),
                None => "Last hotkey: —".into(),
            },
            12.0,
            16.0
        )
        .mono()
        .color(c(p.muted)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let body = column![
        label(
            "Anchored popovers flip and shift to stay on screen; menus take ↑ ↓ Home End Enter Esc; dialogs and drawers stack, and Esc closes the top one.",
            14.0,
            20.0
        )
        .color(c(p.subtext))
        .wrap(),
        triggers,
        area,
        hotkeys,
        target(
            BLANK,
            label(
                format!("Last menu action: {}", state.last.unwrap_or("—")),
                12.0,
                16.0
            )
            .mono()
            .color(c(p.muted))
        ),
    ]
    .spacing(16);

    card(cx, "Overlays, menus and hotkeys")
        .icon(Ico::Command)
        .body(body)
        .into()
}

/// The open levels of this section, bottom first, for
/// `Frame::view_with`.
pub fn levels<'a>(state: &'a State, cx: &Ctx) -> Vec<Modal<'a, Msg>> {
    let on = cx.on_card();
    let p = on.p();
    let mut v = Vec::new();
    if state.drawer {
        v.push(level(
            Kind::Drawer(Side::Left, 320.0),
            drawer_panel(
                cx,
                Side::Left,
                Some("Drawer"),
                Msg::Drawer(false),
                label(
                    "A slide-in panel. Use it for filters, details, or forms.",
                    14.0,
                    20.0,
                )
                .color(c(p.subtext))
                .wrap(),
            ),
            Msg::Drawer(false),
        ));
    }
    if state.dialog {
        let more = bato_kit::overlay::anchored(
            target(
                DIALOG_MORE,
                dropdown_trigger(
                    &on,
                    "More",
                    state.dialog_menu,
                    Msg::DialogMenu(!state.dialog_menu),
                ),
            ),
            state.dialog_menu.then(|| {
                menu_list(cx, actions())
                    .on_close(Msg::DialogMenu(false))
                    .into()
            }),
        )
        .position(Position::Below {
            align_right: false,
            gap: 4.0,
        })
        .dismiss(Dismiss::DROPDOWN)
        .on_dismiss(|_| Msg::DialogMenu(false));
        let body = column![
            label(
                "Open another dialog or a drawer on top of this one. Each level has its own scrim and blur; Esc closes only the top one.",
                14.0,
                20.0
            )
            .color(c(p.subtext))
            .wrap(),
            row![
                target(
                    NESTED,
                    button(&on)
                        .secondary()
                        .label("Open nested dialog")
                        .on_press(Msg::Nested(true))
                ),
                more,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(20);
        v.push(level(
            Kind::Center,
            modal_panel(
                cx,
                "Overlay stack",
                Some("Modals, drawers and menus over them."),
                None,
                Msg::Dialog(false),
                body,
            ),
            Msg::Dialog(false),
        ));
    }
    if state.nested {
        let body = column![
            label("This cannot be undone.", 14.0, 22.75)
                .color(c(p.subtext))
                .wrap(),
            row![
                target(
                    INNER_DRAWER,
                    button(&on)
                        .secondary()
                        .label("Open drawer")
                        .on_press(Msg::InnerDrawer(true))
                ),
                Space::new().width(Length::Fill),
                button(&on)
                    .secondary()
                    .label("Cancel")
                    .on_press(Msg::Nested(false)),
                button(&on)
                    .danger()
                    .label("Delete")
                    .on_press(Msg::Nested(false)),
            ]
            .spacing(8),
        ]
        .spacing(24);
        v.push(
            level(
                Kind::Center,
                modal_panel(cx, "Delete item?", None, None, Msg::Nested(false), body),
                Msg::Nested(false),
            )
            .max_width(384.0),
        );
    }
    if state.inner_drawer {
        v.push(level(
            Kind::Drawer(Side::Right, 320.0),
            drawer_panel(
                cx,
                Side::Right,
                Some("Details"),
                Msg::InnerDrawer(false),
                label(
                    "A drawer over a nested dialog: the third level of the stack.",
                    14.0,
                    20.0,
                )
                .color(c(p.subtext))
                .wrap(),
            ),
            Msg::InnerDrawer(false),
        ));
    }
    if state.palette {
        let head = container(
            row![
                icon(Ico::Search, 16.0).color(c(p.muted)),
                label("Type a command…", 14.0, 20.0).color(c(p.muted)),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([14.0, 16.0]));
        let panel = surface(column![
            head,
            hline(c(on.line())),
            container(
                label(
                    "Opened with Ctrl K (a global hotkey). Ctrl K again closes it (the palette's scope wins); Esc closes the top level.",
                    13.0,
                    19.5
                )
                .color(c(p.subtext))
                .wrap()
            )
            .padding(Padding::from([16.0, 18.0])),
        ])
        .width(Length::Fill)
        .fixed(BoxStyle {
            bg: Some(c(p.card)),
            radius: 16.0,
            shadows: bato_kit::style::shadows(&p.shadow_lg),
            ..BoxStyle::default()
        });
        v.push(level(Kind::Top, panel, Msg::Palette(false)));
    }
    v
}

/// `BATO_SELFTEST=overlay`: every state, through real input events.
pub fn selftest() -> Vec<Step<Msg>> {
    let none = Modifiers::default();
    let left = mouse::Button::Left;
    let click = |name: &'static str| Step::Click(Target::Center(id(name)), left);
    let outside = Step::Click(Target::Center(id(BLANK)), left);
    // A menu row: 1 px border + 4 px padding, rows 32 px tall.
    let row =
        |menu: &'static str, i: f32| Target::Id(id(menu), Vector::new(60.0, 5.0 + 16.0 + 32.0 * i));
    vec![
        Step::Shot("rest"),
        // Popover: open, dismiss with an outside press.
        click(POP),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("popover"),
        outside.clone(),
        // Prefers right, flips left at the window edge; Esc closes it.
        click(FLIP),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("popover-flip"),
        Step::Key(Named::Escape, none),
        // Dropdown: keyboard navigation (↓ ↓ = Duplicate), then hover.
        click(DROPDOWN),
        Step::Key(Named::ArrowDown, none),
        Step::Key(Named::ArrowDown, none),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("dropdown-keys"),
        Step::Move(row(MENU, 4.25)),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("dropdown-hover-danger"),
        Step::Key(Named::Home, none),
        Step::Key(Named::Enter, none),
        // Context menu at the pointer; ↓ skips the disabled Paste.
        Step::Click(
            Target::Id(id(AREA), Vector::new(140.0, 30.0)),
            mouse::Button::Right,
        ),
        Step::Key(Named::ArrowDown, none),
        Step::Key(Named::ArrowDown, none),
        Step::Key(Named::ArrowDown, none),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("context-menu"),
        Step::Key(Named::Escape, none),
        // The stack: dialog → menu over it → nested dialog → drawer.
        click(DIALOG),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("modal"),
        click(DIALOG_MORE),
        Step::Key(Named::End, none),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("modal-menu"),
        Step::Key(Named::Escape, none),
        click(NESTED),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("modal-nested"),
        click(INNER_DRAWER),
        Step::Wait(Duration::from_millis(400)),
        Step::Shot("stack-drawer"),
        Step::Key(Named::Escape, none),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("stack-esc-closes-top"),
        Step::Key(Named::Escape, none),
        Step::Key(Named::Escape, none),
        // A drawer from the page, closed by a press on its scrim.
        click(DRAWER),
        Step::Wait(Duration::from_millis(400)),
        Step::Shot("drawer"),
        Step::Click(Target::At(Point::new(900.0, 400.0)), left),
        // Hotkeys: the registry resolves Ctrl K (global) and opens the
        // palette level; `?` counts.
        Step::Message(Msg::Key(KeyPress::of("shift+?").unwrap())),
        Step::Message(Msg::Key(KeyPress::of("mod+k").unwrap())),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("hotkey-palette"),
        Step::Message(Msg::Key(KeyPress::of("mod+k").unwrap())),
        Step::Wait(Duration::from_millis(300)),
        Step::Shot("end"),
    ]
}
