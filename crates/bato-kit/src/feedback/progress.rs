//! `Progress` (a 6 px bar) and `CircularProgress` (an SVG ring), both with
//! React's 300 ms transition when the value changes (`transition-[width]` /
//! `transition-[stroke-dashoffset]`, Tailwind's standard easing).
//!
//! The ring is the same SVG React renders (two circles, the second dashed
//! and turned −90°), rasterized by resvg like the icons; a handle is cached
//! per size, thickness and tenth of a percent.
//!
//! ```ignore
//! progress(&cx, 64.0)                       // <Progress value={64} />
//! progress(&cx, 0.0).indeterminate()        // 40 % wide, pulsing
//! circular_progress(&cx, 64.0).label()      // <CircularProgress value={64} showLabel />
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::svg::{self, Renderer as _};
use iced::advanced::widget::{Tree, Widget, tree};
use iced::widget::{container, stack};
use iced::{Border, Color, Element, Length, Radians, Rectangle, Size, mouse};

use crate::El;
use crate::anim::{Tween, current_opacity, ease_standard, epoch, fade, pulse};
use crate::ctx::Ctx;
use crate::label::label;
use crate::scale::snap_rect;
use crate::style::{c, lerp};

/// React's `transition duration-300`.
pub const TRANSITION: Duration = Duration::from_millis(300);

/// `Math.min(100, Math.max(0, value))`.
pub fn clamp_pct(v: f32) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(0.0, 100.0) }
}

/// A value that eases to each new target over [`TRANSITION`].
#[derive(Debug, Clone, Copy)]
struct Motion {
    tween: Tween,
}

impl Motion {
    fn new(v: f32) -> Self {
        Self {
            tween: Tween::settled(v),
        }
    }
    /// Retarget when the value changed (called from `diff`).
    fn set(&mut self, v: f32, now: Instant) {
        if (self.tween.to - v).abs() > f32::EPSILON {
            let from = self.tween.value(now);
            self.tween = Tween::new(from, v, TRANSITION, ease_standard);
            self.tween.start = now;
        }
    }
    fn value(&self, now: Instant) -> f32 {
        self.tween.value(now)
    }
    fn running(&self, now: Instant) -> bool {
        self.tween.is_running(now)
    }
}

fn redraw_while_running<M>(
    m: &Motion,
    event: &iced::Event,
    shell: &mut iced::advanced::Shell<'_, M>,
) {
    if let iced::Event::Window(iced::window::Event::RedrawRequested(now)) = event
        && m.running(*now)
    {
        shell.request_redraw();
    }
}

// ── Progress ────────────────────────────────────────────────────────────

pub struct Progress {
    value: f32,
    indeterminate: bool,
    track: Color,
    fill: Color,
    height: f32,
    width: Length,
}

/// `<Progress value>`: full width, `h-1.5`, `bg-app-border` track,
/// `bg-app-accent` fill.
pub fn progress(cx: &Ctx, value: f32) -> Progress {
    let p = cx.p();
    Progress {
        value: clamp_pct(value),
        indeterminate: false,
        track: c(p.border),
        fill: c(p.accent),
        height: 6.0,
        width: Length::Fill,
    }
}

impl Progress {
    /// 40 % wide and pulsing (`animate-pulse`), for an unknown value.
    pub fn indeterminate(mut self) -> Self {
        self.indeterminate = true;
        self
    }
    pub fn width(mut self, w: impl Into<Length>) -> Self {
        self.width = w.into();
        self
    }
    /// The bar height (default 6, `h-1.5`).
    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }
    fn target(&self) -> f32 {
        if self.indeterminate { 40.0 } else { self.value }
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Progress {
    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Fixed(self.height))
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Motion>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(Motion::new(self.target()))
    }
    fn diff(&self, tree: &mut Tree) {
        tree.state
            .downcast_mut::<Motion>()
            .set(self.target(), Instant::now());
    }
    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let l = limits.width(self.width).height(self.height);
        layout::Node::new(l.resolve(self.width, self.height, Size::ZERO))
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced::Event,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if self.indeterminate
            && matches!(
                event,
                iced::Event::Window(iced::window::Event::RedrawRequested(_))
            )
            && layout.bounds().intersects(viewport)
        {
            shell.request_redraw();
        }
        redraw_while_running(tree.state.downcast_ref::<Motion>(), event, shell);
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
        let b = snap_rect(layout.bounds());
        let r = self.height / 2.0;
        renderer.fill_quad(
            Quad {
                bounds: b,
                border: Border {
                    radius: r.into(),
                    ..Border::default()
                },
                ..Quad::default()
            },
            fade(self.track),
        );
        let pct = tree.state.downcast_ref::<Motion>().value(Instant::now());
        let w = b.width * pct / 100.0;
        if w <= 0.0 {
            return;
        }
        let fill = if self.indeterminate {
            lerp(self.track, self.fill, pulse(epoch().elapsed()))
        } else {
            self.fill
        };
        renderer.with_layer(b, |renderer| {
            renderer.fill_quad(
                Quad {
                    bounds: Rectangle { width: w, ..b },
                    border: Border {
                        radius: r.into(),
                        ..Border::default()
                    },
                    ..Quad::default()
                },
                fade(fill),
            );
        });
    }
}

impl<'a, Message: 'a> From<Progress> for Element<'a, Message, iced::Theme, iced::Renderer> {
    fn from(p: Progress) -> Self {
        Element::new(p)
    }
}

// ── CircularProgress ────────────────────────────────────────────────────

