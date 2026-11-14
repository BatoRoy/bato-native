//! `NumberInput` (`inputs/NumberInput.tsx`): a control box (`p-0
//! overflow-hidden`) holding a `w-7` − button, the centred number and a `w-7`
//! plus button. Steps clamp to `min`/`max`; a button at its bound is disabled
//! (`opacity .4`). ↑/↓ on the focused field step like the native number
//! input (OS key repeat repeats them); the buttons step on press and repeat
//! while held. React puts `focus:` classes on the wrapper `<div>`, so the box
//! shows no focus ring; neither does this one.
//!
//! The field keeps what was typed while it still reads as the value
//! (`1.` while the value is 1), like Chromium's number input; anything that
//! is not a number reads as NaN (an empty field), as `parseFloat` does.
//!
//! ```ignore
//! number_input(&cx, self.qty, Msg::Qty).min(0.0).max(10.0).label("Quantity")
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::keyboard::{Key, key::Named};
use iced::widget::{row, text_input};
use iced::{Alignment, Element, Event, Length, Padding, Rectangle, Size, mouse};

use super::format::{js_number, parse_float};
use super::frame::{CaretToEnd, Look, control_frame, repeat_button, text_style};
use crate::El;
use crate::ctx::Ctx;
use crate::field::{FieldSize, field};
use crate::fonts;
use crate::icon::{Ico, icon};
use crate::style::c;

/// Clamp like `Math.max(min, Math.min(max, n))`, NaN staying NaN.
pub fn clamp(n: f64, min: Option<f64>, max: Option<f64>) -> f64 {
    if n.is_nan() {
        return n;
    }
    let mut n = n;
    if let Some(lo) = min {
        n = n.max(lo);
    }
    if let Some(hi) = max {
        n = n.min(hi);
    }
    n
}

/// One step from `value` (NaN counts as 0), clamped.
pub fn step(value: f64, dir: f64, step: f64, min: Option<f64>, max: Option<f64>) -> f64 {
    let base = if value.is_finite() { value } else { 0.0 };
    // Round away float dust (0.1 + 0.2).
    let next = ((base + dir * step) * 1e9).round() / 1e9;
    clamp(next, min, max)
}

/// What a number field shows for an edit: the characters a native number
/// input accepts.
fn number_chars(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
        .collect()
}

pub struct NumberInput<'a, M> {
    cx: Ctx,
    value: f64,
    on_change: Rc<dyn Fn(f64) -> M + 'a>,
    min: Option<f64>,
    max: Option<f64>,
    step: f64,
    label: Option<String>,
    hint: Option<String>,
    error: Option<String>,
    required: bool,
    disabled: bool,
    size: FieldSize,
    id: Option<widget::Id>,
}

pub fn number_input<'a, M: Clone + 'a>(
    cx: &Ctx,
    value: f64,
    on_change: impl Fn(f64) -> M + 'a,
) -> NumberInput<'a, M> {
    NumberInput {
        cx: cx.clone(),
        value,
        on_change: Rc::new(on_change),
        min: None,
        max: None,
        step: 1.0,
        label: None,
        hint: None,
        error: None,
        required: false,
        disabled: false,
        size: FieldSize::Md,
        id: None,
    }
}

impl<'a, M: Clone + 'a> NumberInput<'a, M> {
    pub fn min(mut self, v: f64) -> Self {
        self.min = Some(v);
        self
    }
    pub fn max(mut self, v: f64) -> Self {
        self.max = Some(v);
        self
    }
    pub fn step(mut self, v: f64) -> Self {
        self.step = v;
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
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn size(mut self, s: FieldSize) -> Self {
        self.size = s;
        self
    }
    pub fn id(mut self, id: impl Into<widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl<'a, M: Clone + 'a> From<NumberInput<'a, M>> for El<'a, M> {
    fn from(n: NumberInput<'a, M>) -> Self {
        let cx = n.cx.clone();
        let p = cx.p();
        let m = n.size.metrics();
        let (min, max, st, v) = (n.min, n.max, n.step, n.value);
        let f = n.on_change.clone();
        let bump = move |dir: f64| f(step(v, dir, st, min, max));
        let can_down = !n.disabled && min.is_none_or(|lo| v.is_nan() || v > lo);
        let can_up = !n.disabled && max.is_none_or(|hi| v.is_nan() || v < hi);
        let btn = |ico: Ico, msg: Option<M>| -> El<'a, M> {
            repeat_button(icon(ico, 14.0), msg, c(p.muted), c(p.text))
                .width(28)
                .height(Length::Fill)
                .transition()
                .disabled_opacity(0.4)
                .into()
        };
        let down = btn(Ico::Minus, can_down.then(|| bump(-1.0)));
        let up = btn(Ico::Plus, can_up.then(|| bump(1.0)));
        let on_change = n.on_change.clone();
        let input = Element::new(DraftInput {
            cx: cx.clone(),
            shown: js_number(v),
            value: v,
            size: m.text,
            line: m.line,
            padding: Padding::from([m.pad_y, m.pad_x]),
            on_input: (!n.disabled).then(|| {
                Rc::new(move |s: String| {
                    on_change(clamp(parse_float(&s).unwrap_or(f64::NAN), min, max))
                }) as Rc<dyn Fn(String) -> M + 'a>
            }),
            id: n.id,
            typed: Rc::default(),
        });
        let inner = row![down, input, up]
            .height(Length::Fixed(m.line + 2.0 * m.pad_y))
            .align_y(Alignment::Center);
        let keys = n.on_change.clone();
        let mut frame = control_frame(&cx, inner)
            .look(Look::Plain)
            .error(n.error.is_some());
        if !n.disabled {
            frame = frame.on_key(move |key, mods, _| {
                if !crate::focus::plain(mods) {
                    return None;
                }
                let dir = match key {
                    Key::Named(Named::ArrowUp) => 1.0,
                    Key::Named(Named::ArrowDown) => -1.0,
                    _ => return None,
                };
                Some(keys(step(v, dir, st, min, max)))
            });
        }
        let mut fld = field(&cx, frame)
            .required(n.required)
            .error(n.error.as_deref());
        if let Some(l) = n.label {
            fld = fld.label(l);
        }
        if let Some(h) = n.hint {
            fld = fld.hint(h);
        }
        fld.into()
    }
}

// ── The number text: keeps a draft that still reads as the value ─────────

struct DraftInput<'a, M> {
    cx: Ctx,
    /// `String(value)`.
    shown: String,
    value: f64,
    size: f32,
    line: f32,
    padding: Padding,
    on_input: Option<Rc<dyn Fn(String) -> M + 'a>>,
    id: Option<widget::Id>,
    typed: Rc<RefCell<Option<String>>>,
}

