//! Touchscreen arbitration for one scroller, around `scroll_physics::TouchDrag`
//! (Chromium's 15 px slop, 1:1 drag, fling, fling-stop), on one or two axes.
//!
//! A finger goes through three phases:
//!
//! - **pending**: inside the slop, may still be a tap; the content sees it;
//! - **scrolling**: this scroller claimed it when it left the slop (it could
//!   move in that direction), the content got `FingerLost`;
//! - **stepped aside**: an inner scroller claimed it, or this one could not
//!   move that way (at its edge, or the wrong axis) and left it to its
//!   parent. Chromium latches a touch scroll to one scroller for the whole
//!   gesture, so nothing chains mid-swipe.
//!
//! Positions are in the axes' units (device px). The horizontal `TouchDrag`
//! is fed swapped coordinates, so its "y" is x.

use std::time::Instant;

use iced::touch;
use scroll_physics::{ScrollAxis, TOUCH_SLOP, TouchDrag, TouchOutcome};

use crate::scale::device_scale;

/// What the content underneath gets for a touch event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forward {
    /// Deliver the event as is (press, pending move, tap lift).
    Pass,
    /// Deliver `FingerLost` instead: the press the content saw is cancelled
    /// (the finger became a scroll, or it only stopped a fling).
    Lost,
    /// Deliver nothing (scrolling finger, or a second finger).
    Swallow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Pending,
    Scrolling,
    SteppedAside,
}

/// The scroller's axes, either of which may be absent.
pub struct Axes<'a> {
    pub x: Option<&'a mut ScrollAxis>,
    pub y: Option<&'a mut ScrollAxis>,
}

#[derive(Debug, Default)]
pub struct TouchRouter {
    dx: TouchDrag,
    dy: TouchDrag,
    finger: Option<u64>,
    start: (f32, f32),
    phase: Option<Phase>,
}

impl TouchRouter {
    pub fn is_scrolling(&self) -> bool {
        self.phase == Some(Phase::Scrolling)
    }

    /// The tracked finger and its phase.
    pub fn tracking(&self) -> Option<(u64, Phase)> {
        self.finger.zip(self.phase)
    }

    /// Movement from the touch-down point.
    pub fn travel(&self, pos: (f32, f32)) -> (f32, f32) {
        (pos.0 - self.start.0, pos.1 - self.start.1)
    }

    /// Would the finger at `pos` have left the slop region?
    pub fn beyond_slop(&self, pos: (f32, f32)) -> bool {
        let (dx, dy) = self.travel(pos);
        dx.hypot(dy) > TOUCH_SLOP * device_scale().max(0.01)
    }

    /// `FingerPressed`. Stops any fling on these axes (Chromium cancels the
    /// fling on touch-down).
    pub fn press(
        &mut self,
        id: u64,
        pos: (f32, f32),
        inside: bool,
        axes: Axes,
    ) -> (TouchOutcome, Forward) {
        if self.finger.is_some() {
            return (TouchOutcome::Ignored, self.other_finger());
        }
        if !inside {
            return (TouchOutcome::Ignored, Forward::Pass);
        }
        let s = device_scale();
        if let Some(a) = axes.x {
            self.dx.set_scale(s);
            self.dx.down(id, pos.1, pos.0, a);
        }
        if let Some(a) = axes.y {
            self.dy.set_scale(s);
            self.dy.down(id, pos.0, pos.1, a);
        }
        self.finger = Some(id);
        self.start = pos;
        self.phase = Some(Phase::Pending);
        (TouchOutcome::Pending, Forward::Pass)
    }

    /// Pending → scrolling: this scroller takes the finger. The content must
    /// get `FingerLost`.
    pub fn claim(&mut self, id: u64, pos: (f32, f32), now: Instant, axes: Axes) -> TouchOutcome {
        self.phase = Some(Phase::Scrolling);
        self.drag(id, pos, now, axes);
        TouchOutcome::ScrollStarted
    }

