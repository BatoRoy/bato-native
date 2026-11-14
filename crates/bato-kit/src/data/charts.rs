//! Charts (`data/Charts.tsx`): [`line_chart`] / [`area_chart`] (one or more
//! series, 4 y-grid lines, ≤ 6 x labels, nearest-x hover with a dashed
//! crosshair, dots and a tooltip box top-right), [`bar_chart`] (hover dims
//! the other bars to .5 over 150 ms, tooltip) and [`sparkline`] (no axes).
//!
//! Drawn with [`super::draw`] (resvg layers, see there for why): the grid,
//! the area and the line are rasterized once per size and data; a hover
//! draws only the crosshair, two small dots per series and the tooltip. The
//! hover state is the widget's own, so the app gets no messages and an idle
//! chart draws nothing.
//!
//! ```ignore
//! line_chart(&cx, vec![Series::new("Visits", points)])                 // height 200
//! area_chart(&cx, series).height(150).format_y(|y| format!("{y}%"))
//! bar_chart(&cx, vec![Bar::new("Mon", 12.0), …])
//! sparkline(&cx, &values).area().size(120.0, 36.0)
//! ```

use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::{Color, Element, Event, Length, Point, Rectangle, Size, mouse, window};

use super::draw::{self, Anchor, Cap, Join, PathData};
use crate::El;
use crate::anim::{Tween, ease_standard};
use crate::ctx::Ctx;
use crate::label::Shaped;
use crate::overlay::tooltip::bubble_style;
use crate::style::c;
use crate::surface::paint;

/// `PAD` of Charts.tsx: the plot inside the SVG.
pub const PAD_L: f32 = 40.0;
pub const PAD_R: f32 = 12.0;
pub const PAD_T: f32 = 12.0;
pub const PAD_B: f32 = 24.0;

/// Axis label size (`fontSize={10}`).
const AXIS: f32 = 10.0;
/// Tooltip text (`text-xs`).
const TIP: f32 = 12.0;
const TIP_LH: f32 = 16.0;

// ── Data ────────────────────────────────────────────────────────────────

/// One series (`ChartSeries`): `(x, y)` points; for [`XType::Time`] x is
/// milliseconds since the Unix epoch (`Date.getTime()`).
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub name: String,
    pub data: Vec<(f64, f64)>,
    /// `None`: the accent.
    pub color: Option<Color>,
}

impl Series {
    pub fn new(name: impl Into<String>, data: Vec<(f64, f64)>) -> Self {
        Self {
            name: name.into(),
            data,
            color: None,
        }
    }
    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }
}

/// `xType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum XType {
    #[default]
    Number,
    /// x in epoch ms, labelled `MMM d` in local time.
    Time,
}

/// One bar (`{ label, value, color }`).
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    pub label: String,
    pub value: f64,
    pub color: Option<Color>,
}

impl Bar {
    pub fn new(label: impl Into<String>, value: f64) -> Self {
        Self {
            label: label.into(),
            value,
            color: None,
        }
    }
    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }
}

// ── Pure maths (unit-tested) ─────────────────────────────────────────────

/// JavaScript's `Math.round` (halves toward +∞).
pub fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// `String(n)` for the numbers the charts print.
pub fn js_string(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    format!("{v}")
}

/// The default y format: `String(Math.round(y * 100) / 100)`.
pub fn default_fy(y: f64) -> String {
    js_string(js_round(y * 100.0) / 100.0)
}

/// The default x format of a number axis: `String(Math.round(x))`.
pub fn default_fx(x: f64) -> String {
    js_string(js_round(x))
}

/// `format(new Date(x), 'MMM d')` in local time.
pub fn time_fx(x: f64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(x as i64).single() {
        Some(t) => crate::datefmt::format(&t.naive_local(), "MMM d"),
        None => String::new(),
    }
}

/// The data range (`bounds` of Charts.tsx).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min_x: f64,
    pub max_x: f64,
    pub min_y: f64,
    pub max_y: f64,
}

