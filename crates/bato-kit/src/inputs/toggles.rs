//! `RadioGroup` (`Form.tsx`) and the native-looking checkbox and radio
//! (`index.css`'s `input[type=checkbox|radio]` rules).
//!
//! - [`radio_group`]: a column (`gap-2`) of rows: a 16 px round button
//!   (`bg@.5`, `control` border, accentBright on hover; selected: accent
//!   border and an 8 px accent dot), the 14 px `subtext` label. Every radio
//!   is a Tab stop with `focusRing` (React does not rove); a click on the
//!   label selects and focuses it. Disabled: `opacity .5`, inert.
//! - [`native_checkbox`], [`native_radio`]: 16 px, `surface` fill, `control`
//!   border, `accent@.6` border on hover; checked: accent fill with a
//!   clip-path tick in `accent_ink` (indeterminate: an 8×2 bar), radio: accent
//!   border and an 8 px dot; the mark scales in over 100 ms ease-out; colours
//!   move over 150 ms; `:focus-visible` is a 2 px accentBright outline 2 px
//!   out. A row is `<label>` (gap 10, 14 px `subtext`). Wrap native radios
//!   of one `name` in [`native_radio_group`]: Chromium's arrow keys move the
//!   checked one and Tab stops only on it.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::svg::{self, Renderer as _};
use iced::advanced::widget::{Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::widget::{Column, Row, Space, row};
use iced::{
    Alignment, Border, Color, Element, Event, Length, Radians, Rectangle, Size, mouse, window,
};

use crate::El;
use crate::anim::{current_opacity, ease_out, fade, opacity};
use crate::ctx::Ctx;
use crate::cursor::Cursor;
use crate::focus::{Ring, roving};
use crate::label::label;
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

// ── RadioGroup ──────────────────────────────────────────────────────────

pub struct RadioGroup<'a, T, M> {
    cx: Ctx,
    value: T,
    options: Vec<(T, String)>,
    on_change: Box<dyn Fn(T) -> M + 'a>,
    disabled: bool,
}

/// React's `RadioGroup`: `options` are `(value, label)`.
pub fn radio_group<'a, T: Clone + PartialEq + 'a, M: Clone + 'a>(
    cx: &Ctx,
    value: T,
    options: Vec<(T, String)>,
    on_change: impl Fn(T) -> M + 'a,
) -> RadioGroup<'a, T, M> {
    RadioGroup {
        cx: cx.clone(),
        value,
        options,
        on_change: Box::new(on_change),
        disabled: false,
    }
}

impl<'a, T: Clone + PartialEq + 'a, M: Clone + 'a> RadioGroup<'a, T, M> {
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
}

impl<'a, T: Clone + PartialEq + 'a, M: Clone + 'a> From<RadioGroup<'a, T, M>> for El<'a, M> {
    fn from(g: RadioGroup<'a, T, M>) -> Self {
        let cx = g.cx.clone();
        let disabled = g.disabled;
        let rows: Vec<El<'a, M>> = g
            .options
            .into_iter()
            .map(|(v, text)| {
                let active = v == g.value;
                let msg = (!disabled).then(|| (g.on_change)(v));
                let t = cx.t.clone();
                let over = cx.over;
                // `opacity-50` on the row, composited once (the dot must not
                // show the fill through it).
                let dim = move |rgb: bato_theme::Rgb| {
                    if disabled {
                        c(bato_theme::blend(rgb, 0.5, over))
                    } else {
                        c(rgb)
                    }
                };
                let dot: El<'a, M> = if active {
                    surface(Space::new().width(8).height(8))
                        .fixed(BoxStyle::default().bg(dim(cx.p().accent)).radius(4.0))
                        .into()
                } else {
                    Space::new().width(8).height(8).into()
                };
                let button = surface(dot)
                    .width(16)
                    .height(16)
                    .center()
                    .on_press_maybe(msg.clone())
                    .focus_ring(Ring::focus(&cx))
                    .style(move |st| {
                        let p = &t.p;
                        let border = if active {
                            p.accent
                        } else if st == Status::Hover && !disabled {
                            p.accent_bright
                        } else {
                            p.control
                        };
                        BoxStyle::default()
                            .bg(dim(t.recessed(over)))
                            .border(dim(border))
                            .radius(8.0)
                    });
                let whole = surface(
                    row![button, label(text, 14.0, 20.0).color(dim(cx.p().subtext))]
                        .spacing(10)
                        .align_y(Alignment::Center),
                )
                .on_press_maybe(msg)
                .focusable(false)
                .labels_child()
                .cursor(if disabled {
                    Cursor::Default
                } else {
                    Cursor::Pointer
                });
                whole.into()
            })
            .collect();
        Column::with_children(rows).spacing(8).into()
    }
}

