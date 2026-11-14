//! `TextArea` (`inputs/TextArea.tsx`): iced's `text_editor` in the control
//! box (`px-3 py-2 text-sm min-h-[5rem]`, focus halo). `autosize` grows the
//! box with its content; otherwise it is 5rem tall and scrolls. The app owns
//! the [`Content`] (iced's model); [`apply`] performs an action with
//! `maxLength` enforced. `show_count` adds the `n/max` counter.
//!
//! ```ignore
//! text_area(&cx, &self.notes, Msg::Notes).label("Notes").autosize(true).placeholder("Type a lot…")
//! // update: inputs::text_area::apply(&mut self.notes, action, None)
//! ```

use iced::widget::text_editor::{self, Action, Content, Edit};
use iced::{Background, Border, Color, Length};

use super::frame::{Look, control_frame};
use crate::El;
use crate::ctx::Ctx;
use crate::field::{DISABLED_OPACITY, field};
use crate::fonts;
use crate::style::{c, rgba};

/// `min-h-[5rem]`.
pub const MIN_HEIGHT: f32 = 80.0;

/// Perform `action`, refusing edits that would pass `max_len` characters
/// (a paste is cut to fit, like `maxLength`).
pub fn apply(content: &mut Content, action: Action, max_len: Option<usize>) {
    let Some(max) = max_len else {
        content.perform(action);
        return;
    };
    let len = content.text().chars().count();
    let selected = content.selection().map_or(0, |s| s.chars().count());
    let room = (max + selected).saturating_sub(len);
    match action {
        Action::Edit(Edit::Insert(_) | Edit::Enter) if room == 0 => {}
        Action::Edit(Edit::Paste(text)) => {
            let cut: String = text.chars().take(room).collect();
            if !cut.is_empty() || selected > 0 {
                content.perform(Action::Edit(Edit::Paste(std::sync::Arc::new(cut))));
            }
        }
        other => content.perform(other),
    }
}

pub struct TextArea<'a, M> {
    cx: Ctx,
    content: &'a Content,
    on_action: Option<Box<dyn Fn(Action) -> M + 'a>>,
    placeholder: String,
    label: Option<String>,
    hint: Option<String>,
    error: Option<String>,
    required: bool,
    autosize: bool,
    max_len: Option<usize>,
    show_count: bool,
    disabled: bool,
    id: Option<iced::widget::Id>,
}

pub fn text_area<'a, M: Clone + 'a>(
    cx: &Ctx,
    content: &'a Content,
    on_action: impl Fn(Action) -> M + 'a,
) -> TextArea<'a, M> {
    TextArea {
        cx: cx.clone(),
        content,
        on_action: Some(Box::new(on_action)),
        placeholder: String::new(),
        label: None,
        hint: None,
        error: None,
        required: false,
        autosize: false,
        max_len: None,
        show_count: false,
        disabled: false,
        id: None,
    }
}

impl<'a, M: Clone + 'a> TextArea<'a, M> {
    pub fn placeholder(mut self, p: impl Into<String>) -> Self {
        self.placeholder = p.into();
        self
    }
    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = Some(l.into());
        self
    }
    pub fn hint(mut self, h: impl Into<String>) -> Self {
        self.hint = Some(h.into());
        self
    }
    pub fn error(mut self, e: Option<&str>) -> Self {
        self.error = e.map(str::to_string);
        self
    }
    pub fn required(mut self, on: bool) -> Self {
        self.required = on;
        self
    }
    /// Grow with the content (no scrolling).
    pub fn autosize(mut self, on: bool) -> Self {
        self.autosize = on;
        self
    }
    /// For the counter; enforce it with [`apply`].
    pub fn max_len(mut self, n: usize) -> Self {
        self.max_len = Some(n);
        self
    }
    pub fn show_count(mut self, on: bool) -> Self {
        self.show_count = on;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn id(mut self, id: impl Into<iced::widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl<'a, M: Clone + 'a> From<TextArea<'a, M>> for El<'a, M> {
    fn from(t: TextArea<'a, M>) -> Self {
        let cx = t.cx.clone();
        let p = cx.p();
        let fade = |rgb| {
            if t.disabled {
                c(cx.blend(rgb, DISABLED_OPACITY as f64))
            } else {
                c(rgb)
            }
        };
        let style = text_editor::Style {
            background: Background::Color(Color::TRANSPARENT),
            border: Border::default(),
            placeholder: fade(p.muted),
            value: fade(p.text),
            selection: rgba(p.accent, 0.35),
        };
        // `min_height` is the text area's, inside its `py-2` padding.
        let inner = MIN_HEIGHT - 2.0 - 16.0;
        let mut ed = text_editor::TextEditor::new(t.content)
            .placeholder(t.placeholder)
            .font(fonts::sans())
            .size(14.0)
            .line_height(iced::widget::text::LineHeight::Absolute(20.0.into()))
            .padding([8.0, 12.0])
            .style(move |_t, _s| style);
        ed = if t.autosize {
            ed.height(Length::Shrink).min_height(inner)
        } else {
            ed.height(Length::Fixed(inner))
        };
        if !t.disabled
            && let Some(f) = t.on_action
        {
            ed = ed.on_action(f);
        }
        if let Some(id) = t.id {
            ed = ed.id(id);
        }
        let frame = control_frame(&cx, ed)
            .look(Look::Halo)
            .error(t.error.is_some())
            .disabled(t.disabled)
            .min_height(MIN_HEIGHT);
        let mut fld = field(&cx, frame)
            .required(t.required)
            .error(t.error.as_deref());
        if let Some(l) = t.label {
            fld = fld.label(l);
        }
        if let Some(h) = t.hint {
            fld = fld.hint(h);
        }
        if t.show_count
            && let Some(n) = t.max_len
        {
            fld = fld.counter(format!("{}/{n}", t.content.text().chars().count()));
        }
        fld.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_len_refuses_and_cuts() {
        let mut c = Content::with_text("abc");
        c.perform(Action::Move(text_editor::Motion::DocumentEnd));
        apply(&mut c, Action::Edit(Edit::Insert('d')), Some(4));
        assert_eq!(c.text(), "abcd");
        apply(&mut c, Action::Edit(Edit::Insert('e')), Some(4));
        assert_eq!(c.text(), "abcd", "full");
        apply(&mut c, Action::Edit(Edit::Backspace), Some(4));
        apply(
            &mut c,
            Action::Edit(Edit::Paste(std::sync::Arc::new("xyz".into()))),
            Some(4),
        );
        assert_eq!(c.text(), "abcx", "paste cut to fit");
    }
}