pub fn bounds(series: &[Series], include_zero: bool) -> Bounds {
    let pts = series.iter().flat_map(|s| s.data.iter());
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for &(x, y) in pts {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    if include_zero {
        min_y = min_y.min(0.0);
    }
    if max_y == min_y {
        max_y += 1.0;
    }
    Bounds {
        min_x,
        max_x,
        min_y,
        max_y,
    }
}

/// The plot geometry for a `w × h` chart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plot {
    pub w: f32,
    pub h: f32,
    pub b: Bounds,
}

impl Plot {
    pub fn iw(&self) -> f32 {
        (self.w - PAD_L - PAD_R).max(0.0)
    }
    pub fn ih(&self) -> f32 {
        self.h - PAD_T - PAD_B
    }
    pub fn sx(&self, x: f64) -> f32 {
        let b = self.b;
        PAD_L
            + if b.max_x == b.min_x {
                0.0
            } else {
                (((x - b.min_x) / (b.max_x - b.min_x)) * self.iw() as f64) as f32
            }
    }
    pub fn sy(&self, y: f64) -> f32 {
        let b = self.b;
        PAD_T + self.ih() - (((y - b.min_y) / (b.max_y - b.min_y)) * self.ih() as f64) as f32
    }
    /// The 5 y ticks, bottom first: `(value, py)`.
    pub fn y_ticks(&self) -> Vec<(f64, f32)> {
        (0..=4)
            .map(|i| {
                let y = self.b.min_y + (i as f64 / 4.0) * (self.b.max_y - self.b.min_y);
                (y, self.sy(y))
            })
            .collect()
    }
}

/// The indices of the ≤ 6 x labels over `n` points.
pub fn x_tick_indices(n: usize) -> Vec<usize> {
    let ticks = n.min(6);
    (0..ticks)
        .map(|i| {
            let f = i as f64 / (ticks.max(2) - 1) as f64;
            js_round(f * (n as f64 - 1.0)) as usize
        })
        .collect()
}

/// The point index nearest to `mx` (first wins a tie, like the React loop).
pub fn nearest(xs: &[f32], mx: f32) -> Option<usize> {
    let mut best = f32::INFINITY;
    let mut out = None;
    for (i, x) in xs.iter().enumerate() {
        let d = (x - mx).abs();
        if d < best {
            best = d;
            out = Some(i);
        }
    }
    out
}

/// Bar geometry: `(x, y, width, height)` of bar `i` of `n` in a `w × h` chart.
pub fn bar_rect(w: f32, h: f32, n: usize, i: usize, value: f64, max: f64) -> Rectangle {
    let iw = (w - PAD_L - PAD_R).max(0.0);
    let ih = h - PAD_T - PAD_B;
    let gap = iw / n as f32;
    let bw = gap * 0.62;
    let bh = ((value / max) as f32) * ih;
    Rectangle {
        x: PAD_L + i as f32 * gap + (gap - bw) / 2.0,
        y: PAD_T + ih - bh,
        width: bw,
        height: bh,
    }
}

// ── Shared drawing ──────────────────────────────────────────────────────

/// A positioned SVG `<text>`.
struct Text {
    shaped: Shaped,
    x: f32,
    baseline: f32,
    anchor: Anchor,
}

impl Text {
    fn new(s: String, x: f32, baseline: f32, anchor: Anchor) -> Self {
        Self {
            shaped: draw::shape(s, AXIS, 400),
            x,
            baseline,
            anchor,
        }
    }
    fn draw(&self, r: &mut iced::Renderer, o: Point, color: Color, clip: &Rectangle) {
        draw::text_at(
            r,
            &self.shaped,
            AXIS,
            o.x + self.x,
            o.y + self.baseline,
            self.anchor,
            color,
            clip,
        );
    }
    /// The glyph cell box (SVG text hit-testing), local px.
    fn cell(&self) -> Rectangle {
        let (left, w) = draw::anchored_x(&self.shaped, self.x, self.anchor);
        Rectangle {
            x: left,
            y: self.baseline - AXIS,
            width: w,
            height: AXIS * 1.25,
        }
    }
}

/// The gridlines: `<line x1=40 y1=py x2=w-12 y2=py strokeWidth=1>` each.
fn grid_body(w: f32, ys: &[f32]) -> String {
    let mut d = PathData::new();
    for &py in ys {
        d = d
            .move_to(Point::new(PAD_L, py))
            .line_to(Point::new(w - PAD_R, py));
    }
    draw::stroke(&d, 1.0, Cap::Butt, Join::Miter, None)
}

