//! `Badge` (§4.9): h 24, px 10, pill, 12/16 medium, inset ring, optional
//! status dot. Tints are 10 % (contrast-checked as text on exactly that).
//!
//! ```ignore
//! badge(&cx, Tone::Success, "Online").dot()
//! badge(&cx, Tone::Info, "v0.4.1").mono()
//! ```

use iced::widget::{Space, row};
use iced::{Alignment, Padding};

use crate::El;
use crate::ctx::Ctx;
use crate::label::label;
use crate::style::c;
use crate::surface::{BoxStyle, surface};

/// Status tone shared by badges, toasts and alerts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Neutral,
    Info,
    Success,
    Warning,
    Error,
}

pub struct Badge<'a, M> {
    cx: Ctx,
    tone: Tone,
    text: String,
    content: Option<El<'a, M>>,
    dot: bool,
    mono: bool,
}

pub fn badge<'a, M: Clone + 'a>(cx: &Ctx, tone: Tone, text: impl Into<String>) -> Badge<'a, M> {
    Badge { cx: cx.clone(), tone, text: text.into(), content: None, dot: false, mono: false }
}

impl<'a, M: Clone + 'a> Badge<'a, M> {
    /// Leading status dot (Online / Offline).
    pub fn dot(mut self) -> Self {
        self.dot = true;
        self
    }
    /// JetBrains Mono text (versions).
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
    /// Custom content instead of the text.
    pub fn content(mut self, el: impl Into<El<'a, M>>) -> Self {
        self.content = Some(el.into());
        self
    }
}

impl<'a, M: Clone + 'a> From<Badge<'a, M>> for El<'a, M> {
    fn from(b: Badge<'a, M>) -> Self {
        let cx = &b.cx;
        let p = cx.p();
        let over = cx.over;
        let (bg, fg, ring, dotc) = match b.tone {
            Tone::Neutral => {
                let bg = cx.wash(0.05);
                (bg, p.subtext, p.line_strong.over(bg), p.muted)
            }
            Tone::Info => {
                let bg = cx.tint(0.10);
                (bg, p.accent_bright, bato_theme::blend(p.accent, 0.25, bg), p.accent)
            }
            Tone::Success | Tone::Warning | Tone::Error => {
                let tone = match b.tone {
                    Tone::Success => p.green,
                    Tone::Warning => p.yellow,
                    _ => p.red,
                };
                let bg = bato_theme::blend(tone, 0.10, over);
                (bg, tone, bato_theme::blend(tone, 0.25, bg), tone)
            }
        };
        let mut items: Vec<El<'a, M>> = Vec::new();
        if b.dot {
            let dc = c(dotc);
            items.push(surface(Space::new().width(6).height(6)).fixed(BoxStyle::default().bg(dc).radius(3.0)).into());
        }
        items.push(b.content.unwrap_or_else(|| {
            let l = label(b.text, 12.0, 16.0).medium();
            if b.mono { l.mono().into() } else { l.into() }
        }));
        let style = BoxStyle::default().bg(c(bg)).inner_ring(c(ring)).radius(12.0).fg(c(fg));
        surface(row(items).spacing(6).align_y(Alignment::Center))
            .height(24)
            .padding(Padding::from([0.0, 10.0]))
            .center_y()
            .fixed(style)
            .into()
    }
}
