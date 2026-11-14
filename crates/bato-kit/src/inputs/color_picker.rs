//! `ColorPicker` (`inputs/ColorPicker.tsx`): a hex value picked in a
//! popover.
//!
//! The trigger is a control box with a 20 px swatch and the value in
//! lowercase mono. The popover (`p-3 w-56`, bottom/start) holds:
//! - the saturation/value area (`h-32 rounded-md`, white → hue across,
//!   transparent → black upwards; a 12 px ring marker), picked by press and
//!   drag (pointer captured until release, touch too, `cursor-crosshair`);
//! - the hue strip, a `type="range"` 0–360 over the 7-stop rainbow (4 px
//!   track, 13 px accent thumb that grows 1.2× on hover; keys ← → ↑ ↓,
//!   PageUp/PageDown, Home/End when focused);
//! - a 28 px swatch and the hex field, which only accepts `#` + up to six
//!   hex digits (`/^#[0-9a-fA-F]{0,6}$/`);
//! - the presets, a 6-column grid of squares that grow 1.1× on hover.
//!
//! Every change publishes `on_change(hex)`; the hue comes from the current
//! value each time, as in React (so at black or grey the hue resets).
//!
//! ```ignore
//! color_picker(&on, &state.color, Msg::Color).label("Color")
//! ```

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::gradient::{self, Gradient};
use iced::keyboard::{self, key::Named};
use iced::widget::{column, row};
use iced::{
    Alignment, Background, Border, Color, Degrees, Element, Event, Length, Padding, Rectangle,
    Shadow, Size, Vector, mouse, touch, window,
};

use super::combobox::{bare_input, control_frame, control_trigger};
use crate::anim::{ease_standard, fade};
use crate::ctx::Ctx;
use crate::field::field;
use crate::focus::{FocusState, Ring, paint_ring};
use crate::fonts;
use crate::label::label;
use crate::layout::{Grid, aspect_ratio};
use crate::overlay::popover::{local, popover, popover_panel};
use crate::style::c;
use crate::surface::{BoxStyle, surface};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

// ── Colour math (ported as is) ──────────────────────────────────────────

/// Hue 0–360, saturation and value 0–1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
}

