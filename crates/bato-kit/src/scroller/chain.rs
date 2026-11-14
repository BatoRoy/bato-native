//! How nested scrollers agree on who scrolls (Chromium's scroll latching and
//! chaining), on the UI thread.
//!
//! iced hands an event to the outer scroller first; a scroller passes it to
//! its content before acting on it itself, so the *innermost* scroller under
//! the pointer decides first. What they share lives here, thread-local:
//!
//! - **the update stack**: the scrollers whose `update` is running, outer →
//!   inner. It is the ancestor chain of whatever runs inside them, so a
//!   widget can post a scroll-into-view that only its own scrollers handle;
//! - **the wheel latch**: a wheel or touchpad gesture latches to the scroller
//!   it started in (the innermost one that could take its first delta). A
//!   mouse-wheel sequence lasts while events come less than 500 ms apart
//!   within 10 px of the first one with the same Shift state (Chromium's
//!   `MouseWheelPhaseHandler`); a touchpad gesture lasts until the fingers
//!   lift. The latched scroller keeps every event, even at its edge: the
//!   parent only scrolls in the *next* gesture;
//! - **the touch latch**: the finger a scroller claimed when it left the
//!   slop region, so the outer ones step aside;
//! - **the keyboard chain**: the scrollers around the last click (Chromium
//!   scrolls the clicked/focused box with the keys, bubbling to its parents);
//! - **reveal requests**: scroll-into-view and scroll-to, see
//!   [`super::reveal`];
//! - **the autoscroll bus** of the nearest scroller that has one, so inner
//!   scrollers can be BatoScroll targets.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use iced::{Point, Rectangle};

use crate::autoscroll::AutoscrollBus;

/// Wheel events further apart than this start a new latching sequence.
pub const WHEEL_SEQUENCE: Duration = Duration::from_millis(500);
/// …and so do wheel events further than this (px, per axis) from the first.
pub const WHEEL_SLOP: f32 = 10.0;
/// A touchpad gesture whose end (`axis_stop`) never came expires after this.
pub const PRECISE_SEQUENCE: Duration = Duration::from_millis(1500);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A fresh scroller identity (kept in its widget state).
pub fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Mouse wheel (line deltas): animated, no phases.
    Wheel,
    /// Touchpad (pixel deltas): 1:1, ends with a zero delta.
    Precise,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WheelLatch {
    pub id: u64,
    pub kind: Kind,
    pub first: Point,
    pub last: Instant,
    pub shift: bool,
    /// The touchpad fingers lifted: the next precise event starts anew.
    pub ended: bool,
    /// The latched scroller's visible viewport (window coordinates): a
    /// pointer outside it can't reach it, so the latch is void.
    pub bounds: Rectangle,
}

impl WheelLatch {
    /// Does an event of `kind` at `pos` continue this sequence?
    pub fn continues(&self, kind: Kind, pos: Point, shift: bool, now: Instant) -> bool {
        if kind != self.kind || !self.bounds.contains(pos) {
            return false;
        }
        let since = now.saturating_duration_since(self.last);
        match kind {
            Kind::Wheel => {
                since <= WHEEL_SEQUENCE
                    && shift == self.shift
                    && (pos.x - self.first.x).abs() <= WHEEL_SLOP
                    && (pos.y - self.first.y).abs() <= WHEEL_SLOP
            }
            Kind::Precise => !self.ended && since <= PRECISE_SEQUENCE,
        }
    }
}

/// A pending scroll-into-view / scroll-to.
#[derive(Debug, Clone)]
pub struct Request {
    pub target: Target,
    pub align: (super::geom::Align, super::geom::Align),
    pub smooth: bool,
    pub margin: f32,
    /// The scrollers (outer → inner) that contain the target, once known.
    pub chain: Option<Vec<u64>>,
    /// The target rectangle in the coordinates of the next scroller up.
    pub rect: Option<Rectangle>,
    /// Root scrollers left to look for an unresolved target.
    pub tries: u8,
}

