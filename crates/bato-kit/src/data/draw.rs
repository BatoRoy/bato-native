//! Vector drawing (INF-18): SVG fragments rasterized by resvg through iced's
//! SVG pipeline, the rasterizer the icons, the Spinner, the progress ring and
//! the FileDropzone border already use.
//!
//! **Why resvg, not `canvas` + MSAA.** The React charts are hand-written SVG,
//! and Chromium rasterizes SVG with Skia's analytic coverage; resvg is built
//! on tiny-skia, a port of the same rasterizer, so a 1 px gridline at an
//! integer y comes out as the same two half-covered rows and a 2 px round-join
//! polyline as the same edge ramp. iced's `canvas` would need the `canvas`
//! feature (lyon tessellation) plus application-wide MSAA ×4, which changes
//! every mesh, adds a multisampled target, and still gives coarser edges. The
//! cost of resvg is rasterizing on change, so callers split a drawing into
//! **layers** that change at different times: the static plot is rasterized
//! once per size/data (iced caches it by content hash, size and colour) and
//! a hover moves only small layers (a dot, a crosshair) or plain quads.
//!
//! A layer is drawn on the device-pixel grid: its document covers the
//! snapped bounding box and carries the fractional offset in its `viewBox`,
//! so nothing is resampled (iced draws SVGs with nearest filtering).
//!
//! ```ignore
//! let d = polyline(&points);                       // "M x,y L x,y …"
//! let body = stroke(&d, 2.0, Cap::Round, Join::Round, None);
//! layer(renderer, origin, Rectangle::new(Point::ORIGIN, size), &body, accent, 1.0, viewport);
//! text_at(renderer, &label("56", 10.0, 12.0).shape(f32::INFINITY), x, baseline, Anchor::End, muted, viewport);
//! ```

use std::fmt::Write as _;

use iced::advanced::svg::{self, Renderer as _};
use iced::{Color, Point, Radians, Rectangle};

use crate::anim::current_opacity;
use crate::label::Shaped;
use crate::scale::device_scale;

/// A coordinate for SVG data: at most 3 decimals, no trailing zeros.
pub fn num(v: f32) -> String {
    let mut s = format!("{:.3}", v);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" { "0".to_string() } else { s }
}

/// Builds SVG path data (`d`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PathData(String);

impl PathData {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn move_to(mut self, p: Point) -> Self {
        self.cmd('M', &[p.x, p.y]);
        self
    }
    pub fn line_to(mut self, p: Point) -> Self {
        self.cmd('L', &[p.x, p.y]);
        self
    }
    /// A circular arc of radius `r` to `p` (`A r r 0 large sweep x y`).
    pub fn arc_to(mut self, r: f32, large: bool, sweep: bool, p: Point) -> Self {
        let flags = [f32::from(u8::from(large)), f32::from(u8::from(sweep))];
        self.cmd('A', &[r, r, 0.0, flags[0], flags[1], p.x, p.y]);
        self
    }
    /// A cubic Bézier to `p` (node-graph edges).
    pub fn cubic_to(mut self, c1: Point, c2: Point, p: Point) -> Self {
        self.cmd('C', &[c1.x, c1.y, c2.x, c2.y, p.x, p.y]);
        self
    }
    pub fn close(mut self) -> Self {
        self.0.push('Z');
        self
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    fn cmd(&mut self, c: char, args: &[f32]) {
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        self.0.push(c);
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                self.0.push(if args.len() == 2 { ',' } else { ' ' });
            }
            let _ = write!(self.0, "{}", num(*a));
        }
    }
}

/// `M p0 L p1 L p2 …` (React's `${i === 0 ? 'M' : 'L'}${x},${y}`).
pub fn polyline(points: &[Point]) -> PathData {
    points
        .iter()
        .enumerate()
        .fold(PathData::new(), |d, (i, p)| {
            if i == 0 { d.move_to(*p) } else { d.line_to(*p) }
        })
}