/// `#rgb` / `#rrggbb` (the `#` optional) → 0–1 channels; `None` for
/// anything else (React gets NaN there).
fn channels(hex: &str) -> Option<[f32; 3]> {
    let h = hex.trim_start_matches('#');
    let full: String = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect()
    } else {
        h.to_string()
    };
    if full.len() != 6 || !full.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| {
        u8::from_str_radix(&full[i..i + 2], 16)
            .ok()
            .map(|b| b as f32 / 255.0)
    };
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// `hexToHsv`; an unparsable value gives black (React's NaN draws nothing
/// sensible either).
pub fn hex_to_hsv(hex: &str) -> Hsv {
    let Some([r, g, b]) = channels(hex) else {
        return Hsv {
            h: 0.0,
            s: 0.0,
            v: 0.0,
        };
    };
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let mut hue = 0.0;
    if d != 0.0 {
        hue = if max == r {
            // JS `%` keeps the sign.
            ((g - b) / d) % 6.0
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        hue *= 60.0;
        if hue < 0.0 {
            hue += 360.0;
        }
    }
    Hsv {
        h: hue,
        s: if max != 0.0 { d / max } else { 0.0 },
        v: max,
    }
}

/// `hsvToHex`: lowercase `#rrggbb`.
pub fn hsv_to_hex(Hsv { h, s, v }: Hsv) -> String {
    let c = v * s;
    let x = c * (1.0 - (((h / 60.0) % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    // JS Math.round: halves round up.
    let to = |n: f32| ((n + m) * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", to(r), to(g), to(b))
}

/// The hex field's filter: `/^#[0-9a-fA-F]{0,6}$/`.
pub fn hex_input_ok(s: &str) -> bool {
    s.strip_prefix('#')
        .is_some_and(|d| d.len() <= 6 && d.chars().all(|c| c.is_ascii_hexdigit()))
}

/// A CSS `background` for a hex value: `#rgb`, `#rgba`, `#rrggbb`,
/// `#rrggbbaa`; anything else paints nothing.
pub fn css_hex(s: &str) -> Option<Color> {
    let d = s.strip_prefix('#')?;
    if !d.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let full: String = match d.len() {
        3 | 4 => d.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => d.to_string(),
        _ => return None,
    };
    let b = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).ok();
    let a = if full.len() == 8 { b(6)? } else { 255 };
    Some(Color::from_rgba8(b(0)?, b(2)?, b(4)?, a as f32 / 255.0))
}

/// `DEFAULT_PRESETS`.
pub const PRESETS: [&str; 11] = [
    "#ef4444", "#f97316", "#eab308", "#22c55e", "#06b6d4", "#3b82f6", "#8b5cf6", "#ec4899",
    "#ffffff", "#71717a", "#18181b",
];

// ── Saturation / value area ─────────────────────────────────────────────

/// The `h-32` area: drag to pick saturation (x) and value (y).
struct SvArea<'a, E> {
    hsv: Hsv,
    on_pick: Box<dyn Fn(f32, f32) -> E + 'a>,
}

#[derive(Default)]
struct Drag {
    mouse: bool,
    finger: Option<touch::Finger>,
}

fn sv_at(b: Rectangle, p: iced::Point) -> (f32, f32) {
    let s = ((p.x - b.x) / b.width).clamp(0.0, 1.0);
    let v = (1.0 - (p.y - b.y) / b.height).clamp(0.0, 1.0);
    (s, v)
}

fn linear(angle: f32, from: Color, to: Color) -> Background {
    Background::Gradient(Gradient::Linear(
        gradient::Linear::new(Degrees(angle))
            .add_stop(0.0, fade(from))
            .add_stop(1.0, fade(to)),
    ))
}

impl<'a, E> Widget<E, iced::Theme, iced::Renderer> for SvArea<'a, E> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(128.0))
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Drag>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(Drag::default())
    }
    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.width(Length::Fill).height(128).max())
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, E>,
        _viewport: &Rectangle,
    ) {
        let b = layout.bounds();
        let st = tree.state.downcast_mut::<Drag>();
        let mut pick = |p| {
            let (s, v) = sv_at(b, p);
            shell.publish((self.on_pick)(s, v));
            shell.capture_event();
        };
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(p) = cursor.position_over(b) {
                    st.mouse = true;
                    pick(p);
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if st.mouse => pick(*position),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => st.mouse = false,
            Event::Touch(touch::Event::FingerPressed { id, position }) if b.contains(*position) => {
                st.finger = Some(*id);
                pick(*position);
            }
            Event::Touch(touch::Event::FingerMoved { id, position }) if st.finger == Some(*id) => {
                pick(*position)
            }
            Event::Touch(
                touch::Event::FingerLifted { id, .. } | touch::Event::FingerLost { id, .. },
            ) if st.finger == Some(*id) => st.finger = None,
            _ => {}
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) || tree.state.downcast_ref::<Drag>().mouse {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::None
        }
    }
    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let b = layout.bounds();
        let radius = Border {
            radius: 6.0.into(),
            ..Border::default()
        };
        let hue = css_hex(&hsv_to_hex(Hsv {
            h: self.hsv.h,
            s: 1.0,
            v: 1.0,
        }))
        .unwrap_or(Color::WHITE);
        // `linear-gradient(to top, #000, transparent)` over
        // `linear-gradient(to right, #fff, hue)`.
        renderer.fill_quad(
            Quad {
                bounds: b,
                border: radius,
                ..Quad::default()
            },
            linear(90.0, Color::WHITE, hue),
        );
        renderer.fill_quad(
            Quad {
                bounds: b,
                border: radius,
                ..Quad::default()
            },
            linear(0.0, Color::BLACK, Color::TRANSPARENT),
        );
        // The marker: 12 px, `border-2 border-white shadow`, centred on the
        // point.
        let cx = b.x + self.hsv.s * b.width;
        let cy = b.y + (1.0 - self.hsv.v) * b.height;
        let m = Rectangle::new(iced::Point::new(cx - 6.0, cy - 6.0), Size::new(12.0, 12.0));
        for (y, blur, spread) in [(1.0, 3.0, 0.0), (1.0, 2.0, -1.0)] {
            renderer.fill_quad(
                Quad {
                    bounds: m.expand(spread),
                    border: Border {
                        radius: (6.0 + spread).into(),
                        ..Border::default()
                    },
                    shadow: Shadow {
                        color: fade(Color::from_rgba(0.0, 0.0, 0.0, 0.1)),
                        offset: Vector::new(0.0, y),
                        blur_radius: blur,
                    },
                    snap: false,
                },
                Color::TRANSPARENT,
            );
        }
        renderer.fill_quad(
            Quad {
                bounds: m,
                border: Border {
                    radius: 6.0.into(),
                    width: 2.0,
                    color: fade(Color::WHITE),
                },
                ..Quad::default()
            },
            Color::TRANSPARENT,
        );
    }
}

