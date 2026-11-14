//! Toasts (`Toast.tsx`): a bottom-right stack, every toast as wide as the
//! widest one (max 384), slideIn, auto-dismiss after 3.5 s without a timer
//! thread (redraw-at-deadline, see [`crate::timer`]), close button.
//! The [`crate::frame::Frame`] owns a stack; apps call `frame.toast(…)`.

use std::time::{Duration, Instant};

use iced::widget::{container, row};
use iced::{Alignment, Length, Padding};

use crate::El;
use crate::badge::Tone;
use crate::ctx::Ctx;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::layout::stretch_column;
use crate::style::{c, shadows};
use crate::surface::{BoxStyle, Status, surface};
use crate::timer::timer;

/// How long a toast stays.
pub const TOAST_TTL: Duration = Duration::from_millis(3500);

#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub text: String,
    pub tone: Tone,
    pub born: Instant,
}

/// A queue of toasts with ids.
#[derive(Debug, Clone, Default)]
pub struct Toasts {
    pub items: Vec<Toast>,
    next: u64,
}

impl Toasts {
    pub fn push(&mut self, text: impl Into<String>, tone: Tone) -> u64 {
        self.next += 1;
        self.items.push(Toast { id: self.next, text: text.into(), tone, born: Instant::now() });
        self.next
    }
    pub fn dismiss(&mut self, id: u64) {
        self.items.retain(|t| t.id != id);
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// The stack, full-window, aligned bottom-right with 16 px margin; put it
/// in a `stack` above the page. `dismiss(id)` is published on close and at
/// the deadline.
pub fn toast_stack<'a, M: Clone + 'a>(cx: &Ctx, toasts: &[Toast], dismiss: impl Fn(u64) -> M) -> El<'a, M> {
    let cx = cx.on_card();
    let p = cx.p();
    // Every toast is as wide as the widest one; the message is `flex-1`.
    let text_w = toasts
        .iter()
        .map(|t| label(t.text.clone(), 13.0, 19.5).shape(f32::INFINITY).width())
        .fold(0.0f32, f32::max)
        .min(292.0);
    let mut col: Vec<El<'a, M>> = Vec::new();
    for toast in toasts {
        let (ico, color) = match toast.tone {
            Tone::Success => (Ico::CheckCircle, p.green),
            Tone::Error => (Ico::XCircle, p.red),
            Tone::Warning => (Ico::AlertCircle, p.yellow),
            _ => (Ico::Info, p.accent_bright),
        };
        let st = BoxStyle {
            bg: Some(c(p.card)),
            border: Some(c(cx.line_strong())),
            radius: 12.0,
            shadows: shadows(&p.shadow_lg),
            ..BoxStyle::default()
        };
        let (text, muted) = (p.text, p.muted);
        let close = surface(icon(Ico::X, 14.0))
            .on_press(dismiss(toast.id))
            .style(move |s| BoxStyle::default().radius(4.0).fg(c(if s == Status::Hover { text } else { muted })));
        col.push(
            timer(
                surface(
                    row![
                        icon(ico, 16.0).color(c(color)),
                        label(toast.text.clone(), 13.0, 19.5).color(c(p.text)).width(text_w).ellipsis(),
                        container(close).padding(Padding { left: 4.0, ..Padding::ZERO }),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .padding(Padding::from([13.0, 17.0]))
                .fixed(st),
                toast.born,
                TOAST_TTL,
                dismiss(toast.id),
            )
            .slide_in()
            .into(),
        );
    }
    container(stretch_column(col, 8.0, 384.0))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(16)
        .align_x(Alignment::End)
        .align_y(Alignment::End)
        .into()
}
