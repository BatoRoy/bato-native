//! The sidebar (spec §4.1): 224 px expanded, 64 px collapsed (200 ms width
//! animation; labels are removed/added at once and clipped, as in React).

use std::time::Instant;

use iced::widget::{column, container, row};
use iced::{Alignment, Length, Padding};

use super::{Dialog, Frame, Msg};
use crate::El;
use crate::ctx::Ctx;
use crate::focus::Ring;
use crate::icon::{Ico, Icon, icon};
use crate::kbd::{app_mark, kbd};
use crate::label::label;
use crate::layout::{hline, vgap, vline};
use crate::rail_tip::rail_tip;
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

/// `mod+k` as the chip shows it on Linux.
pub const MOD_K: &str = "Ctrl K";

/// One nav / footer row: 36 px, icon 16, 13 px label; active `text@.07`,
/// hover `text@.05`; a rail tip when collapsed. Focus-visible: `ring-2`
/// accentBright, no offset.
fn nav_row<'a, M: Clone + 'a>(
    cx: &Ctx,
    collapsed: bool,
    ico: impl Into<Icon>,
    text: &'a str,
    active: bool,
    msg: M,
) -> El<'a, M> {
    let p = cx.p();
    let icon_color = if active {
        c(p.accent_bright)
    } else {
        c(p.muted)
    };
    let mut items: Vec<El<'a, M>> = vec![icon(ico, 16.0).color(icon_color).into()];
    if !collapsed {
        let mut l = label(text, 13.0, 19.5).ellipsis().width(Length::Fill);
        if active {
            l = l.medium();
        }
        items.push(l.into());
    }
    let t = cx.t.clone();
    let s = surface(row(items).spacing(10).align_y(Alignment::Center))
        .width(Length::Fill)
        .height(36)
        .padding(if collapsed {
            Padding::ZERO
        } else {
            Padding::from([0.0, 12.0])
        })
        .center_y()
        .on_press(msg)
        .focus_ring(Ring::plain(cx))
        .style(move |st| {
            let p = &t.p;
            if active {
                BoxStyle::default()
                    .bg(c(t.wash(0.07, p.bg)))
                    .fg(c(p.text))
                    .radius(8.0)
            } else if st == Status::Hover {
                BoxStyle::default()
                    .bg(c(t.wash(0.05, p.bg)))
                    .fg(c(p.text))
                    .radius(8.0)
            } else {
                BoxStyle::default().fg(c(p.subtext)).radius(8.0)
            }
        });
    if collapsed {
        rail_tip(cx, s.align_x(Alignment::Center), text)
    } else {
        s.into()
    }
}

impl<P: Copy + PartialEq + std::fmt::Debug + Send + 'static> Frame<P> {
    pub(super) fn sidebar_view<'a, M: Clone + 'a>(&'a self, wrap: fn(Msg<P>) -> M) -> El<'a, M> {
        let cx = &self.cx;
        let p = cx.p();
        let collapsed = self.settings.sidebar_collapsed;
        let w = self.sidebar.value(Instant::now());

        let header: El<'a, M> = {
            let mark: El<'a, M> = match self.logo {
                Some(logo) => icon(logo, 32.0).color(c(p.text)).into(),
                None => app_mark(cx, self.mark, 32.0),
            };
            let mut items: Vec<El<'a, M>> = vec![mark];
            if !collapsed {
                items.push(
                    column![
                        label(self.brand.name, 14.0, 17.5)
                            .semibold()
                            .color(c(p.text))
                            .ellipsis(),
                        vgap(2.0),
                        label(format!("v{}", self.brand.version), 11.0, 13.75)
                            .mono()
                            .color(c(p.muted)),
                    ]
                    .width(Length::Fill)
                    .into(),
                );
            }
            let r = row(items).spacing(12).align_y(Alignment::Center);
            let s = surface(r).width(Length::Fill).height(64).center_y();
            if collapsed {
                s.align_x(Alignment::Center).into()
            } else {
                s.padding(Padding::from([0.0, 16.0])).into()
            }
        };

        let search: El<'a, M> = {
            let mut items: Vec<El<'a, M>> = vec![icon(Ico::Search, 14.0).into()];
            if !collapsed {
                items.push(label("Search…", 13.0, 19.5).width(Length::Fill).into());
                items.push(kbd(&cx.on(cx.wash(0.03)), MOD_K));
            }
            let t = cx.t.clone();
            // A button in React that opens the command palette: a Tab stop.
            let s = surface(row(items).spacing(8).align_y(Alignment::Center))
                .width(Length::Fill)
                .height(32)
                .center_y()
                .focusable(true)
                .on_press(wrap(Msg::OpenPalette))
                .focus_ring(Ring::plain(cx))
                .style(move |st| {
                    let p = &t.p;
                    let bg = t.wash(if st == Status::Hover { 0.06 } else { 0.03 }, p.bg);
                    BoxStyle::default()
                        .bg(c(bg))
                        .border(c(t.line_on(bg)))
                        .radius(8.0)
                        .fg(c(if st == Status::Hover {
                            p.subtext
                        } else {
                            p.muted
                        }))
                });
            let s: El<'a, M> = if collapsed {
                rail_tip(
                    cx,
                    s.align_x(Alignment::Center),
                    &format!("Search ({MOD_K})"),
                )
            } else {
                s.padding(Padding::from([0.0, 11.0])).into()
            };
            container(s)
                .padding(Padding {
                    top: 0.0,
                    right: 12.0,
                    bottom: 12.0,
                    left: 12.0,
                })
                .into()
        };

        let nav = column(self.nav.iter().map(|n| {
            nav_row(
                cx,
                collapsed,
                n.icon,
                n.label,
                self.page == n.page,
                wrap(Msg::Navigate(n.page)),
            )
        }))
        .spacing(4)
        .padding(Padding::from([0.0, 12.0]))
        .height(Length::Fill);

        let footer = column![
            hline(c(cx.line())),
            column![
                nav_row(
                    cx,
                    collapsed,
                    Ico::Info,
                    "About",
                    false,
                    wrap(Msg::Open(Dialog::About))
                ),
                nav_row(
                    cx,
                    collapsed,
                    Ico::Settings,
                    "App Options",
                    false,
                    wrap(Msg::Open(Dialog::Options))
                ),
                nav_row(
                    cx,
                    collapsed,
                    if collapsed {
                        Ico::PanelLeftOpen
                    } else {
                        Ico::PanelLeftClose
                    },
                    if collapsed { "Expand" } else { "Collapse" },
                    false,
                    wrap(Msg::ToggleSidebar)
                ),
            ]
            .spacing(4)
            .padding(12),
        ];

        let mut body = column![header, search, nav];
        if let Some(card) = self.sidebar_update_view(collapsed, wrap) {
            body = body.push(card);
        }
        let body = body.push(footer).width(w - 1.0).height(Length::Fill);
        let side = row![body, vline(c(cx.line()))].height(Length::Fill);
        surface(side).width(w).height(Length::Fill).clip().into()
    }
}
