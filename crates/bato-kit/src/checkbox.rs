//! `Checkbox` (`Form.tsx`): a 16 px box, radius 4, `bg@.5` fill and
//! `app-control` border, accentBright border on hover; checked or
//! indeterminate: accent fill with a 12 px Check / Minus at stroke 3 in
//! `accent_ink`. The optional label (14 px subtext, gap 10) toggles too.
//! Disabled: opacity .5 on the whole thing.
//!
//! Focus: the box is the focusable `<button>` (focusRing, Space / Enter
//! toggle); a click on the label focuses it, as `<label>` does.
//!
//! ```ignore
//! checkbox(&cx, self.agree, Msg::Agree).label("I agree")
//! checkbox(&cx, false, Msg::All).indeterminate(true)
//! ```

use iced::widget::{Space, row};
use iced::{Alignment, Element};

use crate::El;
use crate::anim::opacity;
use crate::ctx::Ctx;
use crate::cursor::Cursor;
use crate::focus::Ring;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

pub struct Checkbox<'a, M> {
    cx: Ctx,
    checked: bool,
    indeterminate: bool,
    disabled: bool,
    label: Option<String>,
    on_toggle: Box<dyn Fn(bool) -> M + 'a>,
    id: Option<iced::advanced::widget::Id>,
}

pub fn checkbox<'a, M: Clone + 'a>(
    cx: &Ctx,
    checked: bool,
    on_toggle: impl Fn(bool) -> M + 'a,
) -> Checkbox<'a, M> {
    Checkbox {
        cx: cx.clone(),
        checked,
        indeterminate: false,
        disabled: false,
        label: None,
        on_toggle: Box::new(on_toggle),
        id: None,
    }
}

impl<'a, M: Clone + 'a> Checkbox<'a, M> {
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = Some(text.into());
        self
    }
    /// A dash instead of the tick ("some but not all").
    pub fn indeterminate(mut self, on: bool) -> Self {
        self.indeterminate = on;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    /// For [`crate::focus::focus`].
    pub fn id(mut self, id: impl Into<iced::advanced::widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl<'a, M: Clone + 'a> From<Checkbox<'a, M>> for El<'a, M> {
    fn from(cb: Checkbox<'a, M>) -> Self {
        let filled = cb.checked || cb.indeterminate;
        let t = cb.cx.t.clone();
        let over = cb.cx.over;
        let disabled = cb.disabled;
        let glyph: El<'a, M> = if cb.indeterminate {
            icon(Ico::Minus, 12.0).stroke_width(3.0).into()
        } else if cb.checked {
            icon(Ico::Check, 12.0).stroke_width(3.0).into()
        } else {
            Space::new().width(12).height(12).into()
        };
        let msg = (!disabled).then(|| (cb.on_toggle)(!cb.checked));
        let mut boxed = surface(glyph)
            .width(16)
            .height(16)
            .center()
            .on_press_maybe(msg.clone())
            .focus_ring(Ring::focus(&cb.cx))
            .style(move |st| {
                let p = &t.p;
                if filled {
                    BoxStyle::default()
                        .bg(c(p.accent))
                        .border(c(p.accent))
                        .fg(c(p.accent_ink))
                        .radius(4.0)
                } else {
                    let border = if st == Status::Hover && !disabled {
                        p.accent_bright
                    } else {
                        p.control
                    };
                    BoxStyle::default()
                        .bg(c(t.recessed(over)))
                        .border(c(border))
                        .radius(4.0)
                }
            });
        if let Some(id) = cb.id {
            boxed = boxed.id(id);
        }
        let mut items: Vec<El<'a, M>> = vec![boxed.into()];
        if let Some(l) = cb.label {
            items.push(label(l, 14.0, 20.0).color(c(cb.cx.p().subtext)).into());
        }
        let whole = surface(row(items).spacing(10).align_y(Alignment::Center))
            .on_press_maybe(msg)
            .focusable(false)
            .labels_child()
            .cursor(if disabled {
                Cursor::Default
            } else {
                Cursor::Pointer
            });
        if disabled {
            Element::from(opacity(0.5, whole))
        } else {
            whole.into()
        }
    }
}
