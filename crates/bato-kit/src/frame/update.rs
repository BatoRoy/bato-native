//! Update status in the frame: the About dialog's action and note, and the
//! sidebar's update card (React `AboutDialog.tsx`, `Sidebar.tsx`,
//! `useUpdateStatus.ts`).
//!
//! The frame does no I/O: the app feeds its updater's phases in with
//! [`super::Frame::set_update_status`] (or [`super::Msg::UpdateStatus`]) and
//! answers [`super::Action::RestartToUpdate`] by restarting into the
//! installed release.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::widget::{Space, column, container, row};
use iced::{Alignment, Color, Element, Event, Length, Padding, Rectangle, Size, mouse};

use super::{Frame, Msg};
use crate::El;
use crate::anim::{epoch, pulse, with_opacity};
use crate::button::button;
use crate::ctx::Ctx;
use crate::icon::{Ico, icon};
use crate::label::{Align, label};
use crate::layout::vgap;
use crate::rail_tip::rail_tip;
use crate::style::c;
use crate::surface::{BoxStyle, surface};

/// Where the app's updater is: Electron's `update:status` phases, plus
/// `Idle` before any status arrived.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    /// Found; the download starts on its own (Electron's `autoDownload`).
    Available {
        version: String,
    },
    /// Electron's `none`.
    UpToDate,
    Downloading {
        percent: u8,
    },
    /// Installed; a restart runs it.
    Downloaded {
        version: String,
    },
    Error {
        message: String,
    },
}

const CHECK_FAILED: &str = "Update check failed: ";

impl UpdateStatus {
    /// The About button shows its spinner and is disabled (`loading={busy}`).
    pub fn busy(&self) -> bool {
        matches!(self, Self::Checking | Self::Downloading { .. })
    }

    /// The line under the About button, as `AboutDialog.tsx` words it.
    pub fn note(&self) -> Option<String> {
        Some(match self {
            Self::Checking => "Checking for updates…".into(),
            Self::UpToDate => "You’re up to date.".into(),
            Self::Available { version } => format!("Update v{version} found — downloading…"),
            Self::Downloading { percent } => format!("Downloading update… {percent}%"),
            // Electron's message never carries the prefix; ours may already.
            Self::Error { message } => format!(
                "{CHECK_FAILED}{}",
                message.strip_prefix(CHECK_FAILED).unwrap_or(message)
            ),
            Self::Idle | Self::Downloaded { .. } => return None,
        })
    }

    /// What a freshly opened About shows. React mounts the dialog with
    /// `idle`; the frame keeps what is still going on (a check, a download,
    /// a pending restart) and forgets finished checks.
    pub(super) fn on_about_open(&self) -> Self {
        match self {
            Self::UpToDate | Self::Error { .. } => Self::Idle,
            s => s.clone(),
        }
    }
}

impl<P: Copy + PartialEq + std::fmt::Debug + Send + 'static> Frame<P> {
    /// About: "Restart to install vX" (primary) once downloaded, else
    /// "Check for updates" (secondary, spinner while busy), then the note.
    pub(super) fn about_update_view<'a, M: Clone + 'a>(
        &'a self,
        cx: &Ctx,
        wrap: fn(Msg<P>) -> M,
    ) -> El<'a, M> {
        let p = cx.p();
        let action: El<'a, M> = match &self.update {
            UpdateStatus::Downloaded { version } => button(cx)
                .icon(Ico::RefreshCw)
                .label(format!("Restart to install v{version}"))
                .on_press(wrap(Msg::RestartToUpdate))
                .into(),
            s => button(cx)
                .secondary()
                .icon(Ico::RefreshCw)
                .label("Check for updates")
                .loading(s.busy())
                .on_press(wrap(Msg::CheckUpdates))
                .into(),
        };
        // `unsupported` (the app's note) wins, as in React.
        let note = self
            .update_note
            .clone()
            .or_else(|| self.update.note())
            .unwrap_or_default();
        column![
            action,
            vgap(8.0),
            // `min-h-[1rem]`: one empty line, growing for a long error.
            row![
                Space::new().width(0).height(16),
                label(note, 12.0, 16.0)
                    .color(c(p.muted))
                    .wrap()
                    .align(Align::Center)
                    .width(Length::Fill),
            ],
        ]
        .align_x(Alignment::Center)
        .width(Length::Fill)
        .into()
    }

    /// The sidebar's update card between the nav and the footer: progress
    /// while downloading, "vX is ready" + Restart once downloaded. Nothing
    /// otherwise.
    pub(super) fn sidebar_update_view<'a, M: Clone + 'a>(
        &'a self,
        collapsed: bool,
        wrap: fn(Msg<P>) -> M,
    ) -> Option<El<'a, M>> {
        let cx = &self.cx;
        let p = cx.p();
        let card = |content: El<'a, M>| -> El<'a, M> {
            // rounded-xl p-3 bg-app-text/[0.07]
            surface(content)
                .width(Length::Fill)
                .padding(12)
                .fixed(BoxStyle::default().bg(c(cx.wash(0.07))).radius(12.0))
                .into()
        };
        let body: El<'a, M> = match (&self.update, collapsed) {
            (UpdateStatus::Downloading { percent }, true) => rail_tip(
                cx,
                container(Pulsing::new(
                    icon(Ico::Download, 16.0).color(c(p.accent_bright)),
                ))
                .width(Length::Fill)
                .padding(Padding::from([8.0, 0.0]))
                .center_x(Length::Fill),
                &format!("Updating… {percent}%"),
            ),
            (UpdateStatus::Downloading { percent }, false) => {
                let on = cx.on(cx.wash(0.07));
                let track = on.wash(0.10);
                // FillPortion(0) is not "nothing" in iced: the ends are explicit.
                let pct = u16::from((*percent).min(100));
                let filled = || {
                    surface(Space::new())
                        .height(4)
                        .fixed(BoxStyle::default().bg(c(p.accent)).radius(2.0))
                };
                let bar: El<'a, M> = match pct {
                    0 => Space::new().width(Length::Fill).height(4).into(),
                    100 => filled().width(Length::Fill).into(),
                    _ => row![
                        filled().width(Length::FillPortion(pct)),
                        Space::new().width(Length::FillPortion(100 - pct)).height(4),
                    ]
                    .into(),
                };
                card(
                    column![
                        row![
                            label("Downloading update", 12.0, 16.0)
                                .color(c(p.subtext))
                                .width(Length::Fill),
                            label(format!("{percent}%"), 12.0, 16.0)
                                .mono()
                                .color(c(p.subtext)),
                        ],
                        vgap(8.0),
                        surface(bar)
                            .width(Length::Fill)
                            .height(4)
                            .clip()
                            .fixed(BoxStyle::default().bg(c(track)).radius(2.0)),
                    ]
                    .into(),
                )
            }
            (UpdateStatus::Downloaded { version }, true) => rail_tip(
                cx,
                button(cx)
                    .square(32)
                    .width(Length::Fill)
                    .icon(Ico::RefreshCw)
                    .on_press(wrap(Msg::RestartToUpdate)),
                &format!("Restart to update to v{version}"),
            ),
            (UpdateStatus::Downloaded { version }, false) => card(
                column![
                    label(format!("v{version} is ready"), 13.0, 19.5)
                        .medium()
                        .color(c(p.text)),
                    vgap(2.0),
                    label("Restart to finish updating.", 12.0, 16.0).color(c(p.subtext)),
                    vgap(10.0),
                    button(cx)
                        .sm()
                        .width(Length::Fill)
                        .icon(Ico::RefreshCw)
                        .label("Restart")
                        .on_press(wrap(Msg::RestartToUpdate)),
                ]
                .into(),
            ),
            _ => return None,
        };
        // px-3 pb-2
        Some(
            container(body)
                .padding(Padding {
                    top: 0.0,
                    right: 12.0,
                    bottom: 8.0,
                    left: 12.0,
                })
                .into(),
        )
    }
}

