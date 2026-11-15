//! Interaction lab section: kit 1.1, what the BatoFetch and BatoBrowse ports
//! needed (BatoFetch `NATIVE-MIGRATION.md` §10, BatoBrowse `UI-INVENTORY.md`
//! §5): images and thumbnails, app icons and the new lucide set, submenus,
//! a rename prompt with the stem preselected, multi-line and hinted labels,
//! hover/focus reveal, a periodic tick, control heights and the virtual
//! file grid.

use std::time::{Duration, Instant};

use bato_kit::focus::{self, stem_range};
use bato_kit::iced::widget::{column, row};
use bato_kit::iced::{ContentFit, Point, Subscription, Task};
use bato_kit::icon::Icon;
use bato_kit::menu::MenuEntry;
use bato_kit::modal::{Kind, Modal, level, modal_panel};
use bato_kit::overlay::{context_menu, dropdown};
use bato_kit::reveal::{reveal_group, revealed};
use bato_kit::scroller::{GridLayout, cached_heights, virtual_grid};
use bato_kit::thumb::{self, Loaded, Thumbs, picture, vector};
use bato_kit::timer;
use bato_kit::{Ico, prelude::*};

/// A brand logo in its own colours (what `Frame::logo` takes).
pub static LOGO: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 32 32"><rect width="32" height="32" rx="9" fill="#eab308"/><path d="M8 11h7l2 2h7v9H8z" fill="#ffffff"/><circle cx="23" cy="21" r="4" fill="#2563eb"/></svg>"##;
/// A monochrome app icon in lucide's format (BatoBrowse's custom FileZip).
pub static FILE_ZIP: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#000000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2z"/><path d="M10 6h1M10 9h1M10 12h1M10 15h1"/><rect x="9" y="17" width="3" height="3"/></svg>"##;

const RENAME: &str = "kit11-rename";
const INLINE: &str = "kit11-inline";
const GRID_TILES: usize = 1500;

#[derive(Debug)]
pub struct State {
    thumbs: Thumbs<u32>,
    rename: Option<(String, String)>,
    inline: String,
    ticking: bool,
    ticks: u32,
    grid: Option<GridLayout>,
    cursor: usize,
    menu_at: Option<Point>,
    picked: String,
    names: Vec<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            thumbs: Thumbs::new(64),
            rename: None,
            inline: "holiday-photos.tar.gz".into(),
            ticking: false,
            ticks: 0,
            grid: None,
            cursor: 0,
            menu_at: None,
            picked: "nothing yet".into(),
            names: (0..GRID_TILES).map(name).collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    LoadThumbs,
    Thumb(u32, Loaded),
    OpenRename,
    RenameInput(String),
    RenameDone(bool),
    Inline(String),
    SelectStem,
    Ticking(bool),
    Tick(Instant),
    Grid(GridLayout),
    Move(i32, i32),
    Jump(usize),
    Menu(Option<Point>),
    Picked(&'static str),
}

pub fn update(state: &mut State, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::LoadThumbs => {
            for k in 0..4 {
                state.thumbs.forget(&k);
            }
            let wanted: Vec<u32> = (0..4u32).filter(|k| state.thumbs.want(*k)).collect();
            return Task::batch(wanted.into_iter().map(|k| {
                let token = state.thumbs.token(&k).unwrap_or_default();
                thumb::load(k, token, move || fetch(k), Msg::Thumb)
            }));
        }
        Msg::Thumb(k, r) => state.thumbs.finish(k, r),
        Msg::OpenRename => {
            let name = "Quarterly report (final).pdf".to_string();
            state.rename = Some((name.clone(), name));
        }
        Msg::RenameInput(v) => {
            if let Some((_, cur)) = &mut state.rename {
                *cur = v;
            }
        }
        Msg::RenameDone(ok) => {
            if let Some((_, cur)) = state.rename.take()
                && ok
            {
                state.picked = format!("renamed to “{cur}”");
            }
        }
        Msg::Inline(v) => state.inline = v,
        Msg::SelectStem => {
            return focus::select_range(INLINE, &state.inline, stem_range(&state.inline));
        }
        Msg::Ticking(on) => state.ticking = on,
        Msg::Tick(_) => state.ticks += 1,
        Msg::Grid(g) => state.grid = Some(g),
        Msg::Move(dx, dy) => {
            if let Some(g) = state.grid {
                state.cursor = g.step(state.cursor, GRID_TILES, dx, dy);
            }
        }
        Msg::Jump(i) => state.cursor = i.min(GRID_TILES - 1),
        Msg::Menu(at) => state.menu_at = at,
        Msg::Picked(what) => state.picked = what.into(),
    }
    Task::none()
}

