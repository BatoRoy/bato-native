//! Painting the graph: the colours (index.css's xyflow overrides over the
//! xyflow defaults), Bezier strokes rasterized by resvg (the icons'
//! rasterizer, analytic anti-aliasing, re-rasterized at the current zoom so
//! they stay crisp), zoom-scaled node boxes and handles as quads, the dotted
//! background, the Controls icons and dotted selection boxes.
//!
//! Everything is laid out at `zoom × size` instead of scaling a bitmap, the
//! way Chromium re-rasterizes a `transform: scale()` layer.

use bato_theme::Rgb;
use iced::advanced::renderer::{Quad, Renderer as _};
use iced::advanced::svg::{self, Renderer as _};
use iced::{Border, Color, Point, Rectangle, Size};

use crate::anim::fade;
use crate::ctx::Ctx;
use crate::scale::device_scale;
use crate::style::{c, inset_of, rgba, shadows};
use crate::surface::{BoxStyle, Sh, paint};

use super::model::{Cubic, HANDLE, NODE_RADIUS, cubic_bounds};

/// Every colour the graph paints, resolved from the theme at `view` time.
#[derive(Debug, Clone, PartialEq)]
pub struct Colors {
    /// What the frame sits on (the card): the corners outside the radius.
    pub backdrop: Color,
    pub frame: Color,
    /// `.react-flow.dark` paints `#141414`; light is transparent (the card).
    pub pane: Color,
    pub dot: Color,
    pub node_bg: Color,
    pub node_border: Color,
    pub node_text: Color,
    pub node_shadows: Vec<Sh>,
    pub node_inset: Option<Color>,
    pub handle_bg: Color,
    pub handle_border: Color,
    pub edge: Color,
    pub edge_selected: Color,
    /// `.react-flow__edge.selectable:focus` (xyflow's own selected stroke,
    /// which outranks index.css's `.selected` rule).
    pub edge_focus: Color,
    pub connection: Color,
    pub ctrl_bg: Color,
    pub ctrl_hover: Color,
    pub ctrl_fg: Color,
    pub ctrl_line: Color,
    pub ctrl_shadow: Sh,
    /// Chromium's default `outline: auto` on the control buttons.
    pub ctrl_ring: Color,
    pub mini_bg: Color,
    pub mini_node: Color,
    pub mini_mask: Color,
    pub selection_bg: Color,
    pub selection_border: Color,
}

impl Colors {
    /// `cx` is the card the graph sits in.
    pub fn new(cx: &Ctx) -> Self {
        let p = cx.p();
        let light = p.is_light();
        let sel = if light { (0, 89, 220) } else { (200, 200, 220) };
        Colors {
            backdrop: c(cx.over),
            frame: c(p.border),
            pane: if light {
                c(cx.over)
            } else {
                c(Rgb::hex("#141414").unwrap())
            },
            dot: c(p.border),
            node_bg: c(p.card),
            node_border: c(p.border),
            node_text: c(p.text),
            node_shadows: shadows(&p.shadow_md),
            node_inset: inset_of(&p.shadow_md, p.card),
            handle_bg: c(p.accent),
            handle_border: c(p.bg),
            edge: c(p.muted),
            edge_selected: c(p.accent),
            edge_focus: c(Rgb::hex(if light { "#555555" } else { "#727272" }).unwrap()),
            connection: c(p.accent),
            ctrl_bg: c(p.card),
            ctrl_hover: c(p.border),
            ctrl_fg: c(p.text),
            ctrl_line: c(p.border),
            ctrl_shadow: Sh {
                x: 0.0,
                y: 0.0,
                blur: 2.0,
                spread: 1.0,
                color: Color::from_rgba8(0, 0, 0, 0.08),
            },
            ctrl_ring: if light {
                Color::from_rgb8(0x10, 0x10, 0x10)
            } else {
                Color::WHITE
            },
            mini_bg: c(p.card),
            mini_node: c(p.accent),
            mini_mask: rgba(p.bg, 0.6),
            selection_bg: Color::from_rgba8(sel.0, sel.1, sel.2, 0.08),
            selection_border: Color::from_rgba8(sel.0, sel.1, sel.2, 0.8),
        }
    }
}

