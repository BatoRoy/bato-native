//! `Slider`, `RangeSlider` (`inputs/Slider.tsx`) and `VolumeControl`.
//!
//! A 20 px row holding a 6 px pill track (`control`; the range slider's is
//! `border`), the accent fill and 16 px accent thumbs with a 2 px page-`bg`
//! border and Tailwind's `shadow`, scaling to 1.1 on hover (150 ms).
//! Slider: a press on the track (or its thumb) jumps there and drags
//! (pointer or finger; the value snaps to `step`); cursor `pointer` on the
//! track, `grab` on a thumb, `grabbing` while dragging. RangeSlider: only the
//! thumbs drag, and they never cross. Disabled: `opacity .5`, inert. Like
//! React's `div role=slider` thumbs, neither is focusable.
//!
//! VolumeControl: a 32 px mute button (icon by level: VolumeX / Volume1 below
//! half / Volume2; accentBright while muted), the slider, and the value in
//! a `w-9` right-aligned 12 px column. Muting drops the slider to `min` and
//! remembers the level; unmuting (or dragging up) restores it.
//!
//! ```ignore
//! slider(&cx, self.vol, Msg::Vol)
//! range_slider(&cx, self.range, Msg::Range)
//! volume_control(&cx, &self.media, self.media_vol, Msg::Media).show_value(true)
//! // update: Msg::Media(ev) => self.media.update(ev, &mut self.media_vol, 0.0, 100.0)
//! ```

use std::time::Instant;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::widget::{container, row};
use iced::{
    Alignment, Border, Color, Element, Event, Length, Point, Rectangle, Size, mouse, touch, window,
};

use super::frame::TRANSITION;
use crate::El;
use crate::anim::{ease_standard, fade, opacity};
use crate::ctx::Ctx;
use crate::focus::Ring;
use crate::icon::{Ico, icon};
use crate::label::{Align, label};
use crate::style::c;
use crate::surface::{BoxStyle, Sh, Status, paint, surface};

/// `useTrackValue`: the stepped value at `x` on a track `[left, left+width]`.
pub fn value_at(x: f32, left: f32, width: f32, min: f64, max: f64, step: f64) -> f64 {
    let ratio = (((x - left) / width.max(1e-6)) as f64).clamp(0.0, 1.0);
    let raw = min + ratio * (max - min);
    (raw / step).round() * step
}

/// `pct(v)` as a 0..1 fraction.
pub fn frac(v: f64, min: f64, max: f64) -> f32 {
    if max <= min {
        return 0.0;
    }
    ((v - min) / (max - min)) as f32
}

/// The range rule: a moved thumb never crosses the other.
pub fn move_thumb(range: [f64; 2], which: usize, v: f64) -> [f64; 2] {
    let [lo, hi] = range;
    if which == 0 {
        [v.min(hi), hi]
    } else {
        [lo, v.max(lo)]
    }
}

const THUMB: f32 = 16.0;
const TRACK: f32 = 6.0;
const HEIGHT: f32 = 20.0;

pub struct Slider<'a, M> {
    cx: Ctx,
    values: [f64; 2],
    range: bool,
    on_change: Box<dyn Fn([f64; 2]) -> M + 'a>,
    min: f64,
    max: f64,
    step: f64,
    disabled: bool,
    marks: bool,
    /// Fade itself when disabled (not inside a row that already fades).
    fade_self: bool,
}

/// A single-thumb slider over `0..=100` by default.
pub fn slider<'a, M: Clone + 'a>(
    cx: &Ctx,
    value: f64,
    on_change: impl Fn(f64) -> M + 'a,
) -> Slider<'a, M> {
    Slider {
        cx: cx.clone(),
        values: [value, value],
        range: false,
        on_change: Box::new(move |v| on_change(v[0])),
        min: 0.0,
        max: 100.0,
        step: 1.0,
        disabled: false,
        marks: false,
        fade_self: true,
    }
}

/// A two-thumb slider (`[lo, hi]`).
pub fn range_slider<'a, M: Clone + 'a>(
    cx: &Ctx,
    value: (f64, f64),
    on_change: impl Fn((f64, f64)) -> M + 'a,
) -> Slider<'a, M> {
    Slider {
        cx: cx.clone(),
        values: [value.0, value.1],
        range: true,
        on_change: Box::new(move |v| on_change((v[0], v[1]))),
        min: 0.0,
        max: 100.0,
        step: 1.0,
        disabled: false,
        marks: false,
        fade_self: true,
    }
}

