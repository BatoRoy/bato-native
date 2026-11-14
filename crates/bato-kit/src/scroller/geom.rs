//! Pure scroll geometry: scrollbar thumbs, scroll-into-view, the edge-fade
//! mask, and "can this scroller take a delta in that direction" (Chromium's
//! latching test). No iced state, so all of it is unit-tested.

/// Scrollbar thickness (`::-webkit-scrollbar { width: 5px; height: 5px }`).
pub const BAR: f32 = 5.0;

/// Edge-fade length (`Scrollable.tsx`: `#000 16px … calc(100% - 16px)`).
pub const FADE: f32 = 16.0;

/// Which axes scroll (`Scrollable`'s `axis` prop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Axis {
    /// `overflow-y: auto; overflow-x: hidden` (the default, and the page).
    #[default]
    Vertical,
    /// `overflow-x: auto; overflow-y: hidden`.
    Horizontal,
    /// `overflow: auto`.
    Both,
}

impl Axis {
    pub fn x(self) -> bool {
        !matches!(self, Axis::Vertical)
    }
    pub fn y(self) -> bool {
        !matches!(self, Axis::Horizontal)
    }
}

/// Where scroll-into-view puts the target (CSS `scrollIntoView` `block` /
/// `inline`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    /// Only if needed, by the least movement (`nearest`; what focus does).
    #[default]
    Nearest,
    Start,
    Center,
    End,
}

/// Thumb position and length along a track, in logical px, snapped to device
/// pixels (`s` = device scale). `track` is the track length, `view` the
/// visible length, `content` the content length (all logical);
/// `offset`/`max` are the axis' device px. Chromium's minimum thumb length
/// for a 5 px custom scrollbar is about twice the thickness.
pub fn thumb(track: f32, view: f32, content: f32, offset: f32, max: f32, s: f32) -> (f32, f32) {
    let track_d = track * s;
    let len = (track_d * view / content.max(1.0))
        .round()
        .max(2.0 * BAR * s)
        .min(track_d);
    let pos = ((track_d - len) * offset / max.max(1.0)).round();
    (pos / s, len / s)
}

/// The axis offset (device px) for a thumb dragged from `start` by `moved`
/// logical px, on a track of `track` logical px with a thumb of `len`.
pub fn drag_offset(start: f32, moved: f32, track: f32, len: f32, max: f32, s: f32) -> f32 {
    let free = track - len;
    if free <= 0.0 {
        return start;
    }
    start + moved * s * max / (free * s)
}

/// Chromium's page step: 87.5 % of the visible length.
pub fn page_step(view: f32) -> f32 {
    view * 0.875
}

/// The offset (same units as the inputs) that shows `[lo, hi)` — content
/// coordinates — in a viewport of length `view` currently at `cur`, per CSS
/// `scrollIntoView` alignment, with `margin` around the target, clamped to
/// `0..=max`.
pub fn into_view(cur: f32, view: f32, max: f32, lo: f32, hi: f32, align: Align, margin: f32) -> f32 {
    let (a, b) = (lo - margin, hi + margin);
    let size = b - a;
    let end = cur + view;
    let to = match align {
        Align::Start => a,
        Align::End => b - view,
        Align::Center => (lo + hi) / 2.0 - view / 2.0,
        // CSSOM View "nearest": both edges outside → stay; otherwise align the
        // edge that needs the least movement (the larger-than-viewport cases
        // flip, as the spec says).
        Align::Nearest => {
            if a < cur && b > end {
                cur
            } else if (a < cur && size <= view) || (b > end && size > view) {
                a
            } else if (b > end && size <= view) || (a < cur && size > view) {
                b - view
            } else {
                cur
            }
        }
    };
    to.clamp(0.0, max.max(0.0))
}

/// Mask alpha (0 = faded out, 1 = opaque) at `d` logical px from the start
/// of a scroller `len` long: `linear-gradient(transparent, #000 16px, #000
/// calc(100% - 16px), transparent)`. Colour stops that go backwards are
/// clamped to the previous one, as CSS does, so short boxes fade in for
/// 16 px and out over the rest.
pub fn fade_alpha(d: f32, len: f32) -> f32 {
    let p1 = FADE.min(len);
    let p2 = (len - FADE).max(p1);
    let a = if d <= p1 {
        if p1 <= 0.0 { 1.0 } else { d / p1 }
    } else if d >= p2 {
        if len - p2 <= 0.0 {
            1.0
        } else {
            (len - d) / (len - p2)
        }
    } else {
        1.0
    };
    a.clamp(0.0, 1.0)
}

/// Can an axis at `offset` of `max` (device px) take `delta` (positive =
/// content moves up/left)? Chromium latches a gesture to the innermost
/// scroller that can; at an edge the gesture goes to the parent.
pub fn can_take(offset: f32, max: f32, delta: f32) -> bool {
    const EPS: f32 = 0.5;
    (delta > 0.0 && offset < max - EPS) || (delta < 0.0 && offset > EPS)
}