/// One tooltip line: an optional colour dot, then spans of `(text, color)`.
struct TipLine {
    dot: Option<Color>,
    shaped: Shaped,
    color: Color,
}

/// The tooltip box (`absolute top-2 right-2 … px-2 py-1 text-xs`).
fn draw_tip(
    r: &mut iced::Renderer,
    cx: &Ctx,
    bounds: Rectangle,
    lines: &[TipLine],
    clip: &Rectangle,
) {
    if lines.is_empty() {
        return;
    }
    let row_w = |l: &TipLine| l.shaped.width() + if l.dot.is_some() { 14.0 } else { 0.0 };
    let cw = lines.iter().map(row_w).fold(0.0, f32::max);
    let bw = (cw + 18.0).ceil();
    let bh = lines.len() as f32 * TIP_LH + 10.0;
    let b = Rectangle {
        x: bounds.x + bounds.width - 8.0 - bw,
        y: bounds.y + 8.0,
        width: bw,
        height: bh,
    };
    paint(r, b, &bubble_style(cx));
    for (i, l) in lines.iter().enumerate() {
        let top = b.y + 5.0 + i as f32 * TIP_LH;
        let mut x = b.x + 9.0;
        if let Some(dot) = l.dot {
            let o = Point::new(x, top + 4.0);
            let body = draw::fill(&draw::circle(Point::new(4.0, 4.0), 4.0));
            draw::layer(
                r,
                o,
                Rectangle::new(Point::ORIGIN, Size::new(8.0, 8.0)),
                &body,
                dot,
                1.0,
                clip,
            );
            x += 14.0;
        }
        l.shaped.draw(
            r,
            Rectangle {
                x,
                y: top,
                width: l.shaped.width(),
                height: TIP_LH,
            },
            l.color,
            clip,
        );
    }
}

fn tip_text(s: impl Into<String>) -> Shaped {
    crate::label::label(s, TIP, TIP_LH).shape(f32::INFINITY)
}

fn hash_f64s<H: Hasher>(h: &mut H, v: impl IntoIterator<Item = f64>) {
    for x in v {
        x.to_bits().hash(h);
    }
}

fn over(cursor: mouse::Cursor, b: Rectangle, viewport: &Rectangle) -> Option<Point> {
    let p = cursor.position()?;
    (b.contains(p) && viewport.contains(p)).then(|| Point::new(p.x - b.x, p.y - b.y))
}

// ── Line / area chart ───────────────────────────────────────────────────

type Fmt<'a> = Box<dyn Fn(f64) -> String + 'a>;

pub struct XyChart<'a> {
    cx: Ctx,
    series: Vec<Series>,
    height: f32,
    area: bool,
    x_type: XType,
    format_x: Option<Fmt<'a>>,
    format_y: Option<Fmt<'a>>,
}

/// `LineChart`: height 200, accent series.
pub fn line_chart<'a>(cx: &Ctx, series: Vec<Series>) -> XyChart<'a> {
    XyChart {
        cx: cx.clone(),
        series,
        height: 200.0,
        area: false,
        x_type: XType::Number,
        format_x: None,
        format_y: None,
    }
}

/// `AreaChart`: a line chart with the area under each series at .12 and a
/// y range that includes 0.
pub fn area_chart<'a>(cx: &Ctx, series: Vec<Series>) -> XyChart<'a> {
    XyChart {
        area: true,
        ..line_chart(cx, series)
    }
}

impl<'a> XyChart<'a> {
    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }
    pub fn x_type(mut self, t: XType) -> Self {
        self.x_type = t;
        self
    }
    pub fn format_x(mut self, f: impl Fn(f64) -> String + 'a) -> Self {
        self.format_x = Some(Box::new(f));
        self
    }
    pub fn format_y(mut self, f: impl Fn(f64) -> String + 'a) -> Self {
        self.format_y = Some(Box::new(f));
        self
    }

    fn fx(&self, x: f64) -> String {
        match (&self.format_x, self.x_type) {
            (Some(f), _) => f(x),
            (None, XType::Time) => time_fx(x),
            (None, XType::Number) => default_fx(x),
        }
    }
    fn fy(&self, y: f64) -> String {
        match &self.format_y {
            Some(f) => f(y),
            None => default_fy(y),
        }
    }
    fn plot(&self, w: f32) -> Plot {
        Plot {
            w,
            h: self.height,
            b: bounds(&self.series, self.area),
        }
    }
    fn empty(&self) -> bool {
        self.series.is_empty() || self.series[0].data.is_empty()
    }
}

