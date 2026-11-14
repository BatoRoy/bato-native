//! `Button` (spec §4.8): primary / secondary / ghost / success / danger ×
//! sm / md / icon, `loading` spinner, disabled at opacity .5.
//!
//! ```ignore
//! button(&cx).label("Save").on_press(Msg::Save)                      // primary md
//! button(&cx).secondary().sm().icon(Ico::Bell).label("Notify").on_press(Msg::Notify)
//! button(&cx).ghost().square(32).icon_sized(Ico::X, 16.0).on_press(Msg::Close)
//! button(&cx).label("Saving").loading(true)
//! ```
//!
//! Focusable when enabled, with `focusRing` (2 px accentBright, 2 px offset
//! in the page background); Enter / Space click.

use bato_theme::Rgb;
use iced::widget::row;
use iced::{Alignment, Color, Length, Padding};

use crate::El;
use crate::ctx::Ctx;
use crate::focus::Ring;
use crate::icon::{Ico, icon};
use crate::label::{Label, label};
use crate::spinner::spinner;
use crate::style::{c, sh};
use crate::surface::{BoxStyle, Sh, Status, surface};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Variant {
    #[default]
    Primary,
    Secondary,
    Ghost,
    Success,
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Size {
    /// h 32, px 12, text 12/16
    Sm,
    /// h 36, px 16, text 13/19.5
    #[default]
    Md,
    /// A square icon button of this many px.
    Icon(u16),
}

/// The style function of a button on `cx`'s backdrop. Disabled = opacity .5
/// on the whole button, folded into each colour (exact for this box).
pub fn button_style(cx: &Ctx, v: Variant, disabled: bool) -> impl Fn(Status) -> BoxStyle + use<> {
    let t = cx.t.clone();
    let over = cx.over;
    move |st| {
        let p = &t.p;
        let hover = st == Status::Hover && !disabled;
        let k = if disabled { 0.5 } else { 1.0 };
        let op = |x: Rgb| {
            if disabled {
                c(bato_theme::blend(x, 0.5, over))
            } else {
                c(x)
            }
        };
        match v {
            Variant::Primary => {
                let fill = if hover { p.accent_hover } else { p.accent };
                BoxStyle::default()
                    .bg(op(fill))
                    .fg(if disabled {
                        c(bato_theme::blend(
                            p.accent_ink,
                            0.5,
                            bato_theme::blend(fill, 0.5, over),
                        ))
                    } else {
                        c(p.accent_ink)
                    })
                    .radius(8.0)
                    .inset_top(op(bato_theme::blend(Rgb::WHITE, 0.16, fill)))
                    .shadows(vec![Sh {
                        x: 0.0,
                        y: 1.0,
                        blur: 2.0,
                        spread: 0.0,
                        color: Color::from_rgba(0.0, 0.0, 0.0, 0.2 * k),
                    }])
            }
            Variant::Secondary => {
                let bg = t.wash(if hover { 0.08 } else { 0.04 }, over);
                BoxStyle::default()
                    .bg(op(bg))
                    .border(op(t.line_strong_on(bg)))
                    .fg(op(p.text))
                    .radius(8.0)
                    .shadows(p.shadow_sm.iter().map(|s| sh(s, k)).collect())
            }
            Variant::Ghost => {
                let mut s =
                    BoxStyle::default()
                        .radius(8.0)
                        .fg(op(if hover { p.text } else { p.subtext }));
                if hover {
                    s = s.bg(c(t.wash(0.06, over)));
                }
                s
            }
            Variant::Success | Variant::Danger => {
                let tone = if v == Variant::Success {
                    p.green
                } else {
                    p.red
                };
                let bg = bato_theme::blend(tone, if hover { 0.15 } else { 0.10 }, over);
                BoxStyle::default()
                    .bg(op(bg))
                    .border(op(bato_theme::blend(tone, 0.25, bg)))
                    .fg(op(tone))
                    .radius(8.0)
            }
        }
    }
}

/// The label text of a button of `size` (12/16 or 13/19.5, medium).
pub fn btn_label(text: &str, size: Size) -> Label {
    match size {
        Size::Sm => label(text, 12.0, 16.0).medium(),
        _ => label(text, 13.0, 19.5).medium(),
    }
}

enum Item<'a, M> {
    Label(String),
    Icon(Ico, Option<f32>),
    El(El<'a, M>),
}

/// Builder; becomes a [`crate::surface::Surface`] on `into()`.
pub struct Button<'a, M> {
    cx: Ctx,
    variant: Variant,
    size: Size,
    items: Vec<Item<'a, M>>,
    on_press: Option<M>,
    loading: bool,
    disabled: bool,
    width: Option<Length>,
    id: Option<iced::advanced::widget::Id>,
}