#[derive(Debug, Clone)]
pub enum Target {
    /// A rectangle in layout coordinates.
    Rect(Rectangle),
    /// The container with this id (`container(..).id(..)`).
    Container(iced::widget::Id),
    /// A scroller (by its `.id()`) to an absolute offset (logical px).
    Offset(iced::widget::Id, Option<f32>, Option<f32>),
}

#[derive(Default)]
struct Shared {
    stack: Vec<u64>,
    /// The offsets (logical px) of the scrollers on the stack.
    shifts: Vec<(f32, f32)>,
    buses: Vec<(u64, AutoscrollBus)>,
    wheel: Option<WheelLatch>,
    /// (finger, scroller)
    touch: Option<(u64, u64)>,
    keys: Vec<u64>,
    drawn: HashMap<u64, Instant>,
    last_draw: Option<Instant>,
    requests: Vec<Request>,
    offsets: HashMap<u64, (f32, f32)>,
    names: HashMap<u64, iced::widget::Id>,
}

thread_local! {
    static SHARED: RefCell<Shared> = RefCell::new(Shared::default());
    static SHIFT: Cell<bool> = const { Cell::new(false) };
    static SYNTHETIC_LOST: Cell<bool> = const { Cell::new(false) };
    static PAGE: Cell<(f32, f32)> = const { Cell::new((0.0, 0.0)) };
}

/// The page scroller (the one owning the autoscroll bus) was drawn at these
/// offsets.
pub fn set_page(offsets: (f32, f32)) {
    PAGE.with(|p| p.set(offsets));
}

pub fn page() -> (f32, f32) {
    PAGE.with(Cell::get)
}

fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    SHARED.with(|s| f(&mut s.borrow_mut()))
}

// ── Update stack ────────────────────────────────────────────────────────

/// Pops the scroller (and its bus) when its `update` returns.
pub struct Entered {
    bus: bool,
}

impl Drop for Entered {
    fn drop(&mut self) {
        with(|s| {
            s.stack.pop();
            s.shifts.pop();
            if self.bus {
                s.buses.pop();
            }
        });
    }
}

/// A scroller's `update` starts; `offset` is its scroll offset (logical px).
pub fn enter(id: u64, bus: Option<&AutoscrollBus>, offset: (f32, f32)) -> Entered {
    with(|s| {
        s.stack.push(id);
        s.shifts.push(offset);
        if let Some(b) = bus {
            s.buses.push((id, b.clone()));
        }
    });
    Entered { bus: bus.is_some() }
}

/// The scrollers whose `update` is running, outer → inner.
pub fn stack() -> Vec<u64> {
    with(|s| s.stack.clone())
}

/// What to add to a position the running scroller received to get window
/// coordinates: its ancestors' offsets (latches and the autoscroll origin
/// are kept in window coordinates).
pub fn to_window() -> (f32, f32) {
    with(|s| {
        let n = s.shifts.len().saturating_sub(1);
        s.shifts[..n]
            .iter()
            .fold((0.0, 0.0), |a, o| (a.0 - o.0, a.1 - o.1))
    })
}

/// Is the running scroller the outermost one?
pub fn is_root() -> bool {
    with(|s| s.stack.len() <= 1)
}

/// The autoscroll bus of the nearest enclosing scroller that has one, and
/// that scroller's id.
pub fn bus() -> Option<(u64, AutoscrollBus)> {
    with(|s| s.buses.last().cloned())
}

// ── Modifiers ───────────────────────────────────────────────────────────

pub fn set_shift(on: bool) {
    SHIFT.with(|c| c.set(on));
}

pub fn shift() -> bool {
    SHIFT.with(Cell::get)
}

// ── Wheel / touchpad latch ──────────────────────────────────────────────

/// The scroller the current gesture is latched to, if this event continues it.
pub fn wheel_latched(kind: Kind, pos: Point, shift: bool, now: Instant) -> Option<u64> {
    with(|s| {
        s.wheel
            .filter(|l| l.continues(kind, pos, shift, now))
            .map(|l| l.id)
    })
}