#[derive(Default)]
struct XyState {
    key: u64,
    grid: String,
    /// Per series: (area body, line body).
    paths: Vec<(Option<String>, String)>,
    texts: Vec<Text>,
    /// x of each point of the first series (hover).
    xs: Vec<f32>,
    hover: Option<usize>,
    tip: Vec<TipLine>,
}

impl<'a> XyChart<'a> {
    fn key(&self, w: f32) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        w.to_bits().hash(&mut h);
        self.height.to_bits().hash(&mut h);
        self.area.hash(&mut h);
        for s in &self.series {
            hash_f64s(&mut h, s.data.iter().flat_map(|&(x, y)| [x, y]));
        }
        // Labels depend on the formatters, which cannot be hashed: hash
        // their output for the points they print.
        if let Some(s) = self.series.first() {
            for i in x_tick_indices(s.data.len()) {
                self.fx(s.data[i].0).hash(&mut h);
            }
            for (y, _) in self.plot(w).y_ticks() {
                self.fy(y).hash(&mut h);
            }
        }
        h.finish()
    }

    fn rebuild(&self, st: &mut XyState, w: f32) {
        let plot = self.plot(w);
        let ticks = plot.y_ticks();
        st.grid = grid_body(w, &ticks.iter().map(|t| t.1).collect::<Vec<_>>());
        st.texts.clear();
        for (y, py) in &ticks {
            st.texts
                .push(Text::new(self.fy(*y), PAD_L - 6.0, py + 3.0, Anchor::End));
        }
        let ref0 = &self.series[0].data;
        for i in x_tick_indices(ref0.len()) {
            let x = ref0[i].0;
            st.texts.push(Text::new(
                self.fx(x),
                plot.sx(x),
                self.height - 6.0,
                Anchor::Middle,
            ));
        }
        st.paths = self
            .series
            .iter()
            .map(|s| {
                let pts: Vec<Point> = s
                    .data
                    .iter()
                    .map(|&(x, y)| Point::new(plot.sx(x), plot.sy(y)))
                    .collect();
                let line = draw::polyline(&pts);
                let area = (self.area && !pts.is_empty()).then(|| {
                    let base = plot.sy(plot.b.min_y);
                    let d = line
                        .clone()
                        .line_to(Point::new(pts[pts.len() - 1].x, base))
                        .line_to(Point::new(pts[0].x, base))
                        .close();
                    draw::fill(&d)
                });
                (
                    area,
                    draw::stroke(&line, 2.0, Cap::Round, Join::Round, None),
                )
            })
            .collect();
        st.xs = ref0.iter().map(|&(x, _)| plot.sx(x)).collect();
    }

    fn tip_lines(&self, i: usize) -> Vec<TipLine> {
        let p = self.cx.p();
        let Some(&(x, _)) = self.series[0].data.get(i) else {
            return Vec::new();
        };
        let mut out = vec![TipLine {
            dot: None,
            shaped: tip_text(self.fx(x)),
            color: c(p.muted),
        }];
        for s in &self.series {
            if let Some(&(_, y)) = s.data.get(i) {
                out.push(TipLine {
                    dot: Some(s.color.unwrap_or(c(p.accent))),
                    shaped: tip_text(format!("{}: {}", s.name, self.fy(y))),
                    color: c(p.text),
                });
            }
        }
        out
    }
}

