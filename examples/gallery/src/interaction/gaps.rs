//! Interaction lab section: Restart to update, the window icon and file
//! dialogs parented to the window (Phase 2 "gaps").
//!
//! The update buttons ask the app to show a status in the frame (About, the
//! sidebar card): the app routes [`simulated`] messages to
//! `Frame::set_update_status`. The dialog buttons open real portal dialogs,
//! modal to this window on X11 and Wayland.
//!
//! `BATO_SELFTEST` ([`selftest`]) drives the same paths without input:
//! `update=downloaded` (`checking`, `available`, `downloading`, `uptodate`,
//! `error`) shows a status from the start; `dialog-parent` prints the
//! window's portal parent (`x11:…` / `wayland:…`) and exits, opening no
//! dialog; `restart` presses "Restart" as soon as the real updater reports
//! a downloaded release.

use bato_kit::frame::UpdateStatus;
use bato_kit::iced::Task;
use bato_kit::iced::widget::{column, row};
use bato_kit::prelude::*;
use bato_shell::dialogs::{DialogError, FileFilter, OpenDirectory, OpenFiles, SaveFile};
use bato_shell::iced_ext;

pub const ENV_SELFTEST: &str = "BATO_SELFTEST";

#[derive(Debug, Default)]
pub struct State {
    result: String,
    parent: String,
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Routed by the app to the frame ([`simulated`]).
    Simulate(UpdateStatus),
    OpenFiles,
    OpenFolder,
    SaveAs,
    Picked(String),
    ExportParent,
    Parent(String),
}

/// The status a message asks the frame to show.
pub fn simulated(msg: &Msg) -> Option<&UpdateStatus> {
    match msg {
        Msg::Simulate(s) => Some(s),
        _ => None,
    }
}

/// What `BATO_SELFTEST` asks of this section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelfTest {
    Update(UpdateStatus),
    DialogParent,
    Restart,
}

pub fn selftest() -> Option<SelfTest> {
    parse_selftest(&std::env::var(ENV_SELFTEST).ok()?)
}

pub fn parse_selftest(s: &str) -> Option<SelfTest> {
    let s = s.trim();
    if let Some(phase) = s.strip_prefix("update=") {
        return status_named(phase).map(SelfTest::Update);
    }
    match s {
        "dialog-parent" => Some(SelfTest::DialogParent),
        "restart" => Some(SelfTest::Restart),
        _ => None,
    }
}

/// The demo statuses, by name.
pub fn status_named(name: &str) -> Option<UpdateStatus> {
    Some(match name {
        "idle" => UpdateStatus::Idle,
        "checking" => UpdateStatus::Checking,
        "available" => UpdateStatus::Available {
            version: "0.5.0".into(),
        },
        "uptodate" => UpdateStatus::UpToDate,
        "downloading" => UpdateStatus::Downloading { percent: 42 },
        "downloaded" => UpdateStatus::Downloaded {
            version: "0.5.0".into(),
        },
        "error" => UpdateStatus::Error {
            message: "The bato CLI was not found. To update, run: bato install batopoc-gallery"
                .into(),
        },
        _ => return None,
    })
}

fn describe<T: std::fmt::Debug>(r: Result<T, DialogError>) -> String {
    match r {
        Ok(v) => format!("{v:?}"),
        Err(e) => e.to_string(),
    }
}

pub fn update(state: &mut State, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Simulate(_) => {}
        Msg::OpenFiles => {
            return iced_ext::open_files(OpenFiles {
                filters: vec![
                    FileFilter::new("Images", ["png", "jpg", "svg"]),
                    FileFilter::new("All Files", ["*"]),
                ],
                ..OpenFiles::default()
            })
            .map(|r| Msg::Picked(describe(r)));
        }
        Msg::OpenFolder => {
            return iced_ext::open_directory(OpenDirectory::default())
                .map(|r| Msg::Picked(describe(r)));
        }
        Msg::SaveAs => {
            return iced_ext::save_file(SaveFile {
                default_path: Some("notes.md".into()),
                ..SaveFile::default()
            })
            .map(|r| Msg::Picked(describe(r)));
        }
        Msg::Picked(r) => state.result = r,
        Msg::ExportParent => {
            return iced_ext::latest_dialog_parent().map(|p| Msg::Parent(p.portal_string()));
        }
        Msg::Parent(p) => {
            state.parent = if p.is_empty() {
                "none (no exporter)".into()
            } else {
                p
            }
        }
    }
    Task::none()
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    let sim = |name: &'static str, text: &'static str| -> El<'a, Msg> {
        button(&on)
            .secondary()
            .sm()
            .label(text)
            .on_press_maybe(status_named(name).map(Msg::Simulate))
            .into()
    };
    let note = |t: String| label(t, 12.0, 16.0).mono().color(c(p.muted)).wrap();
    card(cx, "Updates, window icon and file dialogs")
        .body(
            column![
                label("Show an updater status in About and the sidebar. “Restart” there starts the installed release (packaged builds only).", 14.0, 20.0)
                    .color(c(p.subtext))
                    .wrap(),
                vgap(12.0),
                row![
                    sim("idle", "Idle"),
                    sim("checking", "Checking"),
                    sim("downloading", "Downloading 42%"),
                    sim("downloaded", "Downloaded"),
                    sim("error", "Error"),
                ]
                .spacing(8),
                vgap(24.0),
                label("Portal file dialogs, modal to this window (X11 window id, Wayland xdg-foreign export).", 14.0, 20.0)
                    .color(c(p.subtext))
                    .wrap(),
                vgap(12.0),
                row![
                    button(&on).secondary().sm().icon(Ico::Upload).label("Open files…").on_press(Msg::OpenFiles),
                    button(&on).secondary().sm().icon(Ico::FileText).label("Choose folder…").on_press(Msg::OpenFolder),
                    button(&on).secondary().sm().icon(Ico::Save).label("Save as…").on_press(Msg::SaveAs),
                    button(&on).ghost().sm().label("Show parent").on_press(Msg::ExportParent),
                ]
                .spacing(8),
                vgap(8.0),
                note(format!("parent: {}", state.parent)),
                note(format!("result: {}", state.result)),
            ],
        )
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selftests_parse() {
        assert_eq!(
            parse_selftest("update=downloaded"),
            Some(SelfTest::Update(UpdateStatus::Downloaded {
                version: "0.5.0".into()
            }))
        );
        assert_eq!(
            parse_selftest("dialog-parent"),
            Some(SelfTest::DialogParent)
        );
        assert_eq!(parse_selftest(" restart "), Some(SelfTest::Restart));
        assert_eq!(parse_selftest("update=nope"), None);
        assert_eq!(parse_selftest("focus-trap"), None, "another agent's");
    }

    #[test]
    fn simulate_routes_to_the_frame() {
        let m = Msg::Simulate(UpdateStatus::Checking);
        assert_eq!(simulated(&m), Some(&UpdateStatus::Checking));
        assert_eq!(simulated(&Msg::OpenFiles), None);
    }
}