impl<'a, M: Clone + 'a> Slider<'a, M> {
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min = min;
        self.max = max;
        self
    }
    pub fn step(mut self, s: f64) -> Self {
        self.step = s;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    /// A 2 px `muted` dot at every step (single slider).
    pub fn marks(mut self, on: bool) -> Self {
        self.marks = on;
        self
    }
}

#[derive(Debug, Default)]
struct ThumbAnim {
    hovered: bool,
    from: f32,
    t: f32,
    started: Option<Instant>,
}

impl ThumbAnim {
    fn set(&mut self, on: bool, now: Instant) -> bool {
        if on == self.hovered {
            return false;
        }
        self.hovered = on;
        self.from = self.t;
        self.started = Some(now);
        true
    }
    fn advance(&mut self, now: Instant) -> bool {
        let Some(s) = self.started else { return false };
        let target = if self.hovered { 1.0 } else { 0.0 };
        let k =
            (now.saturating_duration_since(s).as_secs_f32() / TRANSITION.as_secs_f32()).min(1.0);
        self.t = self.from + (target - self.from) * ease_standard(k);
        if k >= 1.0 {
            self.started = None;
            false
        } else {
            true
        }
    }
}

#[derive(Debug, Default)]
struct SliderState {
    /// The thumb being dragged (0 or 1).
    drag: Option<usize>,
    finger: Option<u64>,
    thumbs: [ThumbAnim; 2],
}