impl<'a, M> Widget<M, iced::Theme, iced::Renderer> for XyChart<'a> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(self.height))
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<XyState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(XyState::default())
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = limits.resolve(Length::Fill, Length::Fixed(self.height), Size::ZERO);
        let st = tree.state.downcast_mut::<XyState>();
        if self.empty() || size.width <= 0.0 {
            st.key = 0;
            st.paths.clear();
            st.texts.clear();
            st.xs.clear();
            st.grid.clear();
        } else {
            let key = self.key(size.width);
            if key != st.key {
                st.key = key;
                self.rebuild(st, size.width);
                if let Some(i) = st.hover {
                    st.tip = self.tip_lines(i);
                }
            }
        }
        layout::Node::new(size)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        if !matches!(
            event,
            Event::Mouse(
                mouse::Event::CursorMoved { .. }
                    | mouse::Event::CursorLeft
                    | mouse::Event::WheelScrolled { .. }
            ) | Event::Window(window::Event::RedrawRequested(_))
        ) {
            return;
        }
        let st = tree.state.downcast_mut::<XyState>();
        let hover = over(cursor, layout.bounds(), viewport).and_then(|p| nearest(&st.xs, p.x));
        if hover != st.hover {
            st.hover = hover;
            st.tip = hover.map(|i| self.tip_lines(i)).unwrap_or_default();
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_ref::<XyState>();
        if st.key == 0 {
            return;
        }
        let b = layout.bounds();
        let Some(clip) = b.expand(16.0).intersection(viewport) else {
            return;
        };
        let p = self.cx.p();
        let o = draw::snap_point(b.position());
        let full = Rectangle::new(Point::ORIGIN, b.size());
        draw::layer(renderer, o, full, &st.grid, c(p.border), 0.5, &clip);
        let muted = c(p.muted);
        for t in &st.texts {
            t.draw(renderer, o, muted, &clip);
        }
        for (s, (area, line)) in self.series.iter().zip(&st.paths) {
            let color = s.color.unwrap_or(c(p.accent));
            if let Some(area) = area {
                draw::layer(renderer, o, full, area, color, 0.12, &clip);
            }
            draw::layer(renderer, o, full, line, color, 1.0, &clip);
        }
        let Some(i) = st.hover else {
            return;
        };
        let plot = self.plot(b.width);
        let Some(&(x0, _)) = self.series[0].data.get(i) else {
            return;
        };
        // Crosshair: `strokeDasharray="3 3"`, 1 px, muted.
        let x = plot.sx(x0);
        let ih = plot.ih();
        let d = PathData::new()
            .move_to(Point::new(x, PAD_T))
            .line_to(Point::new(x, PAD_T + ih));
        let body = draw::stroke(&d, 1.0, Cap::Butt, Join::Miter, Some((3.0, 3.0)));
        let bb = Rectangle::new(Point::new(x - 1.0, PAD_T), Size::new(2.0, ih));
        draw::layer(renderer, o, bb, &body, muted, 1.0, &clip);
        // Dots: r 3.5 in the series colour, 2 px `stroke-app-bg` (centred
        // on the edge: a bg disc of 4.5 under a colour disc of 2.5).
        for s in &self.series {
            let Some(&(px, py)) = s.data.get(i) else {
                continue;
            };
            let cpt = Point::new(plot.sx(px), plot.sy(py));
            let color = s.color.unwrap_or(c(p.accent));
            for (r, col) in [(4.5, c(p.bg)), (2.5, color)] {
                let body = draw::fill(&draw::circle(cpt, r));
                let bb = draw::bbox_of(&[cpt], r);
                draw::layer(renderer, o, bb, &body, col, 1.0, &clip);
            }
        }
        draw_tip(renderer, &self.cx, b, &st.tip, viewport);
    }
}

impl<'a, M: 'a> From<XyChart<'a>> for El<'a, M> {
    fn from(c: XyChart<'a>) -> Self {
        Element::new(c)
    }
}

// ── Bar chart ───────────────────────────────────────────────────────────

/// `transition-opacity` (150 ms, Tailwind's ease).
const BAR_FADE: Duration = Duration::from_millis(150);

pub struct BarChart<'a> {
    cx: Ctx,
    data: Vec<Bar>,
    height: f32,
    format_y: Option<Fmt<'a>>,
}

/// `BarChart`: height 200, accent bars.
pub fn bar_chart<'a>(cx: &Ctx, data: Vec<Bar>) -> BarChart<'a> {
    BarChart {
        cx: cx.clone(),
        data,
        height: 200.0,
        format_y: None,
    }
}