// ── Hue strip ───────────────────────────────────────────────────────────

/// Chromium's range thumb (13 px) and its `scale(1.2)` hover (100 ms).
const THUMB: f32 = 13.0;
const GROW: Duration = Duration::from_millis(100);

/// The hue `<input type="range" min=0 max=360>`.
struct HueStrip<'a, E> {
    cx: Ctx,
    hue: f32,
    on_hue: Box<dyn Fn(f32) -> E + 'a>,
}

#[derive(Default)]
struct HueState {
    focus: FocusState,
    drag: bool,
    finger: Option<touch::Finger>,
    hovered: bool,
    hover_at: Option<Instant>,
}

/// The value under `x` on a track `b` (the thumb centre travels from
/// `THUMB / 2` to `width - THUMB / 2`), rounded to the step of 1.
pub fn hue_at(b: Rectangle, x: f32) -> f32 {
    let span = (b.width - THUMB).max(1.0);
    (((x - b.x - THUMB / 2.0) / span).clamp(0.0, 1.0) * 360.0).round()
}

/// A range input's keys: the new value, or `None` for keys it ignores.
pub fn range_key(v: f32, key: Named) -> Option<f32> {
    let v = v.round();
    let n = match key {
        Named::ArrowRight | Named::ArrowUp => v + 1.0,
        Named::ArrowLeft | Named::ArrowDown => v - 1.0,
        Named::PageUp => v + 36.0,
        Named::PageDown => v - 36.0,
        Named::Home => 0.0,
        Named::End => 360.0,
        _ => return None,
    };
    Some(n.clamp(0.0, 360.0))
}

impl<'a, E: Clone> HueStrip<'a, E> {
    fn thumb(&self, b: Rectangle) -> Rectangle {
        let x = b.x + THUMB / 2.0 + (b.width - THUMB) * (self.hue / 360.0).clamp(0.0, 1.0);
        let y = b.y + b.height / 2.0;
        Rectangle::new(
            iced::Point::new(x - THUMB / 2.0, y - THUMB / 2.0),
            Size::new(THUMB, THUMB),
        )
    }
    /// The hit area: the track plus the overflowing thumb.
    fn hit(&self, b: Rectangle) -> Rectangle {
        Rectangle {
            y: b.y + b.height / 2.0 - THUMB / 2.0,
            height: THUMB,
            ..b
        }
    }
}

