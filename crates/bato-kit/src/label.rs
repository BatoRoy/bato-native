//! `Label`: text that sits exactly where Chromium puts it.
//!
//! iced's own `text` widget centres glyphs with cosmic-text's unrounded
//! metrics, which lands baselines up to a pixel away from Chromium's. This
//! widget shapes with cosmic-text (the same shaper iced uses) but places every
//! line itself, following Blink's line-box model:
//!
//! ```text
//! A = round(ascent_em × size), D = round(descent_em × size)   (device px)
//! baseline = line_top + A + floor((line_height − (A + D)) / 2)
//! ```
//!
//! then rounds the baseline to a device pixel, like Skia does for horizontal
//! text. Each visual line is drawn as its own buffer through `fill_raw`, so
//! wrapped paragraphs and mixed fonts cannot drift. Glyph x positions keep
//! cosmic-text's quarter-pixel subpixel bins, which is also what Skia uses.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use iced::advanced::graphics::text::cosmic_text as ct;
use iced::advanced::graphics::text::{Raw, Renderer as RawRenderer, font_system};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Widget, tree};
use iced::{Color, Element, Length, Rectangle, Size, mouse};

use crate::fonts;
use crate::scale::device_scale;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fam {
    Sans,
    Mono,
}

impl Fam {
    fn name(self) -> &'static str {
        match self {
            Fam::Sans => fonts::SANS,
            Fam::Mono => fonts::MONO,
        }
    }
    fn asc_desc(self) -> (f32, f32) {
        match self {
            Fam::Sans => (fonts::INTER_ASC, fonts::INTER_DESC),
            Fam::Mono => (fonts::JBM_ASC, fonts::JBM_DESC),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub fam: Option<Fam>,
    pub size: Option<f32>,
    pub weight: Option<u16>,
    pub color: Option<Color>,
    /// `italic` (the bundled Inter / JetBrains Mono italic faces).
    pub italic: Option<bool>,
    /// `underline` / `line-through` (in the span's colour).
    pub deco: Deco,
}

/// Text decorations (`text-decoration-line`), drawn in the text's colour
/// with the font's underline metrics, the way Blink places them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Deco {
    pub underline: bool,
    pub strike: bool,
}

impl Deco {
    fn any(self) -> bool {
        self.underline || self.strike
    }
    fn or(self, o: Deco) -> Deco {
        Deco {
            underline: self.underline || o.underline,
            strike: self.strike || o.strike,
        }
    }
}

impl Span {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            fam: None,
            size: None,
            weight: None,
            color: None,
            italic: None,
            deco: Deco::default(),
        }
    }
    pub fn mono(mut self) -> Self {
        self.fam = Some(Fam::Mono);
        self
    }
    /// `font-bold` (700).
    pub fn bold(self) -> Self {
        self.weight(700)
    }
    /// `italic`.
    pub fn italic(mut self) -> Self {
        self.italic = Some(true);
        self
    }
    /// `underline`.
    pub fn underline(mut self) -> Self {
        self.deco.underline = true;
        self
    }
    /// `line-through`.
    pub fn strike(mut self) -> Self {
        self.deco.strike = true;
        self
    }
    pub fn size(mut self, s: f32) -> Self {
        self.size = Some(s);
        self
    }
    pub fn weight(mut self, w: u16) -> Self {
        self.weight = Some(w);
        self
    }
    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub struct Label {
    spans: Vec<Span>,
    fam: Fam,
    size: f32,
    line_height: f32,
    weight: u16,
    color: Option<Color>,
    tracking_em: f32,
    wrap: bool,
    ellipsis: bool,
    align: Align,
    width: Length,
    max_width: f32,
    tabular: bool,
    /// `<wbr>` positions (byte offsets of the whole text).
    hints: Vec<usize>,
    /// `overflow-wrap: anywhere`.
    anywhere: bool,
    /// `line-clamp-N`.
    max_lines: Option<usize>,
    italic: bool,
    deco: Deco,
    /// `hover:underline`.
    hover_underline: bool,
}