pub fn subscription(state: &State) -> Subscription<Msg> {
    // Idle-friendly: no thread, no ticks unless switched on.
    if state.ticking {
        timer::every(Duration::from_secs(1)).map(Msg::Tick)
    } else {
        Subscription::none()
    }
}

/// The rename prompt: opens on its field with the stem selected.
pub fn levels<'a>(state: &'a State, cx: &Ctx) -> Vec<Modal<'a, Msg>> {
    let Some((orig, cur)) = &state.rename else {
        return Vec::new();
    };
    let on = cx.on_card();
    let body = column![
        bato_kit::inputs::text_field::text_field(&on, cur, Msg::RenameInput)
            .label("New name")
            .id(RENAME),
        row![
            Space::new().width(Length::Fill),
            button(&on)
                .secondary()
                .label("Cancel")
                .on_press(Msg::RenameDone(false)),
            button(&on)
                .primary()
                .label("Rename")
                .on_press(Msg::RenameDone(true)),
        ]
        .spacing(8),
    ]
    .spacing(20);
    let panel = modal_panel(
        cx,
        "Rename",
        Some("Modal::initial_select: focus the field, stem selected."),
        Some(Ico::Pencil.into()),
        Msg::RenameDone(false),
        body,
    );
    vec![
        level(Kind::Center, panel, Msg::RenameDone(false)).initial_select(
            RENAME,
            orig,
            stem_range(orig),
        ),
    ]
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p().clone();
    let note = |t: &str| label(t.to_string(), 13.0, 19.5).color(c(p.subtext)).wrap();
    column![
        card(cx, "Images and icons")
            .icon(Ico::Image)
            .body(images(state, &on, &p)),
        card(cx, "Submenus")
            .icon(Ico::FolderTree)
            .body(column![
                note("Hover a › row (100 ms intent), or → / Enter on it; ← or Esc closes it. Right-click near the window's right edge: the fly-out flips left."),
                submenus(state, &on),
            ]
            .spacing(12)),
        card(cx, "Rename, initial focus, control heights")
            .icon(Ico::TextSelect)
            .body(rename(state, &on, &p)),
        card(cx, "Labels and reveal")
            .icon(Ico::List)
            .body(labels(&on, &p)),
        card(cx, "Virtual file grid")
            .icon(Ico::Grid3x3)
            .body(grid(state, &on, &p)),
    ]
    .spacing(24)
    .into()
}

