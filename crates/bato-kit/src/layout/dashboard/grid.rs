//! The Dashboard's pure grid maths, ported 1:1 from `layout/Dashboard.tsx`:
//! `collides`, `compact`, `clamp`, the move / resize `apply`s, the cell
//! metrics and the pointer-delta rounding.

use iced::Rectangle;
use serde::{Deserialize, Serialize};

/// `GAP`: px between columns and rows.
pub const GAP: f32 = 10.0;
/// `cols` default.
pub const COLS: i32 = 12;
/// `rowHeight` default (the Showcase passes 56).
pub const ROW_HEIGHT: f32 = 64.0;
/// The grid's `minHeight` when it has no rows.
pub const EMPTY_HEIGHT: f32 = 120.0;

/// `DashboardItem`: a widget on the grid, in cells. Serialised exactly like
/// React's (`type` for the kind), so a stored layout reads either way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardItem {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl DashboardItem {
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            x,
            y,
            w,
            h,
        }
    }
}

/// `collides(a, b)`: two different items overlap.
pub fn collides(a: &DashboardItem, b: &DashboardItem) -> bool {
    a.id != b.id && a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
}

/// `compact(items)`: vertical compaction. Items in (y, x) order (stable)
/// each float up to the smallest row where they collide with nothing
/// placed before them.
pub fn compact(items: &[DashboardItem]) -> Vec<DashboardItem> {
    let mut sorted = items.to_vec();
    sorted.sort_by(|a, b| a.y.cmp(&b.y).then(a.x.cmp(&b.x)));
    let mut placed: Vec<DashboardItem> = Vec::with_capacity(sorted.len());
    for it in sorted {
        let mut probe = DashboardItem { y: 0, ..it };
        while placed.iter().any(|p| collides(&probe, p)) {
            probe.y += 1;
        }
        placed.push(probe);
    }
    placed
}

/// `clamp(n, lo, hi)` = `Math.max(lo, Math.min(hi, n))` (`lo` wins when
/// `hi < lo`).
pub fn clamp(n: i32, lo: i32, hi: i32) -> i32 {
    n.min(hi).max(lo)
}

/// What a gesture changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// Dragging the header: `x`, `y`.
    Move,
    /// Dragging the bottom-right corner: `w`, `h`.
    Resize,
}

/// `onDrag` / `onResize`'s `apply`: `it` moved or resized by whole cells.
pub fn apply(it: &DashboardItem, handle: Handle, dx: i32, dy: i32, cols: i32) -> DashboardItem {
    let mut n = it.clone();
    match handle {
        Handle::Move => {
            n.x = clamp(it.x + dx, 0, cols - it.w);
            n.y = (it.y + dy).max(0);
        }
        Handle::Resize => {
            n.w = clamp(it.w + dx, 1, cols - it.x);
            n.h = (it.h + dy).max(1);
        }
    }
    n
}

/// The rows in use: the lowest item bottom (`addWidget`'s `maxY`).
pub fn rows(items: &[DashboardItem]) -> i32 {
    items.iter().map(|it| it.y + it.h).max().unwrap_or(0).max(0)
}

/// `Math.round(px / cell)`: halves round up (towards +∞), as in JS.
pub fn cells(px: f32, cell: f32) -> i32 {
    if cell <= 0.0 {
        return 0;
    }
    (px / cell + 0.5).floor() as i32
}

/// The cell geometry for a grid `width` px wide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub cols: i32,
    pub row_height: f32,
    pub width: f32,
}

impl Metrics {
    /// `colW`: 0 until the grid has a width.
    pub fn col_w(&self) -> f32 {
        if self.width > 0.0 {
            (self.width - (self.cols - 1) as f32 * GAP) / self.cols as f32
        } else {
            0.0
        }
    }
    pub fn cell_w(&self) -> f32 {
        self.col_w() + GAP
    }
    pub fn cell_h(&self) -> f32 {
        self.row_height + GAP
    }
    /// The item's box (`left`, `top`, `width`, `height`).
    pub fn rect(&self, it: &DashboardItem) -> Rectangle {
        let col_w = self.col_w();
        Rectangle {
            x: it.x as f32 * self.cell_w(),
            y: it.y as f32 * self.cell_h(),
            width: it.w as f32 * col_w + (it.w - 1) as f32 * GAP,
            height: pixel_height(it.h, self.row_height),
        }
    }
    /// The grid's `minHeight`: the rows, or 120 px when empty.
    pub fn height(&self, items: &[DashboardItem]) -> f32 {
        let min = rows(items) as f32 * self.cell_h() - GAP;
        if min > 0.0 { min } else { EMPTY_HEIGHT }
    }
}