impl<'a> BarChart<'a> {
    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }
    pub fn format_y(mut self, f: impl Fn(f64) -> String + 'a) -> Self {
        self.format_y = Some(Box::new(f));
        self
    }
    fn fy(&self, y: f64) -> String {
        match &self.format_y {
            Some(f) => f(y),
            None => default_fx(y),
        }
    }
    fn max(&self) -> f64 {
        self.data.iter().map(|d| d.value).fold(1.0, f64::max)
    }
}

#[derive(Default)]
struct BarState {
    key: u64,
    grid: String,
    texts: Vec<Text>,
    /// Per bar: its rectangle (local) and body.
    bars: Vec<(Rectangle, String)>,
    /// Per bar: the opacity tween.
    fade: Vec<Tween>,
    hover: Option<usize>,
    tip: Vec<TipLine>,
}

impl<'a> BarChart<'a> {
    fn key(&self, w: f32) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        w.to_bits().hash(&mut h);
        self.height.to_bits().hash(&mut h);
        for d in &self.data {
            d.label.hash(&mut h);
            d.value.to_bits().hash(&mut h);
        }
        let max = self.max();
        for i in 0..5 {
            self.fy(max - (i as f64 / 4.0) * max).hash(&mut h);
        }
        h.finish()
    }

    fn rebuild(&self, st: &mut BarState, w: f32) {
        let ih = self.height - PAD_T - PAD_B;
        let max = self.max();
        let ys: Vec<f32> = (0..5).map(|i| PAD_T + (i as f32 / 4.0) * ih).collect();
        st.grid = grid_body(w, &ys);
        st.texts.clear();
        for (i, py) in ys.iter().enumerate() {
            let val = max - (i as f64 / 4.0) * max;
            st.texts
                .push(Text::new(self.fy(val), PAD_L - 6.0, py + 3.0, Anchor::End));
        }
        let n = self.data.len();
        st.bars = self
            .data
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let r = bar_rect(w, self.height, n, i, d.value, max);
                (r, draw::fill(&draw::rounded_rect(r, 3.0)))
            })
            .collect();
        for ((r, _), d) in st.bars.iter().zip(&self.data) {
            st.texts.push(Text::new(
                d.label.clone(),
                r.x + r.width / 2.0,
                self.height - 6.0,
                Anchor::Middle,
            ));
        }
        st.fade.resize(n, Tween::settled(1.0));
    }

    /// The bar under `p`: its rect or its label (the `<g>`'s painted parts).
    fn hit(&self, st: &BarState, p: Point) -> Option<usize> {
        let labels = &st.texts[st.texts.len() - st.bars.len()..];
        st.bars
            .iter()
            .zip(labels)
            .position(|((r, _), t)| r.contains(p) || t.cell().contains(p))
    }
}