// ── The mark: tick, dash or dot, scaling in ─────────────────────────────

/// `clip-path: polygon(14% 44%, 0 65%, 50% 100%, 100% 16%, 80% 0%, 43% 62%)`.
const TICK: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100"><polygon points="14,44 0,65 50,100 100,16 80,0 43,62" fill="#000000"/></svg>"##;

fn tick_handle() -> svg::Handle {
    thread_local! {
        static H: svg::Handle = svg::Handle::from_memory(TICK.as_bytes());
    }
    H.with(Clone::clone)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// The checkbox tick (10×10).
    Tick,
    /// The indeterminate bar (8×2, radius 1).
    Dash,
    /// The radio dot (8×8 round).
    Dot,
}

/// A mark that scales from 0 to 1 over 100 ms ease-out when `on` turns
/// true (`transform: scale(0)` → `scale(1)`; the reverse too).
struct Glyph {
    mark: Mark,
    on: bool,
    color: Color,
}

#[derive(Debug, Default)]
struct GlyphState {
    init: bool,
    shown: bool,
    from: f32,
    t: f32,
    started: Option<Instant>,
}

const MARK_IN: Duration = Duration::from_millis(100);

impl<M> Widget<M, iced::Theme, iced::Renderer> for Glyph {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(10.0), Length::Fixed(10.0))
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<GlyphState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(GlyphState::default())
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        _r: &iced::Renderer,
        _l: &layout::Limits,
    ) -> layout::Node {
        let st = tree.state.downcast_mut::<GlyphState>();
        if !st.init {
            st.init = true;
            st.shown = self.on;
            st.t = if self.on { 1.0 } else { 0.0 };
        }
        layout::Node::new(Size::new(10.0, 10.0))
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _r: &iced::Renderer,
        _c: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _v: &Rectangle,
    ) {
        let st = tree.state.downcast_mut::<GlyphState>();
        if st.shown != self.on {
            st.shown = self.on;
            st.from = st.t;
            st.started = Some(Instant::now());
            shell.request_redraw();
        }
        if let Event::Window(window::Event::RedrawRequested(now)) = event
            && let Some(s) = st.started
        {
            let target = if self.on { 1.0 } else { 0.0 };
            let k =
                (now.saturating_duration_since(s).as_secs_f32() / MARK_IN.as_secs_f32()).min(1.0);
            st.t = st.from + (target - st.from) * ease_out(k);
            if k >= 1.0 {
                st.started = None;
            } else {
                shell.request_redraw();
            }
        }
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let s = tree.state.downcast_ref::<GlyphState>().t;
        if s <= 0.0 {
            return;
        }
        let b = layout.bounds();
        let (cx, cy) = (b.center_x(), b.center_y());
        let (w, h, r) = match self.mark {
            Mark::Tick => (10.0, 10.0, 0.0),
            Mark::Dash => (8.0, 2.0, 1.0),
            Mark::Dot => (8.0, 8.0, 4.0),
        };
        let rect = Rectangle {
            x: cx - w * s / 2.0,
            y: cy - h * s / 2.0,
            width: w * s,
            height: h * s,
        };
        match self.mark {
            Mark::Tick => renderer.draw_svg(
                svg::Svg {
                    handle: tick_handle(),
                    color: Some(Color {
                        a: 1.0,
                        ..self.color
                    }),
                    rotation: Radians(0.0),
                    opacity: self.color.a * current_opacity(),
                },
                rect,
                viewport.expand(2.0),
            ),
            _ => renderer.fill_quad(
                Quad {
                    bounds: rect,
                    border: Border {
                        radius: (r * s).into(),
                        ..Border::default()
                    },
                    ..Quad::default()
                },
                fade(self.color),
            ),
        }
    }
}

// ── Native checkbox / radio ─────────────────────────────────────────────

/// A native `<input type=checkbox|radio>` row.
pub struct Native<'a, M> {
    cx: Ctx,
    radio: bool,
    checked: bool,
    indeterminate: bool,
    disabled: bool,
    label: Option<String>,
    on_press: Option<M>,
    tab_stop: bool,
    _p: std::marker::PhantomData<&'a ()>,
}

/// `<label><input type="checkbox"> label</label>`, themed by index.css.
pub fn native_checkbox<'a, M: Clone + 'a>(
    cx: &Ctx,
    checked: bool,
    on_toggle: impl Fn(bool) -> M,
) -> Native<'a, M> {
    Native {
        cx: cx.clone(),
        radio: false,
        checked,
        indeterminate: false,
        disabled: false,
        label: None,
        on_press: Some(on_toggle(!checked)),
        tab_stop: true,
        _p: std::marker::PhantomData,
    }
}