impl<'a, E: Clone> Widget<E, iced::Theme, iced::Renderer> for HueStrip<'a, E> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(4.0))
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<HueState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(HueState::default())
    }
    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.width(Length::Fill).height(4).max())
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, E>,
        _viewport: &Rectangle,
    ) {
        let b = layout.bounds();
        let hit = self.hit(b);
        let st = tree.state.downcast_mut::<HueState>();
        let inside = match event {
            Event::Touch(touch::Event::FingerPressed { position, .. }) => hit.contains(*position),
            _ => cursor.is_over(hit),
        };
        if st.focus.pointer(event, inside, true) {
            shell.request_redraw();
        }
        // No `reveal`: in a popover, window coordinates are not the page's.
        let set = |shell: &mut Shell<'_, E>, v: f32| {
            shell.publish((self.on_hue)(v));
            shell.capture_event();
        };
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if inside => {
                st.drag = true;
                set(shell, hue_at(b, cursor.position().unwrap_or_default().x));
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if st.drag {
                    set(shell, hue_at(b, position.x));
                }
                let over = self.thumb(b).contains(*position);
                if over != st.hovered {
                    st.hovered = over;
                    st.hover_at = Some(Instant::now());
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => st.drag = false,
            Event::Touch(touch::Event::FingerPressed { id, position }) if inside => {
                st.finger = Some(*id);
                set(shell, hue_at(b, position.x));
            }
            Event::Touch(touch::Event::FingerMoved { id, position }) if st.finger == Some(*id) => {
                set(shell, hue_at(b, position.x))
            }
            Event::Touch(
                touch::Event::FingerLifted { id, .. } | touch::Event::FingerLost { id, .. },
            ) if st.finger == Some(*id) => st.finger = None,
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(k),
                modifiers,
                ..
            }) if st.focus.is_focused() && crate::focus::plain(*modifiers) => {
                if let Some(v) = range_key(self.hue, *k) {
                    set(shell, v);
                }
            }
            Event::Window(window::Event::RedrawRequested(now))
                if st
                    .hover_at
                    .is_some_and(|t| now.saturating_duration_since(t) < GROW) =>
            {
                shell.request_redraw();
            }
            _ => {}
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(self.hit(layout.bounds())) || tree.state.downcast_ref::<HueState>().drag {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let st = tree.state.downcast_mut::<HueState>();
        st.focus
            .report(operation, None, layout.bounds(), true, true);
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_ref::<HueState>();
        let b = layout.bounds();
        let stops = [
            "#ff0000", "#ffff00", "#00ff00", "#00ffff", "#0000ff", "#ff00ff", "#ff0000",
        ];
        let mut g = gradient::Linear::new(Degrees(90.0));
        for (i, s) in stops.iter().enumerate() {
            g = g.add_stop(i as f32 / 6.0, fade(css_hex(s).unwrap()));
        }
        renderer.fill_quad(
            Quad {
                bounds: b,
                border: Border {
                    radius: 2.0.into(),
                    ..Border::default()
                },
                ..Quad::default()
            },
            Background::Gradient(Gradient::Linear(g)),
        );
        let k = match st.hover_at {
            Some(t) => {
                let f = (t.elapsed().as_secs_f32() / GROW.as_secs_f32()).min(1.0);
                let e = ease_standard(f);
                if st.hovered { e } else { 1.0 - e }
            }
            None => 0.0,
        };
        let thumb = self.thumb(b);
        let thumb = thumb.expand(THUMB * 0.1 * k);
        if st.focus.ring_visible() {
            // `0 0 0 2px bg, 0 0 0 4px accentBright` on the thumb.
            paint_ring(renderer, thumb, thumb.width / 2.0, &Ring::focus(&self.cx));
        }
        renderer.fill_quad(
            Quad {
                bounds: thumb,
                border: Border {
                    radius: (thumb.width / 2.0).into(),
                    ..Border::default()
                },
                ..Quad::default()
            },
            fade(c(self.cx.p().accent)),
        );
    }
}

// ── ColorPicker ─────────────────────────────────────────────────────────

pub struct ColorPicker<'a, M> {
    cx: Ctx,
    value: String,
    on_change: Box<dyn Fn(String) -> M + 'a>,
    presets: Vec<String>,
    label: Option<String>,
    hint: Option<String>,
    error: Option<String>,
}

/// `<ColorPicker value onChange />` on `cx` (the field's backdrop).
pub fn color_picker<'a, M>(
    cx: &Ctx,
    value: &str,
    on_change: impl Fn(String) -> M + 'a,
) -> ColorPicker<'a, M> {
    ColorPicker {
        cx: cx.clone(),
        value: value.to_string(),
        on_change: Box::new(on_change),
        presets: PRESETS.iter().map(|s| s.to_string()).collect(),
        label: None,
        hint: None,
        error: None,
    }
}

