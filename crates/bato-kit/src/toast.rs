//! Toasts (`Toast.tsx`): a bottom-right stack, every toast as wide as the
//! widest one (max 384, longer messages wrap), slideIn, auto-dismiss after
//! 3.5 s (or the toast's own duration) without a timer thread
//! (redraw-at-deadline, see [`crate::timer`]), close button; optionally a
//! title and an action button.
//! The [`crate::frame::Frame`] owns a stack; apps call `frame.toast(…)`.

use std::time::{Duration, Instant};

use iced::widget::{column, container, row};
use iced::{Alignment, Length, Padding};

use crate::El;
use crate::badge::Tone;
use crate::ctx::Ctx;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::layout::{stretch_column, vgap};
use crate::style::{c, shadows};
use crate::surface::{BoxStyle, Status, surface};
use crate::timer::timer;

/// How long a toast stays (`useToast`'s default).
pub const TOAST_TTL: Duration = Duration::from_millis(3500);

/// A toast's action button ("Undo", "Open folder"): `label` on the button,
/// `command` handed back when it is pressed (the frame reports it as
/// [`crate::frame::Action::Command`], like a palette command). Pressing it
/// also dismisses the toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastAction {
    pub label: String,
    pub command: String,
}

/// One toast. Build one with [`Toast::new`] and the optional parts, then
/// show it with [`Toasts::show`] (or `frame.show_toast`):
///
/// ```ignore
/// frame.show_toast(
///     Toast::new("3 files converted", Tone::Success)
///         .title("Done")
///         .action("Open folder", "open-output")
///         .duration(Duration::from_millis(6500)),
/// );
/// ```
#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    /// The message; wraps (the toast is at most 384 px wide).
    pub text: String,
    pub tone: Tone,
    pub born: Instant,
    /// A medium-weight first line.
    pub title: Option<String>,
    pub action: Option<ToastAction>,
    /// Auto-dismiss after this long; `None` stays until closed.
    pub ttl: Option<Duration>,
}

impl Toast {
    /// A toast with the default lifetime ([`TOAST_TTL`]).
    pub fn new(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            id: 0,
            text: text.into(),
            tone,
            born: Instant::now(),
            title: None,
            action: None,
            ttl: Some(TOAST_TTL),
        }
    }
    /// A medium-weight title line over the message.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
    /// An action button under the message; `command` comes back when it
    /// is pressed.
    pub fn action(mut self, label: impl Into<String>, command: impl Into<String>) -> Self {
        self.action = Some(ToastAction {
            label: label.into(),
            command: command.into(),
        });
        self
    }
    /// How long it stays (React's `toast(msg, type, duration)`).
    pub fn duration(mut self, d: Duration) -> Self {
        self.ttl = Some(d);
        self
    }
    /// No auto-dismiss: it stays until closed (or its action is pressed).
    pub fn persistent(mut self) -> Self {
        self.ttl = None;
        self
    }
    /// A title or an action: the two-line layout (`items-start`).
    fn rich(&self) -> bool {
        self.title.is_some() || self.action.is_some()
    }
}

/// A queue of toasts with ids.
#[derive(Debug, Clone, Default)]
pub struct Toasts {
    pub items: Vec<Toast>,
    next: u64,
}

