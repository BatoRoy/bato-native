//! `OtpInput` (`inputs/OtpInput.tsx`): one 40×44 box per character
//! (`text-lg font-medium`, centred, its own focus border and halo), `gap-2`.
//! Typing a character into an empty box moves to the next one (each box is
//! `maxLength=1`, so a filled box refuses more), Backspace in an empty box
//! goes back, ←/→ move between boxes, and a paste anywhere fills the code
//! from its digits and focuses the box after the last one pasted.
//! `numeric` (default) accepts digits only. The app owns the value; the
//! code is complete when it is `length` characters long.
//!
//! ```ignore
//! otp_input(&cx, &self.otp, Msg::Otp).length(6)
//! ```

use iced::advanced::clipboard::Kind;
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::keyboard::{self, Key, key::Named};
use iced::widget::Row;
use iced::{Alignment, Element, Event, Length, Rectangle, Size, Vector, mouse};

use super::frame::{WhichFocused, control_frame, focus_nth, plain_input};
use crate::El;
use crate::ctx::Ctx;

/// React's `setChar(i, ch)`: put `ch` (or nothing) at character `i` of
/// `value` as a JS array would (holes join as nothing), cut to `length`.
pub fn set_char(value: &str, i: usize, ch: Option<char>, length: usize) -> String {
    let mut arr: Vec<Option<char>> = value.chars().map(Some).collect();
    if arr.len() <= i {
        arr.resize(i + 1, None);
    }
    arr[i] = ch;
    arr.into_iter().flatten().take(length).collect()
}

/// The paste rule: trimmed, digits only when `numeric`, cut to `length`.
pub fn pasted(text: &str, numeric: bool, length: usize) -> String {
    let t = text.trim();
    let t: String = if numeric {
        t.chars().filter(char::is_ascii_digit).collect()
    } else {
        t.to_string()
    };
    t.chars().take(length).collect()
}

pub struct OtpInput<'a, M> {
    cx: Ctx,
    value: String,
    on_change: std::rc::Rc<dyn Fn(String) -> M + 'a>,
    length: usize,
    numeric: bool,
}

pub fn otp_input<'a, M: Clone + 'a>(
    cx: &Ctx,
    value: &str,
    on_change: impl Fn(String) -> M + 'a,
) -> OtpInput<'a, M> {
    OtpInput {
        cx: cx.clone(),
        value: value.to_string(),
        on_change: std::rc::Rc::new(on_change),
        length: 6,
        numeric: true,
    }
}

impl<'a, M: Clone + 'a> OtpInput<'a, M> {
    pub fn length(mut self, n: usize) -> Self {
        self.length = n.max(1);
        self
    }
    /// Digits only (default `true`).
    pub fn numeric(mut self, on: bool) -> Self {
        self.numeric = on;
        self
    }
}

impl<'a, M: Clone + 'a> From<OtpInput<'a, M>> for El<'a, M> {
    fn from(o: OtpInput<'a, M>) -> Self {
        let chars: Vec<String> = (0..o.length)
            .map(|i| o.value.chars().nth(i).map(String::from).unwrap_or_default())
            .collect();
        let boxes: Vec<El<'a, M>> = chars
            .iter()
            .enumerate()
            .map(|(i, ch)| {
                let f = o.on_change.clone();
                let value = o.value.clone();
                let (len, numeric, cur) = (o.length, o.numeric, ch.clone());
                let input = plain_input(&o.cx, "", ch, 18.0, 28.0, false)
                    .font(iced::Font {
                        weight: iced::font::Weight::Medium,
                        ..crate::fonts::sans()
                    })
                    .align_x(Alignment::Center)
                    .padding([7.0, 0.0])
                    .width(Length::Fixed(38.0))
                    .on_input(move |s: String| {
                        // `maxLength=1`: a filled box refuses another character.
                        if s.chars().count() > 1 && !cur.is_empty() {
                            return f(value.clone());
                        }
                        let last = s.chars().last();
                        if numeric && last.is_some_and(|c| !c.is_ascii_digit()) {
                            return f(value.clone());
                        }
                        f(set_char(&value, i, last, len))
                    });
                control_frame(&o.cx, input).width(Length::Shrink).into()
            })
            .collect();
        Element::new(Otp {
            row: Row::with_children(boxes).spacing(8).into(),
            chars,
            numeric: o.numeric,
            length: o.length,
            on_change: o.on_change,
        })
    }
}

