//! Interaction lab section: kit 1.3, what the app ports asked for. Rich
//! toasts (wrapping, title, action, duration), the frame sidebar's badges,
//! footer rows and header note, italic / bold / decorated text, a full-width
//! select, a grid row with pinned footers, a sticky bar, a pulsing progress
//! that keeps its value, the window file-drop overlay, segment icons and
//! tabular digits, and a QR code.
//!
//! The frame-facing buttons return an [`Effect`] the app applies to its
//! [`bato_kit::frame::Frame`].

use std::time::Duration;

use bato_kit::data::qr_code;
use bato_kit::dnd::window_drop::{DropMsg, WindowDrop};
use bato_kit::feedback::progress;
use bato_kit::frame::{HeaderNote, NavBadge, SidebarRow};
use bato_kit::iced::widget::{Space, column, row};
use bato_kit::iced::{Subscription, Task};
use bato_kit::label::Span;
use bato_kit::layout::{fill_end, sticky};
use bato_kit::overlay::select::{opt, select};
use bato_kit::scroller::scroll_area;
use bato_kit::toast::Toast;
use bato_kit::{Ico, prelude::*};

/// The anchor `BATO_VIEW=view=interaction&card=kit13` scrolls to.
pub const ANCHOR: &str = "kit13";

#[derive(Debug, Default)]
pub struct State {
    badge: bool,
    footer: bool,
    note: bool,
    fruit: u8,
    seg: u8,
    count: u32,
    drop_on: bool,
    drop: WindowDrop,
    dropped: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    LongToast,
    RichToast,
    StickyToast,
    Badge(bool),
    Footer(bool),
    Note(bool),
    Fruit(u8),
    Seg(u8),
    Count,
    DropOn(bool),
    Drop(DropMsg),
    Noop,
}

/// What a message asks of the frame.
pub enum Effect {
    Toast(Toast),
    Badge(Option<NavBadge>),
    Footer(Vec<SidebarRow>),
    Note(Option<HeaderNote>),
}

/// The frame part of `msg`, given the state after [`update`].
pub fn effect(state: &State, msg: &Msg) -> Option<Effect> {
    Some(match msg {
        Msg::LongToast => Effect::Toast(Toast::new(
            "This message is far too long for one line of a toast, so it wraps onto a second line instead of being cut short with an ellipsis.",
            Tone::Info,
        )),
        Msg::RichToast => Effect::Toast(
            Toast::new("3 files were converted to MP4.", Tone::Success)
                .title("Conversion finished")
                .action("Open output folder", "kit13-open")
                .duration(Duration::from_millis(6500)),
        ),
        Msg::StickyToast => Effect::Toast(
            Toast::new("This one stays until you close it.", Tone::Warning).persistent(),
        ),
        Msg::Badge(_) | Msg::Count => {
            Effect::Badge(state.badge.then_some(NavBadge::Count(state.count)))
        }
        Msg::Footer(on) => Effect::Footer(if *on {
            vec![
                SidebarRow::new(Ico::History, "History", "kit13-history").badge(NavBadge::Muted(2)),
            ]
        } else {
            Vec::new()
        }),
        Msg::Note(on) => {
            Effect::Note(on.then(|| HeaderNote::new("⚠ server v1.2.9").tone(Tone::Warning)))
        }
        _ => return None,
    })
}

pub fn update(state: &mut State, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Badge(on) => {
            state.badge = on;
            if state.count == 0 {
                state.count = 3;
            }
        }
        Msg::Count => state.count += 1,
        Msg::Footer(on) => state.footer = on,
        Msg::Note(on) => state.note = on,
        Msg::Fruit(f) => state.fruit = f,
        Msg::Seg(s) => state.seg = s,
        Msg::DropOn(on) => {
            state.drop_on = on;
            state.drop = WindowDrop::new();
        }
        Msg::Drop(m) => {
            let files = state.drop.update(m);
            if !files.is_empty() {
                state.dropped = files.iter().map(|p| p.display().to_string()).collect();
            }
        }
        Msg::LongToast | Msg::RichToast | Msg::StickyToast | Msg::Noop => {}
    }
    Task::none()
}

pub fn subscription(state: &State) -> Subscription<Msg> {
    if state.drop_on {
        state.drop.subscription().map(Msg::Drop)
    } else {
        Subscription::none()
    }
}

