//! The pure parts of @dnd-kit, ported 1:1: collision detection
//! (`closestCenter`, `closestCorners`), the sortable strategies
//! (`verticalListSortingStrategy`, `horizontalListSortingStrategy`),
//! `arrayMove`, `sortableKeyboardCoordinates` and the auto-scroll speed
//! (`getScrollDirectionAndSpeed`). Rectangles are layout rectangles (dnd-kit
//! measures droppables ignoring their transforms).

use iced::{Point, Rectangle, Vector};

/// Which way a list runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Axis {
    #[default]
    Vertical,
    Horizontal,
}

/// `arrayMove`: move `v[from]` to `to`, shifting the items between.
pub fn array_move<T>(v: &mut Vec<T>, from: usize, to: usize) {
    if from >= v.len() || to >= v.len() || from == to {
        return;
    }
    let item = v.remove(from);
    v.insert(to, item);
}

fn center(r: &Rectangle) -> Point {
    Point::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
}

fn corners(r: &Rectangle) -> [Point; 4] {
    [
        Point::new(r.x, r.y),
        Point::new(r.x + r.width, r.y),
        Point::new(r.x, r.y + r.height),
        Point::new(r.x + r.width, r.y + r.height),
    ]
}

/// Sort `(index, value)` ascending, stable (dnd-kit's `sortCollisionsAsc`).
fn sorted(mut v: Vec<(usize, f32)>) -> Vec<usize> {
    v.sort_by(|a, b| a.1.total_cmp(&b.1));
    v.into_iter().map(|(i, _)| i).collect()
}

/// `closestCenter`: the droppables by the distance between their centre and
/// the collision rectangle's, nearest first.
pub fn closest_center<'a>(
    collision: Rectangle,
    droppables: impl IntoIterator<Item = (usize, &'a Rectangle)>,
) -> Vec<usize> {
    let c = center(&collision);
    sorted(
        droppables
            .into_iter()
            .map(|(i, r)| (i, c.distance(center(r))))
            .collect(),
    )
}

/// `closestCorners`: the droppables by the mean distance between their four
/// corners and the collision rectangle's (rounded to 4 decimals, as
/// dnd-kit's `toFixed(4)`), nearest first.
pub fn closest_corners<'a>(
    collision: Rectangle,
    droppables: impl IntoIterator<Item = (usize, &'a Rectangle)>,
) -> Vec<usize> {
    let cs = corners(&collision);
    sorted(
        droppables
            .into_iter()
            .map(|(i, r)| {
                let d: f32 = corners(r)
                    .iter()
                    .zip(&cs)
                    .map(|(a, b)| a.distance(*b))
                    .sum();
                (i, ((d / 4.0) * 10_000.0).round() / 10_000.0)
            })
            .collect(),
    )
}

/// `(start, length)` of `r` along `axis`.
fn span(r: &Rectangle, axis: Axis) -> (f32, f32) {
    match axis {
        Axis::Vertical => (r.y, r.height),
        Axis::Horizontal => (r.x, r.width),
    }
}

/// `getItemGap`: the space between item `index` and its neighbour on the
/// side the active item comes from.
fn item_gap(rects: &[Rectangle], index: usize, active: usize, axis: Axis) -> f32 {
    let gap = |a: &Rectangle, b: &Rectangle| {
        let (alo, alen) = span(a, axis);
        span(b, axis).0 - (alo + alen)
    };
    let cur = &rects[index];
    let prev = index.checked_sub(1).and_then(|i| rects.get(i));
    let next = rects.get(index + 1);
    match (prev, next) {
        (None, None) => 0.0,
        (Some(p), _) if active < index => gap(p, cur),
        (None, Some(n)) if active < index => gap(cur, n),
        (_, Some(n)) => gap(cur, n),
        (Some(p), None) => gap(p, cur),
    }
}

/// The list sorting strategies: where item `index` is displaced to while
/// `active` is over `over` (`rects` are the slots, in order).
pub fn strategy(
    rects: &[Rectangle],
    active: usize,
    over: usize,
    index: usize,
    axis: Axis,
) -> Vector {
    let (Some(a), Some(_)) = (rects.get(active), rects.get(index)) else {
        return Vector::ZERO;
    };
    let along = |v: f32| match axis {
        Axis::Vertical => Vector::new(0.0, v),
        Axis::Horizontal => Vector::new(v, 0.0),
    };
    let (alo, alen) = span(a, axis);
    if index == active {
        let Some(o) = rects.get(over) else {
            return Vector::ZERO;
        };
        let (olo, olen) = span(o, axis);
        return along(if active < over {
            olo + olen - (alo + alen)
        } else {
            olo - alo
        });
    }
    let gap = item_gap(rects, index, active, axis);
    if index > active && index <= over {
        along(-alen - gap)
    } else if index < active && index >= over {
        along(alen + gap)
    } else {
        Vector::ZERO
    }
}

