//! `TextField` (`inputs/TextField.tsx`), `MaskedInput` and `CurrencyInput`.
//!
//! TextField = [`field`] + [`text_control`] with the adornments of the React
//! component: left icon or prefix (text at `pl-9`), a right group (`right-2.5
//! gap-1`, text ending at `pr-9`) holding, in order, the loading spinner, the
//! clear button (only with a value), the password eye, the right icon and the
//! suffix. The clear and eye buttons are `tabIndex=-1` (a click focuses them,
//! Tab skips them), `muted`, `text` on hover. `max_len` is `maxLength`
//! (typing into a full field is refused, a paste is cut); `show_count` adds
//! the `n/max` counter under it.
//!
//! The field is controlled like a React input: when the value the app sets
//! differs from what was typed (a mask, a reformat, a clear), the caret
//! jumps to the end, as Chromium does for a script-set value.
//!
//! ```ignore
//! text_field(&cx, &self.q, Msg::Q).label("Search").left_icon(Ico::Search)
//!     .placeholder("Search…").clearable(Msg::Q(String::new()))
//! text_field(&cx, &self.pw, Msg::Pw).password(self.reveal, Msg::Reveal)
//! masked_input(&cx, "(###) ###-####", &self.phone, Msg::Phone).label("Phone")
//! currency_input(&cx, &self.price_edit, self.price, Msg::Price).label("Price")
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};

use super::format::{
    apply_mask, format_currency, js_number, limit_len, parse_money, system_locale,
};
use super::frame::CaretToEnd;
use crate::El;
use crate::ctx::Ctx;
use crate::field::{FieldSize, field, text_control};
use crate::icon::{Ico, Icon, icon};
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

type Filter<'a> = Box<dyn Fn(&str) -> String + 'a>;
type OnBool<'a, M> = Box<dyn Fn(bool) -> M + 'a>;

pub struct TextField<'a, M> {
    cx: Ctx,
    value: String,
    on_input: Box<dyn Fn(String) -> M + 'a>,
    placeholder: String,
    label: Option<String>,
    hint: Option<String>,
    error: Option<String>,
    required: bool,
    size: FieldSize,
    left_icon: Option<Icon>,
    prefix: Option<String>,
    right_icon: Option<Icon>,
    suffix: Option<String>,
    clear: Option<M>,
    password: Option<(bool, OnBool<'a, M>)>,
    loading: bool,
    max_len: Option<usize>,
    show_count: bool,
    disabled: bool,
    id: Option<widget::Id>,
    on_submit: Option<M>,
    on_focus: Option<Box<dyn Fn(bool) -> M + 'a>>,
    filter: Option<Filter<'a>>,
    mono: bool,
}

/// A text field showing `value`; edits go through `on_input`.
pub fn text_field<'a, M: Clone + 'a>(
    cx: &Ctx,
    value: &str,
    on_input: impl Fn(String) -> M + 'a,
) -> TextField<'a, M> {
    TextField {
        cx: cx.clone(),
        value: value.to_string(),
        on_input: Box::new(on_input),
        placeholder: String::new(),
        label: None,
        hint: None,
        error: None,
        required: false,
        size: FieldSize::Md,
        left_icon: None,
        prefix: None,
        right_icon: None,
        suffix: None,
        clear: None,
        password: None,
        loading: false,
        max_len: None,
        show_count: false,
        disabled: false,
        id: None,
        on_submit: None,
        on_focus: None,
        filter: None,
        mono: false,
    }
}

impl<'a, M: Clone + 'a> TextField<'a, M> {
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
    /// Red border/halo and the message in place of the hint.
    pub fn error(mut self, e: Option<&str>) -> Self {
        self.error = e.map(str::to_string);
        self
    }
    pub fn required(mut self, on: bool) -> Self {
        self.required = on;
        self
    }
    pub fn size(mut self, s: FieldSize) -> Self {
        self.size = s;
        self
    }
    pub fn left_icon(mut self, i: impl Into<Icon>) -> Self {
        self.left_icon = Some(i.into());
        self
    }
    pub fn prefix(mut self, t: impl Into<String>) -> Self {
        self.prefix = Some(t.into());
        self
    }
    pub fn right_icon(mut self, i: impl Into<Icon>) -> Self {
        self.right_icon = Some(i.into());
        self
    }
    pub fn suffix(mut self, t: impl Into<String>) -> Self {
        self.suffix = Some(t.into());
        self
    }
    /// The × button (shown while there is a value), publishing `on_clear`.
    pub fn clearable(mut self, on_clear: M) -> Self {
        self.clear = Some(on_clear);
        self
    }
    /// `type="password"` with the eye toggle; `revealed` is the caller's.
    pub fn password(mut self, revealed: bool, on_reveal: impl Fn(bool) -> M + 'a) -> Self {
        self.password = Some((revealed, Box::new(on_reveal)));
        self
    }
    /// A 14 px spinner in the right group (hides clear, eye and right icon).
    pub fn loading(mut self, on: bool) -> Self {
        self.loading = on;
        self
    }
    /// `maxLength`.
    pub fn max_len(mut self, n: usize) -> Self {
        self.max_len = Some(n);
        self
    }
    /// `showCount`: the `n/max` counter (needs [`TextField::max_len`]).
    pub fn show_count(mut self, on: bool) -> Self {
        self.show_count = on;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn id(mut self, id: impl Into<widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }
    pub fn on_submit(mut self, m: M) -> Self {
        self.on_submit = Some(m);
        self
    }
    /// `onFocus` / `onBlur`.
    pub fn on_focus(mut self, f: impl Fn(bool) -> M + 'a) -> Self {
        self.on_focus = Some(Box::new(f));
        self
    }
    /// Rewrite every edit before `on_input` sees it (a mask, a filter).
    pub fn filter(mut self, f: impl Fn(&str) -> String + 'a) -> Self {
        self.filter = Some(Box::new(f));
        self
    }
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
}