/// `text-[N]px` with Tailwind's default `line-height: 1.5`.
pub fn label(text: impl Into<String>, size: f32, line_height: f32) -> Label {
    Label {
        spans: vec![Span::new(text)],
        fam: Fam::Sans,
        size,
        line_height,
        weight: 400,
        color: None,
        tracking_em: 0.0,
        wrap: false,
        ellipsis: false,
        align: Align::Left,
        width: Length::Shrink,
        max_width: f32::INFINITY,
        tabular: false,
        hints: Vec::new(),
        anywhere: false,
        max_lines: None,
        italic: false,
        deco: Deco::default(),
        hover_underline: false,
    }
}

pub fn rich(spans: Vec<Span>, size: f32, line_height: f32) -> Label {
    let mut l = label("", size, line_height);
    l.spans = spans;
    l
}

impl Label {
    pub fn mono(mut self) -> Self {
        self.fam = Fam::Mono;
        self
    }
    pub fn weight(mut self, w: u16) -> Self {
        self.weight = w;
        self
    }
    pub fn medium(self) -> Self {
        self.weight(500)
    }
    pub fn semibold(self) -> Self {
        self.weight(600)
    }
    /// `font-bold` (700).
    pub fn bold(self) -> Self {
        self.weight(700)
    }
    /// `italic`: Inter's and JetBrains Mono's real italic faces (bundled).
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }
    /// `underline`, in the text colour, every line.
    pub fn underline(mut self) -> Self {
        self.deco.underline = true;
        self
    }
    /// `line-through`, in the text colour, every line.
    pub fn strikethrough(mut self) -> Self {
        self.deco.strike = true;
        self
    }
    /// `hover:underline`: underlined while the pointer is over the label
    /// (links, a toast's action).
    pub fn hover_underline(mut self) -> Self {
        self.hover_underline = true;
        self
    }
    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }
    pub fn color_maybe(mut self, c: Option<Color>) -> Self {
        self.color = c;
        self
    }
    /// letter-spacing in em (`tracking-tight` = −0.025).
    pub fn tracking(mut self, em: f32) -> Self {
        self.tracking_em = em;
        self
    }
    /// Wrap at the available width (`\n` always breaks a line, wrapped or
    /// not: `white-space: pre-wrap` / `pre`).
    pub fn wrap(mut self) -> Self {
        self.wrap = true;
        self.width = Length::Fill;
        self
    }
    /// Extra wrap opportunities (`<wbr>`) at these byte offsets of the text
    /// (BatoBrowse's `nameSegments`: `ruby-|pickaxe.png`). Only matter with
    /// [`Label::wrap`].
    pub fn wrap_hints(mut self, offsets: impl IntoIterator<Item = usize>) -> Self {
        self.hints = offsets.into_iter().collect();
        self.hints.sort_unstable();
        self.hints.dedup();
        self
    }
    /// `overflow-wrap: anywhere`: a word wider than the line breaks between
    /// any two characters instead of overflowing.
    pub fn break_anywhere(mut self) -> Self {
        self.anywhere = true;
        self
    }
    /// `line-clamp-N`: at most `n` lines, the last one cut with "…" when
    /// there is more.
    pub fn max_lines(mut self, n: usize) -> Self {
        self.max_lines = Some(n.max(1));
        self
    }
    pub fn ellipsis(mut self) -> Self {
        self.ellipsis = true;
        self
    }
    pub fn align(mut self, a: Align) -> Self {
        self.align = a;
        self
    }
    pub fn width(mut self, w: impl Into<Length>) -> Self {
        self.width = w.into();
        self
    }
    pub fn max_width(mut self, w: f32) -> Self {
        self.max_width = w;
        self
    }
    pub fn tabular(mut self) -> Self {
        self.tabular = true;
        self
    }

    fn key(&self, avail: f32) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        for s in &self.spans {
            s.text.hash(&mut h);
            s.fam.hash(&mut h);
            s.size.map(f32::to_bits).hash(&mut h);
            s.weight.hash(&mut h);
            s.italic.hash(&mut h);
            s.deco.hash(&mut h);
            s.color
                .map(|c| [c.r, c.g, c.b, c.a].map(f32::to_bits))
                .hash(&mut h);
        }
        self.fam.hash(&mut h);
        self.size.to_bits().hash(&mut h);
        self.line_height.to_bits().hash(&mut h);
        self.weight.hash(&mut h);
        self.tracking_em.to_bits().hash(&mut h);
        self.wrap.hash(&mut h);
        self.ellipsis.hash(&mut h);
        self.tabular.hash(&mut h);
        self.hints.hash(&mut h);
        self.anywhere.hash(&mut h);
        self.max_lines.hash(&mut h);
        self.italic.hash(&mut h);
        self.deco.hash(&mut h);
        if self.wrap || self.ellipsis {
            avail.to_bits().hash(&mut h);
        }
        h.finish()
    }

    fn attrs<'a>(&self, s: &'a Span) -> ct::Attrs<'a> {
        let fam = s.fam.unwrap_or(self.fam);
        let size = s.size.unwrap_or(self.size);
        let mut a = ct::Attrs::new()
            .family(ct::Family::Name(fam.name()))
            .weight(ct::Weight(s.weight.unwrap_or(self.weight)))
            .metrics(ct::Metrics::new(size, self.line_height));
        if s.italic.unwrap_or(self.italic) {
            a = a.style(ct::Style::Italic);
        }
        if self.tracking_em != 0.0 {
            a = a.letter_spacing(self.tracking_em);
        }
        if let Some(c) = s.color {
            let [r, g, b, al] = c.into_rgba8();
            a = a.color(ct::Color::rgba(r, g, b, al));
        }
        if self.tabular {
            let mut f = ct::FontFeatures::new();
            f.enable(ct::FeatureTag::new(b"tnum"));
            a = a.font_features(f);
        }
        a
    }

    fn buffer(
        &self,
        fs: &mut ct::FontSystem,
        spans: &[(String, usize)],
        width: Option<f32>,
    ) -> ct::Buffer {
        let mut buf = ct::Buffer::new(fs, ct::Metrics::new(self.size, self.line_height));
        let mut default = ct::Attrs::new()
            .family(ct::Family::Name(self.fam.name()))
            .weight(ct::Weight(self.weight));
        if self.italic {
            default = default.style(ct::Style::Italic);
        }
        buf.set_wrap(
            fs,
            if width.is_some() {
                if self.anywhere {
                    ct::Wrap::WordOrGlyph
                } else {
                    ct::Wrap::Word
                }
            } else {
                ct::Wrap::None
            },
        );
        buf.set_size(fs, width, None);
        buf.set_rich_text(
            fs,
            spans
                .iter()
                .map(|(t, i)| (t.as_str(), self.attrs(&self.spans[*i]).metadata(*i))),
            &default,
            ct::Shaping::Advanced,
            None,
        );
        buf.shape_until_scroll(fs, false);
        buf
    }

    fn build(&self, avail: f32) -> Cache {
        let mut guard = font_system().write().expect("font system");
        let fs = guard.raw();
        let spans: Vec<(String, usize)> = with_hints(
            self.spans
                .iter()
                .enumerate()
                .map(|(i, s)| (s.text.clone(), i))
                .collect(),
            &self.hints,
        );
        let wrap_w = if self.wrap && avail.is_finite() {
            Some(avail.max(1.0))
        } else {
            None
        };
        let full = self.buffer(fs, &spans, wrap_w);

        let full_text: String = spans.iter().map(|(t, _)| t.as_str()).collect();
        // Glyph offsets count from the start of their `\n`-separated line.
        let starts: Vec<usize> = ct::LineIter::new(&full_text)
            .map(|(r, _)| r.start)
            .collect();
        // Byte ranges of each visual line in the concatenated text.
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for run in full.layout_runs() {
            let base = starts.get(run.line_i).copied().unwrap_or(0);
            let (Some(first), Some(last)) = (run.glyphs.first(), run.glyphs.last()) else {
                ranges.push((base, base));
                continue;
            };
            let start = run
                .glyphs
                .iter()
                .map(|g| g.start)
                .min()
                .unwrap_or(first.start);
            let end = run.glyphs.iter().map(|g| g.end).max().unwrap_or(last.end);
            ranges.push((base + start, base + end));
        }
        if ranges.is_empty() {
            ranges.push((0, 0));
        }
        // line-clamp: the last kept line runs to the end of the text.
        let clamped = match self.max_lines {
            Some(n) if ranges.len() > n => {
                ranges.truncate(n);
                if let Some(last) = ranges.last_mut() {
                    last.1 = full_text.len();
                }
                true
            }
            _ => false,
        };
        let line_count = ranges.len();

        let mut lines = Vec::with_capacity(ranges.len());
        for (n, (start, mut end)) in ranges.into_iter().enumerate() {
            let cut_here = clamped && n + 1 == line_count;
            // Trailing spaces hang in CSS; they do not count toward width.
            while end > start && matches!(full_text.as_bytes()[end - 1], b' ' | b'\n' | b'\r') {
                end -= 1;
            }
            let mut line_spans = slice_spans(&spans, start, end);
            if cut_here {
                for s in &mut line_spans {
                    s.0 = s.0.replace(['\n', '\r'], " ");
                }
            }
            let mut buf = self.buffer(fs, &line_spans, None);
            let mut w = line_width(&buf);
            if cut_here && (w <= avail || !avail.is_finite()) {
                // More text follows on hidden lines: "…" after this one.
                let idx = line_spans.last().map(|x| x.1).unwrap_or(0);
                line_spans.push(("…".to_string(), idx));
                buf = self.buffer(fs, &line_spans, None);
                w = line_width(&buf);
            }
            if (self.ellipsis || cut_here) && avail.is_finite() && w > avail + 0.01 {
                // Cut at the last glyph boundary that leaves room for "…".
                let ell = {
                    let last_style = line_spans.last().map(|s| s.1).unwrap_or(0);
                    let b = self.buffer(fs, &[("…".to_string(), last_style)], None);
                    line_width(&b)
                };
                let mut cut = 0usize;
                if let Some(run) = buf.layout_runs().next() {
                    for g in run.glyphs {
                        if g.x + g.w + ell <= avail {
                            cut = g.end;
                        } else {
                            break;
                        }
                    }
                }
                let mut s = slice_spans(&line_spans, 0, cut);
                while let Some(last) = s.last_mut() {
                    let t = last.0.trim_end().to_string();
                    if t.is_empty() {
                        s.pop();
                    } else {
                        last.0 = t;
                        break;
                    }
                }
                let idx = s
                    .last()
                    .map(|x| x.1)
                    .or(line_spans.first().map(|x| x.1))
                    .unwrap_or(0);
                s.push(("…".to_string(), idx));
                line_spans = s;
                buf = self.buffer(fs, &line_spans, None);
                w = line_width(&buf);
            }
            let line_y = buf
                .layout_runs()
                .next()
                .map(|r| r.line_y)
                .unwrap_or(self.line_height);
            let decos = self.decorations(&buf);
            // Dominant family of the line decides the strut metrics: the
            // block's own family (CSS uses the parent's strut).
            lines.push(Line {
                buffer: Arc::new(buf),
                width: w,
                line_y,
                decos,
            });
        }
        let width = lines.iter().map(|l| l.width).fold(0.0, f32::max);
        Cache {
            key: 0,
            lines,
            width,
            hovered: false,
        }
    }

    /// The decorated stretches of a one-line buffer: consecutive glyphs of
    /// spans (or a label) with the same decoration and colour.
    fn decorations(&self, buf: &ct::Buffer) -> Vec<DecoRun> {
        let mut out: Vec<DecoRun> = Vec::new();
        let Some(run) = buf.layout_runs().next() else {
            return out;
        };
        for g in run.glyphs {
            let span = self.spans.get(g.metadata);
            let deco = span.map_or(Deco::default(), |s| s.deco).or(self.deco);
            if !deco.any() {
                continue;
            }
            let color = span.and_then(|s| s.color);
            match out.last_mut() {
                Some(last)
                    if last.deco == deco && last.color == color && (last.x1 - g.x).abs() < 0.5 =>
                {
                    last.x1 = g.x + g.w;
                }
                _ => out.push(DecoRun {
                    x0: g.x,
                    x1: g.x + g.w,
                    deco,
                    color,
                }),
            }
        }
        out
    }
}

