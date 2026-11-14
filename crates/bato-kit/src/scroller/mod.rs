//! `Scroller` (INF-7): every scroll box, from the page to a bounded list, a
//! horizontal strip or a two-axis grid. All motion comes from
//! `scroll_physics::ScrollAxis` (Chromium's wheel animation, 1:1 touchpad,
//! fling), one per axis; iced's `scrollable` is not used, so there is no
//! second smoothing layer on top.
//!
//! ```ignore
//! scroller(page, thumb, thumb_hover)                     // the page: fills, vertical
//! scroll_area(&cx, list).max_height(150.0).faded()       // React <Scrollable maxHeight fade>
//! scroll_area(&cx, strip).horizontal()                   // overflow-x
//! scroll_area(&cx, grid).both().height(240.0).id(GRID)   // overflow: auto
//! scroller::scroll_to(ROW_42, Align::Nearest)            // smooth scroll-into-view
//! ```
//!
//! **Nesting** follows Chromium's scroll latching ([`chain`] has the rules):
//! a wheel, touchpad or touch gesture goes to the innermost scroller under
//! the pointer that can move in its direction, and stays there until it
//! ends, even at that scroller's edge; the next gesture chains to the
//! parent. Shift+wheel scrolls horizontally. Middle-click autoscroll picks
//! the innermost scroller that can scroll at all. The keys (arrows 40 px,
//! PageUp/PageDown/Space 87.5 %, Home/End) scroll the scroller of the last
//! click and bubble to its parents at an edge.
//!
//! **Look**: the classic Chromium scrollbar of `index.css` per axis (5 px,
//! `border` thumb, `muted` on hover, takes layout space, no auto-hide), and
//! `Scrollable`'s 16 px edge fade, painted in the backdrop colour over the
//! content and the scrollbar (iced has no alpha masks).
//!
//! Input path on Wayland (iced 0.14 + iced's winit fork, see README):
//! - wheel: `axis_value120` → winit `LineDelta(v120 / 120)` → iced
//!   `ScrollDelta::Lines` → [`ScrollAxis::wheel_notches`];
//! - touchpad / continuous: `axis` → `PixelDelta` → `ScrollDelta::Pixels` →
//!   × [`CHROMIUM_TOUCHPAD_GAIN`] → [`ScrollAxis::precise`];
//! - fingers lifted: `axis_stop` → winit `PixelDelta(0, 0)` with
//!   `TouchPhase::Ended`. iced_winit drops the phase
//!   (`iced_winit/src/conversion.rs`, `WindowEvent::MouseWheel { delta, .. }`)
//!   but forwards the zero delta, so a zero pixel delta is the gesture end →
//!   [`ScrollAxis::precise_end`].
//!
//! The axes work in device pixels (what the compositor reports), so one
//! notch is 120 screen px at every UI scale, like Chromium's page zoom.

pub mod chain;
pub mod geom;
pub mod touch;
pub mod virtual_list;

use std::time::Instant;

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::touch as itouch;
use iced::{
    Border, Color, Element, Event, Length, Point, Rectangle, Size, Vector, keyboard, mouse, window,
};
use scroll_physics::{ScrollAxis, TouchOutcome};

use crate::ctx::Ctx;
use crate::scale::device_scale;
use crate::style::c;

pub use geom::{Align, Axis, BAR, FADE};
pub use touch::{Axes, Forward, Phase, TouchRouter};
pub use virtual_list::{Heights, VirtualList, virtual_list};

use chain::{Kind, Request, Target};
use touch::{finger_of, moved_to};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollInput {
    Notches(f32),
    Pixels(f32),
    End,
    Ignore,
}

/// Map an iced wheel delta to a vertical physics input. Positive = content
/// moves up (offset grows), i.e. wheel turned toward the user.
pub fn classify(delta: mouse::ScrollDelta) -> ScrollInput {
    match delta {
        mouse::ScrollDelta::Lines { y, .. } if y != 0.0 => ScrollInput::Notches(-y),
        mouse::ScrollDelta::Lines { .. } => ScrollInput::Ignore,
        mouse::ScrollDelta::Pixels { x, y } if x == 0.0 && y == 0.0 => ScrollInput::End,
        mouse::ScrollDelta::Pixels { y, .. } if y != 0.0 => ScrollInput::Pixels(-y),
        mouse::ScrollDelta::Pixels { .. } => ScrollInput::Ignore,
    }
}

/// Both axes of a wheel delta, `(x, y)`. Shift turns a vertical wheel into
/// a horizontal one (Chromium on Linux); positive = content moves left/up.
pub fn classify_xy(delta: mouse::ScrollDelta, shift: bool) -> (ScrollInput, ScrollInput) {
    let notch = |v: f32| {
        if v != 0.0 {
            ScrollInput::Notches(-v)
        } else {
            ScrollInput::Ignore
        }
    };
    let px = |v: f32| {
        if v != 0.0 {
            ScrollInput::Pixels(-v)
        } else {
            ScrollInput::Ignore
        }
    };
    match delta {
        mouse::ScrollDelta::Lines { x, y } if shift && x == 0.0 => (notch(y), ScrollInput::Ignore),
        mouse::ScrollDelta::Lines { x, y } => (notch(x), notch(y)),
        mouse::ScrollDelta::Pixels { x, y } if x == 0.0 && y == 0.0 => {
            (ScrollInput::End, ScrollInput::End)
        }
        mouse::ScrollDelta::Pixels { x, y } => (px(x), px(y)),
    }
}

fn amount(i: ScrollInput) -> f32 {
    match i {
        ScrollInput::Notches(n) => n,
        ScrollInput::Pixels(p) => p,
        _ => 0.0,
    }
}

/// Chromium on Wayland scales a touchpad `axis` unit by `kWheelDelta / 10`
/// = 12 px (`ui/ozone/platform/wayland/host/wayland_pointer.cc`, unless the
/// off-by-default `WaylandUnscaledTouchpadScrolling` feature is on). winit's
/// `PixelDelta` is the raw axis value, so without this gain a touchpad moves
/// 12x slower than in Electron. `BATO_TOUCHPAD_GAIN` overrides it (same knob
/// as the GPUI PoC).
pub const CHROMIUM_TOUCHPAD_GAIN: f32 = 12.0;

fn touchpad_gain() -> f32 {
    static GAIN: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *GAIN.get_or_init(|| {
        std::env::var("BATO_TOUCHPAD_GAIN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(CHROMIUM_TOUCHPAD_GAIN)
    })
}

fn debug_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BATO_SCROLL_DEBUG").is_some_and(|v| v != "0"))
}

