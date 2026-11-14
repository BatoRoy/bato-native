//! `Alert`: an inline callout. `flex gap-3 px-4 py-3 rounded-lg border`, a
//! 10 % tone tint with a 30 % tone border, a 16 px tone icon (`mt-0.5`), a
//! 14/20 medium title and 14/20 subtext body (`mt-0.5`), and an optional
//! close button (14 px `X`, muted → text on hover, stretched to the row's
//! height so the icon sits in the middle). Laid out as a grid so the cells
//! stretch like the flex row's items.
//!
//! ```ignore
//! alert(&cx, Tone::Info, "Heads up").body("This is an informational message.")
//! alert(&cx, Tone::Warning, "Careful").on_close(Msg::Dismiss)
//! ```

use iced::widget::Column;
use iced::{Length, Padding};

use crate::El;
use crate::badge::Tone;
use crate::ctx::Ctx;
use crate::focus::Ring;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::layout::{Grid, Track, vgap};
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

pub struct Alert<'a, M> {
    cx: Ctx,
    tone: Tone,
    title: Option<String>,
    body: Option<El<'a, M>>,
    on_close: Option<M>,
}

/// An alert of `tone` (`Neutral` reads as `Info`, React's default).
pub fn alert<'a, M: Clone + 'a>(cx: &Ctx, tone: Tone, title: impl Into<String>) -> Alert<'a, M> {
    let title = title.into();
    Alert {
        cx: cx.clone(),
        tone,
        title: (!title.is_empty()).then_some(title),
        body: None,
        on_close: None,
    }
}

/// The tone's icon (`Info`, `CheckCircle`, `AlertTriangle`, `XCircle`).
pub fn tone_icon(tone: Tone) -> Ico {
    match tone {
        Tone::Success => Ico::CheckCircle,
        Tone::Warning => Ico::AlertTriangle,
        Tone::Error => Ico::XCircle,
        Tone::Info | Tone::Neutral => Ico::Info,
    }
}

impl<'a, M: Clone + 'a> Alert<'a, M> {
    /// The body text (14/20 subtext, wraps).
    pub fn body(self, text: impl Into<String>) -> Self {
        let p = self.cx.p();
        let l = label(text, 14.0, 20.0).color(c(p.subtext)).wrap();
        self.content(l)
    }
    /// Any body content instead of text.
    pub fn content(mut self, el: impl Into<El<'a, M>>) -> Self {
        self.body = Some(el.into());
        self
    }
    /// Show the close button.
    pub fn on_close(mut self, m: M) -> Self {
        self.on_close = Some(m);
        self
    }
}

impl<'a, M: Clone + 'a> From<Alert<'a, M>> for El<'a, M> {
    fn from(a: Alert<'a, M>) -> Self {
        let cx = &a.cx;
        let p = cx.p();
        let (tone, ink) = match a.tone {
            Tone::Success => (p.green, p.green),
            Tone::Warning => (p.yellow, p.yellow),
            Tone::Error => (p.red, p.red),
            Tone::Info | Tone::Neutral => (p.accent, p.accent_bright),
        };
        let bg = cx.blend(tone, 0.10);
        let border = bato_theme::blend(tone, 0.30, bg);
        let inner = cx.on(bg);

        let mut text: Column<'a, M, iced::Theme, iced::Renderer> =
            Column::new().width(Length::Fill);
        if let Some(t) = a.title {
            text = text.push(label(t, 14.0, 20.0).medium().color(c(p.text)).wrap());
        }
        if let Some(b) = a.body {
            text = text.push(vgap(2.0)).push(b);
        }
        let mut tracks = vec![Track::Px(16.0), Track::Fr(1.0)];
        let mut items: Vec<El<'a, M>> = vec![
            Column::new()
                .push(vgap(2.0))
                .push(icon(tone_icon(a.tone), 16.0).color(c(ink)))
                .into(),
            text.into(),
        ];
        if let Some(m) = a.on_close {
            let (muted, hover) = (c(p.muted), c(p.text));
            tracks.push(Track::Px(14.0));
            // The grid stretches the button to the row (`align-items:
            // stretch`), so the X sits in the middle and the whole strip
            // is the hit area.
            items.push(
                surface(icon(Ico::X, 14.0))
                    .center_y()
                    .on_press(m)
                    .transition_ms(0)
                    .style(move |s| {
                        BoxStyle::default().fg(if s == Status::Hover { hover } else { muted })
                    })
                    .focus_ring(Ring::plain(&inner))
                    .into(),
            );
        }
        let grid = items
            .into_iter()
            .fold(Grid::tracks(tracks).gap(12.0), |g, el| g.push(el));
        surface(grid)
            .width(Length::Fill)
            // px-4 py-3 plus the 1 px border, which iced draws inside.
            .padding(Padding::from([13.0, 17.0]))
            .fixed(BoxStyle::default().bg(c(bg)).border(c(border)).radius(8.0))
            .into()
    }
}
