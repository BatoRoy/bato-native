//! The xyflow maths, ported 1:1 (no iced): the viewport transform, zoom
//! limits and wheel factor, `fitView`, the default Bezier edge, handle
//! positions, the dotted background, the MiniMap's view box, auto-pan and
//! d3's double-click zoom. Everything here is in CSS px; "flow" coordinates
//! are the graph's, "local" ones are the pane's (inside its 1 px border).

use iced::{Point, Rectangle, Vector};

/// xyflow's `minZoom` / `maxZoom` defaults.
pub const MIN_ZOOM: f32 = 0.5;
pub const MAX_ZOOM: f32 = 2.0;
/// `fitView` padding (fraction of the pane).
pub const FIT_PADDING: f32 = 0.1;
/// `<Background gap={16} size={1}>` as the React card sets it.
pub const DOT_GAP: f32 = 16.0;
pub const DOT_SIZE: f32 = 1.0;
/// `connectionRadius`: a connection snaps to a handle this close (flow px).
pub const CONNECTION_RADIUS: f32 = 20.0;
/// `interactionWidth`: an edge is clickable this wide (flow px, both sides).
pub const EDGE_INTERACTION: f32 = 20.0;
/// Pixels the pointer must travel before a node drag / connection starts
/// (`nodeDragThreshold`, `connectionDragThreshold`) and before a pane press
/// stops being a click (`paneClickDistance`).
pub const DRAG_THRESHOLD: f32 = 1.0;
/// d3-zoom's double-click transition.
pub const DBLCLICK_MS: u64 = 250;
/// Two clicks within this time and distance make a double click (GTK's
/// defaults, which Chromium on Linux follows).
pub const DBLCLICK_TIME_MS: u64 = 400;
pub const DBLCLICK_DIST: f32 = 5.0;
/// `autoPanSpeed` and the 40 px edge band of `calcAutoPan` (per frame at 60 Hz).
pub const AUTOPAN_SPEED: f32 = 15.0;
pub const AUTOPAN_BAND: f32 = 40.0;
/// Chromium on Linux: one wheel notch is 120 px of `deltaY`.
pub const PX_PER_NOTCH: f32 = 120.0;

/// The themed node (`ThemedNode`): `min-w-[7rem] px-3.5 py-2 text-sm border`.
pub const NODE_MIN_W: f32 = 112.0;
pub const NODE_PAD_X: f32 = 14.0;
pub const NODE_PAD_Y: f32 = 8.0;
pub const NODE_FONT: f32 = 14.0;
pub const NODE_LINE: f32 = 20.0;
pub const NODE_RADIUS: f32 = 8.0;
/// `.react-flow__handle`: 6 px, 1 px border (border-box under Tailwind).
pub const HANDLE: f32 = 6.0;

/// The pan / zoom transform: `local = flow × zoom + (x, y)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

impl Viewport {
    pub fn to_local(&self, p: Point) -> Point {
        Point::new(p.x * self.zoom + self.x, p.y * self.zoom + self.y)
    }
    pub fn to_flow(&self, p: Point) -> Point {
        Point::new((p.x - self.x) / self.zoom, (p.y - self.y) / self.zoom)
    }
    pub fn rect_to_local(&self, r: Rectangle) -> Rectangle {
        let o = self.to_local(r.position());
        Rectangle {
            x: o.x,
            y: o.y,
            width: r.width * self.zoom,
            height: r.height * self.zoom,
        }
    }
    /// Zoom to `k` (clamped) keeping the flow point under `at` (local) in
    /// place: d3's `scaleTo(k, point)`.
    pub fn zoom_at(&self, k: f32, at: Point) -> Viewport {
        let k = clamp_zoom(k);
        let f = self.to_flow(at);
        Viewport {
            x: at.x - f.x * k,
            y: at.y - f.y * k,
            zoom: k,
        }
    }
    /// The pane rectangle in flow coordinates (the MiniMap's `viewBB`).
    pub fn visible(&self, w: f32, h: f32) -> Rectangle {
        Rectangle {
            x: -self.x / self.zoom,
            y: -self.y / self.zoom,
            width: w / self.zoom,
            height: h / self.zoom,
        }
    }
    pub fn pan(&self, d: Vector) -> Viewport {
        Viewport {
            x: self.x + d.x,
            y: self.y + d.y,
            zoom: self.zoom,
        }
    }
}