/// A touch drag's component on one axis counts for latching when it is at
/// least half the other one (within ~63° of the axis): a mostly horizontal
/// swipe over a vertical list goes to a horizontal parent.
pub fn counts(along: f32, across: f32) -> bool {
    along != 0.0 && along.abs() * 2.0 >= across.abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_matches_the_page_scroller() {
        // 600 px viewport over 2400 px content at scale 1, offset at middle.
        let (pos, len) = thumb(600.0, 600.0, 2400.0, 900.0, 1800.0, 1.0);
        assert_eq!(len, 150.0);
        assert_eq!(pos, 225.0);
        // Minimum length: twice the thickness.
        let (_, len) = thumb(100.0, 100.0, 100_000.0, 0.0, 99_900.0, 1.0);
        assert_eq!(len, 10.0);
        // Device-pixel snapping at 1.5x.
        let (pos, len) = thumb(101.0, 101.0, 303.0, 101.0, 303.0, 1.5);
        assert_eq!((pos * 1.5).fract(), 0.0);
        assert_eq!((len * 1.5).fract(), 0.0);
    }

    #[test]
    fn thumb_drag_maps_track_to_range() {
        // Free track 450 logical px for a max of 1800 device px: 4 px per px.
        assert_eq!(drag_offset(0.0, 10.0, 600.0, 150.0, 1800.0, 1.0), 40.0);
        assert_eq!(drag_offset(0.0, 10.0, 600.0, 150.0, 3600.0, 2.0), 80.0);
        assert_eq!(drag_offset(7.0, 10.0, 100.0, 100.0, 50.0, 1.0), 7.0);
    }

    #[test]
    fn into_view_nearest_follows_cssom() {
        // Visible: no move.
        assert_eq!(into_view(100.0, 200.0, 1000.0, 150.0, 180.0, Align::Nearest, 0.0), 100.0);
        // Above: align start; below: align end.
        assert_eq!(into_view(100.0, 200.0, 1000.0, 40.0, 60.0, Align::Nearest, 0.0), 40.0);
        assert_eq!(into_view(100.0, 200.0, 1000.0, 400.0, 450.0, Align::Nearest, 0.0), 250.0);
        // Larger than the viewport and covering it: stay.
        assert_eq!(into_view(100.0, 200.0, 1000.0, 50.0, 500.0, Align::Nearest, 0.0), 100.0);
        // Larger, below: align its start (shows as much of it as possible).
        assert_eq!(into_view(100.0, 200.0, 1000.0, 250.0, 800.0, Align::Nearest, 0.0), 250.0);
        // Margins and clamping.
        assert_eq!(into_view(100.0, 200.0, 1000.0, 40.0, 60.0, Align::Nearest, 8.0), 32.0);
        assert_eq!(into_view(100.0, 200.0, 1000.0, 2.0, 20.0, Align::Nearest, 8.0), 0.0);
        assert_eq!(into_view(0.0, 200.0, 300.0, 480.0, 500.0, Align::Nearest, 0.0), 300.0);
    }

    #[test]
    fn into_view_explicit_alignments() {
        assert_eq!(into_view(0.0, 200.0, 1000.0, 500.0, 520.0, Align::Start, 0.0), 500.0);
        assert_eq!(into_view(0.0, 200.0, 1000.0, 500.0, 520.0, Align::End, 0.0), 320.0);
        assert_eq!(into_view(0.0, 200.0, 1000.0, 500.0, 520.0, Align::Center, 0.0), 410.0);
        assert_eq!(into_view(0.0, 200.0, 900.0, 990.0, 1200.0, Align::Start, 0.0), 900.0);
    }

    #[test]
    fn fade_mask_is_16px_ramps() {
        assert_eq!(fade_alpha(0.0, 150.0), 0.0);
        assert_eq!(fade_alpha(8.0, 150.0), 0.5);
        assert_eq!(fade_alpha(16.0, 150.0), 1.0);
        assert_eq!(fade_alpha(75.0, 150.0), 1.0);
        assert_eq!(fade_alpha(142.0, 150.0), 0.5);
        assert_eq!(fade_alpha(150.0, 150.0), 0.0);
        // Short box: in over 16 px, out over the rest.
        assert_eq!(fade_alpha(16.0, 24.0), 1.0);
        assert_eq!(fade_alpha(20.0, 24.0), 0.5);
    }

    #[test]
    fn latching_direction_tests() {
        assert!(can_take(0.0, 100.0, 1.0));
        assert!(!can_take(0.0, 100.0, -1.0));
        assert!(!can_take(100.0, 100.0, 1.0));
        assert!(can_take(100.0, 100.0, -1.0));
        assert!(!can_take(0.0, 0.0, 1.0), "nothing to scroll");
        assert!(counts(20.0, 5.0));
        assert!(counts(10.0, 10.0), "diagonal counts on both axes");
        assert!(!counts(4.0, 20.0));
        assert!(!counts(0.0, 0.0));
    }
}
