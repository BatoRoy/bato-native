//! `AppShell`: the generic page scaffold. A row of the sidebar (as given,
//! full height) and a column of header (`border-b bg-app-bg`), the main
//! region (fills, scrolls: `overflow-auto`) and footer (`border-t
//! bg-app-bg`). It fills the space it is given.
//!
//! ```ignore
//! app_shell(&cx, content).header(top).sidebar(nav).footer(status)
//! ```

use iced::widget::{Column, Row, column};
use iced::{Element, Length};

use super::hline;
use crate::ctx::Ctx;
use crate::scroller::scroll_area;
use crate::style::c;
use crate::surface::{BoxStyle, surface};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

pub struct AppShell<'a, M> {
    cx: Ctx,
    content: El<'a, M>,
    header: Option<El<'a, M>>,
    sidebar: Option<El<'a, M>>,
    footer: Option<El<'a, M>>,
}

/// `<AppShell>{content}</AppShell>`; `cx` is the backdrop of the main
/// region.
pub fn app_shell<'a, M: Clone + 'a>(cx: &Ctx, content: impl Into<El<'a, M>>) -> AppShell<'a, M> {
    AppShell {
        cx: cx.clone(),
        content: content.into(),
        header: None,
        sidebar: None,
        footer: None,
    }
}

impl<'a, M: Clone + 'a> AppShell<'a, M> {
    /// The top bar over the content column.
    pub fn header(mut self, el: impl Into<El<'a, M>>) -> Self {
        self.header = Some(el.into());
        self
    }
    /// The left rail, rendered as is (give it `Length::Fill` height).
    pub fn sidebar(mut self, el: impl Into<El<'a, M>>) -> Self {
        self.sidebar = Some(el.into());
        self
    }
    /// The bottom bar of the content column.
    pub fn footer(mut self, el: impl Into<El<'a, M>>) -> Self {
        self.footer = Some(el.into());
        self
    }
}

impl<'a, M: Clone + 'a> From<AppShell<'a, M>> for El<'a, M> {
    fn from(s: AppShell<'a, M>) -> Self {
        let p = s.cx.p();
        let (bg, border) = (c(p.bg), c(p.border));
        let bar = |content: El<'a, M>, top: bool| -> El<'a, M> {
            let inner: Column<'a, M, iced::Theme, iced::Renderer> = if top {
                column![content, hline(border)]
            } else {
                column![hline(border), content]
            };
            surface(inner.width(Length::Fill))
                .width(Length::Fill)
                .fixed(BoxStyle::default().bg(bg))
                .into()
        };
        let mut col: Column<'a, M, iced::Theme, iced::Renderer> =
            Column::new().width(Length::Fill).height(Length::Fill);
        if let Some(h) = s.header {
            col = col.push(bar(h, true));
        }
        col = col.push(
            scroll_area(&s.cx, s.content)
                .both()
                .width(Length::Fill)
                .height(Length::Fill),
        );
        if let Some(f) = s.footer {
            col = col.push(bar(f, false));
        }
        let mut r: Row<'a, M, iced::Theme, iced::Renderer> =
            Row::new().width(Length::Fill).height(Length::Fill);
        if let Some(sb) = s.sidebar {
            r = r.push(sb);
        }
        r.push(col).into()
    }
}