/// The full-window drop overlay while files hover (the switch is on).
pub fn drop_overlay<'a>(state: &State, cx: &Ctx) -> Option<El<'a, Msg>> {
    state.drop.overlay(
        cx,
        "Drop files to add them",
        "Any file type, as many as you like",
        true,
    )
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p().clone();
    let note = |t: &str| label(t.to_string(), 13.0, 19.5).color(c(p.subtext)).wrap();
    let toggle = |text: &str, v: bool, m: fn(bool) -> Msg| {
        row![
            label(text.to_string(), 13.0, 19.5)
                .color(c(p.text))
                .width(Length::Fill),
            switch(&on, v, m),
        ]
        .align_y(Alignment::Center)
    };
    let toasts = card(cx, "Toasts and the sidebar")
        .icon(Ico::Bell)
        .body(
            column![
                note("Long messages wrap; a toast can carry a title, an action (it reports Action::Command) and its own duration. The switches drive the frame's sidebar."),
                row![
                    button(&on).secondary().sm().label("Long toast").on_press(Msg::LongToast),
                    button(&on).secondary().sm().label("Title + action").on_press(Msg::RichToast),
                    button(&on).secondary().sm().label("Persistent").on_press(Msg::StickyToast),
                ]
                .spacing(8),
                toggle("Badge on “Interaction lab”", state.badge, Msg::Badge),
                row![
                    Space::new().width(Length::Fill),
                    button(&on).ghost().sm().label("Count +1").on_press_maybe(state.badge.then_some(Msg::Count)),
                ],
                toggle("Footer row (History, a quiet badge)", state.footer, Msg::Footer),
                toggle("Header note under the version", state.note, Msg::Note),
            ]
            .spacing(12),
        );
    let muted = c(p.muted);
    let text = card(cx, "Text faces and decorations")
        .icon(Ico::FileType)
        .body(
            column![
                rich(
                    vec![
                        Span::new("Regular, "),
                        Span::new("italic").italic(),
                        Span::new(", "),
                        Span::new("bold").bold(),
                        Span::new(", "),
                        Span::new("bold italic").bold().italic(),
                        Span::new(", "),
                        Span::new("struck").strike().color(muted),
                        Span::new(", "),
                        Span::new("underlined").underline(),
                        Span::new("."),
                    ],
                    14.0,
                    20.0,
                )
                .color(c(p.text)),
                rich(
                    vec![
                        Span::new("mono, "),
                        Span::new("italic").italic(),
                        Span::new(", "),
                        Span::new("semibold").weight(600),
                        Span::new(", "),
                        Span::new("bold").bold(),
                        Span::new(", "),
                        Span::new("bold italic").bold().italic(),
                    ],
                    13.0,
                    19.5,
                )
                .mono()
                .color(c(p.text)),
                label("Hover me: hover:underline", 13.0, 19.5)
                    .color(c(p.accent_bright))
                    .hover_underline(),
                label("A finished task, struck through and muted", 13.0, 19.5)
                    .strikethrough()
                    .color(c(p.muted)),
            ]
            .spacing(10),
        );
    let fill_cell = |title: &str, blurb: &str| -> El<'a, Msg> {
        card(&on, title.to_string())
            .body(fill_end(
                label(blurb.to_string(), 13.0, 19.5)
                    .color(c(p.subtext))
                    .wrap(),
                button(&on.on_card())
                    .secondary()
                    .sm()
                    .label("Install")
                    .on_press(Msg::Noop),
                16.0,
            ))
            .into()
    };
    let layout_card = card(cx, "Layout helpers")
        .icon(Ico::LayoutGrid)
        .body(
            column![
                note("select(..).width(Fill): the chevron sits at the right edge."),
                select(
                    &on,
                    state.fruit,
                    vec![opt(0, "Apple"), opt(1, "Banana"), opt(2, "Cherry")],
                    Msg::Fruit,
                )
                .width(Length::Fill),
                note("fill_end: the buttons stay level in a stretched grid row."),
                bato_kit::layout::Grid::columns(3)
                    .gap(12.0)
                    .push(fill_cell("Short", "One line."))
                    .push(fill_cell("Longer", "A longer description that wraps over a few lines, so this card sets the row's height."))
                    .push(fill_cell("Medium", "Two lines of text, more or less.")),
                note("sticky: the bar pins to the top of its scroller."),
                scroll_area(
                    &on,
                    sticky(
                        label("Above the bar", 13.0, 19.5).color(c(p.subtext)),
                        container(label("Sticky bar", 13.0, 19.5).medium().color(c(p.text)))
                            .width(Length::Fill)
                            .padding([6, 0]),
                        column((0..12).map(|i| {
                            label(format!("Row {}", i + 1), 13.0, 24.0).color(c(p.subtext)).into()
                        })),
                        8.0,
                    )
                    .backdrop(c(p.card), c(on.line()), 0.0),
                )
                .height(140),
                note("Progress: pulsing() keeps its value (indeterminate draws 40 %)."),
                progress(&on, 100.0).pulsing(),
                progress(&on, 0.0).indeterminate(),
            ]
            .spacing(12),
        );
    let seg = card(cx, "Segments, QR code, file drop")
        .icon(Ico::Grid2x2)
        .body(
            column![
                segmented(
                    &on,
                    state.seg,
                    [
                        Segment::new(0u8, "Files").icon(Ico::File),
                        Segment::new(1u8, "Images").icon(Ico::Image),
                        Segment::rich(
                            2u8,
                            [
                                ("Queue ", false),
                                (format!("{}", 100 + state.count * 11).as_str(), true)
                            ]
                        )
                        .tabular(),
                    ],
                    Msg::Seg,
                ),
                tabs(
                    &on,
                    state.seg,
                    [
                        Segment::new(0u8, "Files").icon(Ico::File),
                        Segment::new(1u8, "Images").icon(Ico::Image),
                        Segment::new(2u8, "Queue").icon(Ico::List),
                    ],
                    Msg::Seg,
                ),
                row![
                    qr_code("https://example.com/s/kit13").size(132.0),
                    column![
                        toggle("Window drop overlay", state.drop_on, Msg::DropOn),
                        note(if state.drop_on {
                            "Drag files over the window."
                        } else {
                            "Off: drops go to the page as before."
                        }),
                        label(
                            if state.dropped.is_empty() {
                                "Nothing dropped yet".to_string()
                            } else {
                                state.dropped.join("\n")
                            },
                            12.0,
                            18.0,
                        )
                        .mono()
                        .color(c(p.muted))
                        .wrap(),
                    ]
                    .spacing(8)
                    .width(Length::Fill),
                ]
                .spacing(16),
            ]
            .spacing(16),
        );
    container(column![toasts, text, layout_card, seg].spacing(24))
        .id(ANCHOR)
        .into()
}