/// Latch a new gesture, or refresh the running one.
pub fn wheel_latch(id: u64, kind: Kind, pos: Point, shift: bool, now: Instant, bounds: Rectangle) {
    with(|s| match &mut s.wheel {
        Some(l) if l.id == id && l.continues(kind, pos, shift, now) => {
            l.last = now;
            l.bounds = bounds;
        }
        w => {
            *w = Some(WheelLatch {
                id,
                kind,
                first: pos,
                last: now,
                shift,
                ended: false,
                bounds,
            })
        }
    });
}

/// The touchpad fingers lifted; returns the scroller the gesture was latched
/// to (it starts the fling).
pub fn wheel_end(now: Instant) -> Option<u64> {
    with(|s| match &mut s.wheel {
        Some(l) if l.kind == Kind::Precise && !l.ended => {
            if now.saturating_duration_since(l.last) > PRECISE_SEQUENCE {
                return None;
            }
            l.ended = true;
            Some(l.id)
        }
        _ => None,
    })
}

#[cfg(test)]
pub fn wheel_reset() {
    with(|s| s.wheel = None);
}

// ── Touch latch ─────────────────────────────────────────────────────────

pub fn touch_claim(finger: u64, id: u64) {
    with(|s| s.touch = Some((finger, id)));
}

/// The scroller that claimed `finger`.
pub fn touch_holder(finger: u64) -> Option<u64> {
    with(|s| s.touch.filter(|t| t.0 == finger).map(|t| t.1))
}

pub fn touch_release(finger: u64, id: u64) {
    with(|s| {
        if s.touch == Some((finger, id)) {
            s.touch = None;
        }
    });
}

/// Run `f` while a `FingerLost` being delivered only cancels presses (an
/// outer scroller stepping aside for an inner one): the scroller that holds
/// the finger must not stop.
pub fn synthetic_lost<R>(f: impl FnOnce() -> R) -> R {
    SYNTHETIC_LOST.with(|c| c.set(true));
    let r = f();
    SYNTHETIC_LOST.with(|c| c.set(false));
    r
}

pub fn is_synthetic_lost() -> bool {
    SYNTHETIC_LOST.with(Cell::get)
}

// ── Keyboard chain ──────────────────────────────────────────────────────

/// A press landed in the running scroller: it and its ancestors take the
/// keys. Outer scrollers see the press first, so the innermost one wins.
pub fn press_here() {
    with(|s| s.keys = s.stack.clone());
}

/// May the running scroller `id` act on a scroll key? The clicked
/// scroller's chain does; a root scroller also does while no scroller of the
/// chain is on screen (nothing clicked yet, or it was removed).
pub fn key_eligible(id: u64, root: bool) -> bool {
    with(|s| {
        if s.keys.contains(&id) {
            return true;
        }
        if !root {
            return false;
        }
        match s.keys.first() {
            None => true,
            Some(r) => !is_live(s, *r),
        }
    })
}

fn is_live(s: &Shared, id: u64) -> bool {
    match (s.drawn.get(&id), s.last_draw) {
        (Some(t), Some(last)) => last.saturating_duration_since(*t) < Duration::from_millis(30),
        _ => false,
    }
}

/// The scroller was drawn (liveness for the keyboard chain) at these
/// offsets (logical px; debug and self-test readout).
pub fn drawn(id: u64, name: Option<&iced::widget::Id>, offsets: (f32, f32)) {
    let now = Instant::now();
    with(|s| {
        s.drawn.insert(id, now);
        s.last_draw = Some(now);
        s.offsets.insert(id, offsets);
        if let Some(n) = name {
            s.names.insert(id, n.clone());
        }
    });
}

/// The last drawn offsets of the scroller with this `.id()`.
pub fn offsets_of(name: &iced::widget::Id) -> Option<(f32, f32)> {
    with(|s| {
        s.names
            .iter()
            .filter(|(id, n)| *n == name && is_live(s, **id))
            .find_map(|(id, _)| s.offsets.get(id).copied())
    })
}

/// The last drawn offsets of scroller `id`.
pub fn offsets_by_id(id: u64) -> Option<(f32, f32)> {
    with(|s| s.offsets.get(&id).copied())
}

