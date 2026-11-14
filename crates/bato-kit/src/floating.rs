//! Floating positioning (INF-3): a port of `lib/floating.ts`, pure rectangle
//! arithmetic against the window, in CSS px.
//!
//! - [`compute_position`]: place a floating element on a side of an anchor
//!   (`placement`), aligned along it (`align`), `offset` px away; **flip** to
//!   the opposite side when the preferred one lacks room *and* the opposite
//!   one has it; then **shift** it back inside the window with
//!   [`clamp_to_viewport`] (`padding` px from every edge). Popover, Tooltip,
//!   Combobox, the pickers.
//! - [`clamp_to_viewport`]: nudge a top-left so a box of the measured size
//!   stays on screen. ContextMenu (at the pointer) and Dropdown (under the
//!   trigger) use it directly: they never flip.
//!
//! `floating.ts` has no arrow and no size middleware (no max-height when
//! space is short): an element larger than the window is pinned at the
//! top/left padding and overflows the far edge, exactly like React.

use iced::{Point, Rectangle, Size};

/// The side of the anchor the floating element goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Placement {
    #[default]
    Bottom,
    Top,
    Left,
    Right,
}

impl Placement {
    pub fn opposite(self) -> Self {
        match self {
            Placement::Bottom => Placement::Top,
            Placement::Top => Placement::Bottom,
            Placement::Left => Placement::Right,
            Placement::Right => Placement::Left,
        }
    }
    pub fn is_vertical(self) -> bool {
        matches!(self, Placement::Bottom | Placement::Top)
    }
}

/// Cross-axis alignment along the anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

/// `PositionOptions` with React's defaults (bottom, start, offset 6,
/// padding 8).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    pub placement: Placement,
    pub align: Align,
    /// Gap between the anchor and the floating element.
    pub offset: f32,
    /// Minimum distance from the window edges.
    pub padding: f32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            placement: Placement::Bottom,
            align: Align::Start,
            offset: 6.0,
            padding: 8.0,
        }
    }
}

impl Options {
    pub fn placement(mut self, p: Placement) -> Self {
        self.placement = p;
        self
    }
    pub fn align(mut self, a: Align) -> Self {
        self.align = a;
        self
    }
    pub fn offset(mut self, o: f32) -> Self {
        self.offset = o;
        self
    }
    pub fn padding(mut self, p: f32) -> Self {
        self.padding = p;
        self
    }
}

/// Where the floating element goes: its top-left, and the side it ended up
/// on after flipping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coords {
    pub x: f32,
    pub y: f32,
    pub placement: Placement,
}

impl Coords {
    pub fn point(&self) -> Point {
        Point::new(self.x, self.y)
    }
}

/// `clampToViewport`: keep a `width`×`height` box whose desired top-left is
/// (`x`, `y`) inside `viewport`, `margin` px from the edges. When it can't
/// fit, the top/left edge wins.
pub fn clamp_to_viewport(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    margin: f32,
    viewport: Size,
) -> Point {
    let (vw, vh) = (viewport.width, viewport.height);
    let mut nx = x;
    let mut ny = y;
    if x + width > vw - margin {
        nx = margin.max(vw - width - margin);
    }
    if y + height > vh - margin {
        ny = margin.max(vh - height - margin);
    }
    Point::new(margin.max(nx), margin.max(ny))
}