fn hex(col: Color) -> String {
    let [r, g, b, _] = col.into_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Chromium's 1 px edge strokes (inside the scaled viewport) cover about
/// 27 % more than resvg's at the same width (measured on the reference
/// capture at four points of the edges); widen ours to match.
pub const STROKE_GAIN: f32 = 1.27;

/// A stroked cubic (window coordinates, logical px) as an SVG handle and the
/// device-pixel-aligned box to draw it in. Identical geometry gives
/// identical bytes, so iced's raster cache reuses it while nothing moves.
pub fn curve(c: &Cubic, width: f32, color: Color) -> (svg::Handle, Rectangle) {
    let s = device_scale();
    let bb = cubic_bounds(c).expand(width + 2.0);
    let x0 = (bb.x * s).floor() / s;
    let y0 = (bb.y * s).floor() / s;
    let x1 = ((bb.x + bb.width) * s).ceil() / s;
    let y1 = ((bb.y + bb.height) * s).ceil() / s;
    let (w, h) = (x1 - x0, y1 - y0);
    let q = |p: Point| format!("{:.3},{:.3}", p.x - x0, p.y - y0);
    let src = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.3}\" height=\"{h:.3}\" \
         viewBox=\"0 0 {w:.3} {h:.3}\"><path d=\"M{} C{} {} {}\" fill=\"none\" \
         stroke=\"{}\" stroke-opacity=\"{:.3}\" stroke-width=\"{width:.4}\"/></svg>",
        q(c[0]),
        q(c[1]),
        q(c[2]),
        q(c[3]),
        hex(color),
        color.a,
    );
    (
        svg::Handle::from_memory(src.into_bytes()),
        Rectangle {
            x: x0,
            y: y0,
            width: w,
            height: h,
        },
    )
}

pub fn draw_curve(r: &mut iced::Renderer, c: &Cubic, width: f32, color: Color, clip: Rectangle) {
    let (handle, b) = curve(c, width, color);
    if b.intersection(&clip).is_none() {
        return;
    }
    r.draw_svg(
        svg::Svg {
            handle,
            color: None,
            rotation: iced::Radians(0.0),
            opacity: crate::anim::current_opacity(),
        },
        b,
        clip,
    );
}

/// A filled circle (anti-aliased quad, not snapped).
pub fn disc(r: &mut iced::Renderer, center: Point, radius: f32, color: Color) {
    r.fill_quad(
        Quad {
            bounds: Rectangle {
                x: center.x - radius,
                y: center.y - radius,
                width: radius * 2.0,
                height: radius * 2.0,
            },
            border: Border {
                radius: radius.into(),
                ..Border::default()
            },
            snap: false,
            ..Quad::default()
        },
        fade(color),
    );
}

/// A rectangle quad, optionally rounded and bordered, not snapped.
pub fn rect(
    r: &mut iced::Renderer,
    b: Rectangle,
    radius: f32,
    fill: Color,
    border: Option<(f32, Color)>,
) {
    let (width, color) = border.unwrap_or((0.0, Color::TRANSPARENT));
    r.fill_quad(
        Quad {
            bounds: b,
            border: Border {
                radius: radius.into(),
                width,
                color: fade(color),
            },
            snap: false,
            ..Quad::default()
        },
        fade(fill),
    );
}

/// The themed node box at zoom `z`: `shadow-app-md`, `bg-app-card`, a
/// `z`-wide `border-app-border`, `rounded-lg`, the inset top highlight.
pub fn node_box(r: &mut iced::Renderer, b: Rectangle, z: f32, col: &Colors) {
    let radius = NODE_RADIUS * z;
    let scaled: Vec<Sh> = col
        .node_shadows
        .iter()
        .map(|s| Sh {
            x: s.x * z,
            y: s.y * z,
            blur: s.blur * z,
            spread: s.spread * z,
            color: s.color,
        })
        .collect();
    paint(
        r,
        b,
        &BoxStyle {
            radius,
            shadows: scaled,
            ..BoxStyle::default()
        },
    );
    rect(r, b, radius, col.node_bg, Some((z, col.node_border)));
    if let Some(hl) = col.node_inset {
        let inner = Rectangle {
            x: b.x + z,
            y: b.y + z,
            width: b.width - 2.0 * z,
            height: b.height - 2.0 * z,
        };
        let ir = (radius - z).max(0.0);
        rect(r, inner, ir, hl, None);
        rect(
            r,
            Rectangle {
                y: inner.y + z,
                height: inner.height - z,
                ..inner
            },
            ir,
            col.node_bg,
            None,
        );
    }
}