impl<'a, M> Widget<M, iced::Theme, iced::Renderer> for BarChart<'a> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(self.height))
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<BarState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(BarState::default())
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = limits.resolve(Length::Fill, Length::Fixed(self.height), Size::ZERO);
        let st = tree.state.downcast_mut::<BarState>();
        if size.width <= 0.0 {
            st.key = 0;
        } else {
            let key = self.key(size.width);
            if key != st.key {
                st.key = key;
                self.rebuild(st, size.width);
                if st.hover.is_some_and(|i| i >= self.data.len()) {
                    st.hover = None;
                    st.tip.clear();
                }
            }
        }
        layout::Node::new(size)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_mut::<BarState>();
        if st.key == 0 {
            return;
        }
        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                if st.fade.iter().any(|t| t.is_running(*now)) {
                    shell.request_redraw();
                }
            }
            Event::Mouse(
                mouse::Event::CursorMoved { .. }
                | mouse::Event::CursorLeft
                | mouse::Event::WheelScrolled { .. },
            ) => {
                let hover = over(cursor, layout.bounds(), viewport).and_then(|p| self.hit(st, p));
                if hover != st.hover {
                    st.hover = hover;
                    let now = Instant::now();
                    for (i, t) in st.fade.iter_mut().enumerate() {
                        let to = if hover.is_none() || hover == Some(i) {
                            1.0
                        } else {
                            0.5
                        };
                        if t.to != to {
                            t.retarget(to, now);
                            t.duration = BAR_FADE;
                            t.easing = ease_standard;
                        }
                    }
                    st.tip = hover
                        .map(|i| {
                            let d = &self.data[i];
                            vec![TipLine {
                                dot: None,
                                shaped: tip_text(format!("{}: {}", d.label, self.fy(d.value))),
                                color: c(self.cx.p().text),
                            }]
                        })
                        .unwrap_or_default();
                    shell.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_ref::<BarState>();
        if st.key == 0 {
            return;
        }
        let b = layout.bounds();
        let Some(clip) = b.expand(16.0).intersection(viewport) else {
            return;
        };
        let p = self.cx.p();
        let o = draw::snap_point(b.position());
        let full = Rectangle::new(Point::ORIGIN, b.size());
        draw::layer(renderer, o, full, &st.grid, c(p.border), 0.5, &clip);
        let muted = c(p.muted);
        let n_grid = st.texts.len() - st.bars.len();
        for t in &st.texts[..n_grid] {
            t.draw(renderer, o, muted, &clip);
        }
        let now = Instant::now();
        for (i, ((r, body), d)) in st.bars.iter().zip(&self.data).enumerate() {
            let a = st.fade.get(i).map_or(1.0, |t| t.value(now));
            let color = d.color.unwrap_or(c(p.accent));
            draw::layer(renderer, o, r.expand(1.0), body, color, a, &clip);
            st.texts[n_grid + i].draw(renderer, o, muted, &clip);
        }
        draw_tip(renderer, &self.cx, b, &st.tip, viewport);
    }
}

impl<'a, M: 'a> From<BarChart<'a>> for El<'a, M> {
    fn from(c: BarChart<'a>) -> Self {
        Element::new(c)
    }
}

// ── Sparkline ───────────────────────────────────────────────────────────

pub struct Sparkline {
    color: Color,
    data: Vec<f64>,
    width: f32,
    height: f32,
    area: bool,
}

/// `Sparkline`: 100 × 28, accent, no area. Fewer than 2 points draw nothing
/// (React returns `null`; here the widget keeps its size).
pub fn sparkline(cx: &Ctx, data: &[f64]) -> Sparkline {
    Sparkline {
        color: c(cx.p().accent),
        data: data.to_vec(),
        width: 100.0,
        height: 28.0,
        area: false,
    }
}

impl Sparkline {
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }
    pub fn color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }
    /// The area under the line at .15.
    pub fn area(mut self) -> Self {
        self.area = true;
        self
    }

    /// The line's points (`sx`, `sy` of Sparkline).
    pub fn points(&self) -> Vec<Point> {
        let n = self.data.len();
        if n < 2 {
            return Vec::new();
        }
        let min = self.data.iter().copied().fold(f64::INFINITY, f64::min);
        let max = self.data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let range = if max - min == 0.0 { 1.0 } else { max - min };
        let (w, h) = (self.width as f64, self.height as f64);
        self.data
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let x = (i as f64 / (n - 1) as f64) * w;
                let y = h - ((v - min) / range) * (h - 2.0) - 1.0;
                Point::new(x as f32, y as f32)
            })
            .collect()
    }
}

impl<M> Widget<M, iced::Theme, iced::Renderer> for Sparkline {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.width), Length::Fixed(self.height))
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(Size::new(self.width, self.height))
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let pts = self.points();
        if pts.is_empty() {
            return;
        }
        let b = layout.bounds();
        let Some(clip) = b.expand(2.0).intersection(viewport) else {
            return;
        };
        let o = draw::snap_point(b.position());
        let line = draw::polyline(&pts);
        let bb = Rectangle::new(Point::ORIGIN, b.size()).expand(1.0);
        if self.area {
            let d = line
                .clone()
                .line_to(Point::new(self.width, self.height))
                .line_to(Point::new(0.0, self.height))
                .close();
            draw::layer(renderer, o, bb, &draw::fill(&d), self.color, 0.15, &clip);
        }
        let body = draw::stroke(&line, 1.5, Cap::Round, Join::Round, None);
        draw::layer(renderer, o, bb, &body, self.color, 1.0, &clip);
    }
}