// ── Requests ────────────────────────────────────────────────────────────

pub fn push(r: Request) {
    with(|s| s.requests.push(r));
}

pub fn has_requests() -> bool {
    with(|s| !s.requests.is_empty())
}

/// Take the pending requests (the running scroller handles them and puts
/// back what is not done).
pub fn take_requests() -> Vec<Request> {
    with(|s| std::mem::take(&mut s.requests))
}

pub fn put_back(mut rs: Vec<Request>) {
    with(|s| {
        rs.append(&mut s.requests);
        s.requests = rs;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> Rectangle {
        Rectangle::new(Point::ORIGIN, iced::Size::new(500.0, 500.0))
    }

    #[test]
    fn wheel_sequence_rules() {
        wheel_reset();
        let t = Instant::now();
        let p = Point::new(100.0, 100.0);
        assert_eq!(wheel_latched(Kind::Wheel, p, false, t), None);
        wheel_latch(7, Kind::Wheel, p, false, t, r());
        let ms = |n| t + Duration::from_millis(n);
        assert_eq!(wheel_latched(Kind::Wheel, p, false, ms(400)), Some(7));
        wheel_latch(7, Kind::Wheel, p, false, ms(400), r());
        // 500 ms after the *last* event, still the same sequence.
        assert_eq!(wheel_latched(Kind::Wheel, p, false, ms(850)), Some(7));
        assert_eq!(wheel_latched(Kind::Wheel, p, false, ms(901)), None, "gap");
        let moved = Point::new(111.0, 100.0);
        assert_eq!(
            wheel_latched(Kind::Wheel, moved, false, ms(450)),
            None,
            "slop"
        );
        assert_eq!(wheel_latched(Kind::Wheel, p, true, ms(450)), None, "shift");
        assert_eq!(
            wheel_latched(Kind::Precise, p, false, ms(450)),
            None,
            "kind"
        );
        let out = Point::new(600.0, 100.0);
        assert_eq!(
            wheel_latched(Kind::Wheel, out, false, ms(450)),
            None,
            "pointer outside the latched scroller"
        );
    }

    #[test]
    fn touchpad_gesture_lasts_until_the_fingers_lift() {
        wheel_reset();
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        let p = Point::new(10.0, 10.0);
        wheel_latch(3, Kind::Precise, p, false, t, r());
        assert_eq!(wheel_latched(Kind::Precise, p, false, ms(900)), Some(3));
        assert_eq!(wheel_end(ms(950)), Some(3));
        assert_eq!(wheel_latched(Kind::Precise, p, false, ms(960)), None);
        assert_eq!(wheel_end(ms(970)), None, "ends once");
    }

    #[test]
    fn stack_and_key_chain() {
        let a = enter(100, None, (0.0, 50.0));
        assert!(is_root());
        press_here();
        {
            let _b = enter(101, None, (7.0, 9.0));
            assert!(!is_root());
            assert_eq!(to_window(), (0.0, -50.0), "the parent's offset only");
            assert_eq!(stack(), vec![100, 101]);
            press_here();
        }
        drop(a);
        assert!(stack().is_empty());
        drawn(100, None, (0.0, 0.0));
        drawn(101, None, (0.0, 0.0));
        assert!(key_eligible(101, false));
        assert!(key_eligible(100, true));
        assert!(!key_eligible(102, false));
        assert!(
            !key_eligible(103, true),
            "another root while the chain is on screen"
        );
    }

    #[test]
    fn touch_latch_and_synthetic_lost() {
        touch_claim(1, 9);
        assert_eq!(touch_holder(1), Some(9));
        assert_eq!(touch_holder(2), None);
        touch_release(1, 8);
        assert_eq!(touch_holder(1), Some(9), "only the holder releases");
        touch_release(1, 9);
        assert_eq!(touch_holder(1), None);
        assert!(!is_synthetic_lost());
        assert!(synthetic_lost(is_synthetic_lost));
        assert!(!is_synthetic_lost());
    }
}
