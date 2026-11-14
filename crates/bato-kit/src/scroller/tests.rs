use super::*;
use mouse::ScrollDelta::{Lines, Pixels};

#[test]
fn wheel_toward_user_scrolls_down() {
    // Wayland value120 = +120 (toward the user) → winit LineDelta y = -1.
    assert_eq!(
        classify(Lines { x: 0.0, y: -1.0 }),
        ScrollInput::Notches(1.0)
    );
    // Hi-res wheel: 1/8 notch.
    assert_eq!(
        classify(Lines { x: 0.0, y: 0.125 }),
        ScrollInput::Notches(-0.125)
    );
}

#[test]
fn touchpad_is_precise_and_zero_frame_ends_gesture() {
    assert_eq!(
        classify(Pixels { x: 0.0, y: -12.5 }),
        ScrollInput::Pixels(12.5)
    );
    assert_eq!(classify(Pixels { x: 0.0, y: 0.0 }), ScrollInput::End);
    // Pure horizontal motion is not the vertical axis', and is not an end.
    assert_eq!(classify(Pixels { x: 3.0, y: 0.0 }), ScrollInput::Ignore);
    assert_eq!(classify(Lines { x: 1.0, y: 0.0 }), ScrollInput::Ignore);
}

#[test]
fn both_axes_and_shift_wheel() {
    use ScrollInput::{End, Ignore, Notches};
    let px = ScrollInput::Pixels;
    assert_eq!(
        classify_xy(Lines { x: 0.0, y: -1.0 }, false),
        (Ignore, Notches(1.0))
    );
    // Shift: the vertical wheel scrolls horizontally, same direction rule.
    assert_eq!(
        classify_xy(Lines { x: 0.0, y: -1.0 }, true),
        (Notches(1.0), Ignore)
    );
    // A tilt wheel already has x; Shift changes nothing.
    assert_eq!(
        classify_xy(Lines { x: -1.0, y: 0.0 }, true),
        (Notches(1.0), Ignore)
    );
    assert_eq!(
        classify_xy(Pixels { x: -3.0, y: 2.0 }, false),
        (px(3.0), px(-2.0))
    );
    assert_eq!(classify_xy(Pixels { x: 0.0, y: 0.0 }, false), (End, End));
    assert_eq!(amount(Notches(2.0)), 2.0);
    assert_eq!(amount(End), 0.0);
}

#[test]
fn touchpad_stream_then_end_flings() {
    let t = Instant::now();
    let mut a = ScrollAxis::default();
    a.set_max(100_000.0);
    for i in 0..10u64 {
        if let ScrollInput::Pixels(px) = classify(Pixels { x: 0.0, y: -10.0 }) {
            a.precise(px, t + std::time::Duration::from_millis(i * 10));
        }
    }
    assert_eq!(classify(Pixels { x: 0.0, y: 0.0 }), ScrollInput::End);
    assert!(a.precise_end(t + std::time::Duration::from_millis(95)));
}

#[test]
fn touch_hover_follows_chromium_rules() {
    let p = Point::new(10.0, 500.0);
    let q = Point::new(10.0, 900.0);
    let area = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };
    // Finger down: touch events see it (taps must hit buttons) but nothing
    // is drawn hovered, so a starting scroll never flashes a row.
    let h = hover_after(HoverSource::Mouse, TouchOutcome::Pending, p);
    assert_eq!(h, HoverSource::TouchPending(p));
    let pending = State {
        hover: h,
        ..State::default()
    };
    assert_eq!(
        pending.content_cursor(mouse::Cursor::Available(p), area, Vector::ZERO),
        mouse::Cursor::Unavailable
    );
    // Becomes a scroll: nothing hovers while the content slides.
    let h = hover_after(h, TouchOutcome::ScrollStarted, q);
    assert_eq!(h, HoverSource::TouchNone);
    assert_eq!(
        hover_after(h, TouchOutcome::Scrolling, q),
        HoverSource::TouchNone
    );
    assert_eq!(
        hover_after(h, TouchOutcome::Ended, q),
        HoverSource::TouchNone
    );
    // A tap leaves the hover on the tapped point (content coordinates).
    assert_eq!(
        hover_after(h, TouchOutcome::Tap, q),
        HoverSource::TouchAt(q)
    );
    // Other fingers change nothing.
    assert_eq!(
        hover_after(HoverSource::TouchAt(q), TouchOutcome::Ignored, p),
        HoverSource::TouchAt(q)
    );

    let mut st = State::default();
    let at = mouse::Cursor::Available(Point::new(5.0, 5.0));
    st.hover = HoverSource::TouchNone;
    assert_eq!(
        st.content_cursor(at, area, Vector::new(0.0, 100.0)),
        mouse::Cursor::Unavailable
    );
    st.hover = HoverSource::TouchAt(q);
    assert_eq!(
        st.content_cursor(at, area, Vector::new(0.0, 100.0)),
        mouse::Cursor::Available(q)
    );
    st.hover = HoverSource::Mouse;
    assert_eq!(
        st.content_cursor(at, area, Vector::new(30.0, 100.0)),
        mouse::Cursor::Available(Point::new(35.0, 105.0))
    );
    // Outside the visible viewport (over the scrollbar, or clipped by an
    // outer scroller): the content sees no mouse.
    assert_eq!(
        st.content_cursor(
            mouse::Cursor::Available(Point::new(900.0, 5.0)),
            area,
            Vector::ZERO
        ),
        mouse::Cursor::Unavailable
    );
}