    /// Pending → stepped aside: an inner scroller took the finger, or this
    /// one can't move that way. Nothing moves.
    pub fn step_aside(&mut self, axes: Axes) {
        if let Some(a) = axes.x {
            self.dx.cancel(a);
        }
        if let Some(a) = axes.y {
            self.dy.cancel(a);
        }
        self.phase = Some(Phase::SteppedAside);
    }

    /// A move of the scrolling finger: the content follows it 1:1 (beyond
    /// the slop on the first move, so it does not jump).
    pub fn drag(&mut self, id: u64, pos: (f32, f32), now: Instant, axes: Axes) -> TouchOutcome {
        if let Some(a) = axes.x {
            self.dx.moved(id, pos.1, pos.0, now, a);
        }
        if let Some(a) = axes.y {
            self.dy.moved(id, pos.0, pos.1, now, a);
        }
        TouchOutcome::Scrolling
    }

    /// `FingerLifted` of the tracked finger.
    pub fn lift(&mut self, id: u64, now: Instant, axes: Axes) -> (TouchOutcome, Forward) {
        let phase = self.phase.take();
        self.finger = None;
        match phase {
            Some(Phase::Pending) | Some(Phase::Scrolling) => {
                let mut tap = true;
                if let Some(a) = axes.x {
                    tap &= self.dx.up(id, now, a) == TouchOutcome::Tap;
                }
                if let Some(a) = axes.y {
                    tap &= self.dy.up(id, now, a) == TouchOutcome::Tap;
                }
                if phase == Some(Phase::Scrolling) {
                    (TouchOutcome::Ended, Forward::Swallow)
                } else if tap {
                    (TouchOutcome::Tap, Forward::Pass)
                } else {
                    // Ended without scrolling: the touch only stopped a fling.
                    (TouchOutcome::Ended, Forward::Lost)
                }
            }
            _ => (TouchOutcome::Ignored, Forward::Pass),
        }
    }

    /// `FingerLost` of the tracked finger (`wl_touch.cancel`): no fling, no tap.
    pub fn lost(&mut self, axes: Axes) -> (TouchOutcome, Forward) {
        let phase = self.phase.take();
        self.finger = None;
        if let Some(a) = axes.x {
            self.dx.cancel(a);
        }
        if let Some(a) = axes.y {
            self.dy.cancel(a);
        }
        match phase {
            Some(Phase::Scrolling) => (TouchOutcome::Ended, Forward::Swallow),
            Some(Phase::Pending) => (TouchOutcome::Ended, Forward::Pass),
            _ => (TouchOutcome::Ignored, Forward::Pass),
        }
    }

    /// An event of a finger this router does not track.
    pub fn other_finger(&self) -> Forward {
        // An inner widget holds the first finger (a pinch): it gets the rest.
        if self.finger.is_some() && self.phase != Some(Phase::SteppedAside) {
            Forward::Swallow
        } else {
            Forward::Pass
        }
    }
}

/// The finger id and position of a touch event.
pub fn finger_of(ev: &touch::Event) -> (u64, iced::Point) {
    match *ev {
        touch::Event::FingerPressed { id, position }
        | touch::Event::FingerMoved { id, position }
        | touch::Event::FingerLifted { id, position }
        | touch::Event::FingerLost { id, position } => (id.0, position),
    }
}