impl Toasts {
    pub fn push(&mut self, text: impl Into<String>, tone: Tone) -> u64 {
        self.show(Toast::new(text, tone))
    }
    /// Show a built [`Toast`]; returns its id. Its lifetime starts now.
    pub fn show(&mut self, mut toast: Toast) -> u64 {
        self.next += 1;
        toast.id = self.next;
        toast.born = Instant::now();
        self.items.push(toast);
        self.next
    }
    pub fn dismiss(&mut self, id: u64) {
        self.items.retain(|t| t.id != id);
    }
    /// The action of toast `id` was pressed: dismiss it and return its
    /// command (`None` if it is gone or has no action).
    pub fn take_action(&mut self, id: u64) -> Option<String> {
        let i = self.items.iter().position(|t| t.id == id)?;
        let t = self.items.remove(i);
        t.action.map(|a| a.command)
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// px-4 × 2 + border × 2, the icon, two gaps, the close button and its
/// `ml-1`: what is left of `max-w-sm` (384) for the text.
const TEXT_MAX: f32 = 384.0 - 34.0 - 16.0 - 24.0 - 14.0 - 4.0;

/// The width every toast's text column gets: the widest single-line text,
/// at most [`TEXT_MAX`] (longer text wraps).
fn text_width(toasts: &[Toast]) -> f32 {
    let natural = |l: crate::label::Label| l.shape(f32::INFINITY).width();
    toasts
        .iter()
        .flat_map(|t| {
            let mut w = vec![natural(label(t.text.clone(), 13.0, 19.5))];
            if let Some(title) = &t.title {
                w.push(natural(label(title.clone(), 13.0, 16.25).medium()));
            }
            if let Some(a) = &t.action {
                w.push(natural(label(a.label.clone(), 12.0, 16.0).semibold()));
            }
            w
        })
        .fold(0.0f32, f32::max)
        .min(TEXT_MAX)
}

/// `l` in the text column `text_w` wide: on one line when it fits (as wide
/// as the column, exactly as before wrapping existed), wrapping otherwise.
fn fit(l: crate::label::Label, text_w: f32) -> crate::label::Label {
    if l.shape(f32::INFINITY).width() <= text_w + 0.01 {
        l.width(text_w)
    } else {
        l.wrap().width(text_w)
    }
}

/// The stack, full-window, aligned bottom-right with 16 px margin; put it
/// in a `stack` above the page. `dismiss(id)` is published on close and at
/// the deadline. An action button publishes `dismiss(id)` too; use
/// [`toast_stack_actions`] to tell it apart.
pub fn toast_stack<'a, M: Clone + 'a>(
    cx: &Ctx,
    toasts: &[Toast],
    dismiss: impl Fn(u64) -> M,
) -> El<'a, M> {
    toast_stack_actions(cx, toasts, &dismiss, &dismiss)
}

/// [`toast_stack`] whose action buttons publish `action(id)` (then hand
/// the id to [`Toasts::take_action`] for the command).
///
/// Layout (`Toast.tsx`): a toast with only a message is one row, centred
/// (`items-center`), the message wrapping at the 384 px maximum; one with
/// a title or an action is the two-line form (`items-start`): the title
/// (13 px medium, `leading-tight`, 2 px under it), the message (13 px
/// `leading-snug`), the action (12 px semibold `accentBright`,
/// `hover:underline`, 6 px above).
pub fn toast_stack_actions<'a, M: Clone + 'a>(
    cx: &Ctx,
    toasts: &[Toast],
    dismiss: impl Fn(u64) -> M,
    action: impl Fn(u64) -> M,
) -> El<'a, M> {
    let cx = cx.on_card();
    let p = cx.p();
    // Every toast is as wide as the widest one; the message is `flex-1`.
    let text_w = text_width(toasts);
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
            .focus_ring(crate::focus::Ring::plain(&cx))
            .style(move |s| {
                BoxStyle::default()
                    .radius(4.0)
                    .fg(c(if s == Status::Hover { text } else { muted }))
            });
        let body: El<'a, M> = if toast.rich() {
            let mut lines: Vec<El<'a, M>> = Vec::new();
            if let Some(title) = &toast.title {
                lines.push(
                    fit(
                        label(title.clone(), 13.0, 16.25).medium().color(c(p.text)),
                        text_w,
                    )
                    .into(),
                );
                lines.push(vgap(2.0));
            }
            lines.push(
                fit(
                    label(toast.text.clone(), 13.0, 17.875).color(c(p.text)),
                    text_w,
                )
                .into(),
            );
            if let Some(a) = &toast.action {
                // An inline-block button (`mt-1.5`) on the message's line
                // box (13 px / 19.5): 6 px above it, 17.25 px for the row.
                let button = surface(
                    label(a.label.clone(), 12.0, 16.0)
                        .semibold()
                        .color(c(p.accent_bright))
                        .hover_underline(),
                )
                .on_press(action(toast.id))
                .focus_ring(crate::focus::Ring::plain(&cx))
                .fixed(BoxStyle::default().radius(4.0));
                lines.push(vgap(6.0));
                lines.push(container(button).height(17.25).into());
            }
            row![
                container(icon(ico, 16.0).color(c(color))).padding(Padding {
                    top: 2.0,
                    ..Padding::ZERO
                }),
                column(lines),
                container(close).padding(Padding {
                    left: 4.0,
                    top: 2.0,
                    ..Padding::ZERO
                }),
            ]
            .spacing(12)
            .align_y(Alignment::Start)
            .into()
        } else {
            row![
                icon(ico, 16.0).color(c(color)),
                fit(
                    label(toast.text.clone(), 13.0, 19.5).color(c(p.text)),
                    text_w,
                ),
                container(close).padding(Padding {
                    left: 4.0,
                    ..Padding::ZERO
                }),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into()
        };
        col.push(
            timer(
                surface(body).padding(Padding::from([13.0, 17.0])).fixed(st),
                toast.born,
                toast.ttl.unwrap_or(Duration::MAX),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_take_action_and_lifetimes() {
        let mut q = Toasts::default();
        let a = q.push("plain", Tone::Info);
        let b = q.show(
            Toast::new("saved", Tone::Success)
                .title("Done")
                .action("Open", "open-folder")
                .duration(Duration::from_secs(6)),
        );
        let c = q.show(Toast::new("stays", Tone::Warning).persistent());
        assert_eq!((a, b, c), (1, 2, 3));
        assert_eq!(q.items[0].ttl, Some(TOAST_TTL));
        assert!(!q.items[0].rich() && q.items[1].rich());
        assert_eq!(q.items[1].ttl, Some(Duration::from_secs(6)));
        assert_eq!(q.items[2].ttl, None);
        assert_eq!(q.take_action(a), None, "no action: just dismissed");
        assert_eq!(q.take_action(b).as_deref(), Some("open-folder"));
        assert_eq!(q.take_action(b), None);
        assert_eq!(q.items.len(), 1);
    }

    #[test]
    fn long_text_wraps_at_the_toast_maximum() {
        crate::fonts::setup();
        let short = [Toast::new("Saved", Tone::Success)];
        let w = text_width(&short);
        assert!(w > 20.0 && w < 60.0, "{w}");
        let long = [
            Toast::new("Saved", Tone::Success),
            Toast::new(
                "A very long message that would never fit on a single line of a toast",
                Tone::Error,
            ),
        ];
        assert_eq!(text_width(&long), TEXT_MAX);
        // The message wraps instead of being cut.
        let lines = label(long[1].text.clone(), 13.0, 19.5)
            .wrap()
            .shape(TEXT_MAX)
            .line_count();
        assert_eq!(lines, 2);
    }
}