/// A handle centred on `c` at zoom `z`: `bg-app-accent`, `border-app-bg`.
pub fn handle(r: &mut iced::Renderer, c: Point, z: f32, col: &Colors) {
    let d = HANDLE * z;
    rect(
        r,
        Rectangle {
            x: c.x - d / 2.0,
            y: c.y - d / 2.0,
            width: d,
            height: d,
        },
        d / 2.0,
        col.handle_bg,
        Some((z, col.handle_border)),
    );
}

/// `border: 1px dotted` (scaled by `z`): square dots, one dot apart,
/// over a translucent fill.
pub fn dotted_box(r: &mut iced::Renderer, b: Rectangle, z: f32, fill: Color, dot: Color) {
    rect(r, b, 0.0, fill, None);
    let d = z.max(0.5);
    let mut put = |x: f32, y: f32| {
        r.fill_quad(
            Quad {
                bounds: Rectangle {
                    x,
                    y,
                    width: d,
                    height: d,
                },
                snap: false,
                ..Quad::default()
            },
            fade(dot),
        )
    };
    let nx = ((b.width / (2.0 * d)).floor() as i32).max(1);
    let ny = ((b.height / (2.0 * d)).floor() as i32).max(1);
    for i in 0..nx {
        let x = b.x + i as f32 * 2.0 * d;
        put(x, b.y);
        put(x, b.y + b.height - d);
    }
    for j in 1..ny {
        let y = b.y + j as f32 * 2.0 * d;
        put(b.x, y);
        put(b.x + b.width - d, y);
    }
}

/// A drop shadow alone (the Controls panel's `0 0 2px 1px`).
pub fn shadow(r: &mut iced::Renderer, b: Rectangle, s: Sh) {
    paint(
        r,
        b,
        &BoxStyle {
            shadows: vec![s],
            ..BoxStyle::default()
        },
    );
}

// ── Controls icons (xyflow's own SVGs, `fill: currentColor`) ────────────

const PLUS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><path d="M32 18.133H18.133V32h-4.266V18.133H0v-4.266h13.867V0h4.266v13.867H32z"/></svg>"#;
const MINUS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 5"><path d="M0 0h32v4.2H0z"/></svg>"#;
const FIT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 30"><path d="M3.692 4.63c0-.53.4-.938.939-.938h5.215V0H4.708C2.13 0 0 2.054 0 4.63v5.216h3.692V4.631zM27.354 0h-5.2v3.692h5.17c.53 0 .984.4.984.939v5.215H32V4.631A4.624 4.624 0 0027.354 0zm.954 24.83c0 .532-.4.94-.939.94h-5.215v3.768h5.215c2.577 0 4.631-2.13 4.631-4.707v-5.139h-3.692v5.139zm-23.677.94c-.531 0-.939-.4-.939-.94v-5.138H0v5.139c0 2.577 2.13 4.707 4.708 4.707h5.138V25.77H4.631z"/></svg>"#;
const LOCK: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 25 32"><path d="M21.333 10.667H19.81V7.619C19.81 3.429 16.38 0 12.19 0 8 0 4.571 3.429 4.571 7.619v3.048H3.048A3.056 3.056 0 000 13.714v15.238A3.056 3.056 0 003.048 32h18.285a3.056 3.056 0 003.048-3.048V13.714a3.056 3.056 0 00-3.048-3.047zM12.19 24.533a3.056 3.056 0 01-3.047-3.047 3.056 3.056 0 013.047-3.048 3.056 3.056 0 013.048 3.048 3.056 3.056 0 01-3.048 3.047zm4.724-13.866H7.467V7.619c0-2.59 2.133-4.724 4.723-4.724 2.591 0 4.724 2.133 4.724 4.724v3.048z"/></svg>"#;
const UNLOCK: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 25 32"><path d="M21.333 10.667H19.81V7.619C19.81 3.429 16.38 0 12.19 0c-4.114 1.828-1.37 2.133.305 2.438 1.676.305 4.42 2.59 4.42 5.181v3.048H3.047A3.056 3.056 0 000 13.714v15.238A3.056 3.056 0 003.048 32h18.285a3.056 3.056 0 003.048-3.048V13.714a3.056 3.056 0 00-3.048-3.047zM12.19 24.533a3.056 3.056 0 01-3.047-3.047 3.056 3.056 0 013.047-3.048 3.056 3.056 0 013.048 3.048 3.056 3.056 0 01-3.048 3.047z"/></svg>"#;