impl<'a, M: 'a> From<Sparkline> for El<'a, M> {
    fn from(s: Sparkline) -> Self {
        Element::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn showcase_series() -> Vec<Series> {
        let data = (0..14)
            .map(|i| {
                let i = i as f64;
                (i, 20.0 + js_round(30.0 * (i / 2.0).sin() + i * 2.0))
            })
            .collect();
        vec![Series::new("Visits", data)]
    }

    #[test]
    fn js_number_formatting() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(default_fy(44.25), "44.25");
        assert_eq!(default_fy(20.754), "20.75");
        assert_eq!(default_fy(9.0), "9");
        assert_eq!(default_fy(-0.001), "0");
        assert_eq!(default_fx(16.5), "17");
    }

    #[test]
    fn bounds_and_ticks_match_the_reference_chart() {
        let s = showcase_series();
        // Line chart: 9 … 56 (labels 9, 20.75, 32.5, 44.25, 56).
        let b = bounds(&s, false);
        assert_eq!((b.min_y, b.max_y), (9.0, 56.0));
        let plot = Plot {
            w: 368.0,
            h: 200.0,
            b,
        };
        let labels: Vec<String> = plot.y_ticks().iter().map(|t| default_fy(t.0)).collect();
        assert_eq!(labels, ["9", "20.75", "32.5", "44.25", "56"]);
        assert_eq!(plot.y_ticks()[4].1, 12.0);
        assert_eq!(plot.y_ticks()[0].1, 176.0);
        // Area chart includes zero: 0, 14, 28, 42, 56.
        let b = bounds(&s, true);
        let plot = Plot {
            w: 368.0,
            h: 200.0,
            b,
        };
        let labels: Vec<String> = plot.y_ticks().iter().map(|t| default_fy(t.0)).collect();
        assert_eq!(labels, ["0", "14", "28", "42", "56"]);
        // x labels 0 3 5 8 10 13.
        assert_eq!(x_tick_indices(14), [0, 3, 5, 8, 10, 13]);
        assert_eq!(x_tick_indices(1), [0]);
        assert_eq!(x_tick_indices(3), [0, 1, 2]);
    }

    #[test]
    fn flat_data_gets_a_unit_range() {
        let b = bounds(&[Series::new("a", vec![(0.0, 5.0), (1.0, 5.0)])], false);
        assert_eq!((b.min_y, b.max_y), (5.0, 6.0));
    }

    #[test]
    fn nearest_point_first_wins_ties() {
        assert_eq!(nearest(&[0.0, 10.0, 20.0], 14.0), Some(1));
        assert_eq!(nearest(&[0.0, 10.0], 5.0), Some(0));
        assert_eq!(nearest(&[], 5.0), None);
    }

    #[test]
    fn bars_and_their_labels() {
        // Mon 12 … Thu 22: y labels 22, 17 (16.5 rounds up), 11, 6, 0.
        let max = 22.0;
        let labels: Vec<String> = (0..5)
            .map(|i| default_fx(max - (i as f64 / 4.0) * max))
            .collect();
        assert_eq!(labels, ["22", "17", "11", "6", "0"]);
        let r = bar_rect(368.0, 200.0, 5, 0, 12.0, max);
        let gap = (368.0 - 52.0) / 5.0;
        assert!((r.width - gap * 0.62).abs() < 1e-4);
        assert!((r.x - (40.0 + (gap - gap * 0.62) / 2.0)).abs() < 1e-4);
        assert!((r.y + r.height - 176.0).abs() < 1e-4);
    }

    #[test]
    fn sparkline_points() {
        let cx = Ctx::from_palette(bato_theme::palette(
            bato_theme::ThemeId::Dark,
            bato_theme::Rgb::hex("#8b5cf6").unwrap(),
        ));
        let s = sparkline(&cx, &[1.0, 3.0, 2.0]).size(100.0, 28.0);
        let p = s.points();
        assert_eq!(p[0], Point::new(0.0, 27.0));
        assert_eq!(p[1], Point::new(50.0, 1.0));
        assert_eq!(p[2], Point::new(100.0, 14.0));
        assert!(sparkline(&cx, &[1.0]).points().is_empty());
    }
}