/// Insert a zero-width space (a break opportunity, like `<wbr>`) at each
/// byte offset of the concatenated spans.
fn with_hints(spans: Vec<(String, usize)>, hints: &[usize]) -> Vec<(String, usize)> {
    if hints.is_empty() {
        return spans;
    }
    let mut pos = 0;
    spans
        .into_iter()
        .map(|(t, i)| {
            let (s0, s1) = (pos, pos + t.len());
            pos = s1;
            let mut out = String::with_capacity(t.len() + 3 * hints.len());
            let mut last = 0;
            for &h in hints.iter().filter(|h| **h > s0 && **h < s1) {
                let at = h - s0;
                if t.is_char_boundary(at) {
                    out.push_str(&t[last..at]);
                    out.push('\u{200B}');
                    last = at;
                }
            }
            out.push_str(&t[last..]);
            (out, i)
        })
        .collect()
}

fn line_width(buf: &ct::Buffer) -> f32 {
    buf.layout_runs().map(|r| r.line_w).fold(0.0, f32::max)
}

/// The spans restricted to bytes `start..end` of their concatenation.
fn slice_spans(spans: &[(String, usize)], start: usize, end: usize) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut pos = 0;
    for (t, i) in spans {
        let (s0, s1) = (pos, pos + t.len());
        pos = s1;
        let a = start.max(s0);
        let b = end.min(s1);
        if a < b {
            out.push((t[a - s0..b - s0].to_string(), *i));
        }
    }
    out
}