/// The same event at another position.
pub fn moved_to(ev: &touch::Event, position: iced::Point) -> touch::Event {
    match *ev {
        touch::Event::FingerPressed { id, .. } => touch::Event::FingerPressed { id, position },
        touch::Event::FingerMoved { id, .. } => touch::Event::FingerMoved { id, position },
        touch::Event::FingerLifted { id, .. } => touch::Event::FingerLifted { id, position },
        touch::Event::FingerLost { id, .. } => touch::Event::FingerLost { id, position },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn axis() -> ScrollAxis {
        let mut a = ScrollAxis::default();
        a.set_max(100_000.0);
        a.set_offset(5_000.0);
        a
    }

    /// A vertical scroller that always claims when the slop is left (the
    /// pre-Phase-2 page behaviour).
    fn feed(
        r: &mut TouchRouter,
        a: &mut ScrollAxis,
        t: Instant,
        steps: &[(u8, u64, f32, u64)],
    ) -> Vec<(TouchOutcome, Forward)> {
        steps
            .iter()
            .map(|&(k, id, y, ms)| {
                let now = t + Duration::from_millis(ms);
                let pos = (100.0, y);
                match k {
                    0 => r.press(
                        id,
                        pos,
                        true,
                        Axes {
                            x: None,
                            y: Some(a),
                        },
                    ),
                    1 => match r.tracking() {
                        Some((f, Phase::Pending)) if f == id => {
                            if r.beyond_slop(pos) {
                                (
                                    r.claim(
                                        id,
                                        pos,
                                        now,
                                        Axes {
                                            x: None,
                                            y: Some(a),
                                        },
                                    ),
                                    Forward::Lost,
                                )
                            } else {
                                (TouchOutcome::Pending, Forward::Pass)
                            }
                        }
                        Some((f, Phase::Scrolling)) if f == id => (
                            r.drag(
                                id,
                                pos,
                                now,
                                Axes {
                                    x: None,
                                    y: Some(a),
                                },
                            ),
                            Forward::Swallow,
                        ),
                        _ => (TouchOutcome::Ignored, r.other_finger()),
                    },
                    2 => match r.tracking() {
                        Some((f, _)) if f == id => r.lift(
                            id,
                            now,
                            Axes {
                                x: None,
                                y: Some(a),
                            },
                        ),
                        _ => (TouchOutcome::Ignored, r.other_finger()),
                    },
                    _ => match r.tracking() {
                        Some((f, _)) if f == id => r.lost(Axes {
                            x: None,
                            y: Some(a),
                        }),
                        _ => (TouchOutcome::Ignored, r.other_finger()),
                    },
                }
            })
            .collect()
    }

    #[test]
    fn tap_reaches_children_once() {
        let (mut r, mut a, t) = (TouchRouter::default(), axis(), Instant::now());
        let out = feed(
            &mut r,
            &mut a,
            t,
            &[(0, 1, 300.0, 0), (1, 1, 306.0, 20), (2, 1, 306.0, 60)],
        );
        assert_eq!(
            out.iter().map(|o| o.1).collect::<Vec<_>>(),
            [Forward::Pass, Forward::Pass, Forward::Pass]
        );
        assert_eq!(out[2].0, TouchOutcome::Tap);
        assert_eq!(a.offset(), 5_000.0, "inside the slop nothing scrolls");
    }

    #[test]
    fn drag_cancels_the_press_and_tracks_the_finger_1_to_1() {
        let (mut r, mut a, t) = (TouchRouter::default(), axis(), Instant::now());
        let out = feed(
            &mut r,
            &mut a,
            t,
            &[
                (0, 1, 300.0, 0),
                (1, 1, 280.0, 16),
                (1, 1, 200.0, 32),
                (2, 1, 200.0, 2_000),
            ],
        );
        assert_eq!(out[1], (TouchOutcome::ScrollStarted, Forward::Lost));
        assert_eq!(out[2], (TouchOutcome::Scrolling, Forward::Swallow));
        assert_eq!(
            out[3],
            (TouchOutcome::Ended, Forward::Swallow),
            "no click after a scroll"
        );
        // 100 px up minus the 15 px slop: content moved 85 px, no jump.
        assert_eq!(a.offset(), 5_085.0);
        assert!(!a.is_animating(), "fingers rested before lifting: no fling");
    }

    #[test]
    fn quick_lift_flings() {
        let (mut r, mut a, t) = (TouchRouter::default(), axis(), Instant::now());
        let mut steps = vec![(0u8, 1u64, 600.0f32, 0u64)];
        for i in 1..=10 {
            steps.push((1, 1, 600.0 - 20.0 * i as f32, 8 * i));
        }
        steps.push((2, 1, 400.0, 85));
        let out = feed(&mut r, &mut a, t, &steps);
        assert_eq!(out.last().unwrap().0, TouchOutcome::Ended);
        assert!(a.is_animating(), "fling started");
    }

    #[test]
    fn touch_that_stops_a_fling_does_not_click() {
        let (mut r, mut a, t) = (TouchRouter::default(), axis(), Instant::now());
        a.wheel_notches(3.0, t);
        let out = feed(&mut r, &mut a, t, &[(0, 1, 300.0, 50), (2, 1, 300.0, 120)]);
        assert_eq!(out[0].1, Forward::Pass);
        assert_eq!(
            out[1],
            (TouchOutcome::Ended, Forward::Lost),
            "the press is cancelled"
        );
        assert!(!a.is_animating());
    }

    #[test]
    fn second_finger_and_cancel() {
        let (mut r, mut a, t) = (TouchRouter::default(), axis(), Instant::now());
        let out = feed(
            &mut r,
            &mut a,
            t,
            &[
                (0, 1, 300.0, 0),
                (0, 2, 500.0, 10),
                (2, 2, 500.0, 20),
                (1, 1, 250.0, 30),
                (3, 1, 250.0, 40),
            ],
        );
        assert_eq!(
            out[1].1,
            Forward::Swallow,
            "second finger never reaches children"
        );
        assert_eq!(out[2].1, Forward::Swallow);
        assert_eq!(
            out[4],
            (TouchOutcome::Ended, Forward::Swallow),
            "cancel after scroll"
        );
        assert!(!a.is_animating());
        let out = feed(&mut r, &mut a, t, &[(0, 3, 300.0, 100), (2, 3, 300.0, 150)]);
        assert_eq!(out[1].0, TouchOutcome::Tap, "a fresh touch works again");
    }

    #[test]
    fn outside_the_viewport_passes_through() {
        let (mut r, mut a) = (TouchRouter::default(), axis());
        let o = r.press(
            1,
            (100.0, 300.0),
            false,
            Axes {
                x: None,
                y: Some(&mut a),
            },
        );
        assert_eq!(o, (TouchOutcome::Ignored, Forward::Pass));
        assert_eq!(r.tracking(), None);
    }

    #[test]
    fn stepping_aside_moves_nothing_and_lifts_quietly() {
        let (mut r, mut a, t) = (TouchRouter::default(), axis(), Instant::now());
        r.press(
            1,
            (100.0, 300.0),
            true,
            Axes {
                x: None,
                y: Some(&mut a),
            },
        );
        assert!(r.beyond_slop((100.0, 200.0)));
        r.step_aside(Axes {
            x: None,
            y: Some(&mut a),
        });
        assert_eq!(r.tracking(), Some((1, Phase::SteppedAside)));
        let o = r.lift(
            1,
            t,
            Axes {
                x: None,
                y: Some(&mut a),
            },
        );
        assert_eq!(o, (TouchOutcome::Ignored, Forward::Pass));
        assert_eq!(a.offset(), 5_000.0);
        assert!(!a.is_animating());
    }

    #[test]
    fn two_axes_pan_together() {
        let (mut r, mut x, mut y, t) = (TouchRouter::default(), axis(), axis(), Instant::now());
        r.press(
            1,
            (300.0, 300.0),
            true,
            Axes {
                x: Some(&mut x),
                y: Some(&mut y),
            },
        );
        // 60 px left and 80 px up: 100 px travel, both axes beyond the slop.
        let pos = (240.0, 220.0);
        assert!(r.beyond_slop(pos));
        r.claim(
            1,
            pos,
            t,
            Axes {
                x: Some(&mut x),
                y: Some(&mut y),
            },
        );
        r.drag(
            1,
            (140.0, 120.0),
            t + Duration::from_millis(16),
            Axes {
                x: Some(&mut x),
                y: Some(&mut y),
            },
        );
        assert_eq!(x.offset(), 5_000.0 + 45.0 + 100.0);
        assert_eq!(y.offset(), 5_000.0 + 65.0 + 100.0);
    }
}