impl<'a, M> ColorPicker<'a, M> {
    pub fn presets(mut self, p: Vec<String>) -> Self {
        self.presets = p;
        self
    }
    pub fn label(mut self, s: impl Into<String>) -> Self {
        self.label = Some(s.into());
        self
    }
    pub fn hint(mut self, s: impl Into<String>) -> Self {
        self.hint = Some(s.into());
        self
    }
    pub fn error(mut self, s: Option<&str>) -> Self {
        self.error = s.map(str::to_string);
        self
    }
}

#[derive(Debug, Default)]
pub struct PickerState {
    pub open: bool,
}

#[derive(Debug, Clone)]
pub enum PickerEvent {
    Toggle,
    Close,
    Set(String),
}

/// A `rounded border-app-border` swatch of `size` px.
fn swatch<'a, E: Clone + 'a>(cx: &Ctx, value: &str, size: f32) -> El<'a, E> {
    let mut st = BoxStyle::default().border(c(cx.p().border)).radius(4.0);
    if let Some(col) = css_hex(value) {
        // The border paints over the fill (CSS), so blend the border's
        // inner half-pixel the way iced draws it: fill under, border over.
        st = st.bg(col);
    }
    surface(iced::widget::Space::new())
        .width(size)
        .height(size)
        .fixed(st)
        .into()
}