/// An arrow key during a keyboard drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// `sortableKeyboardCoordinates` for one list: the slot the active item
/// (now at `collision`, over `over`) moves to with an arrow key, and the
/// new top-left of its collision rectangle. `None`: nothing that way.
pub fn keyboard_target(
    rects: &[Rectangle],
    active: usize,
    collision: Rectangle,
    over: Option<usize>,
    dir: Dir,
) -> Option<(usize, Point)> {
    let candidates: Vec<(usize, &Rectangle)> = rects
        .iter()
        .enumerate()
        .filter(|(_, r)| match dir {
            Dir::Down => collision.y < r.y,
            Dir::Up => collision.y > r.y,
            Dir::Left => collision.x > r.x,
            Dir::Right => collision.x < r.x,
        })
        .collect();
    let order = closest_corners(collision, candidates);
    let mut pick = *order.first()?;
    if Some(pick) == over && order.len() > 1 {
        pick = order[1];
    }
    let r = rects[pick];
    let after = active < pick;
    let offset = if after {
        Vector::new(collision.width - r.width, collision.height - r.height)
    } else {
        Vector::ZERO
    };
    let at = if offset.x != 0.0 && offset.y != 0.0 {
        Point::new(r.x, r.y)
    } else {
        Point::new(r.x - offset.x, r.y - offset.y)
    };
    Some((pick, at))
}

/// dnd-kit's auto-scroll interval (ms): it scrolls `speed` px per tick.
pub const EDGE_INTERVAL_MS: f32 = 5.0;
/// `acceleration`: the speed (px per tick) at the very edge.
pub const EDGE_ACCELERATION: f32 = 10.0;
/// `threshold`: the edge band, a fraction of the scroller's viewport.
pub const EDGE_THRESHOLD: f32 = 0.2;

