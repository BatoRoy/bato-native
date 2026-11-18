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
//!
//! Focus: every segment / tab is a Tab stop, as in React (buttons with
//! `role=radio`, no roving): Enter / Space select, arrows scroll the page.
//! Segments ring with `focusRing` (offset in the page bg), tabs with a plain
//! `ring-2`. `.roving()` opts into the ARIA radio-group keys instead: one
//! Tab stop (the selected segment), arrows move the selection.

use iced::widget::row;
use iced::{Alignment, Length, Padding};

use crate::El;
use crate::ctx::Ctx;
use crate::focus::{Ring, roving};
use crate::icon::{Icon, icon};
use crate::label::{Span, label, rich};
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

/// One option. `muted` spans of a rich label are drawn `muted`, or
/// accentBright when selected.
#[derive(Debug, Clone)]
pub struct Segment<T> {
    pub value: T,
    pub spans: Vec<(String, bool)>,
    /// A leading icon (`inline-flex items-center gap-1.5`), in the text colour.
    pub icon: Option<Icon>,
    /// `tabular-nums` (counts that change while you watch).
    pub tabular: bool,
}

impl<T> Segment<T> {
    pub fn new(value: T, text: impl Into<String>) -> Self {
        Self {
            value,
            spans: vec![(text.into(), false)],
            icon: None,
            tabular: false,
        }
    }
    /// A leading icon (14 px in tabs, 13 in a segmented control, 12 in a
    /// small one), 6 px before the text.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }
    /// Tabular digits (`tabular-nums`), so a live count does not jiggle.
    pub fn tabular(mut self) -> Self {
        self.tabular = true;
        self
    }
    /// `(text, muted)` spans.
    pub fn rich<S: Into<String>>(value: T, spans: impl IntoIterator<Item = (S, bool)>) -> Self {
        Self {
            value,
            spans: spans.into_iter().map(|(s, m)| (s.into(), m)).collect(),
            icon: None,
            tabular: false,
        }
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
            BoxStyle::default()
                .bg(c(bg))
                .inner_ring(c(bato_theme::blend(p.accent, 0.40, bg)))
                .fg(c(p.accent_bright))
                .radius(radius)
        } else if st == Status::Hover {
            BoxStyle::default()
                .bg(c(t.wash(0.05, track)))
                .fg(c(p.text))
                .radius(radius)
        } else {
            BoxStyle::default().fg(c(p.subtext)).radius(radius)
        }
    }
}

/// The segment's content: its icon (if any) and its label.
fn seg_content<'a, T, M: 'a>(
    cx: &Ctx,
    o: &Segment<T>,
    active: bool,
    size: f32,
    lh: f32,
    icon_size: f32,
) -> El<'a, M> {
    let text = seg_label(cx, &o.spans, active, size, lh, o.tabular);
    match o.icon {
        Some(i) => row![icon(i, icon_size), text]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
        None => text,
    }
}

fn seg_label<'a, M: 'a>(
    cx: &Ctx,
    spans: &[(String, bool)],
    active: bool,
    size: f32,
    lh: f32,
    tabular: bool,
) -> El<'a, M> {
    if spans.len() == 1 && !spans[0].1 {
        let l = label(spans[0].0.clone(), size, lh).medium();
        return if tabular { l.tabular() } else { l }.into();
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
    let l = rich(spans, size, lh).medium();
    if tabular { l.tabular() } else { l }.into()
}

pub struct Segmented<'a, T, M> {
    cx: Ctx,
    value: T,
    options: Vec<Segment<T>>,
    on_change: Box<dyn Fn(T) -> M + 'a>,
    size: SegSize,
    roving: bool,
}

pub fn segmented<'a, T: Copy + PartialEq + 'a, M: Clone + 'a>(
    cx: &Ctx,
    value: T,
    options: impl IntoIterator<Item = Segment<T>>,
    on_change: impl Fn(T) -> M + 'a,
) -> Segmented<'a, T, M> {
    Segmented {
        cx: cx.clone(),
        value,
        options: options.into_iter().collect(),
        on_change: Box::new(on_change),
        size: SegSize::Md,
        roving: false,
    }
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
    /// Radio-group keys: only the selected segment is a Tab stop; arrows
    /// move focus and the selection (wrapping). Not what React does.
    pub fn roving(mut self) -> Self {
        self.roving = true;
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
        let ring = Ring::focus(&s.cx);
        let any_active = s.options.iter().any(|o| o.value == s.value);
        let segs: Vec<El<'a, M>> = s
            .options
            .iter()
            .map(|o| {
                let active = o.value == s.value;
                let ico = if s.size == SegSize::Sm { 12.0 } else { 13.0 };
                surface(seg_content(&tcx, o, active, 12.0, 16.0, ico))
                    .height(h)
                    .padding(Padding::from([0.0, px]))
                    .center()
                    .on_press((s.on_change)(o.value))
                    .style(seg_style(&tcx, active, 6.0))
                    .focus_ring(ring)
                    .tab_stop(!s.roving || active || !any_active)
                    .into()
            })
            .collect();
        let box_style = BoxStyle::default()
            .bg(c(track))
            .border(c(s.cx.t.line_on(track)))
            .radius(8.0);
        let segs: El<'a, M> = if s.roving {
            let values: Vec<T> = s.options.iter().map(|o| o.value).collect();
            let on_change = s.on_change;
            roving(row(segs))
                .on_move(move |i| on_change(values[i]))
                .into()
        } else {
            row(segs).into()
        };
        surface(segs).padding(3).fixed(box_style).into()
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
            surface(seg_content(&tcx, &o, active, 13.0, 19.5, 14.0))
                .width(Length::Fill)
                .height(32)
                .padding(Padding::from([0.0, 12.0]))
                .center()
                .on_press(on_change(o.value))
                .style(seg_style(&tcx, active, 8.0))
                .focus_ring(Ring::plain(cx))
                .into()
        })
        .collect();
    let box_style = BoxStyle::default()
        .bg(c(track))
        .border(c(cx.t.line_on(track)))
        .radius(12.0);
    surface(row(cells).spacing(4).width(Length::Fill))
        .width(Length::Fill)
        .padding(5)
        .fixed(box_style)
        .into()
}