/// A rectangle with corner radius `rx` (SVG `<rect rx>`: clamped to half
/// the smaller side).
pub fn rounded_rect(r: Rectangle, rx: f32) -> PathData {
    let rx = rx.min(r.width / 2.0).min(r.height / 2.0).max(0.0);
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.width, r.y + r.height);
    if rx <= 0.0 {
        return PathData::new()
            .move_to(Point::new(x0, y0))
            .line_to(Point::new(x1, y0))
            .line_to(Point::new(x1, y1))
            .line_to(Point::new(x0, y1))
            .close();
    }
    PathData::new()
        .move_to(Point::new(x0 + rx, y0))
        .line_to(Point::new(x1 - rx, y0))
        .arc_to(rx, false, true, Point::new(x1, y0 + rx))
        .line_to(Point::new(x1, y1 - rx))
        .arc_to(rx, false, true, Point::new(x1 - rx, y1))
        .line_to(Point::new(x0 + rx, y1))
        .arc_to(rx, false, true, Point::new(x0, y1 - rx))
        .line_to(Point::new(x0, y0 + rx))
        .arc_to(rx, false, true, Point::new(x0 + rx, y0))
        .close()
}

/// A full circle as path data.
pub fn circle(c: Point, r: f32) -> PathData {
    PathData::new()
        .move_to(Point::new(c.x - r, c.y))
        .arc_to(r, true, true, Point::new(c.x + r, c.y))
        .arc_to(r, true, true, Point::new(c.x - r, c.y))
        .close()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Join {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// A stroked path element (`stroke-dasharray="dash gap"` when `dash`).
pub fn stroke(d: &PathData, width: f32, cap: Cap, join: Join, dash: Option<(f32, f32)>) -> String {
    let cap = match cap {
        Cap::Butt => "butt",
        Cap::Round => "round",
        Cap::Square => "square",
    };
    let join = match join {
        Join::Miter => "miter",
        Join::Round => "round",
        Join::Bevel => "bevel",
    };
    let dash = dash
        .map(|(a, b)| format!(r#" stroke-dasharray="{} {}""#, num(a), num(b)))
        .unwrap_or_default();
    format!(
        r##"<path d="{}" fill="none" stroke="#000" stroke-width="{}" stroke-linecap="{cap}" stroke-linejoin="{join}"{dash}/>"##,
        d.as_str(),
        num(width)
    )
}

/// A filled path element.
pub fn fill(d: &PathData) -> String {
    format!(r##"<path d="{}" fill="#000"/>"##, d.as_str())
}

/// The SVG document for `body` covering `bbox` (local CSS px) of a drawing
/// whose origin sits at `origin` (window CSS px), snapped outward to device
/// pixels. Returns the document and where to draw it, or `None` when empty.
pub fn document(origin: Point, bbox: Rectangle, body: &str) -> Option<(String, Rectangle)> {
    let s = device_scale();
    let x0 = ((origin.x + bbox.x) * s + 1e-3).floor();
    let y0 = ((origin.y + bbox.y) * s + 1e-3).floor();
    let x1 = ((origin.x + bbox.x + bbox.width) * s - 1e-3).ceil();
    let y1 = ((origin.y + bbox.y + bbox.height) * s - 1e-3).ceil();
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (dw, dh) = (x1 - x0, y1 - y0);
    let doc = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="{} {} {} {}">{body}</svg>"#,
        dw,
        dh,
        num(x0 / s - origin.x),
        num(y0 / s - origin.y),
        num(dw / s),
        num(dh / s),
    );
    // iced rasterizes at ceil(bounds × scale): keep that exactly `dw × dh`.
    let at = Rectangle {
        x: x0 / s,
        y: y0 / s,
        width: (dw - 1e-3) / s,
        height: (dh - 1e-3) / s,
    };
    Some((doc, at))
}

/// Draw `body` (SVG elements in local CSS px, painted `#000`) as one layer in
/// `color`, with `opacity` on top of the colour's alpha and the current group
/// opacity. `bbox` bounds what `body` paints (strokes included).
pub fn layer(
    renderer: &mut iced::Renderer,
    origin: Point,
    bbox: Rectangle,
    body: &str,
    color: Color,
    opacity: f32,
    clip: &Rectangle,
) {
    let Some((doc, at)) = document(origin, bbox, body) else {
        return;
    };
    let a = color.a * opacity * current_opacity();
    if a <= 0.0 {
        return;
    }
    renderer.draw_svg(
        svg::Svg {
            handle: svg::Handle::from_memory(doc.into_bytes()),
            color: Some(Color { a: 1.0, ..color }),
            rotation: Radians(0.0),
            opacity: a,
        },
        at,
        clip.expand(2.0),
    );
}

/// `p` rounded to a device pixel: Chromium paints an `<svg>` root at a
/// pixel-snapped offset, so drawings should use this as their origin.
pub fn snap_point(p: Point) -> Point {
    let s = device_scale();
    Point::new((p.x * s).round() / s, (p.y * s).round() / s)
}

/// The bounding box of points, grown by `pad` (half a stroke width, a dot
/// radius).
pub fn bbox_of(points: &[Point], pad: f32) -> Rectangle {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in points {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    if x0 > x1 {
        return Rectangle::default();
    }
    Rectangle {
        x: x0 - pad,
        y: y0 - pad,
        width: x1 - x0 + 2.0 * pad,
        height: y1 - y0 + 2.0 * pad,
    }
}

/// SVG `text-anchor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

/// The line height [`text_at`] lays a one-line label out with.
fn line_box(size: f32) -> f32 {
    (size * 1.5).round()
}

/// Shape a one-line label for [`text_at`] (Inter, `size`, `weight`).
pub fn shape(text: impl Into<String>, size: f32, weight: u16) -> Shaped {
    crate::label::label(text, size, line_box(size))
        .weight(weight)
        .shape(f32::INFINITY)
}

/// The horizontal extent `(left, width)` of a shaped label anchored at `x`.
pub fn anchored_x(shaped: &Shaped, x: f32, anchor: Anchor) -> (f32, f32) {
    let w = shaped.width();
    let left = match anchor {
        Anchor::Start => x,
        Anchor::Middle => x - w / 2.0,
        Anchor::End => x - w,
    };
    (left, w)
}

/// Draw a label made by [`shape`] like SVG `<text x y text-anchor>`: the
/// baseline at `baseline` (window CSS px, rounded to a device pixel as Skia
/// does), anchored at `x`.
#[allow(clippy::too_many_arguments)]
pub fn text_at(
    renderer: &mut iced::Renderer,
    shaped: &Shaped,
    size: f32,
    x: f32,
    baseline: f32,
    anchor: Anchor,
    color: Color,
    clip: &Rectangle,
) {
    let s = device_scale();
    let lh = line_box(size);
    let base = crate::label::blink_baseline(crate::label::Fam::Sans, size, lh, s);
    let top = (baseline * s - base) / s;
    let (left, w) = anchored_x(shaped, x, anchor);
    shaped.draw(
        renderer,
        Rectangle {
            x: left,
            y: top,
            width: w,
            height: lh,
        },
        color,
        clip,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_compact() {
        assert_eq!(num(12.0), "12");
        assert_eq!(num(12.5), "12.5");
        assert_eq!(num(1.0 / 3.0), "0.333");
        assert_eq!(num(-0.0001), "0");
    }

    #[test]
    fn paths_match_react_strings() {
        let d = polyline(&[Point::new(40.0, 12.5), Point::new(66.25, 20.0)]);
        assert_eq!(d.as_str(), "M40,12.5 L66.25,20");
        let r = rounded_rect(
            Rectangle::new(Point::ORIGIN, iced::Size::new(10.0, 4.0)),
            3.0,
        );
        // rx clamps to half the height.
        assert!(r.as_str().starts_with("M2,0 L8,0 A2 2 0 0 1 10 2"));
        assert!(
            circle(Point::new(5.0, 5.0), 2.0)
                .as_str()
                .contains("A2 2 0 1 1 7 5")
        );
        let s = stroke(&d, 2.0, Cap::Round, Join::Round, Some((3.0, 3.0)));
        assert!(s.contains(r#"stroke-dasharray="3 3""#));
        assert!(s.contains(r#"stroke-linejoin="round""#));
    }

    #[test]
    fn documents_snap_outward_and_keep_the_offset() {
        crate::scale::set_device_scale(1.0);
        let (doc, at) = document(
            Point::new(100.0, 50.0),
            Rectangle::new(Point::new(10.4, 0.5), iced::Size::new(5.0, 2.0)),
            "",
        )
        .unwrap();
        assert_eq!(at.x, 110.0);
        assert_eq!(at.y, 50.0);
        assert!((at.width - 6.0).abs() < 0.01);
        assert!(
            doc.contains(r#"width="6" height="3" viewBox="10 0 6 3""#),
            "{doc}"
        );
        assert!(document(Point::ORIGIN, Rectangle::default(), "").is_none());
    }

    #[test]
    fn bbox_grows_by_the_pad() {
        let b = bbox_of(&[Point::new(1.0, 2.0), Point::new(5.0, 4.0)], 1.0);
        assert_eq!(
            b,
            Rectangle::new(Point::new(0.0, 1.0), iced::Size::new(6.0, 4.0))
        );
    }
}