#[derive(Debug, Default)]
struct DraftState {
    draft: Option<String>,
    last: Option<String>,
}

/// Whether `draft` still stands for `value` (both NaN counts).
fn reads_as(draft: &str, value: f64) -> bool {
    match parse_float(draft) {
        Some(d) => d == value,
        None => value.is_nan(),
    }
}

impl<'a, M: Clone + 'a> DraftInput<'a, M> {
    fn text(&self, st: &DraftState) -> String {
        match &st.draft {
            Some(d) if reads_as(d, self.value) => d.clone(),
            _ => self.shown.clone(),
        }
    }
    fn input(&self, text: &str) -> El<'a, M> {
        let style = text_style(&self.cx, false);
        let mut ti = text_input::TextInput::new("", text)
            .font(fonts::sans())
            .size(self.size)
            .line_height(iced::widget::text::LineHeight::Absolute(self.line.into()))
            .padding(self.padding)
            .align_x(Alignment::Center)
            .width(Length::Fill)
            .style(move |_t, _s| style);
        if let Some(f) = &self.on_input {
            let f = f.clone();
            let typed = self.typed.clone();
            ti = ti.on_input(move |s| {
                let s = number_chars(&s);
                *typed.borrow_mut() = Some(s.clone());
                f(s)
            });
        }
        if let Some(id) = &self.id {
            ti = ti.id(id.clone());
        }
        ti.into()
    }
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for DraftInput<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<DraftState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(DraftState::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(self.input(&self.shown))]
    }
    fn diff(&self, tree: &mut Tree) {
        let text = self.text(tree.state.downcast_ref::<DraftState>());
        tree.diff_children(&[self.input(&text)]);
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let text = self.text(tree.state.downcast_ref::<DraftState>());
        let n = self
            .input(&text)
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
        let text = self.text(tree.state.downcast_ref::<DraftState>());
        let mut el = self.input(&text);
        {
            let st = tree.state.downcast_mut::<DraftState>();
            if st.last.as_ref() != Some(&text) {
                let typed = st.draft.as_ref() == Some(&text);
                let first = st.last.is_none();
                st.last = Some(text.clone());
                if !typed && !first {
                    el.as_widget_mut().operate(
                        &mut tree.children[0],
                        child,
                        renderer,
                        &mut CaretToEnd,
                    );
                }
            }
        }
        el.as_widget_mut().update(
            &mut tree.children[0],
            event,
            child,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        if let Some(t) = self.typed.borrow_mut().take() {
            tree.state.downcast_mut::<DraftState>().draft = Some(t);
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
        let text = self.text(tree.state.downcast_ref::<DraftState>());
        self.input(&text).as_widget().mouse_interaction(
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
        let text = self.text(tree.state.downcast_ref::<DraftState>());
        self.input(&text).as_widget_mut().operate(
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
        let text = self.text(tree.state.downcast_ref::<DraftState>());
        self.input(&text).as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_keeps_nan() {
        assert!(clamp(f64::NAN, Some(0.0), Some(10.0)).is_nan());
        assert_eq!(clamp(12.0, Some(0.0), Some(10.0)), 10.0);
        assert_eq!(clamp(-1.0, Some(0.0), None), 0.0);
    }

    #[test]
    fn steps_clamp_and_start_from_zero() {
        assert_eq!(step(3.0, 1.0, 1.0, Some(0.0), Some(10.0)), 4.0);
        assert_eq!(step(10.0, 1.0, 1.0, Some(0.0), Some(10.0)), 10.0);
        assert_eq!(step(f64::NAN, 1.0, 1.0, None, None), 1.0);
        assert_eq!(step(0.1, 1.0, 0.2, None, None), 0.3);
    }

    #[test]
    fn drafts_that_read_as_the_value() {
        assert!(reads_as("1.", 1.0));
        assert!(reads_as("1.50", 1.5));
        assert!(reads_as("", f64::NAN));
        assert!(reads_as("-", f64::NAN));
        assert!(!reads_as("2", 1.0));
        assert_eq!(number_chars("1a2.b3"), "12.3");
    }
}