/// React's two circles: the track, and (`pct` > 0) the dashed arc from the
/// top, clockwise. Black strokes; the colour is applied when drawn.
pub fn ring_svg(size: f32, thickness: f32, pct: Option<f32>) -> String {
    let r = (size - thickness) / 2.0;
    let h = size / 2.0;
    let body = match pct {
        None => format!(
            r##"<circle cx="{h}" cy="{h}" r="{r}" fill="none" stroke="#000000" stroke-width="{thickness}"/>"##
        ),
        Some(p) => {
            let circ = 2.0 * std::f32::consts::PI * r;
            let off = circ - p / 100.0 * circ;
            format!(
                r##"<circle cx="{h}" cy="{h}" r="{r}" fill="none" stroke="#000000" stroke-width="{thickness}" stroke-linecap="round" stroke-dasharray="{circ}" stroke-dashoffset="{off}" transform="rotate(-90 {h} {h})"/>"##
            )
        }
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}">{body}</svg>"##
    )
}

type RingKey = (u32, u32, Option<i32>);

thread_local! {
    static RINGS: RefCell<HashMap<RingKey, svg::Handle>> = RefCell::new(HashMap::new());
}

fn ring_handle(size: f32, thickness: f32, pct: Option<f32>) -> svg::Handle {
    // Tenths of a percent: smooth enough for the transition, few handles.
    let q = pct.map(|p| (p * 10.0).round() as i32);
    RINGS.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() > 256 {
            m.clear();
        }
        m.entry((size.to_bits(), thickness.to_bits(), q))
            .or_insert_with(|| {
                let p = q.map(|q| q as f32 / 10.0);
                svg::Handle::from_memory(ring_svg(size, thickness, p).into_bytes())
            })
            .clone()
    })
}

struct Ring {
    value: f32,
    size: f32,
    thickness: f32,
    track: Color,
    arc: Color,
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Ring {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.size), Length::Fixed(self.size))
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Motion>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(Motion::new(self.value))
    }
    fn diff(&self, tree: &mut Tree) {
        tree.state
            .downcast_mut::<Motion>()
            .set(self.value, Instant::now());
    }
    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(Size::new(self.size, self.size))
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced::Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        redraw_while_running(tree.state.downcast_ref::<Motion>(), event, shell);
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
        let b = snap_rect(layout.bounds());
        let o = current_opacity();
        let mut draw = |handle: svg::Handle, color: Color| {
            renderer.draw_svg(
                svg::Svg {
                    handle,
                    color: Some(Color { a: 1.0, ..color }),
                    rotation: Radians(0.0),
                    opacity: color.a * o,
                },
                b,
                viewport.expand(2.0),
            );
        };
        draw(ring_handle(self.size, self.thickness, None), self.track);
        let pct = tree.state.downcast_ref::<Motion>().value(Instant::now());
        if pct > 0.0 {
            draw(ring_handle(self.size, self.thickness, Some(pct)), self.arc);
        }
    }
}

pub struct CircularProgress {
    cx: Ctx,
    value: f32,
    size: f32,
    thickness: f32,
    label: bool,
}

/// `<CircularProgress value>`: 48 px, 4 px stroke, `stroke-app-border`
/// track, `stroke-app-accent` arc.
pub fn circular_progress(cx: &Ctx, value: f32) -> CircularProgress {
    CircularProgress {
        cx: cx.clone(),
        value: clamp_pct(value),
        size: 48.0,
        thickness: 4.0,
        label: false,
    }
}

impl CircularProgress {
    pub fn size(mut self, px: f32) -> Self {
        self.size = px;
        self
    }
    pub fn thickness(mut self, px: f32) -> Self {
        self.thickness = px;
        self
    }
    /// `showLabel`: the rounded percentage in the centre (12/16 medium).
    pub fn label(mut self) -> Self {
        self.label = true;
        self
    }
}

impl<'a, Message: 'a> From<CircularProgress> for El<'a, Message> {
    fn from(cp: CircularProgress) -> Self {
        let p = cp.cx.p();
        let ring = Element::new(Ring {
            value: cp.value,
            size: cp.size,
            thickness: cp.thickness,
            track: c(p.border),
            arc: c(p.accent),
        });
        if !cp.label {
            return ring;
        }
        let text = label(format!("{}%", cp.value.round() as i32), 12.0, 16.0)
            .medium()
            .color(c(p.text));
        stack![ring, container(text).center_x(cp.size).center_y(cp.size)].into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_like_react() {
        assert_eq!(clamp_pct(-5.0), 0.0);
        assert_eq!(clamp_pct(150.0), 100.0);
        assert_eq!(clamp_pct(f32::NAN), 0.0);
        assert_eq!(clamp_pct(64.0), 64.0);
    }

    #[test]
    fn ring_dash_matches_react() {
        // size 48, thickness 4: r = 22, c = 138.23; 64 % → offset 49.76.
        let s = ring_svg(48.0, 4.0, Some(64.0));
        assert!(s.contains(r#"r="22""#), "{s}");
        assert!(s.contains("stroke-dasharray=\"138.2"), "{s}");
        assert!(s.contains("stroke-dashoffset=\"49.76"), "{s}");
        assert!(s.contains("rotate(-90 24 24)"));
        assert!(!ring_svg(48.0, 4.0, None).contains("dasharray"));
    }

    #[test]
    fn motion_eases_to_new_targets() {
        let mut m = Motion::new(10.0);
        let t0 = Instant::now();
        assert!(!m.running(t0));
        m.set(10.0, t0);
        assert!(!m.running(t0), "same value: no transition");
        m.set(60.0, t0);
        assert!(m.running(t0 + Duration::from_millis(150)));
        let mid = m.value(t0 + Duration::from_millis(150));
        assert!(
            mid > 35.0 && mid < 60.0,
            "standard easing is ahead at half time: {mid}"
        );
        assert_eq!(m.value(t0 + TRANSITION), 60.0);
    }
}