pub fn clamp_zoom(k: f32) -> f32 {
    k.clamp(MIN_ZOOM, MAX_ZOOM)
}

/// d3-zoom's `wheelDelta` for a pixel `deltaY` (browser sign: positive =
/// scroll down = zoom out), as a scale factor.
pub fn wheel_factor(delta_y: f32) -> f32 {
    2f32.powf(-delta_y * 0.002)
}

/// `parsePadding(number)`: `floor((v − v / (1 + p)) / 2)`.
fn parse_padding(p: f32, v: f32) -> f32 {
    ((v - v / (1.0 + p)) * 0.5).floor()
}

/// `getViewportForBounds` with a numeric padding (`fitView`).
pub fn fit_view(bounds: Rectangle, w: f32, h: f32, padding: f32) -> Viewport {
    if bounds.width <= 0.0 || bounds.height <= 0.0 || w <= 0.0 || h <= 0.0 {
        return Viewport::default();
    }
    let px = parse_padding(padding, w);
    let py = parse_padding(padding, h);
    let zoom = ((w - 2.0 * px) / bounds.width).min((h - 2.0 * py) / bounds.height);
    let k = clamp_zoom(zoom);
    let cx = bounds.x + bounds.width / 2.0;
    let cy = bounds.y + bounds.height / 2.0;
    let x = w / 2.0 - cx * k;
    let y = h / 2.0 - cy * k;
    // calculateAppliedPaddings: shift only when a side got less than asked.
    let left = (bounds.x * k + x).floor();
    let top = (bounds.y * k + y).floor();
    let right = (w - ((bounds.x + bounds.width) * k + x)).floor();
    let bottom = (h - ((bounds.y + bounds.height) * k + y)).floor();
    let off_l = (left - px).min(0.0);
    let off_t = (top - py).min(0.0);
    let off_r = (right - px).min(0.0);
    let off_b = (bottom - py).min(0.0);
    Viewport {
        x: x - off_l + off_r,
        y: y - off_t + off_b,
        zoom: k,
    }
}

/// The union of rectangles (`getBoundsOfRects`).
pub fn union(a: Rectangle, b: Rectangle) -> Rectangle {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.width).max(b.x + b.width);
    let y1 = (a.y + a.height).max(b.y + b.height);
    Rectangle {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    }
}

pub fn bounds_of(rects: impl IntoIterator<Item = Rectangle>) -> Option<Rectangle> {
    rects.into_iter().reduce(union)
}

// ── Handles and edges ───────────────────────────────────────────────────

/// Which handle of a themed node: the target on the left, the source on
/// the right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Target,
    Source,
}

/// A handle's centre in flow coordinates, for a node at `r`. The themed
/// node's div is not positioned, so the handle's containing block is the
/// `.react-flow__node` wrapper (the node's border box):
/// `left: 0 / right: 0; top: 50%; translate(∓50%, -50%)`.
pub fn handle_center(r: Rectangle, side: Side) -> Point {
    let y = r.y + r.height / 2.0;
    match side {
        Side::Target => Point::new(r.x, y),
        Side::Source => Point::new(r.x + r.width, y),
    }
}

/// Where an edge meets a handle (`getHandlePosition`): the handle box's
/// outer side, Left for targets and Right for sources.
pub fn handle_anchor(r: Rectangle, side: Side) -> Point {
    let c = handle_center(r, side);
    match side {
        Side::Target => Point::new(c.x - HANDLE / 2.0, c.y),
        Side::Source => Point::new(c.x + HANDLE / 2.0, c.y),
    }
}

/// `calculateControlOffset` with the default curvature 0.25.
fn control_offset(distance: f32) -> f32 {
    if distance >= 0.0 {
        0.5 * distance
    } else {
        0.25 * 25.0 * (-distance).sqrt()
    }
}

/// A cubic Bezier: start, two control points, end.
pub type Cubic = [Point; 4];

/// `getBezierPath` from a Right (source) position to a Left (target) one.
pub fn bezier(s: Point, t: Point) -> Cubic {
    let c1 = Point::new(s.x + control_offset(t.x - s.x), s.y);
    let c2 = Point::new(t.x - control_offset(t.x - s.x), t.y);
    [s, c1, c2, t]
}