struct Otp<'a, M> {
    row: El<'a, M>,
    chars: Vec<String>,
    numeric: bool,
    length: usize,
    on_change: std::rc::Rc<dyn Fn(String) -> M + 'a>,
}

/// What a key does to focus: `Some(box)` to move there.
fn key_move(key: &Key, i: usize, chars: &[String]) -> Option<usize> {
    match key {
        Key::Named(Named::Backspace) if chars[i].is_empty() && i > 0 => Some(i - 1),
        Key::Named(Named::ArrowLeft) if i > 0 => Some(i - 1),
        Key::Named(Named::ArrowRight) if i + 1 < chars.len() => Some(i + 1),
        _ => None,
    }
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for Otp<'a, M> {
    fn size(&self) -> Size<Length> {
        self.row.as_widget().size()
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.row)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.row));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let n = self
            .row
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        layout::Node::with_children(n.size(), vec![n])
    }
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
        let child = layout.children().next().unwrap();
        let mut advance = None;
        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modifiers,
            text,
            ..
        }) = event
        {
            let mut w = WhichFocused::default();
            self.row
                .as_widget_mut()
                .operate(&mut tree.children[0], child, renderer, &mut w);
            if let Some(i) = w.focused {
                let paste = (modifiers.command() && key.as_ref() == Key::Character("v"))
                    || (modifiers.shift() && *key == Key::Named(Named::Insert));
                if paste {
                    let text = clipboard.read(Kind::Standard).unwrap_or_default();
                    let code = pasted(&text, self.numeric, self.length);
                    let n = code.chars().count();
                    shell.publish((self.on_change)(code));
                    focus_nth(
                        &mut self.row,
                        &mut tree.children[0],
                        child,
                        renderer,
                        n.min(self.length - 1),
                    );
                    shell.capture_event();
                    shell.request_redraw();
                    return;
                }
                if let Some(j) = key_move(key, i, &self.chars) {
                    focus_nth(&mut self.row, &mut tree.children[0], child, renderer, j);
                    shell.capture_event();
                    shell.request_redraw();
                    return;
                }
                // A character typed into an empty box moves on.
                if let Some(t) = text
                    && let Some(ch) = t.chars().last()
                    && !ch.is_control()
                    && crate::focus::plain(*modifiers)
                    && self.chars[i].is_empty()
                    && (!self.numeric || ch.is_ascii_digit())
                    && i + 1 < self.length
                {
                    advance = Some(i + 1);
                }
            }
        }
        self.row.as_widget_mut().update(
            &mut tree.children[0],
            event,
            child,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        if let Some(j) = advance {
            focus_nth(&mut self.row, &mut tree.children[0], child, renderer, j);
            shell.request_redraw();
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.row.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        )
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.row.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
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
        self.row.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        self.row.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_char_like_a_js_array() {
        assert_eq!(set_char("", 0, Some('1'), 6), "1");
        assert_eq!(set_char("12", 1, Some('9'), 6), "19");
        assert_eq!(
            set_char("12", 4, Some('5'), 6),
            "125",
            "holes join as nothing"
        );
        assert_eq!(set_char("123", 1, None, 6), "13");
        assert_eq!(set_char("123456", 5, Some('9'), 6), "123459");
    }

    #[test]
    fn paste_keeps_digits() {
        assert_eq!(pasted(" 12-34 56 78 ", true, 6), "123456");
        assert_eq!(pasted("ab12", false, 6), "ab12");
    }

    #[test]
    fn keys_move_between_boxes() {
        let chars: Vec<String> = ["1", "", "", ""].iter().map(|s| s.to_string()).collect();
        assert_eq!(key_move(&Key::Named(Named::Backspace), 1, &chars), Some(0));
        assert_eq!(key_move(&Key::Named(Named::Backspace), 0, &chars), None);
        assert_eq!(key_move(&Key::Named(Named::ArrowRight), 3, &chars), None);
        assert_eq!(key_move(&Key::Named(Named::ArrowLeft), 2, &chars), Some(1));
        let full: Vec<String> = ["1", "2"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            key_move(&Key::Named(Named::Backspace), 1, &full),
            None,
            "deletes first"
        );
    }
}