/// `getScrollDirectionAndSpeed` on one axis: signed px per tick for a
/// pointer at `p` in a viewport `lo..lo + len` that is (or is not) already at
/// its start / end. 0 outside the bands.
pub fn edge_speed(lo: f32, len: f32, p: f32, at_start: bool, at_end: bool) -> f32 {
    let band = len * EDGE_THRESHOLD;
    if band <= 0.0 {
        return 0.0;
    }
    if !at_start && p <= lo + band {
        -EDGE_ACCELERATION * ((lo + band - p) / band).abs()
    } else if !at_end && p >= lo + len - band {
        EDGE_ACCELERATION * ((lo + len - band - p) / band).abs()
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::Size;

    fn col(hs: &[f32], gap: f32) -> Vec<Rectangle> {
        let mut y = 0.0;
        hs.iter()
            .map(|h| {
                let r = Rectangle::new(Point::new(0.0, y), Size::new(100.0, *h));
                y += h + gap;
                r
            })
            .collect()
    }

    #[test]
    fn array_move_matches_dnd_kit() {
        let mut v = vec!['a', 'b', 'c', 'd'];
        array_move(&mut v, 0, 2);
        assert_eq!(v, ['b', 'c', 'a', 'd']);
        array_move(&mut v, 3, 0);
        assert_eq!(v, ['d', 'b', 'c', 'a']);
        array_move(&mut v, 1, 9);
        assert_eq!(v, ['d', 'b', 'c', 'a']);
    }

    #[test]
    fn vertical_strategy_shifts_the_items_between() {
        let r = col(&[46.0; 4], 8.0);
        // Item 0 dragged over item 2: 1 and 2 move up by 46 + 8, 3 stays,
        // the active slot moves down to 2's.
        assert_eq!(
            strategy(&r, 0, 2, 1, Axis::Vertical),
            Vector::new(0.0, -54.0)
        );
        assert_eq!(
            strategy(&r, 0, 2, 2, Axis::Vertical),
            Vector::new(0.0, -54.0)
        );
        assert_eq!(strategy(&r, 0, 2, 3, Axis::Vertical), Vector::ZERO);
        assert_eq!(
            strategy(&r, 0, 2, 0, Axis::Vertical),
            Vector::new(0.0, 108.0)
        );
        // Upwards: 3 over 1.
        assert_eq!(
            strategy(&r, 3, 1, 1, Axis::Vertical),
            Vector::new(0.0, 54.0)
        );
        assert_eq!(strategy(&r, 3, 1, 0, Axis::Vertical), Vector::ZERO);
        assert_eq!(
            strategy(&r, 3, 1, 3, Axis::Vertical),
            Vector::new(0.0, -108.0)
        );
    }

    #[test]
    fn horizontal_strategy_uses_widths() {
        let r = vec![
            Rectangle::new(Point::new(0.0, 0.0), Size::new(133.0, 36.0)),
            Rectangle::new(Point::new(133.0, 0.0), Size::new(141.0, 36.0)),
            Rectangle::new(Point::new(274.0, 0.0), Size::new(158.0, 36.0)),
        ];
        assert_eq!(
            strategy(&r, 0, 2, 1, Axis::Horizontal),
            Vector::new(-133.0, 0.0)
        );
        // The active one lands with its right edge on the last one's.
        assert_eq!(
            strategy(&r, 0, 2, 0, Axis::Horizontal),
            Vector::new(299.0, 0.0)
        );
        assert_eq!(
            strategy(&r, 2, 0, 0, Axis::Horizontal),
            Vector::new(158.0, 0.0)
        );
        assert_eq!(
            strategy(&r, 2, 0, 2, Axis::Horizontal),
            Vector::new(-274.0, 0.0)
        );
    }

    #[test]
    fn collisions() {
        let r = col(&[46.0; 4], 8.0);
        let moved = Rectangle::new(Point::new(5.0, 80.0), Size::new(100.0, 46.0));
        assert_eq!(closest_center(moved, r.iter().enumerate())[0], 1);
        assert_eq!(closest_corners(moved, r.iter().enumerate())[..2], [1, 2]);
        // A tie keeps the first.
        let mid = Rectangle::new(Point::new(0.0, 27.0), Size::new(100.0, 46.0));
        assert_eq!(closest_center(mid, r.iter().enumerate())[0], 0);
    }

    #[test]
    fn keyboard_moves_one_slot() {
        let r = col(&[46.0; 4], 8.0);
        // Picked up at 1: Down goes to 2, aligned on its top.
        let (i, at) = keyboard_target(&r, 1, r[1], Some(1), Dir::Down).unwrap();
        assert_eq!((i, at), (2, Point::new(0.0, 108.0)));
        // From there, Down again.
        let c = Rectangle { y: 108.0, ..r[1] };
        assert_eq!(keyboard_target(&r, 1, c, Some(2), Dir::Down).unwrap().0, 3);
        // Up from the original slot: 0.
        assert_eq!(keyboard_target(&r, 1, r[1], Some(1), Dir::Up).unwrap().0, 0);
        // Nothing below the last, nothing sideways.
        assert!(keyboard_target(&r, 3, r[3], Some(3), Dir::Down).is_none());
        assert!(keyboard_target(&r, 1, r[1], Some(1), Dir::Right).is_none());
        // Different heights: after the active one, bottoms align.
        let r = col(&[46.0, 66.0], 8.0);
        let (_, at) = keyboard_target(&r, 0, r[0], Some(0), Dir::Down).unwrap();
        assert_eq!(at, Point::new(0.0, 54.0 + 20.0));
    }

    #[test]
    fn edge_speed_ramps_inside_the_band() {
        // A 500 px viewport: 100 px bands.
        assert_eq!(edge_speed(0.0, 500.0, 250.0, false, false), 0.0);
        assert_eq!(edge_speed(0.0, 500.0, 0.0, false, false), -10.0);
        assert_eq!(edge_speed(0.0, 500.0, 50.0, false, false), -5.0);
        assert_eq!(edge_speed(0.0, 500.0, 450.0, false, false), 5.0);
        assert_eq!(edge_speed(0.0, 500.0, 520.0, false, false), 12.0);
        // Already at the start: nothing backwards.
        assert_eq!(edge_speed(0.0, 500.0, 10.0, true, false), 0.0);
        assert_eq!(edge_speed(0.0, 500.0, 490.0, false, true), 0.0);
    }
}
