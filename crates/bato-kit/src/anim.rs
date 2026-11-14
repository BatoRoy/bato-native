//! Motion (INF-9): CSS easings, a generic [`Tween`], infinite [`pulse`],
//! press-and-hold [`HoldRepeat`], and the group-opacity multiplier
//! ([`opacity`] widget).
//!
//! The idle rule: a widget asks for a frame (`shell.request_redraw()`) only
//! while one of these is running, and an infinite animation only while it is
//! on screen. Nothing ticks when nothing moves.
//!
//! **Group opacity.** iced has none. [`opacity`] wraps content and multiplies
//! the alpha of everything the kit draws inside it (`Surface`, `Label`,
//! `Icon`, `Spinner`, `Switch`, `Checkbox`) through a thread-local factor read
//! with [`current_opacity`]. Overlapping translucent children double-blend
//! (CSS composites the group once); iced built-ins (`text_input`) ignore it.

use std::cell::Cell;
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::{Color, Element, Event, Length, Rectangle, Size, Vector, mouse};

/// Tailwind's default transition easing `cubic-bezier(0.4, 0, 0.2, 1)`.
pub fn ease_standard(t: f32) -> f32 {
    cubic_bezier(0.4, 0.0, 0.2, 1.0, t)
}

/// `ease-out` = `cubic-bezier(0, 0, 0.58, 1)`.
pub fn ease_out(t: f32) -> f32 {
    cubic_bezier(0.0, 0.0, 0.58, 1.0, t)
}

/// `ease-in-out` = `cubic-bezier(0.42, 0, 0.58, 1)`.
pub fn ease_in_out(t: f32) -> f32 {
    cubic_bezier(0.42, 0.0, 0.58, 1.0, t)
}