/// Tailwind `animate-pulse` on `content`: opacity 1 → .5 → 1 every 2 s,
/// redrawing only while it is drawn.
struct Pulsing<'a, M> {
    content: Element<'a, M, iced::Theme, iced::Renderer>,
}

impl<'a, M> Pulsing<'a, M> {
    fn new(content: impl Into<Element<'a, M, iced::Theme, iced::Renderer>>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

impl<M> Widget<M, iced::Theme, iced::Renderer> for Pulsing<'_, M> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let n = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        layout::Node::with_children(n.size(), vec![n])
    }
    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        if matches!(
            event,
            Event::Window(iced::window::Event::RedrawRequested(_))
        ) {
            shell.request_redraw();
        }
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let alpha = pulse(epoch().elapsed());
        let style = renderer::Style {
            text_color: Color {
                a: style.text_color.a * alpha,
                ..style.text_color
            },
        };
        with_opacity(alpha, || {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                &style,
                layout.children().next().unwrap(),
                cursor,
                viewport,
            );
        });
    }
}

impl<'a, M: 'a> From<Pulsing<'a, M>> for Element<'a, M, iced::Theme, iced::Renderer> {
    fn from(p: Pulsing<'a, M>) -> Self {
        Element::new(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_follow_the_about_dialog() {
        use UpdateStatus::*;
        assert_eq!(Idle.note(), None);
        assert_eq!(Checking.note().unwrap(), "Checking for updates…");
        assert_eq!(UpToDate.note().unwrap(), "You’re up to date.");
        assert_eq!(
            Available {
                version: "1.3.0".into()
            }
            .note()
            .unwrap(),
            "Update v1.3.0 found — downloading…"
        );
        assert_eq!(
            Downloading { percent: 42 }.note().unwrap(),
            "Downloading update… 42%"
        );
        assert_eq!(
            Downloaded {
                version: "1.3.0".into()
            }
            .note(),
            None,
            "the button says it"
        );
        let e = |m: &str| Error { message: m.into() }.note().unwrap();
        assert_eq!(e("404 Not Found"), "Update check failed: 404 Not Found");
        assert_eq!(
            e("Update check failed: no such program"),
            "Update check failed: no such program",
            "no doubled prefix"
        );
    }

    #[test]
    fn busy_and_reopen() {
        use UpdateStatus::*;
        assert!(Checking.busy() && Downloading { percent: 1 }.busy());
        assert!(!Idle.busy() && !UpToDate.busy());
        assert_eq!(UpToDate.on_about_open(), Idle);
        assert_eq!(
            Error {
                message: "x".into()
            }
            .on_about_open(),
            Idle
        );
        let done = Downloaded {
            version: "2.0.0".into(),
        };
        assert_eq!(done.on_about_open(), done);
        assert_eq!(Checking.on_about_open(), Checking);
    }
}
