//! Interaction lab section: OS drags of files to other apps
//! (`bato_kit::dnd::os_drag`, Electron's `webContents.startDrag`).
//!
//! Each chip drags real files (written to a temp folder on the first drag)
//! out of the window: drop them on a file manager, a browser or a chat app.
//! The drop zone below takes any file drop and says whether it was this
//! window's own drag coming back.

use std::path::PathBuf;

use bato_kit::dnd::os_drag::{self, Outcome};
use bato_kit::iced::widget::{column, row};
use bato_kit::iced::{Color, Task};
use bato_kit::inputs::file_dropzone;
use bato_kit::prelude::*;
use bato_kit::surface::BoxStyle;

/// The demo files: name and contents.
const FILES: [(&str, &str); 2] = [
    ("notes.txt", "Dragged out of the bato-native gallery.\n"),
    (
        "swatch.svg",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"64\"><rect width=\"64\" height=\"64\" rx=\"12\" fill=\"#3b82f6\"/></svg>\n",
    ),
];

#[derive(Debug, Default)]
pub struct State {
    outcome: String,
    dropped: String,
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Drag chip `i` (`FILES.len()`: all of them) out, the icon in this color.
    DragOut(usize, Color),
    Ended(Outcome),
    Dropped(Vec<PathBuf>),
}

/// The demo files, written on first use.
fn files(which: usize) -> std::io::Result<Vec<PathBuf>> {
    let dir = std::env::temp_dir().join("bato-gallery-drag");
    std::fs::create_dir_all(&dir)?;
    let mut paths = Vec::new();
    for (i, (name, body)) in FILES.iter().enumerate() {
        if which == i || which == FILES.len() {
            let path = dir.join(name);
            std::fs::write(&path, body)?;
            paths.push(path);
        }
    }
    Ok(paths)
}

pub fn update(state: &mut State, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::DragOut(which, accent) => match files(which) {
            Ok(paths) => {
                state.outcome = format!("dragging {} file(s)…", paths.len());
                let icon = os_drag::file_icon(paths.len(), accent, 2);
                os_drag::Drag::new(paths).icon(icon).start().map(Msg::Ended)
            }
            Err(e) => {
                state.outcome = format!("cannot write the demo files: {e}");
                Task::none()
            }
        },
        Msg::Ended(outcome) => {
            state.outcome = format!("{outcome:?}");
            Task::none()
        }
        Msg::Dropped(paths) => {
            let from = match os_drag::own_drop() {
                Some(id) => format!("this window's own drag #{id}"),
                None => "another app".into(),
            };
            let names: Vec<_> = paths.iter().map(|p| p.display().to_string()).collect();
            state.dropped = format!("{} from {from}: {}", paths.len(), names.join(", "));
            Task::none()
        }
    }
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    let accent = c(p.accent);
    let chip = |which: usize, ico: Ico, text: &'static str| -> El<'a, Msg> {
        let body = row![
            icon(ico, 16.0).color(c(p.text)),
            label(text, 13.0, 18.0).color(c(p.text))
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        os_drag::drag_source(
            surface(body).padding([8, 12]).fixed(
                BoxStyle::default()
                    .bg(c(on.recessed()))
                    .border(c(on.line_strong()))
                    .radius(8.0),
            ),
            Msg::DragOut(which, accent),
        )
        .into()
    };
    let note = |t: String| label(t, 12.0, 16.0).mono().color(c(p.muted)).wrap();
    card(cx, "Drag files to other apps")
        .body(column![
            label("Press a chip and drag it out of the window: the files go to a file manager, a browser or a chat app as a real OS drag (text/uri-list, copy).", 14.0, 20.0)
                .color(c(p.subtext))
                .wrap(),
            vgap(12.0),
            row![
                chip(0, Ico::FileText, FILES[0].0),
                chip(1, Ico::Palette, FILES[1].0),
                chip(FILES.len(), Ico::Copy, "Both files"),
            ]
            .spacing(8),
            vgap(16.0),
            file_dropzone(&on)
                .hint("Drop a chip back here: it is recognised as our own drag")
                .on_drop(Msg::Dropped),
            vgap(8.0),
            note(format!("drag: {}", if state.outcome.is_empty() { "—" } else { &state.outcome })),
            note(format!("drop: {}", if state.dropped.is_empty() { "—" } else { &state.dropped })),
        ])
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_pick_their_files() {
        let one = files(0).unwrap();
        assert_eq!(one.len(), 1);
        assert!(one[0].ends_with("notes.txt"));
        assert_eq!(files(FILES.len()).unwrap().len(), 2);
        assert!(
            std::fs::read_to_string(&one[0])
                .unwrap()
                .contains("gallery")
        );
    }
}