/// The connection line while dragging from `from` (a handle centre on
/// `side`) to `to`: the target end faces the opposite way, or the snapped
/// handle's own way.
pub fn connection_curve(from: Point, side: Side, to: Point, to_side: Option<Side>) -> Cubic {
    let out = |p: Point, sd: Side, other: Point| match sd {
        Side::Source => Point::new(p.x + control_offset(other.x - p.x), p.y),
        Side::Target => Point::new(p.x - control_offset(p.x - other.x), p.y),
    };
    let to_side = to_side.unwrap_or(match side {
        Side::Source => Side::Target,
        Side::Target => Side::Source,
    });
    [from, out(from, side, to), out(to, to_side, from), to]
}

pub fn cubic_at(c: &Cubic, t: f32) -> Point {
    let u = 1.0 - t;
    let (a, b, cc, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Point::new(
        a * c[0].x + b * c[1].x + cc * c[2].x + d * c[3].x,
        a * c[0].y + b * c[1].y + cc * c[2].y + d * c[3].y,
    )
}

fn seg_dist(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (qx, qy) = (a.x + t * dx - p.x, a.y + t * dy - p.y);
    (qx * qx + qy * qy).sqrt()
}

/// Distance from `p` to the curve (a 48-segment polyline is plenty for
/// a 10 px hit band).
pub fn cubic_distance(c: &Cubic, p: Point) -> f32 {
    let mut best = f32::INFINITY;
    let mut prev = c[0];
    for i in 1..=48 {
        let q = cubic_at(c, i as f32 / 48.0);
        best = best.min(seg_dist(p, prev, q));
        prev = q;
    }
    best
}

/// The control-point bounding box (contains the curve).
pub fn cubic_bounds(c: &Cubic) -> Rectangle {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in c {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    Rectangle {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    }
}

// ── Background ──────────────────────────────────────────────────────────

/// The dot centres of `<Background variant="dots">` inside a `w × h` pane,
/// and their radius. The SVG pattern: tile `gap·k` at `x % gap·k`, shifted
/// by `-(1 + gap·k / 2)`, a circle of radius `size·k / 2` at its corner.
pub fn dots(vp: &Viewport, w: f32, h: f32) -> (Vec<Point>, f32) {
    let k = vp.zoom;
    let gap = DOT_GAP * k;
    let r = DOT_SIZE * k / 2.0;
    if gap < 0.5 {
        return (Vec::new(), r);
    }
    let off = 1.0 + gap / 2.0;
    let ox = vp.x % gap - off + r;
    let oy = vp.y % gap - off + r;
    // First centre at or left of −r, last past the edge.
    let first = |o: f32| o - ((o + r) / gap).ceil() * gap;
    let (fx, fy) = (first(ox), first(oy));
    let mut out = Vec::new();
    let mut y = fy;
    while y - r <= h {
        let mut x = fx;
        while x - r <= w {
            if x + r >= 0.0 && y + r >= 0.0 {
                out.push(Point::new(x, y));
            }
            x += gap;
        }
        y += gap;
    }
    (out, r)
}

// ── MiniMap ─────────────────────────────────────────────────────────────

/// `<MiniMap>`: 200 × 150 at the bottom right, `offsetScale` 5.
pub const MINIMAP_W: f32 = 200.0;
pub const MINIMAP_H: f32 = 150.0;
pub const PANEL_MARGIN: f32 = 15.0;
pub const MINIMAP_NODE_RADIUS: f32 = 5.0;

/// The MiniMap's view box (flow coordinates) and its scale (minimap px
/// per flow px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MiniView {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Flow px per minimap px (`viewScale`).
    pub view_scale: f32,
}

impl MiniView {
    pub fn new(nodes: Option<Rectangle>, view: Rectangle) -> Self {
        let b = nodes.map_or(view, |n| union(n, view));
        let view_scale = (b.width / MINIMAP_W).max(b.height / MINIMAP_H).max(1e-6);
        let vw = view_scale * MINIMAP_W;
        let vh = view_scale * MINIMAP_H;
        let offset = 5.0 * view_scale;
        MiniView {
            x: b.x - (vw - b.width) / 2.0 - offset,
            y: b.y - (vh - b.height) / 2.0 - offset,
            width: vw + 2.0 * offset,
            height: vh + 2.0 * offset,
            view_scale,
        }
    }
    /// The SVG's fit of the view box into the 200 × 150 element
    /// (`xMidYMid meet`; the box is 210:160, so the height decides): the
    /// scale (minimap px per flow px) and the centring offset.
    pub fn fit(&self) -> (f32, Vector) {
        let s = (MINIMAP_W / self.width).min(MINIMAP_H / self.height);
        (
            s,
            Vector::new(
                (MINIMAP_W - self.width * s) / 2.0,
                (MINIMAP_H - self.height * s) / 2.0,
            ),
        )
    }
    /// A flow rectangle in minimap-local px.
    pub fn map(&self, r: Rectangle) -> Rectangle {
        let (s, o) = self.fit();
        Rectangle {
            x: (r.x - self.x) * s + o.x,
            y: (r.y - self.y) * s + o.y,
            width: r.width * s,
            height: r.height * s,
        }
    }
    pub fn scale(&self) -> f32 {
        self.fit().0
    }
}

/// XYMinimap's pan: dragging by `d` minimap px moves the viewport by
/// `−d · viewScale · max(k, ln k)` screen px.
pub fn minimap_pan(vp: &Viewport, view_scale: f32, d: Vector) -> Viewport {
    let m = view_scale * vp.zoom.max(vp.zoom.ln());
    Viewport {
        x: vp.x - d.x * m,
        y: vp.y - d.y * m,
        zoom: vp.zoom,
    }
}

// ── Auto-pan ────────────────────────────────────────────────────────────

fn autopan_velocity(v: f32, min: f32, max: f32) -> f32 {
    if v < min {
        (v - min).abs().clamp(1.0, min) / min
    } else if v > max {
        -(v - max).abs().clamp(1.0, min) / min
    } else {
        0.0
    }
}

/// `calcAutoPan`: how far (px per 60 Hz frame) to pan while dragging at
/// `p` (local) in a `w × h` pane.
pub fn autopan(p: Point, w: f32, h: f32) -> Vector {
    Vector::new(
        autopan_velocity(p.x, AUTOPAN_BAND, w - AUTOPAN_BAND) * AUTOPAN_SPEED,
        autopan_velocity(p.y, AUTOPAN_BAND, h - AUTOPAN_BAND) * AUTOPAN_SPEED,
    )
}

// ── Double-click zoom ───────────────────────────────────────────────────

/// d3-transition's default `easeCubicInOut`.
pub fn ease_cubic_in_out(t: f32) -> f32 {
    let t = t * 2.0;
    if t <= 1.0 {
        t * t * t / 2.0
    } else {
        let t = t - 2.0;
        (t * t * t + 2.0) / 2.0
    }
}

/// A d3 zoom transition around a fixed point: `interpolateZoom` between two
/// views with the same centre is geometric in the scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomAnim {
    pub from: Viewport,
    pub to: f32,
    pub at: Point,
}