impl<'a, M: Clone + 'a> Slider<'a, M> {
    fn track(&self, b: Rectangle) -> Rectangle {
        Rectangle {
            x: b.x,
            y: b.y + (HEIGHT - TRACK) / 2.0,
            width: b.width,
            height: TRACK,
        }
    }
    fn thumb_rect(&self, b: Rectangle, v: f64) -> Rectangle {
        let cx = b.x + b.width * frac(v, self.min, self.max);
        Rectangle {
            x: cx - THUMB / 2.0,
            y: b.y + (HEIGHT - THUMB) / 2.0,
            width: THUMB,
            height: THUMB,
        }
    }
    /// The thumbs on screen, topmost last.
    fn thumbs(&self) -> &[f64] {
        if self.range {
            &self.values
        } else {
            &self.values[..1]
        }
    }
    /// The thumb under `p` (topmost first).
    fn thumb_at(&self, b: Rectangle, p: Point) -> Option<usize> {
        (0..self.thumbs().len())
            .rev()
            .find(|&i| self.thumb_rect(b, self.values[i]).contains(p))
    }
    fn on_track(&self, b: Rectangle, p: Point) -> bool {
        self.track(b).contains(p)
    }
    fn value_at(&self, b: Rectangle, x: f32) -> f64 {
        value_at(x, b.x, b.width, self.min, self.max, self.step)
    }
    /// The message for dragging thumb `which` to `x`.
    fn drag_to(&self, b: Rectangle, which: usize, x: f32) -> M {
        let v = self.value_at(b, x).clamp(self.min, self.max);
        if self.range {
            (self.on_change)(move_thumb(self.values, which, v))
        } else {
            (self.on_change)([v, v])
        }
    }
    /// A press at `p`: which thumb starts dragging, and the jump message.
    fn press(&self, b: Rectangle, p: Point) -> Option<(usize, Option<M>)> {
        if self.disabled {
            return None;
        }
        if self.range {
            return self.thumb_at(b, p).map(|i| (i, None));
        }
        if self.thumb_at(b, p).is_some() || self.on_track(b, p) {
            // The thumb's press bubbles to the track: both jump and drag.
            let v = self.value_at(b, p.x);
            return Some((0, Some((self.on_change)([v, v]))));
        }
        None
    }
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for Slider<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(HEIGHT))
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<SliderState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(SliderState::default())
    }
    fn layout(
        &mut self,
        _t: &mut Tree,
        _r: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let l = limits.width(Length::Fill).height(HEIGHT);
        layout::Node::new(l.resolve(Length::Fill, HEIGHT, Size::new(0.0, HEIGHT)))
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _r: &iced::Renderer,
        _c: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let b = crate::scale::snap_rect(layout.bounds());
        let st = tree.state.downcast_mut::<SliderState>();
        let now = Instant::now();
        if self.disabled {
            st.drag = None;
            st.finger = None;
        }
        let visible = |p: Point| viewport.contains(p);
        match event {
            Event::Window(window::Event::RedrawRequested(at)) => {
                let mut more = false;
                for t in &mut st.thumbs {
                    more |= t.advance(*at);
                }
                if more {
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let Some(i) = st.drag
                    && st.finger.is_none()
                {
                    shell.publish(self.drag_to(b, i, position.x));
                }
                let hover = if self.disabled || !visible(*position) {
                    None
                } else {
                    st.drag.or_else(|| self.thumb_at(b, *position))
                };
                for (i, t) in st.thumbs.iter_mut().enumerate() {
                    if t.set(hover == Some(i), now) {
                        shell.request_redraw();
                    }
                }
            }
            Event::Mouse(mouse::Event::CursorLeft) => {
                for t in &mut st.thumbs {
                    if t.set(false, now) {
                        shell.request_redraw();
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(p) = cursor.position()
                    && visible(p)
                    && let Some((i, jump)) = self.press(b, p)
                {
                    st.drag = Some(i);
                    if let Some(m) = jump {
                        shell.publish(m);
                    }
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if st.drag.is_some() =>
            {
                st.drag = None;
                if st.finger.is_none() {
                    shell.capture_event();
                }
            }
            Event::Touch(touch::Event::FingerPressed { id, position })
                if st.finger.is_none() && visible(*position) =>
            {
                if let Some((i, jump)) = self.press(b, *position) {
                    st.drag = Some(i);
                    st.finger = Some(id.0);
                    if let Some(m) = jump {
                        shell.publish(m);
                    }
                    shell.capture_event();
                }
            }
            Event::Touch(touch::Event::FingerMoved { id, position }) if st.finger == Some(id.0) => {
                if let Some(i) = st.drag {
                    shell.publish(self.drag_to(b, i, position.x));
                }
                shell.capture_event();
            }
            Event::Touch(
                touch::Event::FingerLifted { id, .. } | touch::Event::FingerLost { id, .. },
            ) if st.finger == Some(id.0) => {
                st.finger = None;
                st.drag = None;
                shell.capture_event();
            }
            _ => {}
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _v: &Rectangle,
        _r: &iced::Renderer,
    ) -> mouse::Interaction {
        let st = tree.state.downcast_ref::<SliderState>();
        if st.drag.is_some() {
            return mouse::Interaction::Grabbing;
        }
        let Some(p) = cursor.position() else {
            return mouse::Interaction::None;
        };
        let b = crate::scale::snap_rect(layout.bounds());
        if self.disabled {
            return mouse::Interaction::None;
        }
        if self.thumb_at(b, p).is_some() {
            mouse::Interaction::Grab
        } else if !self.range && self.on_track(b, p) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
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
        _viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_ref::<SliderState>();
        let p = self.cx.p();
        let b = crate::scale::snap_rect(layout.bounds());
        let track = self.track(b);
        let pill = |renderer: &mut iced::Renderer, r: Rectangle, color: Color| {
            renderer.fill_quad(
                Quad {
                    bounds: r,
                    border: Border {
                        radius: (TRACK / 2.0).into(),
                        ..Border::default()
                    },
                    ..Quad::default()
                },
                fade(color),
            );
        };
        pill(
            renderer,
            track,
            c(if self.range { p.border } else { p.control }),
        );
        let (f0, f1) = if self.range {
            (
                frac(self.values[0], self.min, self.max),
                frac(self.values[1], self.min, self.max),
            )
        } else {
            (0.0, frac(self.values[0], self.min, self.max))
        };
        if f1 > f0 {
            pill(
                renderer,
                Rectangle {
                    x: track.x + track.width * f0,
                    width: track.width * (f1 - f0),
                    ..track
                },
                c(p.accent),
            );
        }
        if self.marks && !self.range && self.step > 0.0 {
            let n = ((self.max - self.min) / self.step).floor() as usize + 1;
            for i in 0..n {
                let x = track.x
                    + track.width * frac(self.min + i as f64 * self.step, self.min, self.max);
                renderer.fill_quad(
                    Quad {
                        bounds: Rectangle {
                            x: x - 1.0,
                            y: track.center_y() - 1.0,
                            width: 2.0,
                            height: 2.0,
                        },
                        border: Border {
                            radius: 1.0.into(),
                            ..Border::default()
                        },
                        ..Quad::default()
                    },
                    fade(c(p.muted)),
                );
            }
        }
        // `shadow`: 0 1px 3px 0 #000/.1, 0 1px 2px -1px #000/.1.
        let shadow = vec![
            Sh {
                x: 0.0,
                y: 1.0,
                blur: 3.0,
                spread: 0.0,
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.1),
            },
            Sh {
                x: 0.0,
                y: 1.0,
                blur: 2.0,
                spread: -1.0,
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.1),
            },
        ];
        for (i, v) in self.thumbs().iter().enumerate() {
            let scale = 1.0 + 0.1 * st.thumbs[i].t;
            let r = self.thumb_rect(b, *v);
            let d = THUMB * scale;
            let outer = Rectangle {
                x: r.center_x() - d / 2.0,
                y: r.center_y() - d / 2.0,
                width: d,
                height: d,
            };
            paint(
                renderer,
                outer,
                &BoxStyle::default()
                    .bg(c(p.bg))
                    .radius(d / 2.0)
                    .shadows(shadow.clone()),
            );
            let bw = 2.0 * scale;
            paint(
                renderer,
                Rectangle {
                    x: outer.x + bw,
                    y: outer.y + bw,
                    width: d - 2.0 * bw,
                    height: d - 2.0 * bw,
                },
                &BoxStyle::default().bg(c(p.accent)).radius(d / 2.0 - bw),
            );
        }
    }
}

impl<'a, M: Clone + 'a> From<Slider<'a, M>> for El<'a, M> {
    fn from(s: Slider<'a, M>) -> Self {
        if s.disabled && s.fade_self {
            opacity(0.5, Element::new(s)).into()
        } else {
            Element::new(s)
        }
    }
}

// ── VolumeControl ───────────────────────────────────────────────────────

/// VolumeControl's own state: muted (React's `useControllableState`) and
/// the level to restore.
#[derive(Debug, Clone, Default)]
pub struct Volume {
    pub muted: bool,
    last: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VolumeEvent {
    Change(f64),
    ToggleMute,
}

impl Volume {
    /// Apply an event to the state and `value` (`min..max` is the range).
    pub fn update(&mut self, ev: VolumeEvent, value: &mut f64, min: f64, max: f64) {
        match ev {
            VolumeEvent::Change(next) => {
                if next > min {
                    self.last = Some(next);
                    self.muted = false;
                }
                *value = next;
            }
            VolumeEvent::ToggleMute => {
                if self.muted {
                    self.muted = false;
                    *value = self.last.unwrap_or(max);
                } else {
                    // `useRef(value || max)`: the first level is the initial one.
                    let last = self
                        .last
                        .get_or_insert(if *value != 0.0 { *value } else { max });
                    if *value > min {
                        *last = *value;
                    }
                    self.muted = true;
                    *value = min;
                }
            }
        }
    }
}

/// The icon for a level.
pub fn volume_icon(effective: f64, min: f64, max: f64) -> Ico {
    if effective <= min {
        Ico::VolumeX
    } else if (effective - min) / (max - min) < 0.5 {
        Ico::Volume1
    } else {
        Ico::Volume2
    }
}

pub struct VolumeControl<'a, M> {
    cx: Ctx,
    muted: bool,
    value: f64,
    on: std::rc::Rc<dyn Fn(VolumeEvent) -> M + 'a>,
    min: f64,
    max: f64,
    step: f64,
    disabled: bool,
    show_value: bool,
}

pub fn volume_control<'a, M: Clone + 'a>(
    cx: &Ctx,
    state: &Volume,
    value: f64,
    on: impl Fn(VolumeEvent) -> M + 'a,
) -> VolumeControl<'a, M> {
    VolumeControl {
        cx: cx.clone(),
        muted: state.muted,
        value,
        on: std::rc::Rc::new(on),
        min: 0.0,
        max: 100.0,
        step: 1.0,
        disabled: false,
        show_value: false,
    }
}

impl<'a, M: Clone + 'a> VolumeControl<'a, M> {
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min = min;
        self.max = max;
        self
    }
    pub fn step(mut self, s: f64) -> Self {
        self.step = s;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    /// The number after the slider.
    pub fn show_value(mut self, on: bool) -> Self {
        self.show_value = on;
        self
    }
}

impl<'a, M: Clone + 'a> From<VolumeControl<'a, M>> for El<'a, M> {
    fn from(v: VolumeControl<'a, M>) -> Self {
        let cx = v.cx.clone();
        let p = cx.p().clone();
        let effective = if v.muted { v.min } else { v.value };
        let ico = volume_icon(effective, v.min, v.max);
        let (muted, disabled) = (v.muted, v.disabled);
        let st_cx = cx.clone();
        let button = surface(icon(ico, 18.0))
            .width(32)
            .height(32)
            .center()
            .on_press_maybe((!disabled).then(|| (v.on)(VolumeEvent::ToggleMute)))
            .focus_ring(Ring::plain(&cx))
            .style(move |s| {
                let p = st_cx.p();
                let hover = s == Status::Hover && !disabled;
                let fg = if hover {
                    p.text
                } else if muted && !disabled {
                    p.accent_bright
                } else {
                    p.subtext
                };
                let mut b = BoxStyle::default().fg(c(fg)).radius(6.0);
                if hover {
                    b = b.bg(c(st_cx.wash(0.06)));
                }
                b
            });
        let on = v.on.clone();
        let mut track = slider(&cx, effective, move |x| on(VolumeEvent::Change(x)))
            .range(v.min, v.max)
            .step(v.step)
            .disabled(disabled);
        // The slider fades with the row, not twice.
        track.fade_self = false;
        let mut items: Vec<El<'a, M>> = vec![button.into(), track.into()];
        if v.show_value {
            items.push(
                container(
                    label(format!("{}", effective.round()), 12.0, 16.0)
                        .tabular()
                        .align(Align::Right)
                        .width(Length::Fill)
                        .color(c(p.subtext)),
                )
                .width(36)
                .into(),
            );
        }
        let r = row(items).spacing(12).align_y(Alignment::Center);
        if disabled {
            opacity(0.5, r).into()
        } else {
            r.into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_value_snaps_and_clamps() {
        assert_eq!(value_at(50.0, 0.0, 100.0, 0.0, 100.0, 1.0), 50.0);
        assert_eq!(value_at(-20.0, 0.0, 100.0, 0.0, 100.0, 1.0), 0.0);
        assert_eq!(value_at(120.0, 0.0, 100.0, 0.0, 100.0, 1.0), 100.0);
        assert_eq!(value_at(33.4, 0.0, 100.0, 0.0, 100.0, 5.0), 35.0);
        assert_eq!(frac(60.0, 0.0, 100.0), 0.6);
    }

    #[test]
    fn range_thumbs_never_cross() {
        assert_eq!(move_thumb([20.0, 80.0], 0, 90.0), [80.0, 80.0]);
        assert_eq!(move_thumb([20.0, 80.0], 1, 10.0), [20.0, 20.0]);
        assert_eq!(move_thumb([20.0, 80.0], 1, 95.0), [20.0, 95.0]);
    }

    #[test]
    fn volume_mute_remembers_the_level() {
        let mut s = Volume::default();
        let mut v = 40.0;
        s.update(VolumeEvent::ToggleMute, &mut v, 0.0, 100.0);
        assert!(s.muted);
        assert_eq!(v, 0.0);
        s.update(VolumeEvent::ToggleMute, &mut v, 0.0, 100.0);
        assert!(!s.muted);
        assert_eq!(v, 40.0);
        s.update(VolumeEvent::ToggleMute, &mut v, 0.0, 100.0);
        s.update(VolumeEvent::Change(70.0), &mut v, 0.0, 100.0);
        assert!(!s.muted, "dragging up unmutes");
        assert_eq!(v, 70.0);
        assert_eq!(volume_icon(0.0, 0.0, 100.0), Ico::VolumeX);
        assert_eq!(volume_icon(40.0, 0.0, 100.0), Ico::Volume1);
        assert_eq!(volume_icon(50.0, 0.0, 100.0), Ico::Volume2);
    }
}
