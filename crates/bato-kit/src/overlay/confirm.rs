//! `ConfirmDialog` (`ConfirmDialog.tsx`): a confirmation on the base dialog.
//!
//! [`modal_panel`] at `max-w-sm` (384 px) with the title, the message
//! (`text-sm text-app-subtext leading-relaxed`) and a right-aligned
//! Cancel (secondary) / Confirm (primary, or danger) row 24 px below it.
//! It is a level of the overlay stack, so Esc closes it when it is on top,
//! a press on its scrim closes it, Tab is trapped in it, its first control
//! takes focus and focus goes back to the opener when it closes.
//!
//! **Enter confirms**, wherever focus is (React's window `keydown`
//! listener): the dialog takes the key before the focused button, so Enter
//! on a focused Cancel confirms, as in Electron (the listener runs before
//! the button's click).
//!
//! ```ignore
//! if self.confirm {
//!     levels.push(
//!         confirm_dialog(cx, "Delete item?", "This cannot be undone.", Msg::Delete, Msg::Cancel)
//!             .confirm_label("Delete")
//!             .danger(true)
//!             .into_level(),
//!     );
//! }
//! ```

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::keyboard::{self, key::Named};
use iced::widget::{Space, column, row};
use iced::{Event, Length, Rectangle, Size, Vector, mouse};

use crate::El;
use crate::button::button;
use crate::ctx::Ctx;
use crate::label::label;
use crate::modal::{Kind, Modal, level, modal_panel};
use crate::style::c;

/// Builder; [`ConfirmDialog::into_level`] makes the overlay level.
pub struct ConfirmDialog<M> {
    cx: Ctx,
    title: String,
    message: String,
    confirm_label: String,
    cancel_label: String,
    danger: bool,
    loading: bool,
    on_confirm: M,
    on_close: M,
}

/// A confirmation with "Confirm" / "Cancel" buttons.
pub fn confirm_dialog<M: Clone>(
    cx: &Ctx,
    title: impl Into<String>,
    message: impl Into<String>,
    on_confirm: M,
    on_close: M,
) -> ConfirmDialog<M> {
    ConfirmDialog {
        cx: cx.clone(),
        title: title.into(),
        message: message.into(),
        confirm_label: "Confirm".into(),
        cancel_label: "Cancel".into(),
        danger: false,
        loading: false,
        on_confirm,
        on_close,
    }
}

impl<M: Clone> ConfirmDialog<M> {
    pub fn confirm_label(mut self, text: impl Into<String>) -> Self {
        self.confirm_label = text.into();
        self
    }
    pub fn cancel_label(mut self, text: impl Into<String>) -> Self {
        self.cancel_label = text.into();
        self
    }
    /// A destructive action: the confirm button is the danger variant.
    pub fn danger(mut self, on: bool) -> Self {
        self.danger = on;
        self
    }
    /// The confirm button shows its spinner and is disabled.
    pub fn loading(mut self, on: bool) -> Self {
        self.loading = on;
        self
    }

    /// The dialog as a level for [`crate::frame::Frame::view_with`].
    pub fn into_level<'a>(self) -> Modal<'a, M>
    where
        M: 'a,
    {
        let on = self.cx.on_card();
        let p = on.p();
        let mut confirm = button(&on)
            .label(self.confirm_label)
            .loading(self.loading)
            .on_press(self.on_confirm.clone());
        if self.danger {
            confirm = confirm.danger();
        }
        let body = column![
            label(self.message, 14.0, 22.75).color(c(p.subtext)).wrap(),
            row![
                Space::new().width(Length::Fill),
                button(&on)
                    .secondary()
                    .label(self.cancel_label)
                    .on_press(self.on_close.clone()),
                confirm,
            ]
            .spacing(8),
        ]
        .spacing(24);
        let panel = modal_panel(
            &self.cx,
            &self.title,
            None,
            None,
            self.on_close.clone(),
            body,
        );
        level(
            Kind::Center,
            EnterConfirms {
                content: panel,
                on_confirm: self.on_confirm,
            },
            self.on_close,
        )
        .max_width(384.0)
    }
}

/// Enter anywhere confirms (the window `keydown` listener).
struct EnterConfirms<'a, M> {
    content: El<'a, M>,
    on_confirm: M,
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for EnterConfirms<'a, M> {
    crate::overlay::palette::delegate!();
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        if is_enter(event) {
            shell.publish(self.on_confirm.clone());
            shell.capture_event();
            return;
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
}

impl<'a, M: Clone + 'a> From<EnterConfirms<'a, M>> for El<'a, M> {
    fn from(w: EnterConfirms<'a, M>) -> Self {
        El::new(w)
    }
}

/// `e.key === 'Enter'` on a key press (numpad Enter included).
fn is_enter(event: &Event) -> bool {
    matches!(
        event,
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(Named::Enter),
            ..
        })
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::script::key_events;
    use iced::keyboard::{Key, Modifiers};

    #[test]
    fn enter_press_confirms_release_and_other_keys_do_not() {
        let [press, release] = key_events(Key::Named(Named::Enter), Modifiers::default());
        assert!(is_enter(&press));
        assert!(!is_enter(&release));
        let [esc, _] = key_events(Key::Named(Named::Escape), Modifiers::default());
        assert!(!is_enter(&esc));
        let [shift_enter, _] = key_events(Key::Named(Named::Enter), Modifiers::SHIFT);
        assert!(is_enter(&shift_enter), "React checks e.key only");
    }

    #[test]
    fn defaults_match_react() {
        let cx = Ctx::from_palette(bato_theme::palette(
            bato_theme::ThemeId::Dark,
            bato_theme::Rgb::new(0x8b, 0x5c, 0xf6),
        ));
        let d = confirm_dialog(&cx, "Delete item?", "This cannot be undone.", 1, 0);
        assert_eq!(
            (d.confirm_label.as_str(), d.cancel_label.as_str()),
            ("Confirm", "Cancel")
        );
        assert!(!d.danger && !d.loading);
        let l = d.danger(true).confirm_label("Delete").into_level();
        assert_eq!(l.kind(), Kind::Center);
    }
}