/// `BATO_DRAW_DEBUG=1`: log every frame drawn (to verify idle = no frames).
fn draw_debug() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BATO_DRAW_DEBUG").is_some_and(|v| v != "0"))
}

// ── Builder ─────────────────────────────────────────────────────────────

pub struct Scroller<'a, Message> {
    content: Element<'a, Message, iced::Theme, iced::Renderer>,
    autoscroll: Option<crate::autoscroll::AutoscrollBus>,
    thumb: Color,
    thumb_hover: Color,
    initial: Option<f32>,
    anchor: Option<widget::Id>,
    axis: Axis,
    width: Length,
    height: Length,
    max_width: Option<f32>,
    max_height: Option<f32>,
    backdrop: Option<Color>,
    fade: bool,
    border: Option<(Color, f32)>,
    id: Option<widget::Id>,
}

/// The page scroller: fills its space, scrolls vertically.
pub fn scroller<'a, Message>(
    content: impl Into<Element<'a, Message, iced::Theme, iced::Renderer>>,
    thumb: Color,
    thumb_hover: Color,
) -> Scroller<'a, Message> {
    Scroller {
        content: content.into(),
        autoscroll: None,
        thumb,
        thumb_hover,
        initial: None,
        anchor: None,
        axis: Axis::Vertical,
        width: Length::Fill,
        height: Length::Fill,
        max_width: None,
        max_height: None,
        backdrop: None,
        fade: false,
        border: None,
        id: None,
    }
}

/// A scroll box inside content (React `<Scrollable>`): as wide as its
/// container, as tall as its content up to [`Scroller::max_height`], with
/// the theme's scrollbar, on `cx`'s backdrop.
pub fn scroll_area<'a, Message>(
    cx: &Ctx,
    content: impl Into<Element<'a, Message, iced::Theme, iced::Renderer>>,
) -> Scroller<'a, Message> {
    let p = cx.p();
    let mut s = scroller(content, c(p.border), c(p.muted));
    s.height = Length::Shrink;
    s.backdrop = Some(c(cx.over));
    s
}

impl<'a, Message> Scroller<'a, Message> {
    /// This scroller owns the BatoScroll bus (the page does): it arms on a
    /// middle press no inner scroller took, and inner scrollers use its bus.
    pub fn autoscroll(mut self, bus: crate::autoscroll::AutoscrollBus) -> Self {
        self.autoscroll = Some(bus);
        self
    }

    /// Initial vertical offset in CSS px (BATO_VIEW `scroll=`), applied once.
    pub fn initial(mut self, px: Option<f32>) -> Self {
        self.initial = px;
        self
    }

    /// Initial position: the content's container with this id
    /// (`iced::widget::container(..).id(..)`) 24 px below the top of the
    /// viewport (BATO_VIEW `card=`). Applied once, at the first layout that
    /// contains it; wins over [`Scroller::initial`].
    pub fn initial_anchor(mut self, id: Option<widget::Id>) -> Self {
        self.anchor = id;
        self
    }

    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }
    /// `overflow-x: auto; overflow-y: hidden`.
    pub fn horizontal(self) -> Self {
        self.axis(Axis::Horizontal)
    }
    /// `overflow: auto`.
    pub fn both(self) -> Self {
        self.axis(Axis::Both)
    }
    pub fn width(mut self, w: impl Into<Length>) -> Self {
        self.width = w.into();
        self
    }
    pub fn height(mut self, h: impl Into<Length>) -> Self {
        self.height = h.into();
        self
    }
    /// CSS `max-width` (px).
    pub fn max_width(mut self, w: f32) -> Self {
        self.max_width = Some(w);
        self
    }
    /// CSS `max-height` (px): the box is as tall as its content up to this.
    pub fn max_height(mut self, h: f32) -> Self {
        self.max_height = Some(h);
        self
    }
    /// The opaque colour behind the box (the edge fade blends into it).
    pub fn backdrop(mut self, c: Color) -> Self {
        self.backdrop = Some(c);
        self
    }
    /// `Scrollable fade`: the first and last 16 px along the scroll axis fade
    /// out, always (needs a [`Scroller::backdrop`]; `scroll_area` sets it).
    pub fn faded(mut self) -> Self {
        self.fade = true;
        self
    }
    /// A 1 px border with this radius around the box (`border
    /// rounded-lg`); inside the fade, like a CSS mask covers the border.
    pub fn border(mut self, color: Color, radius: f32) -> Self {
        self.border = Some((color, radius));
        self
    }
    /// For [`scroll_offset`] and [`offsets_of`].
    pub fn id(mut self, id: impl Into<widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }
}

/// Gap kept above an anchored card (`card=`: "24 px below the top").
pub const ANCHOR_GAP: f32 = 24.0;

/// Finds the bounds of the container with a given id.
struct FindContainer<'a> {
    id: &'a widget::Id,
    found: Option<Rectangle>,
}

impl widget::Operation for FindContainer<'_> {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation)) {
        if self.found.is_none() {
            operate(self);
        }
    }
    fn container(&mut self, id: Option<&widget::Id>, bounds: Rectangle) {
        if self.found.is_none() && id == Some(self.id) {
            self.found = Some(bounds);
        }
    }
}

// ── Programmatic scrolling ──────────────────────────────────────────────

/// How [`reveal`] scrolls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reveal {
    /// Vertical alignment (CSS `block`).
    pub block: Align,
    /// Horizontal alignment (CSS `inline`).
    pub inline: Align,
    /// Animate with the wheel curve ([`ScrollAxis::animate_to`]) or jump.
    pub smooth: bool,
    /// Extra room kept around the target (CSS `scroll-margin`).
    pub margin: f32,
}

impl Default for Reveal {
    fn default() -> Self {
        Self {
            block: Align::Nearest,
            inline: Align::Nearest,
            smooth: true,
            margin: 0.0,
        }
    }
}

fn request(target: Target, how: Reveal, rect: Option<Rectangle>) {
    let stack = chain::stack();
    chain::push(Request {
        target,
        align: (how.inline, how.block),
        smooth: how.smooth,
        margin: how.margin,
        chain: (!stack.is_empty()).then_some(stack),
        rect,
        tries: 4,
    });
}