pub fn linear(t: f32) -> f32 {
    t.clamp(0.0, 1.0)
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` at time `t` (Newton on x, then y).
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 || t >= 1.0 {
        return t;
    }
    let coord = |p1: f32, p2: f32, s: f32| {
        let inv = 1.0 - s;
        3.0 * inv * inv * s * p1 + 3.0 * inv * s * s * p2 + s * s * s
    };
    let deriv = |p1: f32, p2: f32, s: f32| {
        let inv = 1.0 - s;
        3.0 * inv * inv * p1 + 6.0 * inv * s * (p2 - p1) + 3.0 * s * s * (1.0 - p2)
    };
    let mut s = t;
    for _ in 0..8 {
        let err = coord(x1, x2, s) - t;
        if err.abs() < 1e-5 {
            break;
        }
        let d = deriv(x1, x2, s);
        if d.abs() < 1e-6 {
            break;
        }
        s = (s - err / d).clamp(0.0, 1.0);
    }
    coord(y1, y2, s)
}

/// A value moving from `from` to `to` over `duration` with an easing. Keep it
/// in widget state (or app state); call [`Tween::value`] when drawing and ask
/// for another frame while [`Tween::is_running`].
#[derive(Debug, Clone, Copy)]
pub struct Tween {
    pub from: f32,
    pub to: f32,
    pub start: Instant,
    pub duration: Duration,
    pub easing: fn(f32) -> f32,
}

impl Tween {
    pub fn new(from: f32, to: f32, duration: Duration, easing: fn(f32) -> f32) -> Self {
        Self { from, to, start: Instant::now(), duration, easing }
    }

    /// A tween that has already finished at `v`.
    pub fn settled(v: f32) -> Self {
        Self { from: v, to: v, start: Instant::now(), duration: Duration::ZERO, easing: linear }
    }

    /// Progress 0..1 at `now` (before easing).
    pub fn progress(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        (now.saturating_duration_since(self.start).as_secs_f32() / self.duration.as_secs_f32()).min(1.0)
    }

    pub fn value(&self, now: Instant) -> f32 {
        let f = self.progress(now);
        if f >= 1.0 { self.to } else { self.from + (self.to - self.from) * (self.easing)(f) }
    }

    pub fn is_running(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
    }

    /// Retarget from the current value, keeping the motion continuous.
    pub fn retarget(&mut self, to: f32, now: Instant) {
        self.from = self.value(now);
        self.to = to;
        self.start = now;
    }
}

/// Tailwind `animate-pulse`: opacity 1 → .5 → 1 every 2 s,
/// `cubic-bezier(0.4, 0, 0.6, 1)`. `t` is the time since an epoch.
pub fn pulse(t: Duration) -> f32 {
    let phase = (t.as_secs_f32() % 2.0) / 2.0;
    // keyframes: 0% / 100% opacity 1, 50% opacity .5
    let (a, f) = if phase < 0.5 { (1.0, phase * 2.0) } else { (0.5, (phase - 0.5) * 2.0) };
    let e = cubic_bezier(0.4, 0.0, 0.6, 1.0, f);
    if a == 1.0 { 1.0 - 0.5 * e } else { 0.5 + 0.5 * e }
}

/// A process-wide epoch for infinite animations (spin, pulse), so every
/// instance runs in phase like CSS animations started together.
pub fn epoch() -> Instant {
    static T: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *T.get_or_init(Instant::now)
}

/// `useHoldRepeat`: fire on press, again after 400 ms, then every 60 ms,
/// accelerating to 20 ms. Keep it in widget state; on each redraw call
/// [`HoldRepeat::due`] and publish once per returned step, then
/// `shell.request_redraw_at(next)`.
#[derive(Debug, Clone, Copy)]
pub struct HoldRepeat {
    next: Instant,
    interval: Duration,
}

impl HoldRepeat {
    pub const DELAY: Duration = Duration::from_millis(400);
    pub const START: Duration = Duration::from_millis(60);
    pub const MIN: Duration = Duration::from_millis(20);

    /// Pressed at `now` (the caller fires the first step itself).
    pub fn press(now: Instant) -> Self {
        Self { next: now + Self::DELAY, interval: Self::START }
    }

    /// Steps due by `now`, and the deadline of the next one.
    pub fn due(&mut self, now: Instant) -> (u32, Instant) {
        let mut n = 0;
        while now >= self.next {
            n += 1;
            self.next += self.interval;
            self.interval = Duration::from_secs_f32((self.interval.as_secs_f32() * 0.9).max(Self::MIN.as_secs_f32()));
        }
        (n, self.next)
    }
}

thread_local! {
    static OPACITY: Cell<f32> = const { Cell::new(1.0) };
}

/// The group-opacity factor in effect for the current draw.
pub fn current_opacity() -> f32 {
    OPACITY.with(Cell::get)
}

/// `c` with its alpha multiplied by the current group opacity.
pub fn fade(c: Color) -> Color {
    let o = current_opacity();
    if o >= 1.0 { c } else { Color { a: c.a * o, ..c } }
}

/// Run `f` with the group opacity multiplied by `alpha`.
pub fn with_opacity<R>(alpha: f32, f: impl FnOnce() -> R) -> R {
    let prev = current_opacity();
    OPACITY.with(|o| o.set(prev * alpha.clamp(0.0, 1.0)));
    let r = f();
    OPACITY.with(|o| o.set(prev));
    r
}

/// CSS `opacity` on a group (disabled controls `opacity-50`, fades).
pub struct Opacity<'a, Message> {
    content: Element<'a, Message, iced::Theme, iced::Renderer>,
    alpha: f32,
}

pub fn opacity<'a, Message>(alpha: f32, content: impl Into<Element<'a, Message, iced::Theme, iced::Renderer>>) -> Opacity<'a, Message> {
    Opacity { content: content.into(), alpha }
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for Opacity<'a, Message> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        let n = self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits);
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
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout.children().next().unwrap(), cursor, renderer, clipboard, shell, viewport);
    }
    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &iced::Renderer) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout.children().next().unwrap(), cursor, viewport, renderer)
    }
    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer, operation: &mut dyn widget::Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout.children().next().unwrap(), renderer, operation);
    }
    fn draw(&self, tree: &Tree, renderer: &mut iced::Renderer, theme: &iced::Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        let style = renderer::Style { text_color: Color { a: style.text_color.a * self.alpha, ..style.text_color } };
        with_opacity(self.alpha, || {
            self.content.as_widget().draw(&tree.children[0], renderer, theme, &style, layout.children().next().unwrap(), cursor, viewport);
        });
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout.children().next().unwrap(), renderer, viewport, translation)
    }
}

impl<'a, Message: 'a> From<Opacity<'a, Message>> for Element<'a, Message, iced::Theme, iced::Renderer> {
    fn from(o: Opacity<'a, Message>) -> Self {
        Element::new(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_endpoints_and_midpoint() {
        assert_eq!(ease_standard(0.0), 0.0);
        assert_eq!(ease_standard(1.0), 1.0);
        assert!((ease_standard(0.5) - 0.8).abs() < 0.02);
        assert!(ease_out(0.5) > 0.5);
    }

    #[test]
    fn tween_runs_then_settles() {
        let mut t = Tween::new(0.0, 10.0, Duration::from_millis(200), linear);
        let s = t.start;
        assert_eq!(t.value(s + Duration::from_millis(100)), 5.0);
        assert!(t.is_running(s + Duration::from_millis(100)));
        assert_eq!(t.value(s + Duration::from_millis(300)), 10.0);
        assert!(!t.is_running(s + Duration::from_millis(300)));
        t.retarget(0.0, s + Duration::from_millis(100));
        assert_eq!(t.from, 5.0);
        assert!(!Tween::settled(3.0).is_running(Instant::now()));
    }

    #[test]
    fn pulse_keyframes() {
        assert_eq!(pulse(Duration::ZERO), 1.0);
        assert!((pulse(Duration::from_secs(1)) - 0.5).abs() < 1e-4);
        assert!((pulse(Duration::from_secs(2)) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn hold_repeat_delay_then_accelerates() {
        let t0 = Instant::now();
        let mut h = HoldRepeat::press(t0);
        assert_eq!(h.due(t0 + Duration::from_millis(399)).0, 0);
        assert_eq!(h.due(t0 + Duration::from_millis(400)).0, 1);
        let (n, _) = h.due(t0 + Duration::from_millis(1400));
        assert!(n > 1000 / 60, "accelerates: {n}");
    }

    #[test]
    fn opacity_nests() {
        assert_eq!(current_opacity(), 1.0);
        with_opacity(0.5, || {
            with_opacity(0.5, || assert_eq!(current_opacity(), 0.25));
            assert_eq!(fade(Color::BLACK).a, 0.5);
        });
        assert_eq!(current_opacity(), 1.0);
    }
}
