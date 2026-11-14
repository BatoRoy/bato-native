//! `SegmentedControl` (sm / md, plain or rich labels) and the full-width
//! `Tabs` bar. The selected segment: `accent@.15` tint, accentBright text,
//! `accent@.40` inset ring, on a recessed `bg@.5` track. Muted text inside the
//! selection (a count) turns accentBright.
//!
//! ```ignore
//! segmented(&cx, theme, ThemeId::ALL.map(|t| Segment::new(t, t.label())), Msg::Theme)
//! segmented(&cx, role, vec![Segment::rich(Role::Admin, [("Admin", false), (" 3", true)])], Msg::Role).sm()
//! tabs(&cx, tab, vec![(Tab::Inputs, "Inputs"), …], Msg::Tab)
//! ```

use iced::widget::row;
use iced::{Length, Padding};

use crate::El;
use crate::ctx::Ctx;
use crate::label::{Span, label, rich};
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

/// One option. `muted` spans of a rich label are drawn `muted`, or
/// accentBright when selected.
#[derive(Debug, Clone)]
pub struct Segment<T> {
    pub value: T,
    pub spans: Vec<(String, bool)>,
}

impl<T> Segment<T> {
    pub fn new(value: T, text: impl Into<String>) -> Self {
        Self { value, spans: vec![(text.into(), false)] }
    }
    /// `(text, muted)` spans.
    pub fn rich<S: Into<String>>(value: T, spans: impl IntoIterator<Item = (S, bool)>) -> Self {
        Self { value, spans: spans.into_iter().map(|(s, m)| (s.into(), m)).collect() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SegSize {
    /// h 24, px 10
    Sm,
    /// h 28, px 12
    #[default]
    Md,
}

/// Active / hover / rest style of a segment over `track`.
fn seg_style(cx: &Ctx, active: bool, radius: f32) -> impl Fn(Status) -> BoxStyle + use<> {
    let t = cx.t.clone();
    let track = cx.over;
    move |st| {
        let p = &t.p;
        if active {
            let bg = t.tint(0.15, track);
            BoxStyle::default().bg(c(bg)).inner_ring(c(bato_theme::blend(p.accent, 0.40, bg))).fg(c(p.accent_bright)).radius(radius)
        } else if st == Status::Hover {
            BoxStyle::default().bg(c(t.wash(0.05, track))).fg(c(p.text)).radius(radius)
        } else {
            BoxStyle::default().fg(c(p.subtext)).radius(radius)
        }
    }
}

fn seg_label<'a, M: 'a>(cx: &Ctx, spans: &[(String, bool)], active: bool, size: f32, lh: f32) -> El<'a, M> {
    if spans.len() == 1 && !spans[0].1 {
        return label(spans[0].0.clone(), size, lh).medium().into();
    }
    let p = cx.p();
    let muted = c(if active { p.accent_bright } else { p.muted });
    let spans = spans
        .iter()
        .map(|(s, m)| {
            let sp = Span::new(s.clone());
            if *m { sp.color(muted) } else { sp }
        })
        .collect();
    rich(spans, size, lh).medium().into()
}

pub struct Segmented<'a, T, M> {
    cx: Ctx,
    value: T,
    options: Vec<Segment<T>>,
    on_change: Box<dyn Fn(T) -> M + 'a>,
    size: SegSize,
}

pub fn segmented<'a, T: Copy + PartialEq + 'a, M: Clone + 'a>(
    cx: &Ctx,
    value: T,
    options: impl IntoIterator<Item = Segment<T>>,
    on_change: impl Fn(T) -> M + 'a,
) -> Segmented<'a, T, M> {
    Segmented { cx: cx.clone(), value, options: options.into_iter().collect(), on_change: Box::new(on_change), size: SegSize::Md }
}

impl<'a, T: Copy + PartialEq + 'a, M: Clone + 'a> Segmented<'a, T, M> {
    pub fn sm(mut self) -> Self {
        self.size = SegSize::Sm;
        self
    }
    pub fn size(mut self, s: SegSize) -> Self {
        self.size = s;
        self
    }
}

impl<'a, T: Copy + PartialEq + 'a, M: Clone + 'a> From<Segmented<'a, T, M>> for El<'a, M> {
    fn from(s: Segmented<'a, T, M>) -> Self {
        let track = s.cx.recessed();
        let tcx = s.cx.on(track);
        let (h, px) = match s.size {
            SegSize::Sm => (24.0, 10.0),
            SegSize::Md => (28.0, 12.0),
        };
        let segs: Vec<El<'a, M>> = s
            .options
            .iter()
            .map(|o| {
                let active = o.value == s.value;
                surface(seg_label(&tcx, &o.spans, active, 12.0, 16.0))
                    .height(h)
                    .padding(Padding::from([0.0, px]))
                    .center()
                    .on_press((s.on_change)(o.value))
                    .style(seg_style(&tcx, active, 6.0))
                    .into()
            })
            .collect();
        let box_style = BoxStyle::default().bg(c(track)).border(c(s.cx.t.line_on(track))).radius(8.0);
        surface(row(segs)).padding(3).fixed(box_style).into()
    }
}

/// `Tabs`: the full-width segmented bar (Showcase header). Equal cells,
/// h 32, 13 px medium, radius 8, on a `p-1` track with radius 12.
pub fn tabs<'a, T: Copy + PartialEq + 'a, M: Clone + 'a>(
    cx: &Ctx,
    value: T,
    items: impl IntoIterator<Item = Segment<T>>,
    on_change: impl Fn(T) -> M + 'a,
) -> El<'a, M> {
    let track = cx.recessed();
    let tcx = cx.on(track);
    let cells: Vec<El<'a, M>> = items
        .into_iter()
        .map(|o| {
            let active = o.value == value;
            surface(seg_label(&tcx, &o.spans, active, 13.0, 19.5))
                .width(Length::Fill)
                .height(32)
                .padding(Padding::from([0.0, 12.0]))
                .center()
                .on_press(on_change(o.value))
                .style(seg_style(&tcx, active, 8.0))
                .into()
        })
        .collect();
    let box_style = BoxStyle::default().bg(c(track)).border(c(cx.t.line_on(track))).radius(12.0);
    surface(row(cells).spacing(4).width(Length::Fill)).width(Length::Fill).padding(5).fixed(box_style).into()
}