/// The 14/15 px icon buttons of the right group (`text-app-muted
/// hover:text-app-text`, `tabIndex=-1`).
fn adornment_button<'a, M: Clone + 'a>(cx: &Ctx, ico: Ico, px: f32, msg: M) -> El<'a, M> {
    let p = cx.p();
    let (rest, hover) = (c(p.muted), c(p.text));
    surface(icon(ico, px))
        .style(move |s| BoxStyle::default().fg(if s == Status::Hover { hover } else { rest }))
        .transition_ms(0)
        .tab_stop(false)
        .on_press(msg)
        .into()
}

impl<'a, M: Clone + 'a> From<TextField<'a, M>> for El<'a, M> {
    fn from(f: TextField<'a, M>) -> Self {
        let cx = f.cx.clone();
        let p = cx.p();
        let typed: Rc<RefCell<Option<String>>> = Rc::default();
        let on_input = {
            let typed = typed.clone();
            let old = f.value.clone();
            let max = f.max_len;
            let filter = f.filter;
            let user = f.on_input;
            move |s: String| {
                let s = match max {
                    Some(n) => limit_len(&old, &s, n),
                    None => s,
                };
                *typed.borrow_mut() = Some(s.clone());
                user(match &filter {
                    Some(fl) => fl(&s),
                    None => s,
                })
            }
        };
        let mut ctl = text_control(&cx, &f.placeholder, &f.value, on_input)
            .size(f.size)
            .error(f.error.is_some())
            .disabled(f.disabled)
            .secure(f.password.as_ref().is_some_and(|(r, _)| !r));
        if f.mono {
            ctl = ctl.mono();
        }
        if let Some(i) = f.left_icon {
            ctl = ctl.left_icon(i);
        }
        if let Some(t) = &f.prefix {
            ctl = ctl.prefix(t);
        }
        if f.loading {
            ctl = ctl.right(crate::spinner::spinner(14.0).color(c(p.muted)));
        }
        if let Some(m) = f.clear
            && !f.value.is_empty()
            && !f.loading
        {
            ctl = ctl.right(adornment_button(&cx, Ico::X, 14.0, m));
        }
        if let Some((revealed, on_reveal)) = &f.password
            && !f.loading
        {
            let ico = if *revealed { Ico::EyeOff } else { Ico::Eye };
            ctl = ctl.right(adornment_button(&cx, ico, 15.0, on_reveal(!revealed)));
        }
        if let Some(i) = f.right_icon
            && f.password.is_none()
            && !f.loading
        {
            ctl = ctl.right_icon(i);
        }
        if let Some(t) = &f.suffix {
            ctl = ctl.suffix(t);
        }
        if let Some(m) = f.on_submit {
            ctl = ctl.on_submit(m);
        }
        if let Some(id) = f.id {
            ctl = ctl.id(id);
        }
        if let Some(cb) = f.on_focus {
            ctl = ctl.on_focus(cb);
        }
        let controlled = controlled(ctl.into(), f.value.clone(), typed);
        let mut fld = field(&cx, controlled)
            .required(f.required)
            .error(f.error.as_deref());
        if let Some(l) = f.label {
            fld = fld.label(l);
        }
        if let Some(h) = f.hint {
            fld = fld.hint(h);
        }
        if f.show_count
            && let Some(n) = f.max_len
        {
            fld = fld.counter(format!("{}/{n}", f.value.chars().count()));
        }
        fld.into()
    }
}

// ── Controlled value: caret to the end on a script-set value ────────────

/// `content` (a text input whose `on_input` stores each typed value in
/// `typed`) as a controlled React input: when `value` changes to something
/// other than what was typed (a mask, a reformat, an accepted completion),
/// the caret goes to the end, as Chromium does for a script-set value.
pub(super) fn controlled<'a, M: 'a>(
    content: El<'a, M>,
    value: String,
    typed: Rc<RefCell<Option<String>>>,
) -> El<'a, M> {
    Element::new(Controlled {
        content,
        value,
        typed,
    })
}