/// A shaped label, for widgets that draw text without a widget tree
/// (the virtualized Scroll lab rows).
pub struct Shaped {
    cache: Cache,
    fam: Fam,
    size: f32,
    line_height: f32,
    align: Align,
}

impl Shaped {
    /// The widest line, in logical px (INF-12 text measurement).
    pub fn width(&self) -> f32 {
        self.cache.width
    }
    pub fn line_count(&self) -> usize {
        self.cache.lines.len()
    }
    pub fn height(&self) -> f32 {
        self.line_height * self.cache.lines.len() as f32
    }
    pub fn draw(
        &self,
        renderer: &mut iced::Renderer,
        b: Rectangle,
        color: Color,
        viewport: &Rectangle,
    ) {
        draw_lines(
            renderer,
            &self.cache,
            self.fam,
            self.size,
            self.line_height,
            self.align,
            b,
            color,
            viewport,
        );
    }
}

impl Label {
    pub fn shape(&self, avail: f32) -> Shaped {
        Shaped {
            cache: self.build(avail),
            fam: self.fam,
            size: self.size,
            line_height: self.line_height,
            align: self.align,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_lines(
    renderer: &mut iced::Renderer,
    cache: &Cache,
    fam: Fam,
    size: f32,
    line_height: f32,
    align: Align,
    b: Rectangle,
    color: Color,
    viewport: &Rectangle,
) {
    draw_lines_deco(
        renderer,
        cache,
        fam,
        size,
        line_height,
        align,
        b,
        color,
        viewport,
        false,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_lines_deco(
    renderer: &mut iced::Renderer,
    cache: &Cache,
    fam: Fam,
    size: f32,
    line_height: f32,
    align: Align,
    b: Rectangle,
    color: Color,
    viewport: &Rectangle,
    underline_all: bool,
) {
    use iced::advanced::Renderer as _;
    let s = device_scale();
    let color = crate::anim::fade(color);
    let base = blink_baseline(fam, size, line_height, s);
    let clip = Rectangle {
        x: viewport.x - 2.0,
        y: viewport.y - 2.0,
        width: viewport.width + 4.0,
        height: viewport.height + 4.0,
    };
    for (i, line) in cache.lines.iter().enumerate() {
        let top = b.y + i as f32 * line_height;
        if top > viewport.y + viewport.height + line_height || top + 2.0 * line_height < viewport.y
        {
            continue;
        }
        let baseline_dev = (top * s + base).round();
        let t = baseline_dev - (line.line_y * s).round();
        let x = match align {
            Align::Left => b.x,
            Align::Center => b.x + (b.width - line.width) / 2.0,
            Align::Right => b.x + b.width - line.width,
        };
        renderer.fill_raw(Raw {
            buffer: Arc::downgrade(&line.buffer),
            position: iced::Point::new(x, (t + 0.25) / s),
            color,
            clip_bounds: clip,
        });
        if line.decos.is_empty() && !underline_all {
            continue;
        }
        let (under, strike, thick) = deco_metrics(fam, size, s);
        let mut bar = |x0: f32, x1: f32, dy: f32, c: Color| {
            let left = ((x + x0) * s).round();
            let right = ((x + x1) * s).round();
            if right <= left {
                return;
            }
            renderer.fill_quad(
                iced::advanced::renderer::Quad {
                    bounds: Rectangle {
                        x: left / s,
                        y: (baseline_dev + dy) / s,
                        width: (right - left) / s,
                        height: thick / s,
                    },
                    ..Default::default()
                },
                c,
            );
        };
        if underline_all {
            bar(0.0, line.width, under, color);
        }
        for d in &line.decos {
            let c = d.color.map_or(color, crate::anim::fade);
            if d.deco.underline && !underline_all {
                bar(d.x0, d.x1, under, c);
            }
            if d.deco.strike {
                bar(d.x0, d.x1, strike, c);
            }
        }
    }
}

struct Line {
    buffer: Arc<ct::Buffer>,
    width: f32,
    /// cosmic-text's baseline inside the one-line buffer (logical px).
    line_y: f32,
    decos: Vec<DecoRun>,
}

/// An underlined / struck stretch of a line, x relative to the line start.
#[derive(Debug, Clone, Copy, PartialEq)]
struct DecoRun {
    x0: f32,
    x1: f32,
    deco: Deco,
    color: Option<Color>,
}

#[derive(Default)]
struct Cache {
    key: u64,
    lines: Vec<Line>,
    width: f32,
    /// The pointer is over the label (`hover:underline`).
    hovered: bool,
}

/// Where a decoration line sits, in device px from the rounded baseline
/// (positive = below), and how thick it is: Skia's underline metrics from
/// the font (`post` table: Inter −348/140 of 2048, JetBrains Mono −155/50
/// of 1000), Blink's rounding (thickness at least 1 device px), and
/// `line-through` at a third of the ascent above the baseline.
pub fn deco_metrics(fam: Fam, size: f32, scale: f32) -> (f32, f32, f32) {
    let (pos, thick) = match fam {
        Fam::Sans => (348.0 / 2048.0, 140.0 / 2048.0),
        Fam::Mono => (155.0 / 1000.0, 50.0 / 1000.0),
    };
    let (asc, _) = fam.asc_desc();
    let t = (thick * size * scale).round().max(1.0);
    let under = (pos * size * scale).round();
    let strike = -((asc * size * scale).round() / 3.0 + t / 2.0).round();
    (under, strike, t)
}

/// Blink's baseline offset from the top of a line box, in device px
/// (not yet rounded): `A + floor(half-leading)`.
pub fn blink_baseline(fam: Fam, size: f32, line_height: f32, scale: f32) -> f32 {
    let (asc, desc) = fam.asc_desc();
    let a = (asc * size * scale).round();
    let d = (desc * size * scale).round();
    // LayoutUnit is 1/64 px; half-leading is computed there, then floored.
    let lh = (line_height * scale * 64.0).round() / 64.0;
    let half = (((lh - (a + d)) / 2.0) * 64.0).floor() / 64.0;
    a + half.floor()
}

impl<Message, Theme> Widget<Message, Theme, iced::Renderer> for Label {
    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Shrink)
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Cache>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Cache::default())
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(self.width).max_width(self.max_width);
        let avail = limits.max().width;
        let key = self.key(avail);
        let cache = tree.state.downcast_mut::<Cache>();
        if cache.key != key || cache.lines.is_empty() {
            *cache = self.build(avail);
            cache.key = key;
        }
        let h = self.line_height * cache.lines.len() as f32;
        let w = match self.width {
            Length::Shrink => cache.width.min(avail),
            _ => {
                limits
                    .resolve(self.width, Length::Shrink, Size::new(cache.width, h))
                    .width
            }
        };
        layout::Node::new(Size::new(w, h))
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let cache = tree.state.downcast_ref::<Cache>();
        let color = self.color.unwrap_or(style.text_color);
        draw_lines_deco(
            renderer,
            cache,
            self.fam,
            self.size,
            self.line_height,
            self.align,
            layout.bounds(),
            color,
            viewport,
            self.hover_underline && cache.hovered,
        );
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        if !self.hover_underline {
            return;
        }
        if let iced::Event::Mouse(_) | iced::Event::Touch(_) | iced::Event::Window(_) = event {
            let cache = tree.state.downcast_mut::<Cache>();
            let over = cursor.is_over(layout.bounds());
            if over != cache.hovered {
                cache.hovered = over;
                shell.request_redraw();
            }
        }
    }
}

impl<'a, Message: 'a, Theme: 'a> From<Label> for Element<'a, Message, Theme, iced::Renderer> {
    fn from(l: Label) -> Self {
        Element::new(l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inter is variable: weights 500/600 must come from the same face with
    /// `wght` applied, not from another family (cosmic-text's exact-weight
    /// matching used to pick NotoSans-Medium / FreeSans Bold). Reference
    /// widths are HarfBuzz's for InterVariable at 13 px.
    #[test]
    fn variable_weights_shape_like_harfbuzz() {
        crate::fonts::setup();
        for (t, w, hb) in [
            ("Primary", 400, 47.5625),
            ("Primary", 500, 48.40625),
            ("Secondary", 500, 67.34375),
            ("Secondary", 600, 68.1875),
        ] {
            let got = label(t, 13.0, 19.5).weight(w).shape(f32::INFINITY).width();
            assert!((got - hb).abs() < 0.05, "{t} {w}: {got} vs {hb}");
        }
    }

    #[test]
    fn blink_baselines() {
        // Sidebar nav row: 13/19.5 → baseline 14 px below the line top.
        assert_eq!(blink_baseline(Fam::Sans, 13.0, 19.5, 1.0), 14.0);
        // 14/20 → 15 (A=14, D=3, floor(1.5)=1).
        assert_eq!(blink_baseline(Fam::Sans, 14.0, 20.0, 1.0), 15.0);
        // Sidebar version, JetBrains Mono 11/13.75: negative half-leading floors to -1.
        assert_eq!(blink_baseline(Fam::Mono, 11.0, 13.75, 1.0), 10.0);
    }

    fn lines(l: &Label, avail: f32) -> Vec<String> {
        crate::fonts::setup();
        l.build(avail)
            .lines
            .iter()
            .map(|ln| {
                ln.buffer
                    .lines
                    .iter()
                    .map(|b| b.text().replace('\u{200B}', "|"))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .collect()
    }

    #[test]
    fn newlines_break_lines() {
        let l = label("one\ntwo\n\nfour", 13.0, 20.0);
        assert_eq!(lines(&l, f32::INFINITY), ["one", "two", "", "four"]);
        assert_eq!(l.shape(f32::INFINITY).height(), 80.0);
        // Wrapped too, and the lines after a break keep their own offsets.
        let l = label("alpha beta\ngamma", 13.0, 20.0).wrap();
        assert_eq!(lines(&l, 500.0), ["alpha beta", "gamma"]);
        // Spans across a break.
        let l = rich(vec![Span::new("a\nb"), Span::new("c").mono()], 13.0, 20.0);
        assert_eq!(lines(&l, f32::INFINITY), ["a", "bc"]);
    }

    #[test]
    fn wrap_hints_break_anywhere_and_clamp() {
        let name = "SomeCamelCaseFileName.png";
        let full = label(name, 13.0, 20.0).shape(f32::INFINITY).width();
        let avail = (full * 0.6).floor();
        // No break opportunity: one overflowing line (CSS without hints).
        let plain = label(name, 13.0, 20.0).wrap();
        assert_eq!(lines(&plain, avail).len(), 1);
        // <wbr> at the camel humps: breaks there, text otherwise intact.
        let hinted = label(name, 13.0, 20.0).wrap().wrap_hints([4, 9, 13, 17]);
        let got = lines(&hinted, avail);
        assert!(got.len() >= 2, "{got:?}");
        assert_eq!(got.concat().replace('|', ""), name);
        assert!(
            got[0].ends_with("Camel|") || got[0].ends_with("Case|"),
            "{got:?}"
        );
        let unwrapped = label(name, 13.0, 20.0).wrap_hints([4, 9, 13, 17]);
        assert_eq!(
            unwrapped.shape(f32::INFINITY).width(),
            full,
            "a hint is zero-width"
        );
        // overflow-wrap: anywhere: breaks inside the word.
        let any = label(name, 13.0, 20.0).wrap().break_anywhere();
        let got = lines(&any, avail);
        assert!(got.len() >= 2 && got.concat() == name, "{got:?}");
        // line-clamp-2 with "…".
        let long = "one two three four five six seven eight nine ten eleven twelve";
        let w = label(long, 13.0, 20.0).shape(f32::INFINITY).width();
        let clamp = label(long, 13.0, 20.0).wrap().max_lines(2);
        let got = lines(&clamp, (w / 3.0).floor());
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got[1].ends_with('…'), "{got:?}");
        assert!(clamp.shape((w / 3.0).floor()).width() <= (w / 3.0).floor() + 0.01);
        // Fits: no ellipsis.
        let got = lines(&label("short", 13.0, 20.0).wrap().max_lines(2), 500.0);
        assert_eq!(got, ["short"]);
    }

    #[test]
    fn decorations_follow_their_spans() {
        crate::fonts::setup();
        let l = rich(
            vec![
                Span::new("plain "),
                Span::new("struck").strike(),
                Span::new(" "),
                Span::new("link").underline(),
            ],
            13.0,
            19.5,
        );
        let c = l.build(f32::INFINITY);
        let d = &c.lines[0].decos;
        assert_eq!(d.len(), 2, "{d:?}");
        assert!(d[0].deco.strike && !d[0].deco.underline);
        assert!(d[1].deco.underline && !d[1].deco.strike);
        assert!(d[0].x0 > 30.0 && d[0].x1 < d[1].x0 && d[1].x1 <= c.width + 0.01);
        // A whole-label decoration covers every wrapped line.
        let c = label("one two three four", 13.0, 19.5)
            .wrap()
            .underline()
            .build(40.0);
        assert!(c.lines.len() > 1);
        assert!(
            c.lines
                .iter()
                .all(|l| l.decos.len() == 1 && l.decos[0].x0 < 1.0)
        );
        // No decoration: nothing to draw, nothing cached.
        assert!(
            label("x", 13.0, 19.5).build(100.0).lines[0]
                .decos
                .is_empty()
        );
    }

    #[test]
    fn decoration_metrics_like_blink() {
        // Inter 13 px at 1x: underline 2 px below the baseline, 1 px thick;
        // line-through a third of the 13 px ascent above it.
        assert_eq!(deco_metrics(Fam::Sans, 13.0, 1.0), (2.0, -5.0, 1.0));
        // 2x: twice as far, 2 px thick.
        let (u, s, t) = deco_metrics(Fam::Sans, 13.0, 2.0);
        assert_eq!((u, t), (4.0, 2.0));
        assert!(s < -8.0);
        assert_eq!(deco_metrics(Fam::Mono, 12.0, 1.0).2, 1.0);
    }

    #[test]
    fn italic_changes_the_shape() {
        crate::fonts::setup();
        let up = label("Italic text", 14.0, 20.0).build(f32::INFINITY).width;
        let it = label("Italic text", 14.0, 20.0)
            .italic()
            .build(f32::INFINITY)
            .width;
        let bold = label("Italic text", 14.0, 20.0)
            .bold()
            .build(f32::INFINITY)
            .width;
        assert!((up - it).abs() > 0.1, "{up} {it}");
        assert!(bold > up + 1.0, "{up} {bold}");
    }
}