fn images<'a>(state: &'a State, on: &Ctx, p: &bato_kit::theme::Palette) -> El<'a, Msg> {
    let caption = |k: u32| {
        let t = match state.thumbs.get(&k) {
            None => "not asked",
            Some(thumb::Thumb::Loading) => "loading",
            Some(thumb::Thumb::None) => "no preview",
            Some(thumb::Thumb::Ready(_)) => "ready",
        };
        label(t, 12.0, 16.0).color(c(p.muted))
    };
    let tile = |k: u32, glyph: Ico| -> El<'a, Msg> {
        column![
            thumb::thumb(on, state.thumbs.get(&k), glyph, 72.0),
            caption(k)
        ]
        .spacing(4)
        .align_x(Alignment::Center)
        .into()
    };
    let thumbs = row![
        tile(0, Ico::FileImage),
        tile(1, Ico::FileImage),
        tile(2, Ico::FileAudio),
        tile(3, Ico::FileVideo),
        column![
            vector(thumb::svg_handle(LOGO)).width(72.0).height(72.0),
            label("svg", 12.0, 16.0).color(c(p.muted)),
        ]
        .spacing(4)
        .align_x(Alignment::Center),
    ]
    .spacing(16);
    let wide: El<'a, Msg> = match state.thumbs.image(&0) {
        Some(h) => picture(h.clone())
            .width(160)
            .height(72)
            .fit(ContentFit::Contain)
            .into(),
        None => Space::new().width(160).height(72).into(),
    };
    let new_icons = [
        Ico::Folder,
        Ico::FolderOpen,
        Ico::FolderGit2,
        Ico::FileArchive,
        Ico::FileSpreadsheet,
        Ico::HardDrive,
        Ico::Network,
        Ico::Unplug,
        Ico::ArrowDownAZ,
        Ico::ListFilter,
        Ico::SquareTerminal,
        Ico::Wallpaper,
        Ico::Cookie,
        Ico::SlidersHorizontal,
        Ico::Undo2,
        Ico::Redo2,
    ];
    let icons = row(new_icons
        .into_iter()
        .map(|i| icon(i, 18.0).color(c(p.subtext)).into()))
    .spacing(12);
    let app_icons = row![
        label("App SVGs:", 13.0, 19.5).color(c(p.subtext)),
        icon(Icon::svg(FILE_ZIP), 18.0).color(c(p.accent_bright)),
        icon(Icon::svg_colored(LOGO), 32.0),
        app_mark(on, Ico::FolderTree, 32.0),
        button(on)
            .secondary()
            .sm()
            .icon(Icon::svg(FILE_ZIP))
            .label("Compress")
            .on_press(Msg::Picked("compress")),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    column![
        row![
            button(on)
                .secondary()
                .icon(Ico::RefreshCw)
                .label("Load thumbnails")
                .on_press(Msg::LoadThumbs),
            label(
                "Decoded on 4 workers (400 ms fake latency); 3 is a transient error, 2 has no preview.",
                13.0,
                19.5
            )
            .color(c(p.subtext))
            .wrap(),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
        row![thumbs, wide].spacing(24).align_y(Alignment::Start),
        icons,
        app_icons,
    ]
    .spacing(16)
    .into()
}

fn file_menu() -> Vec<MenuEntry<Msg>> {
    use bato_kit::menu::{action, separator};
    vec![
        action("Open", Msg::Picked("open"))
            .icon(Ico::FolderOpen)
            .into(),
        action("Open with", Msg::Picked("-"))
            .icon(Ico::AppWindow)
            .submenu(vec![
                action("Text editor", Msg::Picked("open with: editor")).into(),
                action("Image viewer", Msg::Picked("open with: viewer")).into(),
                separator(),
                action("More", Msg::Picked("-"))
                    .submenu(vec![
                        action("Hex view", Msg::Picked("open with: hex")).into(),
                        action("Terminal here", Msg::Picked("terminal"))
                            .icon(Ico::SquareTerminal)
                            .into(),
                    ])
                    .into(),
            ])
            .into(),
        action("Copy to", Msg::Picked("-"))
            .icon(Ico::Copy)
            .submenu(vec![
                action("Home", Msg::Picked("copy to: home"))
                    .icon(Ico::Home)
                    .into(),
                action("Downloads", Msg::Picked("copy to: downloads"))
                    .icon(Ico::FolderDown)
                    .into(),
                action("Network share", Msg::Picked("-"))
                    .disabled(true)
                    .into(),
            ])
            .into(),
        separator(),
        action("Move to trash", Msg::Picked("trash"))
            .icon(Ico::Trash2)
            .danger()
            .into(),
    ]
}

fn submenus<'a>(state: &'a State, on: &Ctx) -> El<'a, Msg> {
    let p = on.p().clone();
    let area = surface(label("Right-click anywhere in this box", 13.0, 19.5).color(c(p.muted)))
        .width(Length::Fill)
        .height(72)
        .center()
        .fixed(
            BoxStyle::default()
                .border(c(on.line()))
                .radius(8.0)
                .bg(c(on.wash(0.02))),
        );
    column![
        row![
            dropdown(on, "File", file_menu()),
            label(format!("Picked: {}", state.picked), 13.0, 19.5).color(c(p.subtext)),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
        context_menu(
            on,
            area,
            state.menu_at,
            file_menu(),
            |at| Msg::Menu(Some(at)),
            Msg::Menu(None),
        ),
    ]
    .spacing(12)
    .into()
}