impl<'a, M: Clone + 'a> From<ColorPicker<'a, M>> for El<'a, M> {
    fn from(cp: ColorPicker<'a, M>) -> Self {
        let ColorPicker {
            cx,
            value,
            on_change,
            presets,
            label: lab,
            hint,
            error,
        } = cp;
        let fcx = cx.clone();
        let has_error = error.is_some();
        let control = local(
            move |s: &PickerState| {
                let p = cx.p();
                let hsv = hex_to_hsv(&value);
                let trigger = control_trigger(
                    &cx,
                    row![
                        swatch(&cx, &value, 20.0),
                        label(value.to_lowercase(), 12.0, 19.2)
                            .mono()
                            .color(c(p.text)),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .into(),
                    Padding::from([9.0, 13.0]),
                    has_error,
                    false,
                    PickerEvent::Toggle,
                );
                let panel = s.open.then(|| {
                    let on = cx.on_card();
                    let sv = Element::new(SvArea {
                        hsv,
                        on_pick: Box::new(move |s, v| {
                            PickerEvent::Set(hsv_to_hex(Hsv { h: hsv.h, s, v }))
                        }),
                    });
                    let hue = Element::new(HueStrip {
                        cx: on.clone(),
                        hue: hsv.h,
                        on_hue: Box::new(move |h| PickerEvent::Set(hsv_to_hex(Hsv { h, ..hsv }))),
                    });
                    let hex = control_frame(
                        &on,
                        bare_input(&on, "", &value, |s| {
                            if hex_input_ok(&s) {
                                PickerEvent::Set(s)
                            } else {
                                PickerEvent::Set(String::new())
                            }
                        })
                        .font(fonts::mono())
                        .size(12)
                        .line_height(iced::widget::text::LineHeight::Absolute(19.2.into()))
                        .padding(Padding::from([7.0, 9.0])),
                    )
                    .radius(6.0)
                    .text(true)
                    .in_panel();
                    let mut grid = Grid::columns(6).gap(6.0);
                    for preset in &presets {
                        let st = BoxStyle::default()
                            .bg(css_hex(preset).unwrap_or(Color::TRANSPARENT))
                            .border(c(on.p().border))
                            .radius(4.0);
                        grid = grid.push(aspect_ratio(
                            1.0,
                            surface(iced::widget::Space::new())
                                .width(Length::Fill)
                                .height(Length::Fill)
                                .fixed(st)
                                .hover_scale(1.1)
                                .on_press(PickerEvent::Set(preset.clone())),
                        ));
                    }
                    popover_panel(
                        &cx,
                        column![
                            sv,
                            iced::widget::Space::new().height(12),
                            hue,
                            iced::widget::Space::new().height(12),
                            row![swatch(&on, &value, 28.0), hex]
                                .spacing(8)
                                .align_y(Alignment::Center),
                            iced::widget::Space::new().height(12),
                            grid,
                        ],
                    )
                    .padding(13)
                    .width(224)
                    .into()
                });
                popover(trigger, panel)
                    .on_dismiss(|_| PickerEvent::Close)
                    .into()
            },
            move |s: &mut PickerState, e| match e {
                PickerEvent::Toggle => {
                    s.open = !s.open;
                    None
                }
                PickerEvent::Close => {
                    s.open = false;
                    None
                }
                // A rejected hex edit: nothing changes (the field shows the
                // value again).
                PickerEvent::Set(hex) if hex.is_empty() => None,
                PickerEvent::Set(hex) => Some(on_change(hex)),
            },
        );
        let mut f = field(&fcx, control).error(error.as_deref());
        if let Some(l) = lab {
            f = f.label(l);
        }
        if let Some(h) = hint {
            f = f.hint(h);
        }
        f.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_hsv_round_trip() {
        let hsv = hex_to_hsv("#7c3aed");
        assert!((hsv.h - 262.1).abs() < 0.1, "{hsv:?}");
        assert!((hsv.s - 0.7553).abs() < 1e-3);
        assert!((hsv.v - 0.9294).abs() < 1e-3);
        assert_eq!(hsv_to_hex(hsv), "#7c3aed");
        for p in PRESETS {
            assert_eq!(hsv_to_hex(hex_to_hsv(p)), p, "{p}");
        }
        assert_eq!(hex_to_hsv("#f00").h, 0.0);
        assert_eq!(hsv_to_hex(hex_to_hsv("#0f0")), "#00ff00");
        // Hue of a red with more blue than green: negative, wrapped.
        assert!((hex_to_hsv("#ff0080").h - 329.88).abs() < 0.1);
    }

    #[test]
    fn hue_is_lost_at_grey_like_react() {
        let grey = hex_to_hsv("#71717a");
        assert!(grey.h > 200.0);
        assert_eq!(hex_to_hsv("#000000").h, 0.0);
        assert_eq!(hex_to_hsv("#ffffff").s, 0.0);
        // Unparsable values are black.
        assert_eq!(hex_to_hsv("#7c3").v, hex_to_hsv("#77cc33").v);
        assert_eq!(hex_to_hsv("#7c").v, 0.0);
    }

    #[test]
    fn hex_field_filter() {
        assert!(hex_input_ok("#"));
        assert!(hex_input_ok("#7C3aEd"));
        assert!(!hex_input_ok("7c3aed"));
        assert!(!hex_input_ok("#7c3aed0"));
        assert!(!hex_input_ok("#7g"));
    }

    #[test]
    fn css_backgrounds() {
        assert_eq!(css_hex("#fff"), Some(Color::WHITE));
        assert_eq!(css_hex("#7c3a").map(|c| c.a), Some(170.0 / 255.0));
        assert_eq!(css_hex("#7c3ae"), None);
        assert_eq!(css_hex("#"), None);
    }

    #[test]
    fn range_input_geometry_and_keys() {
        let b = Rectangle::new(iced::Point::new(100.0, 0.0), Size::new(198.0, 4.0));
        assert_eq!(hue_at(b, 100.0), 0.0);
        assert_eq!(hue_at(b, 106.5), 0.0);
        assert_eq!(hue_at(b, 100.0 + 99.0), 180.0);
        assert_eq!(hue_at(b, 291.5), 360.0);
        assert_eq!(hue_at(b, 400.0), 360.0);
        assert_eq!(range_key(262.1, Named::ArrowRight), Some(263.0));
        assert_eq!(range_key(0.0, Named::ArrowLeft), Some(0.0));
        assert_eq!(range_key(350.0, Named::PageUp), Some(360.0));
        assert_eq!(range_key(10.0, Named::End), Some(360.0));
        assert_eq!(range_key(10.0, Named::Enter), None);
    }
}