struct Controlled<'a, M> {
    content: El<'a, M>,
    value: String,
    typed: Rc<RefCell<Option<String>>>,
}

#[derive(Debug, Default)]
struct ControlledState {
    last: Option<String>,
    typed: Option<String>,
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for Controlled<'a, M> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<ControlledState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(ControlledState {
            last: Some(self.value.clone()),
            typed: None,
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let n = self
            .content
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
        let st = tree.state.downcast_mut::<ControlledState>();
        if st.last.as_ref() != Some(&self.value) {
            let moved = st.typed.as_ref() != Some(&self.value);
            st.last = Some(self.value.clone());
            st.typed = None;
            if moved {
                self.content.as_widget_mut().operate(
                    &mut tree.children[0],
                    child,
                    renderer,
                    &mut CaretToEnd,
                );
            }
        }
        self.content.as_widget_mut().update(
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
            tree.state.downcast_mut::<ControlledState>().typed = Some(t);
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
        self.content.as_widget().mouse_interaction(
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
        self.content.as_widget_mut().operate(
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
        self.content.as_widget().draw(
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
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

// ── MaskedInput ─────────────────────────────────────────────────────────

/// `MaskedInput`: a TextField re-masked on every edit (`#` digit, `A`
/// letter, `*` alphanumeric, anything else a literal).
pub fn masked_input<'a, M: Clone + 'a>(
    cx: &Ctx,
    mask: &str,
    value: &str,
    on_change: impl Fn(String) -> M + 'a,
) -> TextField<'a, M> {
    let mask = mask.to_string();
    text_field(cx, value, on_change).filter(move |s| apply_mask(s, &mask))
}

// ── CurrencyInput ───────────────────────────────────────────────────────

/// CurrencyInput's own state (React's `text` and `focused`): raw digits
/// while focused, the formatted amount otherwise.
#[derive(Debug, Clone)]
pub struct CurrencyEdit {
    text: String,
    focused: bool,
    /// ISO 4217 code (`USD`).
    pub currency: String,
    /// BCP 47 locale (`en-GB`); the system's by default.
    pub locale: String,
}

impl Default for CurrencyEdit {
    fn default() -> Self {
        Self::new("USD")
    }
}

/// What a currency field reports.
#[derive(Debug, Clone, PartialEq)]
pub enum CurrencyEvent {
    Input(String),
    Focus(bool),
}

impl CurrencyEdit {
    pub fn new(currency: &str) -> Self {
        Self {
            text: String::new(),
            focused: false,
            currency: currency.to_string(),
            locale: system_locale(),
        }
    }

    /// Apply an event; `Some(v)` is the `onChange(v)` to perform.
    pub fn update(&mut self, ev: CurrencyEvent, value: Option<f64>) -> Option<Option<f64>> {
        match ev {
            CurrencyEvent::Input(s) => {
                let v = parse_money(&s);
                self.text = s;
                Some(v)
            }
            CurrencyEvent::Focus(true) => {
                self.focused = true;
                self.text = value.map(js_number).unwrap_or_default();
                None
            }
            CurrencyEvent::Focus(false) => {
                self.focused = false;
                Some(parse_money(&self.text))
            }
        }
    }

    /// The text the field shows.
    pub fn text(&self, value: Option<f64>) -> String {
        if self.focused {
            self.text.clone()
        } else {
            value
                .map(|v| format_currency(v, &self.currency, &self.locale))
                .unwrap_or_default()
        }
    }
}

/// `CurrencyInput`: formatted when blurred, raw while focused.
pub fn currency_input<'a, M: Clone + 'a>(
    cx: &Ctx,
    edit: &CurrencyEdit,
    value: Option<f64>,
    on: impl Fn(CurrencyEvent) -> M + 'a,
) -> TextField<'a, M> {
    let on = Rc::new(on);
    let focus = on.clone();
    text_field(cx, &edit.text(value), move |s| on(CurrencyEvent::Input(s)))
        .on_focus(move |f| focus(CurrencyEvent::Focus(f)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_edit_cycle() {
        let mut e = CurrencyEdit {
            locale: "en-GB".into(),
            ..CurrencyEdit::new("USD")
        };
        let mut v = Some(1999.0);
        assert_eq!(e.text(v), "US$1,999.00");
        assert_eq!(e.update(CurrencyEvent::Focus(true), v), None);
        assert_eq!(e.text(v), "1999");
        if let Some(n) = e.update(CurrencyEvent::Input("1999.5".into()), v) {
            v = n;
        }
        assert_eq!(v, Some(1999.5));
        assert_eq!(e.text(v), "1999.5", "raw while focused");
        assert_eq!(e.update(CurrencyEvent::Focus(false), v), Some(Some(1999.5)));
        assert_eq!(e.text(v), "US$1,999.50");
        assert_eq!(e.update(CurrencyEvent::Input(String::new()), v), Some(None));
    }
}