/// `h * rowHeight + (h - 1) * GAP`.
pub fn pixel_height(h: i32, row_height: f32) -> f32 {
    h as f32 * row_height + (h - 1) as f32 * GAP
}

/// The widget header: `py-1.5` around a 16 px line, plus its `border-b`.
pub const HEADER: f32 = 29.0;
/// The body's `p-3`.
pub const BODY_PAD: f32 = 12.0;

/// The height of a widget body's content box (inside `p-3`) for an item
/// `h` rows tall: the box minus its 1 px borders, the header and the
/// padding.
pub fn content_height(h: i32, row_height: f32) -> f32 {
    pixel_height(h, row_height) - 2.0 - HEADER - 2.0 * BODY_PAD
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(id: &str, x: i32, y: i32, w: i32, h: i32) -> DashboardItem {
        DashboardItem::new(id, "stat", x, y, w, h)
    }

    fn xywh(items: &[DashboardItem]) -> Vec<(&str, i32, i32, i32, i32)> {
        items
            .iter()
            .map(|i| (i.id.as_str(), i.x, i.y, i.w, i.h))
            .collect()
    }

    /// The Showcase's `defaultItems`.
    fn showcase() -> Vec<DashboardItem> {
        vec![
            it("d1", 0, 0, 3, 2),
            it("d2", 3, 0, 6, 4),
            it("d3", 9, 0, 3, 4),
            it("d4", 0, 2, 3, 2),
        ]
    }

    #[test]
    fn collides_is_strict_overlap_of_different_ids() {
        let a = it("a", 0, 0, 3, 2);
        assert!(collides(&a, &it("b", 2, 1, 3, 2)));
        assert!(!collides(&a, &it("b", 3, 0, 3, 2)), "touching edges");
        assert!(!collides(&a, &it("b", 0, 2, 3, 2)), "touching rows");
        assert!(
            !collides(&a, &it("a", 0, 0, 3, 2)),
            "same id never collides"
        );
        assert!(collides(&a, &it("b", 1, 1, 1, 1)), "contained");
    }

    #[test]
    fn showcase_layout_is_already_compact() {
        assert_eq!(compact(&showcase()), {
            // Sorted by (y, x): d1, d2, d3, d4.
            showcase()
        });
    }

    #[test]
    fn compact_floats_items_up_in_y_x_order() {
        let items = vec![
            it("a", 0, 5, 3, 2),
            it("b", 3, 9, 3, 2),
            it("c", 0, 8, 3, 1),
        ];
        assert_eq!(
            xywh(&compact(&items)),
            vec![("a", 0, 0, 3, 2), ("c", 0, 2, 3, 1), ("b", 3, 0, 3, 2)]
        );
    }

    #[test]
    fn compact_ties_keep_input_order() {
        // Same (y, x): the stable sort keeps a before b, so b goes under a.
        let items = vec![it("a", 0, 1, 2, 2), it("b", 0, 1, 2, 1)];
        assert_eq!(
            xywh(&compact(&items)),
            vec![("a", 0, 0, 2, 2), ("b", 0, 2, 2, 1)]
        );
        let swapped = vec![items[1].clone(), items[0].clone()];
        assert_eq!(
            xywh(&compact(&swapped)),
            vec![("b", 0, 0, 2, 1), ("a", 0, 1, 2, 2)]
        );
    }

    #[test]
    fn dropped_item_y_decides_where_it_lands() {
        // Drag d4 (0,2) up over d1 (0,0) to y 0: (0,0) ties with d1 and the
        // stable sort keeps d1 first, so d4 lands under it again.
        let mut items = showcase();
        items[3] = apply(&items[3], Handle::Move, 0, -2, COLS);
        assert_eq!((items[3].x, items[3].y), (0, 0));
        let c = compact(&items);
        assert_eq!(xywh(&c)[0], ("d1", 0, 0, 3, 2));
        assert_eq!(xywh(&c)[1], ("d4", 0, 2, 3, 2));
        // Drag d1 down to y 3 (below d4's top): d4 floats to 0, d1 under it.
        let mut items = showcase();
        items[0] = apply(&items[0], Handle::Move, 0, 3, COLS);
        assert_eq!(
            xywh(&compact(&items)),
            vec![
                ("d2", 3, 0, 6, 4),
                ("d3", 9, 0, 3, 4),
                ("d4", 0, 0, 3, 2),
                ("d1", 0, 2, 3, 2)
            ]
        );
    }

    #[test]
    fn drop_onto_another_widget_pushes_the_later_one_down() {
        // d1 dragged right by 4 cells onto the chart (x 4, y 0): sorted
        // d2 (3,0), d1 (4,0), d3, d4 → d2 stays, d1 goes below the chart.
        let mut items = showcase();
        items[0] = apply(&items[0], Handle::Move, 4, 0, COLS);
        let c = compact(&items);
        assert_eq!(
            xywh(&c),
            vec![
                ("d2", 3, 0, 6, 4),
                ("d1", 4, 4, 3, 2),
                ("d3", 9, 0, 3, 4),
                ("d4", 0, 0, 3, 2)
            ]
        );
    }

    #[test]
    fn move_clamps_to_the_grid() {
        let a = it("a", 9, 2, 3, 2);
        let m = |dx, dy| {
            let n = apply(&a, Handle::Move, dx, dy, COLS);
            (n.x, n.y, n.w, n.h)
        };
        assert_eq!(m(5, 0), (9, 2, 3, 2), "right edge: cols - w");
        assert_eq!(m(-20, -9), (0, 0, 3, 2), "left / top");
        assert_eq!(m(-2, 7), (7, 9, 3, 2), "no bottom limit");
    }

    #[test]
    fn resize_clamps_to_one_cell_and_the_right_edge() {
        let a = it("a", 9, 2, 3, 2);
        let r = |dx, dy| {
            let n = apply(&a, Handle::Resize, dx, dy, COLS);
            (n.x, n.y, n.w, n.h)
        };
        assert_eq!(r(4, 3), (9, 2, 3, 5), "w ≤ cols - x");
        assert_eq!(r(-9, -9), (9, 2, 1, 1), "at least 1 × 1");
        assert_eq!(r(-1, 1), (9, 2, 2, 3));
    }

    #[test]
    fn clamp_is_js_max_of_min() {
        assert_eq!(clamp(5, 0, 3), 3);
        assert_eq!(clamp(-1, 0, 3), 0);
        assert_eq!(clamp(2, 4, 1), 4, "lo wins when hi < lo");
    }

    #[test]
    fn cells_round_like_math_round() {
        let cell = 75.833_336; // (806 - 110) / 12 + 10
        assert_eq!(cells(37.0, cell), 0);
        assert_eq!(cells(38.0, cell), 1);
        assert_eq!(cells(-37.0, cell), 0);
        assert_eq!(cells(-38.0, cell), -1);
        assert_eq!(cells(-33.0, 66.0), 0, "-0.5 rounds up to -0");
        assert_eq!(cells(33.0, 66.0), 1, "0.5 rounds up to 1");
        assert_eq!(cells(-99.0, 66.0), -1, "-1.5 rounds up to -1");
        assert_eq!(cells(10.0, 0.0), 0);
    }

    #[test]
    fn metrics_match_the_showcase_grid() {
        // The content column is 848 px wide in a 1280 px window.
        let m = Metrics {
            cols: COLS,
            row_height: 56.0,
            width: 848.0,
        };
        assert_eq!(m.col_w(), (848.0 - 110.0) / 12.0);
        let items = showcase();
        let r = m.rect(&items[1]);
        assert_eq!((r.x, r.y), (3.0 * (61.5 + 10.0), 0.0));
        assert_eq!((r.width, r.height), (6.0 * 61.5 + 50.0, 4.0 * 56.0 + 30.0));
        let r = m.rect(&items[3]);
        assert_eq!((r.y, r.height), (2.0 * 66.0, 122.0));
        assert_eq!(m.height(&items), 4.0 * 66.0 - 10.0);
        assert_eq!(m.height(&[]), EMPTY_HEIGHT);
        let zero = Metrics { width: 0.0, ..m };
        assert_eq!(zero.col_w(), 0.0);
    }

    #[test]
    fn rows_and_content_height() {
        assert_eq!(rows(&showcase()), 4);
        assert_eq!(rows(&[]), 0);
        // A 2-row stat: 122 − 2 − 29 − 24 = 67 (the sparkline shrinks to 19).
        assert_eq!(content_height(2, 56.0), 67.0);
        assert_eq!(content_height(4, 56.0), 199.0);
    }

    #[test]
    fn items_serialise_like_react() {
        let s = serde_json::to_string(&it("d1", 0, 0, 3, 2)).unwrap();
        assert_eq!(s, r#"{"id":"d1","type":"stat","x":0,"y":0,"w":3,"h":2}"#);
        let r: DashboardItem =
            serde_json::from_str(r#"{"id":"w1","type":"list","x":1,"y":2,"w":3,"h":4,"minW":1}"#)
                .unwrap();
        assert_eq!(r, DashboardItem::new("w1", "list", 1, 2, 3, 4));
    }
}