/// Scroll every scroller around `rect` (layout coordinates) so it shows,
/// inner first, like `Element.scrollIntoView`.
///
/// Called from a widget's `update`, exactly the scrollers around that widget
/// move (even when nested), during the same event; call
/// `shell.request_redraw()` too. Called from an app's `update`, the next
/// frame does it, and the scroller whose content contains `rect` is guessed:
/// prefer [`scroll_to`] with a container id there.
pub fn reveal(rect: Rectangle, how: Reveal) {
    let inside = !chain::stack().is_empty();
    request(Target::Rect(rect), how, inside.then_some(rect));
}

/// Smooth scroll-into-view of `rect` (see [`reveal`]).
pub fn scroll_into_view(rect: Rectangle, align: Align) {
    reveal(
        rect,
        Reveal {
            block: align,
            inline: align,
            ..Reveal::default()
        },
    );
}

/// Smooth scroll to the container with this id
/// (`iced::widget::container(..).id(id)`), through every scroller around it.
/// Call it from the app's `update` (or from a widget's `update`, with
/// `shell.request_redraw()`).
pub fn scroll_to(id: impl Into<widget::Id>, align: Align) {
    request(
        Target::Container(id.into()),
        Reveal {
            block: align,
            inline: align,
            ..Reveal::default()
        },
        None,
    );
}

/// [`scroll_to`] without the animation (`scrollIntoView()` with the default
/// `behavior: auto`, e.g. the command palette's active row).
pub fn jump_to(id: impl Into<widget::Id>, align: Align) {
    request(
        Target::Container(id.into()),
        Reveal {
            block: align,
            inline: align,
            smooth: false,
            margin: 0.0,
        },
        None,
    );
}

/// The focus hook: the focus system calls this from the newly focused
/// widget's `update` with its layout bounds (and `shell.request_redraw()`).
/// Like Chromium's focus scrolling it jumps (no animation) by the least
/// amount (`nearest`), through every scroller around the widget.
pub fn focus_into_view(rect: Rectangle) {
    reveal(
        rect,
        Reveal {
            smooth: false,
            ..Reveal::default()
        },
    );
}

/// Scroll the scroller with this [`Scroller::id`] to an offset (CSS px; an
/// axis left `None` stays).
pub fn scroll_offset(id: impl Into<widget::Id>, x: Option<f32>, y: Option<f32>, smooth: bool) {
    request(
        Target::Offset(id.into(), x, y),
        Reveal {
            smooth,
            ..Reveal::default()
        },
        None,
    );
}

/// The offsets (CSS px) the scroller with this id was last drawn at
/// (self-tests and debugging).
pub fn offsets_of(id: &widget::Id) -> Option<(f32, f32)> {
    chain::offsets_of(id)
}

/// The offsets (CSS px) the page scroller (the one with the autoscroll bus)
/// was last drawn at.
pub fn page_offset() -> Vector {
    let (x, y) = chain::page();
    Vector::new(x, y)
}

// ── State ───────────────────────────────────────────────────────────────

/// Where the content's hover comes from. Chromium's touch rules: while a
/// touch may still be a tap the content sees it at the finger; once it is a
/// scroll nothing is hovered (rows must not light up under the finger as the
/// content slides by); a tap leaves the hover on the tapped element until the
/// mouse moves again. iced itself always reports the finger as the cursor and
/// a tap alone triggers no redraw, so without this the highlight followed the
/// finger's screen position during a scroll and never appeared on a tap.
#[derive(Debug, Clone, Copy, PartialEq)]
enum HoverSource {
    Mouse,
    TouchNone,
    /// A finger is down and may still be a tap: touch events reach the
    /// content at the finger (so buttons arm and click), but nothing is drawn
    /// hovered — otherwise the row under the finger flashes when a scroll
    /// begins. Content coordinates.
    TouchPending(Point),
    /// After a tap: hovered like a parked mouse. Content coordinates.
    TouchAt(Point),
}