/// A primary, medium button on `cx`'s backdrop, without content yet.
pub fn button<'a, M: Clone + 'a>(cx: &Ctx) -> Button<'a, M> {
    Button {
        cx: cx.clone(),
        variant: Variant::Primary,
        size: Size::Md,
        items: Vec::new(),
        on_press: None,
        loading: false,
        disabled: false,
        width: None,
        id: None,
    }
}

impl<'a, M: Clone + 'a> Button<'a, M> {
    pub fn variant(mut self, v: Variant) -> Self {
        self.variant = v;
        self
    }
    pub fn primary(self) -> Self {
        self.variant(Variant::Primary)
    }
    pub fn secondary(self) -> Self {
        self.variant(Variant::Secondary)
    }
    pub fn ghost(self) -> Self {
        self.variant(Variant::Ghost)
    }
    pub fn success(self) -> Self {
        self.variant(Variant::Success)
    }
    pub fn danger(self) -> Self {
        self.variant(Variant::Danger)
    }
    pub fn size(mut self, s: Size) -> Self {
        self.size = s;
        self
    }
    pub fn sm(self) -> Self {
        self.size(Size::Sm)
    }
    pub fn md(self) -> Self {
        self.size(Size::Md)
    }
    /// A square icon button (`size="icon"`, `w-9 h-9` = 36).
    pub fn square(self, px: u16) -> Self {
        self.size(Size::Icon(px))
    }
    /// Text, sized for the button size.
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.items.push(Item::Label(text.into()));
        self
    }
    /// An icon at the default size (13 in sm, 14 otherwise).
    pub fn icon(mut self, ico: Ico) -> Self {
        self.items.push(Item::Icon(ico, None));
        self
    }
    pub fn icon_sized(mut self, ico: Ico, px: f32) -> Self {
        self.items.push(Item::Icon(ico, Some(px)));
        self
    }
    /// Any element (a `kbd` chip, a spinning icon).
    pub fn push(mut self, el: impl Into<El<'a, M>>) -> Self {
        self.items.push(Item::El(el.into()));
        self
    }
    pub fn on_press(mut self, m: M) -> Self {
        self.on_press = Some(m);
        self
    }
    /// `None` = disabled.
    pub fn on_press_maybe(mut self, m: Option<M>) -> Self {
        self.on_press = m;
        self
    }
    /// A 16 px spinner before the content; the button is disabled.
    pub fn loading(mut self, on: bool) -> Self {
        self.loading = on;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn width(mut self, w: impl Into<Length>) -> Self {
        self.width = Some(w.into());
        self
    }
    /// For [`crate::focus::focus`].
    pub fn id(mut self, id: impl Into<iced::advanced::widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl<'a, M: Clone + 'a> From<Button<'a, M>> for El<'a, M> {
    fn from(b: Button<'a, M>) -> Self {
        let disabled = b.on_press.is_none() || b.loading || b.disabled;
        let bordered = matches!(
            b.variant,
            Variant::Secondary | Variant::Success | Variant::Danger
        );
        let (h, px, gapw) = match b.size {
            Size::Sm => (32.0, 12.0, 8.0),
            Size::Md => (36.0, 16.0, 8.0),
            Size::Icon(s) => (s as f32, 0.0, 0.0),
        };
        let icon_px = if b.size == Size::Sm { 13.0 } else { 14.0 };
        let mut content: Vec<El<'a, M>> = Vec::with_capacity(b.items.len() + 1);
        if b.loading {
            content.push(spinner(16.0).into());
        }
        for item in b.items {
            content.push(match item {
                Item::Label(t) => btn_label(&t, b.size).into(),
                Item::Icon(i, s) => icon(i, s.unwrap_or(icon_px)).into(),
                Item::El(e) => e,
            });
        }
        let gapw = if b.loading && gapw == 0.0 { 8.0 } else { gapw };
        let pad_x = px + if bordered { 1.0 } else { 0.0 };
        let r = row(content).spacing(gapw).align_y(Alignment::Center);
        let mut s = surface(r)
            .height(h)
            .padding(Padding::from([0.0, pad_x]))
            .center()
            .style(button_style(&b.cx, b.variant, disabled))
            .on_press_maybe(if disabled { None } else { b.on_press })
            .disabled(disabled)
            .focus_ring(Ring::focus(&b.cx));
        if let Some(id) = b.id {
            s = s.id(id);
        }
        if let Size::Icon(sz) = b.size {
            s = s.width(sz as f32);
        }
        if let Some(w) = b.width {
            s = s.width(w);
        }
        if disabled {
            // A pointer-less disabled button still shows `not-allowed`.
            s = s.cursor(crate::cursor::Cursor::NotAllowed);
        }
        s.into()
    }
}