impl ZoomAnim {
    pub fn at_progress(&self, t: f32) -> Viewport {
        let e = ease_cubic_in_out(t.clamp(0.0, 1.0));
        let k0 = self.from.zoom;
        let k = k0 * (self.to / k0).powf(e);
        self.from.zoom_at_unclamped(k, self.at)
    }
}

impl Viewport {
    fn zoom_at_unclamped(&self, k: f32, at: Point) -> Viewport {
        let f = self.to_flow(at);
        Viewport {
            x: at.x - f.x * k,
            y: at.y - f.y * k,
            zoom: k,
        }
    }
}

// ── Touch ───────────────────────────────────────────────────────────────

/// d3-zoom's two-finger gesture: the flow points `l0`, `l1` grabbed at the
/// start stay under the fingers `p0`, `p1` (scale from their distance,
/// clamped; the midpoints then line up).
pub fn pinch(l0: Point, l1: Point, p0: Point, p1: Point) -> Viewport {
    let dl = ((l1.x - l0.x).powi(2) + (l1.y - l0.y).powi(2)).sqrt();
    let dp = ((p1.x - p0.x).powi(2) + (p1.y - p0.y).powi(2)).sqrt();
    let k = if dl > 0.0 { clamp_zoom(dp / dl) } else { 1.0 };
    let l = Point::new((l0.x + l1.x) / 2.0, (l0.y + l1.y) / 2.0);
    let p = Point::new((p0.x + p1.x) / 2.0, (p0.y + p1.y) / 2.0);
    Viewport {
        x: p.x - l.x * k,
        y: p.y - l.y * k,
        zoom: k,
    }
}