/// `<label><input type="radio"> label</label>`, themed by index.css; a
/// press publishes `on_select` (a checked radio stays checked).
pub fn native_radio<'a, M: Clone + 'a>(cx: &Ctx, checked: bool, on_select: M) -> Native<'a, M> {
    Native {
        cx: cx.clone(),
        radio: true,
        checked,
        indeterminate: false,
        disabled: false,
        label: None,
        on_press: Some(on_select),
        tab_stop: checked,
        _p: std::marker::PhantomData,
    }
}

impl<'a, M: Clone + 'a> Native<'a, M> {
    pub fn label(mut self, t: impl Into<String>) -> Self {
        self.label = Some(t.into());
        self
    }
    /// `el.indeterminate = true` (checkboxes): the bar, accent fill.
    pub fn indeterminate(mut self, on: bool) -> Self {
        self.indeterminate = on;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
}

/// The box style of a native control.
fn native_style(cx: &Ctx, radio: bool, filled: bool, ring_accent: bool, hover: bool) -> BoxStyle {
    let p = cx.p();
    let bg = if filled { p.accent } else { p.surface };
    let border = if filled || ring_accent {
        c(p.accent)
    } else if hover {
        c(bato_theme::blend(p.accent, 0.6, bg))
    } else {
        c(p.control)
    };
    BoxStyle::default()
        .bg(c(bg))
        .border(border)
        .radius(if radio { 8.0 } else { 4.0 })
}

impl<'a, M: Clone + 'a> From<Native<'a, M>> for El<'a, M> {
    fn from(n: Native<'a, M>) -> Self {
        let cx = n.cx.clone();
        let p = cx.p();
        let filled = !n.radio && (n.checked || n.indeterminate);
        let (mark, on, color) = if n.radio {
            (Mark::Dot, n.checked, c(p.accent))
        } else if n.indeterminate {
            (Mark::Dash, true, c(p.accent_ink))
        } else {
            (Mark::Tick, n.checked, c(p.accent_ink))
        };
        let msg = if n.disabled { None } else { n.on_press };
        let disabled = n.disabled;
        let (radio, ring_accent) = (n.radio, n.radio && n.checked);
        let st_cx = cx.clone();
        let boxed = surface(Element::new(Glyph { mark, on, color }))
            .width(16)
            .height(16)
            .center()
            .on_press_maybe(msg.clone())
            .tab_stop(n.tab_stop)
            .focus_ring(Ring::offset_on(&cx, cx.over))
            .style(move |s| {
                native_style(
                    &st_cx,
                    radio,
                    filled,
                    ring_accent,
                    s == Status::Hover && !disabled,
                )
            });
        let mut items: Vec<El<'a, M>> = vec![boxed.into()];
        if let Some(l) = n.label {
            items.push(label(l, 14.0, 20.0).color(c(p.subtext)).into());
        }
        let whole = surface(
            Row::with_children(items)
                .spacing(10)
                .align_y(Alignment::Center),
        )
        .on_press_maybe(msg)
        .focusable(false)
        .labels_child()
        .cursor(if disabled {
            Cursor::NotAllowed
        } else {
            Cursor::Pointer
        });
        if disabled {
            opacity(0.5, whole).into()
        } else {
            whole.into()
        }
    }
}

/// Native radios sharing a `name`: arrows move the checked one (wrapping),
/// publishing `on_move(index)`; only the checked one is a Tab stop.
pub fn native_radio_group<'a, M: 'a>(
    radios: Vec<El<'a, M>>,
    spacing: f32,
    on_move: impl Fn(usize) -> M + 'a,
) -> El<'a, M> {
    roving(
        Row::with_children(radios)
            .spacing(spacing)
            .align_y(Alignment::Center),
    )
    .on_move(on_move)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bato_theme::{Rgb, ThemeId, palette};

    #[test]
    fn native_colours_follow_index_css() {
        let cx = Ctx::from_palette(palette(ThemeId::Dark, Rgb::hex("#8b5cf6").unwrap())).on_card();
        let p = cx.p().clone();
        let off = native_style(&cx, false, false, false, false);
        assert_eq!(off.bg, Some(c(p.surface)), "surface, not bg/50");
        assert_eq!(off.border, Some(c(p.control)));
        assert_eq!(off.radius, 4.0);
        let hover = native_style(&cx, false, false, false, true);
        assert_eq!(
            hover.border,
            Some(c(bato_theme::blend(p.accent, 0.6, p.surface)))
        );
        let on = native_style(&cx, false, true, false, false);
        assert_eq!(on.bg, Some(c(p.accent)));
        let radio_on = native_style(&cx, true, false, true, false);
        assert_eq!(radio_on.bg, Some(c(p.surface)));
        assert_eq!(radio_on.border, Some(c(p.accent)));
        assert_eq!(radio_on.radius, 8.0);
    }
}