/// The Controls buttons, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    ZoomIn,
    ZoomOut,
    FitView,
    Interactive,
}

impl Control {
    pub const ALL: [Control; 4] = [
        Control::ZoomIn,
        Control::ZoomOut,
        Control::FitView,
        Control::Interactive,
    ];
}

/// The icon and its box inside a 26 px button whose top-left is `o`:
/// `padding: 4px`, a 1 px bottom border on all but the last, the SVG
/// `width: 100%; max-width/height: 12px` centred by flex (an aspect that
/// overflows 12 px tall is fitted inside the 12 px box, `meet`).
pub fn control_icon(ctl: Control, interactive: bool, o: Point) -> (svg::Handle, Rectangle) {
    static H: std::sync::LazyLock<[svg::Handle; 5]> = std::sync::LazyLock::new(|| {
        [PLUS, MINUS, FIT, LOCK, UNLOCK].map(|s| svg::Handle::from_memory(s.as_bytes()))
    });
    let (i, vw, vh): (usize, f32, f32) = match ctl {
        Control::ZoomIn => (0, 32.0, 32.0),
        Control::ZoomOut => (1, 32.0, 5.0),
        Control::FitView => (2, 32.0, 30.0),
        Control::Interactive => (if interactive { 4 } else { 3 }, 25.0, 32.0),
    };
    let last = ctl == Control::Interactive;
    let content_h = if last { 18.0 } else { 17.0 };
    let box_h = (12.0 * vh / vw).min(12.0);
    let (w, h) = if 12.0 * vh / vw > 12.0 {
        (12.0 * vw / vh, 12.0)
    } else {
        (12.0, box_h)
    };
    let s = device_scale();
    let snap = |v: f32| (v * s).round() / s;
    let x = o.x + 4.0 + (18.0 - 12.0) / 2.0 + (12.0 - w) / 2.0;
    let y = o.y + 4.0 + (content_h - box_h) / 2.0 + (box_h - h) / 2.0;
    (
        H[i].clone(),
        Rectangle {
            x: snap(x),
            y: snap(y),
            width: w,
            height: h,
        },
    )
}

pub fn draw_icon(r: &mut iced::Renderer, h: svg::Handle, b: Rectangle, color: Color, alpha: f32) {
    r.draw_svg(
        svg::Svg {
            handle: h,
            color: Some(color),
            rotation: iced::Radians(0.0),
            opacity: alpha * crate::anim::current_opacity(),
        },
        b,
        b.expand(1.0),
    );
}

/// A point (logical) with its size, for quick rectangles.
pub fn r_at(p: Point, s: Size) -> Rectangle {
    Rectangle {
        x: p.x,
        y: p.y,
        width: s.width,
        height: s.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_box_is_pixel_aligned_and_bytes_are_stable() {
        crate::scale::set_device_scale(1.0);
        let c = [
            Point::new(10.2, 20.7),
            Point::new(60.0, 20.7),
            Point::new(60.0, 80.1),
            Point::new(110.5, 80.1),
        ];
        let (h1, b) = curve(&c, 1.5, Color::WHITE);
        let (h2, _) = curve(&c, 1.5, Color::WHITE);
        assert_eq!(h1.id(), h2.id());
        assert_eq!(b.x, b.x.floor());
        assert_eq!(b.y, b.y.floor());
        assert!(b.x <= 10.2 - 1.5 && b.x + b.width >= 110.5 + 1.5);
    }

    #[test]
    fn control_icons_fit_the_12px_box() {
        crate::scale::set_device_scale(1.0);
        let (_, plus) = control_icon(Control::ZoomIn, true, Point::ORIGIN);
        assert_eq!(
            (plus.x, plus.y, plus.width, plus.height),
            (7.0, 7.0, 12.0, 12.0)
        );
        let (_, lock) = control_icon(Control::Interactive, true, Point::ORIGIN);
        assert!((lock.width - 9.375).abs() < 1e-4 && lock.height == 12.0);
        let (_, minus) = control_icon(Control::ZoomOut, true, Point::ORIGIN);
        assert!((minus.height - 1.875).abs() < 1e-4);
    }
}