/// One finger (or the mouse) keeps the grabbed flow point `l` under `p`.
pub fn grab(l: Point, p: Point, zoom: f32) -> Viewport {
    Viewport {
        x: p.x - l.x * zoom,
        y: p.y - l.y * zoom,
        zoom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rectangle {
        Rectangle {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn fit_view_matches_the_reference_card() {
        // The Showcase graph: nodes span 0..472 × 0..128; the pane is
        // 802 × 318 in the reference capture (zoom ≈ 1.547).
        let v = fit_view(r(0.0, 0.0, 472.0, 128.0), 802.0, 318.0, FIT_PADDING);
        assert!((v.zoom - 730.0 / 472.0).abs() < 1e-4, "{v:?}");
        assert!((v.x - (401.0 - 236.0 * v.zoom)).abs() < 1e-3);
        assert!((v.y - (159.0 - 64.0 * v.zoom)).abs() < 1e-3);
    }

    #[test]
    fn fit_view_clamps_to_max_zoom() {
        let v = fit_view(r(0.0, 0.0, 10.0, 10.0), 800.0, 300.0, FIT_PADDING);
        assert_eq!(v.zoom, MAX_ZOOM);
        assert_eq!((v.x, v.y), (400.0 - 10.0, 150.0 - 10.0));
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let v = Viewport {
            x: 30.0,
            y: -12.0,
            zoom: 1.3,
        };
        let at = Point::new(200.0, 90.0);
        let f = v.to_flow(at);
        let z = v.zoom_at(1.8, at);
        let back = z.to_local(f);
        assert!((back.x - at.x).abs() < 1e-3 && (back.y - at.y).abs() < 1e-3);
        assert_eq!(v.zoom_at(9.0, at).zoom, MAX_ZOOM);
        assert_eq!(v.zoom_at(0.1, at).zoom, MIN_ZOOM);
    }

    #[test]
    fn wheel_notch_is_d3s_factor() {
        // One notch down: deltaY 120 → 2^-0.24.
        assert!((wheel_factor(PX_PER_NOTCH) - 2f32.powf(-0.24)).abs() < 1e-6);
        assert!(wheel_factor(-PX_PER_NOTCH) > 1.0);
    }

    #[test]
    fn bezier_matches_get_bezier_path() {
        // Forward: control offset half the distance.
        let c = bezier(Point::new(0.0, 0.0), Point::new(100.0, 50.0));
        assert_eq!(c[1], Point::new(50.0, 0.0));
        assert_eq!(c[2], Point::new(50.0, 50.0));
        // Backward: curvature · 25 · √−d.
        let c = bezier(Point::new(100.0, 0.0), Point::new(0.0, 0.0));
        let o = 0.25 * 25.0 * 100f32.sqrt();
        assert!((c[1].x - (100.0 + o)).abs() < 1e-4);
        assert!((c[2].x - (0.0 - o)).abs() < 1e-4);
    }

    #[test]
    fn handles_sit_on_the_border_box_edges() {
        let n = r(10.0, 20.0, 112.0, 38.0);
        assert_eq!(handle_center(n, Side::Target), Point::new(10.0, 39.0));
        assert_eq!(handle_center(n, Side::Source), Point::new(122.0, 39.0));
        assert_eq!(handle_anchor(n, Side::Target), Point::new(7.0, 39.0));
        assert_eq!(handle_anchor(n, Side::Source), Point::new(125.0, 39.0));
    }

    #[test]
    fn curve_distance_hits_the_curve() {
        let c = bezier(Point::new(0.0, 0.0), Point::new(100.0, 50.0));
        let mid = cubic_at(&c, 0.5);
        assert!(cubic_distance(&c, mid) < 0.01);
        assert!(cubic_distance(&c, Point::new(mid.x, mid.y + 9.0)) < 10.0);
        assert!(cubic_distance(&c, Point::new(mid.x, mid.y + 30.0)) > 10.0);
    }

    #[test]
    fn dots_follow_the_pattern() {
        let v = Viewport::default();
        let (pts, rad) = dots(&v, 64.0, 32.0);
        assert_eq!(rad, 0.5);
        // Pattern at x % 16 = 0 shifted by -(1 + 8): centres at -8.5 + 16n.
        assert!(
            pts.iter()
                .any(|p| (p.x - 7.5).abs() < 1e-4 && (p.y - 7.5).abs() < 1e-4)
        );
        assert!(
            pts.iter()
                .all(|p| ((p.x + 8.5) / 16.0).fract().abs() < 1e-4)
        );
        // Panning by a full gap gives the same dots.
        let (moved, _) = dots(&v.pan(Vector::new(16.0, 16.0)), 64.0, 32.0);
        assert_eq!(pts, moved);
    }

    #[test]
    fn minimap_fits_nodes_and_view() {
        let m = MiniView::new(
            Some(r(0.0, 0.0, 400.0, 100.0)),
            r(-50.0, -50.0, 500.0, 200.0),
        );
        // Bounds 500 × 200 → scale max(2.5, 1.33) = 2.5, offset 12.5.
        assert!((m.view_scale - 2.5).abs() < 1e-5);
        assert!((m.width - (500.0 + 25.0)).abs() < 1e-3);
        assert!((m.height - (375.0 + 25.0)).abs() < 1e-3);
        // 525 × 400 into 200 × 150: the height decides (0.375), centred.
        let (s, o) = m.fit();
        assert!((s - 0.375).abs() < 1e-5);
        assert!((o.x - (200.0 - 525.0 * 0.375) / 2.0).abs() < 1e-3 && o.y.abs() < 1e-3);
        let mapped = m.map(r(-50.0, -50.0, 500.0, 200.0));
        assert!(mapped.x > 0.0 && mapped.x + mapped.width < MINIMAP_W);
    }

    #[test]
    fn autopan_only_near_the_edges() {
        assert_eq!(
            autopan(Point::new(400.0, 150.0), 800.0, 300.0),
            Vector::new(0.0, 0.0)
        );
        let v = autopan(Point::new(0.0, 299.0), 800.0, 300.0);
        assert_eq!(v.x, AUTOPAN_SPEED);
        assert!(v.y < 0.0 && v.y > -AUTOPAN_SPEED);
    }

    #[test]
    fn dblclick_zoom_is_geometric_and_ends_on_target() {
        let a = ZoomAnim {
            from: Viewport {
                x: 10.0,
                y: 5.0,
                zoom: 1.0,
            },
            to: 2.0,
            at: Point::new(100.0, 50.0),
        };
        let end = a.at_progress(1.0);
        assert!((end.zoom - 2.0).abs() < 1e-5);
        let f = a.from.to_flow(a.at);
        let p = a.at_progress(0.37).to_local(f);
        assert!((p.x - 100.0).abs() < 1e-3 && (p.y - 50.0).abs() < 1e-3);
        assert!((a.at_progress(0.5).zoom - 2f32.sqrt()).abs() < 1e-4);
    }

    #[test]
    fn pinch_keeps_both_points_under_the_fingers() {
        let l0 = Point::new(0.0, 0.0);
        let l1 = Point::new(100.0, 0.0);
        let v = pinch(l0, l1, Point::new(50.0, 40.0), Point::new(200.0, 40.0));
        assert!((v.zoom - 1.5).abs() < 1e-5);
        let a = v.to_local(l0);
        assert!((a.x - 50.0).abs() < 1e-3 && (a.y - 40.0).abs() < 1e-3);
        // Clamped at the limits.
        assert_eq!(
            pinch(l0, l1, Point::ORIGIN, Point::new(1000.0, 0.0)).zoom,
            MAX_ZOOM
        );
    }

    #[test]
    fn minimap_pan_moves_by_view_scale_times_zoom() {
        let v = Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 1.5,
        };
        let p = minimap_pan(&v, 2.0, Vector::new(10.0, -4.0));
        assert_eq!((p.x, p.y), (-30.0, 12.0));
    }
}