fn rename<'a>(state: &'a State, on: &Ctx, p: &bato_kit::theme::Palette) -> El<'a, Msg> {
    let heights = row![
        text_control(on, "h-8 (32)", "", |_| Msg::Picked("-")).height(32.0),
        text_control(on, "h-9 (36)", "", |_| Msg::Picked("-")).height(36.0),
        text_control(on, "md (38)", "", |_| Msg::Picked("-")),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    column![
        row![
            button(on)
                .primary()
                .icon(Ico::Pencil)
                .label("Rename…")
                .on_press(Msg::OpenRename),
            label(
                "Opens on the name field (not ✕) with “Quarterly report (final)” selected.",
                13.0,
                19.5
            )
            .color(c(p.subtext))
            .wrap(),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
        row![
            text_control(on, "", &state.inline, Msg::Inline).id(INLINE),
            button(on)
                .secondary()
                .label("Select stem")
                .on_press(Msg::SelectStem),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        heights,
        row![
            switch(on, state.ticking, Msg::Ticking),
            label(
                format!("timer::every(1 s) while on: {} ticks", state.ticks),
                13.0,
                19.5
            )
            .color(c(p.subtext)),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    ]
    .spacing(16)
    .into()
}

fn labels<'a>(on: &Ctx, p: &bato_kit::theme::Palette) -> El<'a, Msg> {
    let name = "SomeVeryLongCamelCaseFileName-final_v2.backup.tar.gz";
    let boxed = |title: &str, l: bato_kit::label::Label| -> El<'a, Msg> {
        column![
            label(title.to_string(), 12.0, 16.0).color(c(p.muted)),
            surface(l.color(c(p.text)))
                .width(132)
                .padding(8)
                .fixed(BoxStyle::default().border(c(on.line())).radius(6.0)),
        ]
        .spacing(4)
        .into()
    };
    let item = |n: &'static str| -> El<'a, Msg> {
        reveal_group(
            surface(
                row![
                    icon(Ico::File, 16.0).color(c(p.muted)),
                    label(n, 13.0, 19.5).color(c(p.text)).width(Length::Fill),
                    revealed(
                        button(on)
                            .ghost()
                            .sm()
                            .square(28)
                            .icon(Ico::Pencil)
                            .on_press(Msg::Picked("reveal: edit"))
                    ),
                    revealed(
                        button(on)
                            .ghost()
                            .sm()
                            .square(28)
                            .icon(Ico::Trash2)
                            .on_press(Msg::Picked("reveal: delete"))
                    ),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .padding([4, 10])
            .width(Length::Fill)
            .fixed(BoxStyle::default().border(c(on.line())).radius(8.0)),
        )
        .into()
    };
    column![
        row![
            boxed(
                "\\n",
                label("First line\nSecond line\n\nAfter a blank line", 13.0, 19.5)
            ),
            boxed("wrap", label(name, 13.0, 19.5).wrap()),
            boxed(
                "+ hints",
                label(name, 13.0, 19.5).wrap().wrap_hints(hints(name))
            ),
            boxed(
                "+ anywhere, clamp 3",
                label(name, 13.0, 19.5)
                    .wrap()
                    .wrap_hints(hints(name))
                    .break_anywhere()
                    .max_lines(3)
            ),
        ]
        .spacing(12),
        label(
            "Actions show on hover or when Tab moves focus into a row:",
            13.0,
            19.5
        )
        .color(c(p.subtext)),
        column![item("notes.md"), item("budget-2026.xlsx")].spacing(6),
    ]
    .spacing(16)
    .into()
}

fn grid<'a>(state: &'a State, on: &Ctx, p: &bato_kit::theme::Palette) -> El<'a, Msg> {
    let names = &state.names;
    let cursor = state.cursor;
    let (accent, line, text, muted) = (c(p.accent_bright), c(on.line()), c(p.text), c(p.muted));
    let sel_bg = c(on.wash(0.07));
    let name_label = |i: usize| {
        label(names[i].clone(), 12.0, 16.0)
            .wrap()
            .wrap_hints(hints(&names[i]))
            .break_anywhere()
            .max_lines(3)
            .align(bato_kit::label::Align::Center)
    };
    let heights =
        cached_heights(move |i, w| 8.0 + 40.0 + 6.0 + name_label(i).shape(w - 12.0).height() + 8.0);
    let tiles = virtual_grid(GRID_TILES, 104.0, heights, move |i| {
        let ico = match i % 5 {
            0 => Ico::Folder,
            1 => Ico::FileImage,
            2 => Ico::FileText,
            3 => Ico::FileArchive,
            _ => Ico::FileCode,
        };
        let on_it = i == cursor;
        surface(
            column![
                icon(ico, 40.0).color(if on_it { accent } else { muted }),
                name_label(i).color(text).width(Length::Fill),
            ]
            .spacing(6)
            .align_x(Alignment::Center),
        )
        .padding([8, 6])
        .width(Length::Fill)
        .height(Length::Fill)
        .on_press(Msg::Jump(i))
        .fixed(if on_it {
            BoxStyle::default().bg(sel_bg).border(accent).radius(8.0)
        } else {
            BoxStyle::default().radius(8.0)
        })
        .into()
    })
    .gap(8.0)
    .cursor(Some(cursor))
    .on_layout(Msg::Grid);
    let mv = |t: Ico, dx: i32, dy: i32| {
        button(on)
            .secondary()
            .sm()
            .square(30)
            .icon(t)
            .on_press(Msg::Move(dx, dy))
    };
    let cols = state.grid.map_or("…".to_string(), |g| {
        format!("{} columns of {:.0} px", g.columns, g.tile_width)
    });
    column![
        row![
            mv(Ico::ChevronLeft, -1, 0),
            mv(Ico::ChevronUp, 0, -1),
            mv(Ico::ChevronDown, 0, 1),
            mv(Ico::ChevronRight, 1, 0),
            button(on)
                .secondary()
                .sm()
                .label("Jump to 1200")
                .on_press(Msg::Jump(1200)),
            label(
                format!("{GRID_TILES} tiles · cursor {} · {cols}", cursor + 1),
                13.0,
                19.5
            )
            .color(c(p.subtext)),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        scroll_area(on, tiles).max_height(360.0).border(line, 8.0),
    ]
    .spacing(12)
    .into()
}

/// BatoBrowse's `nameSegments` break points (after `-_+,~`, after a `.`
/// not inside a version number, before a bracket, at camel humps), as byte
/// offsets for `Label::wrap_hints`.
fn hints(name: &str) -> Vec<usize> {
    let ch: Vec<(usize, char)> = name.char_indices().collect();
    let mut out = Vec::new();
    let mut start = 0;
    for k in 1..ch.len() {
        let (i, c) = ch[k];
        let prev = ch[k - 1].1;
        let digit = |j: usize| ch.get(j).is_some_and(|x| x.1.is_ascii_digit());
        let after_sep = "-_+,~".contains(prev)
            || (prev == '.' && !(k >= 2 && digit(k - 2) && c.is_ascii_digit()));
        let bracket = "([{".contains(c);
        let camel = prev.is_lowercase() && c.is_uppercase();
        if (after_sep || bracket || camel) && name[start..i].chars().any(char::is_alphanumeric) {
            out.push(i);
            start = i;
        }
    }
    out
}

/// Grid names: a mix of short, long and unbroken ones.
fn name(i: usize) -> String {
    const WORDS: [&str; 8] = [
        "notes",
        "HolidayPhotos",
        "ruby-pickaxe",
        "Q3_report_final",
        "a",
        "SomeVeryLongCamelCaseName",
        "build.2026-11-15",
        "README",
    ];
    const EXT: [&str; 5] = ["md", "png", "tar.gz", "rs", "pdf"];
    format!("{}-{i}.{}", WORDS[i % 8], EXT[(i / 8) % 5])
}

/// The fake thumbnail server: two images, "no preview", a transient error.
fn fetch(k: u32) -> Result<Option<Vec<u8>>, String> {
    std::thread::sleep(Duration::from_millis(400));
    match k {
        0 => Ok(Some(bmp(96, 64, |x, y| [x as u8 * 2, y as u8 * 3, 200]))),
        1 => Ok(Some(bmp(64, 64, |x, y| {
            if (x / 8 + y / 8) % 2 == 0 {
                [234, 179, 8]
            } else {
                [30, 30, 36]
            }
        }))),
        2 => Ok(None),
        _ => Err("connection refused".into()),
    }
}

/// A 24-bit BMP (the smallest encoder there is).
fn bmp(w: u32, h: u32, px: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let stride = (w * 3).div_ceil(4) * 4;
    let size = 54 + stride * h;
    let mut b = Vec::with_capacity(size as usize);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&size.to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&(h as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&[0; 24]);
    for y in (0..h).rev() {
        let mut n = 0;
        for x in 0..w {
            let [r, g, bl] = px(x, y);
            b.extend_from_slice(&[bl, g, r]);
            n += 3;
        }
        while n % 4 != 0 {
            b.push(0);
            n += 1;
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_follow_name_segments() {
        let at = |n: &str| {
            let mut v = Vec::new();
            let mut s = 0;
            for h in hints(n) {
                v.push(&n[s..h]);
                s = h;
            }
            v.push(&n[s..]);
            v.join("|")
        };
        assert_eq!(at("ruby-pickaxe.png"), "ruby-|pickaxe.|png");
        assert_eq!(at(".bashrc"), ".bashrc");
        assert_eq!(at("v0.2.0.tar"), "v0.2.0.|tar");
        assert_eq!(at("HolidayPhotos(1).jpg"), "Holiday|Photos|(1).|jpg");
    }

    #[test]
    fn fake_thumbnails_decode() {
        let b = bmp(5, 3, |_, _| [1, 2, 3]);
        assert!(thumb::decode(&b, None).is_ok());
    }
}