/// `computePosition`: place `floating` against `anchor` (window
/// coordinates), flipping to the opposite side when there isn't room, then
/// clamping to the window.
pub fn compute_position(
    anchor: Rectangle,
    floating: Size,
    opts: Options,
    viewport: Size,
) -> Coords {
    let (vw, vh) = (viewport.width, viewport.height);
    let (fw, fh) = (floating.width, floating.height);
    let Options {
        placement,
        align,
        offset,
        padding,
    } = opts;
    let (left, top) = (anchor.x, anchor.y);
    let (right, bottom) = (anchor.x + anchor.width, anchor.y + anchor.height);

    // Flip on the main axis if the preferred side lacks space.
    let mut side = placement;
    if placement == Placement::Bottom
        && bottom + offset + fh > vh - padding
        && top - offset - fh > padding
    {
        side = Placement::Top;
    }
    if placement == Placement::Top
        && top - offset - fh < padding
        && bottom + offset + fh < vh - padding
    {
        side = Placement::Bottom;
    }
    if placement == Placement::Right
        && right + offset + fw > vw - padding
        && left - offset - fw > padding
    {
        side = Placement::Left;
    }
    if placement == Placement::Left
        && left - offset - fw < padding
        && right + offset + fw < vw - padding
    {
        side = Placement::Right;
    }

    let (x, y) = if side.is_vertical() {
        let y = if side == Placement::Bottom {
            bottom + offset
        } else {
            top - offset - fh
        };
        let x = match align {
            Align::Start => left,
            Align::End => right - fw,
            Align::Center => left + anchor.width / 2.0 - fw / 2.0,
        };
        (x, y)
    } else {
        let x = if side == Placement::Right {
            right + offset
        } else {
            left - offset - fw
        };
        let y = match align {
            Align::Start => top,
            Align::End => bottom - fh,
            Align::Center => top + anchor.height / 2.0 - fh / 2.0,
        };
        (x, y)
    };

    let p = clamp_to_viewport(x, y, fw, fh, padding, viewport);
    Coords {
        x: p.x,
        y: p.y,
        placement: side,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VP: Size = Size {
        width: 1280.0,
        height: 800.0,
    };

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rectangle {
        Rectangle::new(Point::new(x, y), Size::new(w, h))
    }

    fn at(c: Coords) -> (f32, f32, Placement) {
        (c.x, c.y, c.placement)
    }

    // The cases below are what `computePosition(anchor, size, opts)` returns
    // in the browser for the same numbers (window 1280×800).

    #[test]
    fn defaults_bottom_start_offset_6() {
        let c = compute_position(
            r(100.0, 100.0, 80.0, 30.0),
            Size::new(200.0, 150.0),
            Options::default(),
            VP,
        );
        assert_eq!(at(c), (100.0, 136.0, Placement::Bottom));
    }

    #[test]
    fn bottom_flips_to_top_near_the_bottom_edge() {
        // 730 + 6 + 150 = 886 > 792 and 700 - 6 - 150 = 544 > 8.
        let c = compute_position(
            r(100.0, 700.0, 80.0, 30.0),
            Size::new(200.0, 150.0),
            Options::default(),
            VP,
        );
        assert_eq!(at(c), (100.0, 544.0, Placement::Top));
    }

    #[test]
    fn no_flip_when_the_other_side_is_short_too_then_clamp() {
        // Window 280 tall: neither side fits a 150 tall element.
        let vp = Size::new(1280.0, 280.0);
        let c = compute_position(
            r(100.0, 100.0, 80.0, 30.0),
            Size::new(200.0, 150.0),
            Options::default(),
            vp,
        );
        // Stays bottom (136), then 136 + 150 > 272 → max(8, 280 - 150 - 8).
        assert_eq!(at(c), (100.0, 122.0, Placement::Bottom));
    }

    #[test]
    fn top_flips_to_bottom_near_the_top_edge() {
        let c = compute_position(
            r(400.0, 40.0, 100.0, 32.0),
            Size::new(120.0, 24.0),
            Options::default().placement(Placement::Top).offset(8.0),
            VP,
        );
        // 40 - 8 - 24 = 8, not < 8: no flip (strict comparison, like JS).
        assert_eq!(at(c), (400.0, 8.0, Placement::Top));
        let c = compute_position(
            r(400.0, 30.0, 100.0, 32.0),
            Size::new(120.0, 24.0),
            Options::default().placement(Placement::Top).offset(8.0),
            VP,
        );
        assert_eq!(at(c), (400.0, 70.0, Placement::Bottom));
    }

    #[test]
    fn right_flips_left_and_left_flips_right() {
        let c = compute_position(
            r(1150.0, 300.0, 100.0, 40.0),
            Size::new(160.0, 40.0),
            Options::default()
                .placement(Placement::Right)
                .align(Align::Center),
            VP,
        );
        assert_eq!(at(c), (1150.0 - 6.0 - 160.0, 300.0, Placement::Left));
        let c = compute_position(
            r(20.0, 300.0, 100.0, 40.0),
            Size::new(160.0, 40.0),
            Options::default()
                .placement(Placement::Left)
                .align(Align::Center),
            VP,
        );
        assert_eq!(at(c), (126.0, 300.0, Placement::Right));
    }

    #[test]
    fn cross_axis_alignment() {
        let a = r(500.0, 100.0, 100.0, 30.0);
        let s = Size::new(60.0, 20.0);
        let o = Options::default();
        assert_eq!(compute_position(a, s, o.align(Align::Start), VP).x, 500.0);
        assert_eq!(compute_position(a, s, o.align(Align::Center), VP).x, 520.0);
        assert_eq!(compute_position(a, s, o.align(Align::End), VP).x, 540.0);
        let side = o.placement(Placement::Right);
        let a = r(500.0, 100.0, 30.0, 100.0);
        assert_eq!(
            compute_position(a, s, side.align(Align::Start), VP).y,
            100.0
        );
        assert_eq!(
            compute_position(a, s, side.align(Align::Center), VP).y,
            140.0
        );
        assert_eq!(compute_position(a, s, side.align(Align::End), VP).y, 180.0);
    }

    #[test]
    fn shift_keeps_the_padding_on_the_cross_axis() {
        // Centered tooltip on an anchor at the left edge: x would be -40.
        let c = compute_position(
            r(0.0, 300.0, 40.0, 40.0),
            Size::new(120.0, 24.0),
            Options::default()
                .placement(Placement::Top)
                .align(Align::Center)
                .offset(8.0),
            VP,
        );
        assert_eq!(at(c), (8.0, 268.0, Placement::Top));
        // End-aligned at the right edge: 1270 + 10 - … stays ≤ 1280 - 8.
        let c = compute_position(
            r(1200.0, 300.0, 78.0, 30.0),
            Size::new(200.0, 100.0),
            Options::default(),
            VP,
        );
        assert_eq!(at(c), (1072.0, 336.0, Placement::Bottom));
    }

    #[test]
    fn larger_than_the_window_pins_top_left() {
        let c = compute_position(
            r(10.0, 10.0, 10.0, 10.0),
            Size::new(1400.0, 900.0),
            Options::default(),
            VP,
        );
        assert_eq!(at(c), (8.0, 8.0, Placement::Bottom));
    }

    #[test]
    fn clamp_at_the_pointer() {
        // ContextMenu near the bottom-right corner.
        let p = clamp_to_viewport(1250.0, 780.0, 192.0, 170.0, 8.0, VP);
        assert_eq!(p, Point::new(1080.0, 622.0));
        // In the open: unchanged.
        let p = clamp_to_viewport(300.0, 200.0, 192.0, 170.0, 8.0, VP);
        assert_eq!(p, Point::new(300.0, 200.0));
        // Negative coordinates come back to the margin.
        let p = clamp_to_viewport(-5.0, 2.0, 100.0, 100.0, 8.0, VP);
        assert_eq!(p, Point::new(8.0, 8.0));
    }
}