fn hover_after(current: HoverSource, outcome: TouchOutcome, at: Point) -> HoverSource {
    match outcome {
        TouchOutcome::Pending => HoverSource::TouchPending(at),
        TouchOutcome::Tap => HoverSource::TouchAt(at),
        TouchOutcome::ScrollStarted | TouchOutcome::Scrolling | TouchOutcome::Ended => {
            HoverSource::TouchNone
        }
        TouchOutcome::Ignored => current,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ThumbDrag {
    horizontal: bool,
    grab: f32,
    start: f32,
}

#[derive(Debug)]
struct State {
    id: u64,
    hover: HoverSource,
    x: ScrollAxis,
    y: ScrollAxis,
    initial_applied: bool,
    /// Content size and the inner viewport (minus scrollbars), logical px.
    content: Size,
    view: Size,
    /// Scrollbars shown: (horizontal, vertical).
    bars: (bool, bool),
    /// 1 px when the box has a border.
    inset: f32,
    /// Layouts left to look for the initial anchor.
    anchor_tries: u8,
    drag: Option<ThumbDrag>,
    thumb_hover: (bool, bool),
    touch: TouchRouter,
    /// The offsets the content last saw the mouse at (hover after scroll).
    seen: Vector,
    t0: Instant,
}

impl Default for State {
    fn default() -> Self {
        Self {
            id: chain::next_id(),
            hover: HoverSource::Mouse,
            x: ScrollAxis::default(),
            y: ScrollAxis::default(),
            initial_applied: false,
            content: Size::ZERO,
            view: Size::ZERO,
            bars: (false, false),
            inset: 0.0,
            anchor_tries: 3,
            drag: None,
            thumb_hover: (false, false),
            touch: TouchRouter::default(),
            seen: Vector::ZERO,
            t0: Instant::now(),
        }
    }
}

impl State {
    /// The cursor the content sees. `area` is where the real mouse cursor
    /// counts (the visible viewport, scrollbars excluded).
    fn content_cursor(
        &self,
        cursor: mouse::Cursor,
        area: Rectangle,
        offset: Vector,
    ) -> mouse::Cursor {
        match self.hover {
            HoverSource::TouchNone | HoverSource::TouchPending(_) => mouse::Cursor::Unavailable,
            HoverSource::TouchAt(p) => mouse::Cursor::Available(p),
            HoverSource::Mouse => match cursor {
                mouse::Cursor::Available(p) if area.contains(p) => {
                    mouse::Cursor::Available(p + offset)
                }
                mouse::Cursor::Levitating(p) => mouse::Cursor::Levitating(p + offset),
                _ => mouse::Cursor::Unavailable,
            },
        }
    }

    /// Offsets in logical px, snapped to whole device pixels.
    fn offset(&self) -> Vector {
        let s = device_scale();
        Vector::new(self.x.offset().round() / s, self.y.offset().round() / s)
    }

    /// The box inside the border.
    fn inner(&self, b: Rectangle) -> Rectangle {
        Rectangle {
            x: b.x + self.inset,
            y: b.y + self.inset,
            width: b.width - 2.0 * self.inset,
            height: b.height - 2.0 * self.inset,
        }
    }

    /// The content viewport: the box minus the scrollbars.
    fn clip(&self, b: Rectangle) -> Rectangle {
        let i = self.inner(b);
        Rectangle {
            width: (i.width - if self.bars.1 { BAR } else { 0.0 }).max(0.0),
            height: (i.height - if self.bars.0 { BAR } else { 0.0 }).max(0.0),
            ..i
        }
    }

    fn v_thumb(&self, b: Rectangle) -> Option<Rectangle> {
        if !self.bars.1 {
            return None;
        }
        let clip = self.clip(b);
        let (pos, len) = geom::thumb(
            clip.height,
            self.view.height,
            self.content.height,
            self.y.offset(),
            self.y.max(),
            device_scale(),
        );
        Some(Rectangle {
            x: clip.x + clip.width,
            y: clip.y + pos,
            width: BAR,
            height: len,
        })
    }

    fn h_thumb(&self, b: Rectangle) -> Option<Rectangle> {
        if !self.bars.0 {
            return None;
        }
        let clip = self.clip(b);
        let (pos, len) = geom::thumb(
            clip.width,
            self.view.width,
            self.content.width,
            self.x.offset(),
            self.x.max(),
            device_scale(),
        );
        Some(Rectangle {
            x: clip.x + pos,
            y: clip.y + clip.height,
            width: len,
            height: BAR,
        })
    }

    fn animating(&self) -> bool {
        self.x.is_animating() || self.y.is_animating()
    }

    fn log(&self, what: &str, detail: String) {
        if debug_enabled() {
            eprintln!(
                "[scroll#{} {:>9.3}s] {what:<10} {detail:<28} x={:.1}/{:.0} y={:.1}/{:.0} target={:.1} animating={}",
                self.id,
                self.t0.elapsed().as_secs_f64(),
                self.x.offset(),
                self.x.max(),
                self.y.offset(),
                self.y.max(),
                self.y.target(),
                self.animating()
            );
        }
    }
}

/// Everything `update` hands down to the content, bundled.
struct Down<'l, 'c, 'sh, Message> {
    layout: Layout<'l>,
    renderer: &'l iced::Renderer,
    clipboard: &'c mut dyn Clipboard,
    shell: &'c mut Shell<'sh, Message>,
    viewport: Rectangle,
}

impl<'a, Message: 'a> Scroller<'a, Message> {
    fn forward(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        cursor: mouse::Cursor,
        d: &mut Down<'_, '_, '_, Message>,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            d.layout.children().next().unwrap(),
            cursor,
            d.renderer,
            d.clipboard,
            d.shell,
            &d.viewport,
        );
    }

    /// Could this scroller take a delta `(dx, dy)` (device px or notches,
    /// positive = content moves left/up)?
    fn can_take(&self, st: &State, dx: f32, dy: f32) -> bool {
        (self.axis.x() && geom::can_take(st.x.offset(), st.x.max(), dx))
            || (self.axis.y() && geom::can_take(st.y.offset(), st.y.max(), dy))
    }

    /// A touch drag `travel` (device px) from the touch-down point: may this
    /// scroller claim it?
    fn can_claim(&self, st: &State, travel: (f32, f32)) -> bool {
        let (tx, ty) = travel;
        let dx = if geom::counts(tx, ty) { -tx } else { 0.0 };
        let dy = if geom::counts(ty, tx) { -ty } else { 0.0 };
        self.can_take(st, dx, dy)
    }

    /// Touch events: tap vs drag, and which scroller drags.
    fn touch(
        &mut self,
        tree: &mut Tree,
        te: &itouch::Event,
        visible: Rectangle,
        offset: Vector,
        root: bool,
        d: &mut Down<'_, '_, '_, Message>,
    ) {
        let id = tree.state.downcast_ref::<State>().id;
        let now = Instant::now();
        let s = device_scale();
        let (finger, position) = finger_of(te);
        let inside = visible.contains(position);
        // iced reports touch in logical px (window scale × UI scale); the
        // axes work in device px, so the content tracks the finger exactly
        // at every scale.
        let pd = (position.x * s, position.y * s);
        let shifted = position + offset;
        let at = mouse::Cursor::Available(shifted);
        let lost = Event::Touch(itouch::Event::FingerLost {
            id: itouch::Finger(finger),
            position: shifted,
        });
        let here = Event::Touch(moved_to(te, shifted));
        let (ax, ay) = (self.axis.x(), self.axis.y());
        macro_rules! axes {
            ($st:expr) => {
                Axes {
                    x: ax.then_some(&mut $st.x),
                    y: ay.then_some(&mut $st.y),
                }
            };
        }
        let mine = tree
            .state
            .downcast_ref::<State>()
            .touch
            .tracking()
            .filter(|t| t.0 == finger)
            .map(|t| t.1);
        match te {
            itouch::Event::FingerPressed { .. } => {
                if inside {
                    chain::press_here();
                }
                let st = tree.state.downcast_mut::<State>();
                let (out, fwd) = st.touch.press(finger, pd, inside, axes!(st));
                if out != TouchOutcome::Ignored {
                    st.log("touch", format!("{te:?} -> {out:?}/{fwd:?}"));
                }
                st.hover = hover_after(st.hover, out, shifted);
                if fwd == Forward::Pass {
                    self.forward(tree, &here, at, d);
                }
            }
            itouch::Event::FingerMoved { .. } => match mine {
                Some(Phase::Scrolling) => {
                    let st = tree.state.downcast_mut::<State>();
                    st.touch.drag(finger, pd, now, axes!(st));
                    st.hover = HoverSource::TouchNone;
                    d.shell.request_redraw();
                }
                Some(Phase::SteppedAside) => self.forward(tree, &here, at, d),
                Some(Phase::Pending) => {
                    // Inner scrollers decide first.
                    self.forward(tree, &here, at, d);
                    let holder = chain::touch_holder(finger);
                    let st = tree.state.downcast_mut::<State>();
                    if holder.is_some_and(|h| h != id) {
                        st.touch.step_aside(axes!(st));
                        st.hover = HoverSource::TouchNone;
                        st.log("touch", "an inner scroller took it".into());
                        // Cancel presses around the inner scroller (a
                        // pressable card around it must not click).
                        chain::synthetic_lost(|| self.forward(tree, &lost, at, d));
                    } else if st.touch.beyond_slop(pd) {
                        let travel = st.touch.travel(pd);
                        // The root takes what nobody else could (Chromium
                        // latches the viewport then).
                        if root || self.can_claim(st, travel) {
                            st.touch.claim(finger, pd, now, axes!(st));
                            chain::touch_claim(finger, id);
                            st.log("touch", format!("claimed {travel:?}"));
                        } else {
                            st.touch.step_aside(axes!(st));
                            st.log("touch", format!("left to the parent {travel:?}"));
                        }
                        st.hover = HoverSource::TouchNone;
                        d.shell.request_redraw();
                        // Out of the slop: no tap, whoever scrolls.
                        self.forward(tree, &lost, at, d);
                    }
                }
                None => {
                    if tree.state.downcast_ref::<State>().touch.other_finger() == Forward::Pass {
                        self.forward(tree, &here, at, d);
                    }
                }
            },
            itouch::Event::FingerLifted { .. } => match mine {
                Some(phase) => {
                    let st = tree.state.downcast_mut::<State>();
                    let (out, fwd) = st.touch.lift(finger, now, axes!(st));
                    if phase == Phase::Scrolling {
                        chain::touch_release(finger, id);
                    }
                    st.log("touch", format!("{te:?} -> {out:?}/{fwd:?}"));
                    if phase != Phase::SteppedAside {
                        st.hover = hover_after(st.hover, out, shifted);
                    }
                    // Tap too: hover-only content (the Scroll lab rows)
                    // updates on redraw, and a tap alone causes none.
                    d.shell.request_redraw();
                    match fwd {
                        Forward::Pass => self.forward(tree, &here, at, d),
                        Forward::Lost => self.forward(tree, &lost, at, d),
                        Forward::Swallow => {}
                    }
                }
                None => {
                    if tree.state.downcast_ref::<State>().touch.other_finger() == Forward::Pass {
                        self.forward(tree, &here, at, d);
                    }
                }
            },
            itouch::Event::FingerLost { .. } => {
                if chain::is_synthetic_lost() {
                    // An outer scroller cancelling presses: the scroller that
                    // holds the finger keeps scrolling.
                    if chain::touch_holder(finger) != Some(id) {
                        self.forward(tree, &here, at, d);
                    }
                } else if mine.is_some() {
                    let st = tree.state.downcast_mut::<State>();
                    let (out, fwd) = st.touch.lost(axes!(st));
                    chain::touch_release(finger, id);
                    st.hover = hover_after(st.hover, out, shifted);
                    if fwd == Forward::Pass {
                        self.forward(tree, &here, at, d);
                    }
                } else if tree.state.downcast_ref::<State>().touch.other_finger() == Forward::Pass {
                    self.forward(tree, &here, at, d);
                }
            }
        }
        if tree.state.downcast_ref::<State>().animating() {
            d.shell.request_redraw();
        }
        d.shell.capture_event();
    }

    /// Process the pending scroll-into-view requests that concern this
    /// scroller (it runs after its content, so inner scrollers went first).
    fn requests(&mut self, tree: &mut Tree, root: bool, d: &mut Down<'_, '_, '_, Message>) {
        let now = Instant::now();
        let reqs = chain::take_requests();
        let stack = chain::stack();
        let id = tree.state.downcast_ref::<State>().id;
        let content = d.layout.children().next().unwrap();
        let origin = content.bounds();
        let mut keep = Vec::new();
        for mut r in reqs {
            match &r.target {
                Target::Offset(name, ox, oy) => {
                    if self.id.as_ref() == Some(name) {
                        let s = device_scale();
                        let st = tree.state.downcast_mut::<State>();
                        for (ax, v) in [(&mut st.x, ox), (&mut st.y, oy)] {
                            if let Some(v) = v {
                                if r.smooth {
                                    ax.animate_to(v * s, now);
                                } else {
                                    ax.set_offset(v * s);
                                }
                            }
                        }
                        st.log("offset", format!("{ox:?},{oy:?}"));
                        d.shell.request_redraw();
                        continue;
                    }
                }
                Target::Container(cid) if r.rect.is_none() => {
                    let mut find = FindContainer {
                        id: cid,
                        found: None,
                    };
                    self.content.as_widget_mut().operate(
                        &mut tree.children[0],
                        content,
                        d.renderer,
                        &mut find,
                    );
                    if let Some(bounds) = find.found {
                        r.rect = Some(bounds);
                        r.chain = Some(stack.clone());
                    }
                }
                // Posted from outside any scroller: the first (innermost)
                // scroller whose content contains it takes it.
                Target::Rect(rect) if r.chain.is_none() && origin.contains(rect.center()) => {
                    r.rect = Some(*rect);
                    r.chain = Some(stack.clone());
                }
                _ => {}
            }
            match (r.rect, &r.chain) {
                (Some(rect), Some(ch)) if ch.contains(&id) => {
                    let outermost = ch.first() == Some(&id);
                    let s = device_scale();
                    let st = tree.state.downcast_mut::<State>();
                    let mut to = Vector::ZERO;
                    for horizontal in [true, false] {
                        let (ax, on, view, lo, len, align) = if horizontal {
                            (
                                &mut st.x,
                                self.axis.x(),
                                st.view.width,
                                rect.x - origin.x,
                                rect.width,
                                r.align.0,
                            )
                        } else {
                            (
                                &mut st.y,
                                self.axis.y(),
                                st.view.height,
                                rect.y - origin.y,
                                rect.height,
                                r.align.1,
                            )
                        };
                        let cur = ax.target() / s;
                        let new = if on && ax.max() > 0.0 {
                            geom::into_view(cur, view, ax.max() / s, lo, lo + len, align, r.margin)
                        } else {
                            cur
                        };
                        if (new - cur).abs() > 0.01 {
                            if r.smooth {
                                ax.animate_to(new * s, now);
                            } else {
                                ax.set_offset(new * s);
                            }
                        }
                        if horizontal {
                            to.x = new;
                        } else {
                            to.y = new;
                        }
                    }
                    st.log("reveal", format!("{rect:?}"));
                    d.shell.request_redraw();
                    if outermost {
                        continue;
                    }
                    // What the parent must show: the target where it will sit.
                    let clip = st.clip(d.layout.bounds());
                    let moved = Rectangle {
                        x: rect.x - to.x,
                        y: rect.y - to.y,
                        ..rect
                    };
                    r.rect = Some(moved.intersection(&clip).unwrap_or(clip));
                    keep.push(r);
                }
                _ => {
                    if root && r.rect.is_none() {
                        r.tries = r.tries.saturating_sub(1);
                        if r.tries == 0 {
                            continue;
                        }
                    }
                    keep.push(r);
                }
            }
        }
        chain::put_back(keep);
    }
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for Scroller<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
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
        let limits = limits.width(self.width).height(self.height);
        let (maxs, mins) = (limits.max(), limits.min());
        let inset = if self.border.is_some() { 1.0 } else { 0.0 };
        let avail_w = maxs.width.min(self.max_width.unwrap_or(f32::INFINITY));
        let avail_h = maxs.height.min(self.max_height.unwrap_or(f32::INFINITY));
        let fill_w = !matches!(self.width, Length::Shrink) && avail_w.is_finite();
        let fill_h = !matches!(self.height, Length::Shrink) && avail_h.is_finite();
        let csize = self.content.as_widget().size();
        let (cfill_w, cfill_h) = (csize.width.is_fill(), csize.height.is_fill());
        let (ax, ay) = (self.axis.x(), self.axis.y());

        // Classic scrollbars take layout space; reserving one can make the
        // other necessary, so settle in up to three passes.
        let (mut bx, mut by) = (false, false);
        let mut pass = 0;
        let (node, outer) = loop {
            let bar_w = if by { BAR } else { 0.0 };
            let bar_h = if bx { BAR } else { 0.0 };
            let box_w = (avail_w - 2.0 * inset - bar_w).max(0.0);
            let box_h = (avail_h - 2.0 * inset - bar_h).max(0.0);
            let min_w = if fill_w { box_w } else { 0.0 };
            let max_w = if !ax || cfill_w { box_w } else { f32::INFINITY };
            let max_h = if !ay || cfill_h { box_h } else { f32::INFINITY };
            let l = layout::Limits::new(Size::new(min_w, 0.0), Size::new(max_w, max_h));
            let node = self
                .content
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, &l);
            let cs = node.size();
            let w = if fill_w {
                avail_w
            } else {
                (cs.width + 2.0 * inset + bar_w).min(avail_w)
            };
            let h = if fill_h {
                avail_h
            } else {
                (cs.height + 2.0 * inset + bar_h).min(avail_h)
            };
            let outer = Size::new(w.max(mins.width), h.max(mins.height));
            let need_x = ax && cs.width > outer.width - 2.0 * inset - bar_w + 0.5;
            let need_y = ay && cs.height > outer.height - 2.0 * inset - bar_h + 0.5;
            pass += 1;
            if ((need_x && !bx) || (need_y && !by)) && pass < 3 {
                bx |= need_x;
                by |= need_y;
                continue;
            }
            break (node, outer);
        };
        let cs = node.size();
        let s = device_scale();
        let view = Size::new(
            outer.width - 2.0 * inset - if by { BAR } else { 0.0 },
            outer.height - 2.0 * inset - if bx { BAR } else { 0.0 },
        );
        let state = tree.state.downcast_mut::<State>();
        state.content = cs;
        state.view = view;
        state.bars = (bx, by);
        state.inset = inset;
        state.x.set_max(if ax {
            ((cs.width - view.width) * s).max(0.0)
        } else {
            0.0
        });
        state.y.set_max(if ay {
            ((cs.height - view.height) * s).max(0.0)
        } else {
            0.0
        });
        if !state.initial_applied {
            state.initial_applied = true;
            if let Some(px) = self.initial {
                state.y.set_offset(px * s);
            }
        }
        if let Some(id) = &self.anchor
            && state.anchor_tries > 0
        {
            state.anchor_tries -= 1;
            let mut find = FindContainer { id, found: None };
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                Layout::new(&node),
                renderer,
                &mut find,
            );
            let state = tree.state.downcast_mut::<State>();
            if let Some(b) = find.found {
                state.anchor_tries = 0;
                state.y.set_offset(((b.y - ANCHOR_GAP) * s).max(0.0));
            }
        }
        layout::Node::with_children(outer, vec![node.move_to(Point::new(inset, inset))])
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
        let b = layout.bounds();
        let id = tree.state.downcast_ref::<State>().id;
        let (clip, offset, dragging) = {
            let st = tree.state.downcast_ref::<State>();
            (
                st.clip(b),
                st.offset(),
                st.drag.is_some() || st.touch.is_scrolling(),
            )
        };
        let _entered = chain::enter(id, self.autoscroll.as_ref(), (offset.x, offset.y));
        let root = chain::is_root();
        let now = Instant::now();
        let s = device_scale();
        // Window coordinates of what this scroller receives.
        let win = {
            let (x, y) = chain::to_window();
            Vector::new(x, y)
        };
        // Where the pointer reaches this scroller's content.
        let visible = clip.intersection(viewport).unwrap_or(Rectangle {
            width: 0.0,
            height: 0.0,
            ..clip
        });
        let mut d = Down {
            layout,
            renderer,
            clipboard,
            shell,
            viewport: Rectangle {
                x: visible.x + offset.x,
                y: visible.y + offset.y,
                ..visible
            },
        };

        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => chain::set_shift(m.shift()),
            // A real mouse takes hover back from the last touch.
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::WheelScrolled { .. }) => {
                tree.state.downcast_mut::<State>().hover = HoverSource::Mouse;
            }
            Event::Mouse(mouse::Event::ButtonPressed(_)) if cursor.is_over(visible) => {
                chain::press_here();
            }
            _ => {}
        }

        if let Event::Touch(te) = event {
            self.touch(tree, te, visible, offset, root, &mut d);
            return;
        }

        // Everything else: the content first.
        let content_cursor = if dragging {
            mouse::Cursor::Unavailable
        } else {
            tree.state
                .downcast_ref::<State>()
                .content_cursor(cursor, visible, offset)
        };
        self.forward(tree, event, content_cursor, &mut d);
        if let Event::Mouse(mouse::Event::CursorMoved { .. }) = event {
            tree.state.downcast_mut::<State>().seen = offset;
        }
        if chain::has_requests() {
            self.requests(tree, root, &mut d);
        }
        if d.shell.is_event_captured() {
            return;
        }

        let bus = chain::bus();
        let state = tree.state.downcast_mut::<State>();
        if let Some((owner, bus)) = &bus {
            match event {
                Event::Window(window::Event::RedrawRequested(_)) => {
                    let mut bus = bus.borrow_mut();
                    if bus.target.map_or(*owner == id, |t| t == id) {
                        let dx = std::mem::take(&mut bus.pending_dx);
                        let dy = std::mem::take(&mut bus.pending_dy);
                        if dx != 0.0 && self.axis.x() {
                            state.x.set_offset(state.x.offset() + dx * s);
                        }
                        if dy != 0.0 && self.axis.y() {
                            state.y.set_offset(state.y.offset() + dy * s);
                        }
                    }
                }
                // Eligible middle press: inside the viewport, not consumed by
                // a child (text fields consume it), and something to scroll
                // (BatoScroll: the nearest box that can scroll on some axis).
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle)) => {
                    if let Some(p) = cursor.position_over(visible) {
                        let can = state.x.max() > 0.0 || state.y.max() > 0.0;
                        if can {
                            state.x.stop();
                            state.y.stop();
                            let w = p + win;
                            bus.borrow_mut().arm_for(id, w.x, w.y, now);
                            state.log("autoscroll", format!("armed at {p:?}"));
                            d.shell.request_redraw();
                        }
                        if can || *owner == id {
                            d.shell.capture_event();
                        }
                    }
                    return;
                }
                _ => {}
            }
        }
        match event {
            Event::Window(window::Event::RedrawRequested(at)) => {
                let mut more = false;
                for ax in [&mut state.x, &mut state.y] {
                    if ax.is_animating() {
                        more |= ax.tick(*at);
                    }
                }
                if more {
                    d.shell.request_redraw();
                }
                // Hover after scroll: the content moved under a parked mouse,
                // so tell it where the mouse now is (Chromium's fake
                // mousemove), once per changed frame.
                let after = state.offset();
                if after != state.seen
                    && state.hover == HoverSource::Mouse
                    && !dragging
                    && let Some(p) = cursor.position_over(visible)
                {
                    state.seen = after;
                    let moved = Event::Mouse(mouse::Event::CursorMoved { position: p });
                    d.viewport = Rectangle {
                        x: visible.x + after.x,
                        y: visible.y + after.y,
                        ..visible
                    };
                    self.forward(tree, &moved, mouse::Cursor::Available(p + after), &mut d);
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let Some(p) = cursor.position_over(visible) else {
                    return;
                };
                let p = p + win;
                let visible_w = Rectangle {
                    x: visible.x + win.x,
                    y: visible.y + win.y,
                    ..visible
                };
                let shift = chain::shift();
                let (ix, iy) = classify_xy(*delta, shift);
                if (ix, iy) == (ScrollInput::End, ScrollInput::End) {
                    if chain::wheel_end(now) == Some(id) {
                        let fx = state.x.precise_end(now);
                        let fy = state.y.precise_end(now);
                        state.log("end", format!("fling={}", fx || fy));
                        d.shell.request_redraw();
                        d.shell.capture_event();
                    }
                    return;
                }
                let kind = if matches!(ix, ScrollInput::Notches(_))
                    || matches!(iy, ScrollInput::Notches(_))
                {
                    Kind::Wheel
                } else {
                    Kind::Precise
                };
                let mine = match chain::wheel_latched(kind, p, shift, now) {
                    Some(holder) => holder == id,
                    // A new gesture: the innermost scroller that can move
                    // that way; the root takes whatever is left.
                    None => root || self.can_take(state, amount(ix), amount(iy)),
                };
                if !mine {
                    return;
                }
                chain::wheel_latch(id, kind, p, shift, now, visible_w);
                for (input, ax, on) in [
                    (ix, &mut state.x, self.axis.x()),
                    (iy, &mut state.y, self.axis.y()),
                ] {
                    match input {
                        ScrollInput::Notches(n) if on => ax.wheel_notches(n, now),
                        ScrollInput::Pixels(px) if on => ax.precise(px * touchpad_gain(), now),
                        _ => {}
                    }
                }
                state.log(
                    match kind {
                        Kind::Wheel => "wheel",
                        Kind::Precise => "precise",
                    },
                    format!("{delta:?}"),
                );
                d.shell.request_redraw();
                d.shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let Some(drag) = state.drag {
                    let (track, len) = if drag.horizontal {
                        (
                            state.clip(b).width,
                            state.h_thumb(b).unwrap_or_default().width,
                        )
                    } else {
                        (
                            state.clip(b).height,
                            state.v_thumb(b).unwrap_or_default().height,
                        )
                    };
                    let moved = if drag.horizontal {
                        position.x
                    } else {
                        position.y
                    } - drag.grab;
                    let ax = if drag.horizontal {
                        &mut state.x
                    } else {
                        &mut state.y
                    };
                    let max = ax.max();
                    ax.set_offset(geom::drag_offset(drag.start, moved, track, len, max, s));
                    d.shell.request_redraw();
                    d.shell.capture_event();
                } else {
                    let hover = (
                        state.h_thumb(b).is_some_and(|t| cursor.is_over(t)),
                        state.v_thumb(b).is_some_and(|t| cursor.is_over(t)),
                    );
                    if hover != state.thumb_hover {
                        state.thumb_hover = hover;
                        d.shell.request_redraw();
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_over(b) else {
                    return;
                };
                if !viewport.contains(p) {
                    return;
                }
                let clip = state.clip(b);
                for horizontal in [false, true] {
                    let (thumb, in_bar) = if horizontal {
                        (
                            state.h_thumb(b),
                            p.y >= clip.y + clip.height && p.x < clip.x + clip.width,
                        )
                    } else {
                        (
                            state.v_thumb(b),
                            p.x >= clip.x + clip.width && p.y < clip.y + clip.height,
                        )
                    };
                    let Some(t) = thumb.filter(|_| in_bar) else {
                        continue;
                    };
                    let ax = if horizontal {
                        &mut state.x
                    } else {
                        &mut state.y
                    };
                    if t.contains(p) {
                        let start = ax.offset();
                        state.drag = Some(ThumbDrag {
                            horizontal,
                            grab: if horizontal { p.x } else { p.y },
                            start,
                        });
                        state.log("drag", String::new());
                    } else {
                        // Track click: page step, animated.
                        let (view, before) = if horizontal {
                            (clip.width, p.x < t.x)
                        } else {
                            (clip.height, p.y < t.y)
                        };
                        let step = geom::page_step(view) * s * if before { -1.0 } else { 1.0 };
                        ax.animate_to(ax.target() + step, now);
                        state.log("track", format!("{step:+.0}"));
                    }
                    d.shell.request_redraw();
                    d.shell.capture_event();
                    return;
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if state.drag.is_some() =>
            {
                state.drag = None;
                state.thumb_hover = (
                    state.h_thumb(b).is_some_and(|t| cursor.is_over(t)),
                    state.v_thumb(b).is_some_and(|t| cursor.is_over(t)),
                );
                d.shell.request_redraw();
                d.shell.capture_event();
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(named),
                modifiers,
                ..
            }) => {
                // Chromium: arrows 40 px, PageUp/PageDown/Space 87.5 % of the
                // viewport, Home/End; all animated like a wheel notch. The
                // clicked scroller takes them and bubbles at its edge.
                use keyboard::key::Named;
                if !chain::key_eligible(id, root) {
                    return;
                }
                let page = geom::page_step(state.view.height) * s;
                let (horizontal, to) = match named {
                    Named::ArrowDown => (false, state.y.target() + 40.0 * s),
                    Named::ArrowUp => (false, state.y.target() - 40.0 * s),
                    Named::ArrowRight => (true, state.x.target() + 40.0 * s),
                    Named::ArrowLeft => (true, state.x.target() - 40.0 * s),
                    Named::PageDown => (false, state.y.target() + page),
                    Named::PageUp => (false, state.y.target() - page),
                    Named::Space if modifiers.shift() => (false, state.y.target() - page),
                    Named::Space => (false, state.y.target() + page),
                    Named::Home => (false, 0.0),
                    Named::End => (false, state.y.max()),
                    _ => return,
                };
                let (ax, on) = if horizontal {
                    (&mut state.x, self.axis.x())
                } else {
                    (&mut state.y, self.axis.y())
                };
                let takes = on && geom::can_take(ax.offset(), ax.max(), to - ax.offset());
                // The root keeps its axis' keys even at an edge (as before).
                if !(takes || root && on) {
                    return;
                }
                ax.animate_to(to, now);
                state.log("key", format!("{named:?}"));
                d.shell.request_redraw();
                d.shell.capture_event();
            }
            _ => {}
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
        let state = tree.state.downcast_ref::<State>();
        if state.drag.is_some() {
            return mouse::Interaction::Idle;
        }
        let offset = state.offset();
        let clip = state.clip(layout.bounds());
        let visible = clip.intersection(viewport).unwrap_or(clip);
        let c = state.content_cursor(cursor, visible, offset);
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            c,
            &Rectangle {
                x: visible.x + offset.x,
                y: visible.y + offset.y,
                ..visible
            },
            renderer,
        )
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
        let state = tree.state.downcast_ref::<State>();
        let offset = state.offset();
        chain::drawn(state.id, self.id.as_ref(), (offset.x, offset.y));
        if self.autoscroll.is_some() {
            chain::set_page((offset.x, offset.y));
        }
        if draw_debug() {
            eprintln!(
                "[draw#{} {:>9.3}s] offset={:.1},{:.1}",
                state.id,
                state.t0.elapsed().as_secs_f64(),
                state.x.offset(),
                state.y.offset()
            );
        }
        let b = layout.bounds();
        if let Some((color, radius)) = self.border {
            renderer.fill_quad(
                Quad {
                    bounds: b,
                    border: Border {
                        color,
                        width: 1.0,
                        radius: radius.into(),
                    },
                    ..Quad::default()
                },
                Color::TRANSPARENT,
            );
        }
        let clip = state.clip(b);
        let visible = clip.intersection(viewport).unwrap_or(clip);
        let c = if state.drag.is_some() {
            mouse::Cursor::Unavailable
        } else {
            state.content_cursor(cursor, visible, offset)
        };
        renderer.with_layer(clip, |renderer| {
            renderer.with_translation(Vector::new(-offset.x, -offset.y), |renderer| {
                self.content.as_widget().draw(
                    &tree.children[0],
                    renderer,
                    theme,
                    style,
                    layout.children().next().unwrap(),
                    c,
                    &Rectangle {
                        x: visible.x + offset.x,
                        y: visible.y + offset.y,
                        ..visible
                    },
                );
            });
        });
        let dragging = state.drag.map(|d| d.horizontal);
        for (t, hot) in [
            (
                state.v_thumb(b),
                state.thumb_hover.1 || dragging == Some(false),
            ),
            (
                state.h_thumb(b),
                state.thumb_hover.0 || dragging == Some(true),
            ),
        ] {
            if let Some(t) = t {
                renderer.fill_quad(
                    Quad {
                        bounds: t,
                        border: Border {
                            radius: (BAR / 2.0).into(),
                            ..Border::default()
                        },
                        ..Quad::default()
                    },
                    if hot { self.thumb_hover } else { self.thumb },
                );
            }
        }
        if self.fade
            && let Some(bg) = self.backdrop
        {
            draw_fade(renderer, b, self.axis == Axis::Horizontal, bg);
        }
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

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        let state = tree.state.downcast_ref::<State>();
        let offset = state.offset();
        let clip = state.clip(layout.bounds());
        let visible = clip.intersection(viewport).unwrap_or(clip);
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            &Rectangle {
                x: visible.x + offset.x,
                y: visible.y + offset.y,
                ..visible
            },
            translation - offset,
        )
    }
}

/// The edge-fade mask as backdrop-coloured strips, one per device pixel,
/// over everything in `b` (content, scrollbar, border), in a layer of its
/// own so it covers the content's text too.
fn draw_fade(renderer: &mut iced::Renderer, b: Rectangle, horizontal: bool, bg: Color) {
    let s = device_scale().max(0.01);
    let len = if horizontal { b.width } else { b.height };
    let n = (len * s).round() as i32;
    let zone = ((FADE * s).ceil() as i32).min(n);
    renderer.with_layer(b, |renderer| {
        for i in (0..zone).chain((n - zone).max(zone)..n) {
            let d = (i as f32 + 0.5) / s;
            let a = 1.0 - geom::fade_alpha(d, len);
            if a <= 0.001 {
                continue;
            }
            let strip = if horizontal {
                Rectangle {
                    x: b.x + i as f32 / s,
                    y: b.y,
                    width: 1.0 / s,
                    height: b.height,
                }
            } else {
                Rectangle {
                    x: b.x,
                    y: b.y + i as f32 / s,
                    width: b.width,
                    height: 1.0 / s,
                }
            };
            renderer.fill_quad(
                Quad {
                    bounds: strip,
                    ..Quad::default()
                },
                Color { a: bg.a * a, ..bg },
            );
        }
    });
}

impl<'a, Message: 'a> From<Scroller<'a, Message>>
    for Element<'a, Message, iced::Theme, iced::Renderer>
{
    fn from(s: Scroller<'a, Message>) -> Self {
        Element::new(s)
    }
}

#[cfg(test)]
mod tests;