fn laid_out(content: Size, view: Size, bars: (bool, bool), inset: f32) -> State {
    let mut st = State {
        content,
        view,
        bars,
        inset,
        ..State::default()
    };
    st.x.set_max((content.width - view.width).max(0.0));
    st.y.set_max((content.height - view.height).max(0.0));
    st
}

#[test]
fn both_scrollbars_share_the_corner() {
    // A 300×200 box with a 1 px border: inner 298×198, both bars → the
    // content viewport is 293×193 and each track stops at the corner.
    let b = Rectangle {
        x: 10.0,
        y: 20.0,
        width: 300.0,
        height: 200.0,
    };
    let st = laid_out(
        Size::new(1000.0, 800.0),
        Size::new(293.0, 193.0),
        (true, true),
        1.0,
    );
    let clip = st.clip(b);
    assert_eq!(
        clip,
        Rectangle {
            x: 11.0,
            y: 21.0,
            width: 293.0,
            height: 193.0
        }
    );
    let v = st.v_thumb(b).unwrap();
    assert_eq!((v.x, v.y, v.width), (304.0, 21.0, BAR));
    assert!(v.y + v.height <= clip.y + clip.height);
    let h = st.h_thumb(b).unwrap();
    assert_eq!((h.x, h.y, h.height), (11.0, 214.0, BAR));
    assert!(h.x + h.width <= clip.x + clip.width);
    // No horizontal bar: the vertical track runs the whole height.
    let st = laid_out(
        Size::new(298.0, 800.0),
        Size::new(293.0, 198.0),
        (false, true),
        1.0,
    );
    assert_eq!(st.clip(b).height, 198.0);
    assert!(st.h_thumb(b).is_none());
}

#[test]
fn thumbs_follow_the_offsets() {
    let b = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 205.0,
        height: 105.0,
    };
    let mut st = laid_out(
        Size::new(800.0, 400.0),
        Size::new(200.0, 100.0),
        (true, true),
        0.0,
    );
    st.x.set_offset(600.0);
    st.y.set_offset(150.0);
    let h = st.h_thumb(b).unwrap();
    assert_eq!((h.x, h.width), (150.0, 50.0), "at the right end");
    let v = st.v_thumb(b).unwrap();
    assert_eq!(
        (v.y, v.height),
        (38.0, 25.0),
        "half way: (100 - 25) / 2, rounded"
    );
}

#[test]
fn can_take_respects_enabled_axes() {
    let s: Scroller<'_, ()> = scroller(iced::widget::Space::new(), Color::BLACK, Color::WHITE);
    let mut st = laid_out(
        Size::new(800.0, 400.0),
        Size::new(200.0, 100.0),
        (true, true),
        0.0,
    );
    // Vertical only: a horizontal delta is never ours.
    assert!(!s.can_take(&st, 1.0, 0.0));
    assert!(s.can_take(&st, 0.0, 1.0));
    assert!(!s.can_take(&st, 0.0, -1.0), "at the top");
    st.y.set_offset(300.0);
    assert!(!s.can_take(&st, 0.0, 1.0), "at the bottom");
    let s = s.both();
    assert!(s.can_take(&st, 1.0, 1.0), "x can still go right");
    // Touch: a mostly-horizontal swipe left (content moves right → offset
    // grows) is the x axis'; a vertical list steps aside.
    assert!(s.can_claim(&st, (-60.0, 5.0)));
    let v: Scroller<'_, ()> = scroller(iced::widget::Space::new(), Color::BLACK, Color::WHITE);
    st.y.set_offset(100.0);
    assert!(!v.can_claim(&st, (-60.0, 5.0)));
    assert!(v.can_claim(&st, (5.0, -60.0)));
}

#[test]
fn autoscroll_bus_targets_a_scroller() {
    let bus = crate::autoscroll::new_bus();
    let t = Instant::now();
    bus.borrow_mut().arm(1.0, 2.0, t);
    assert_eq!(bus.borrow().target, None, "debug arm: the bus owner");
    let bus = crate::autoscroll::new_bus();
    bus.borrow_mut().arm_for(42, 1.0, 2.0, t);
    assert_eq!(bus.borrow().target, Some(42));
    assert!(bus.borrow().auto.is_engaged());
}
