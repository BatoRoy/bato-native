//! `CalendarView` (`CalendarView.tsx`): month and week event views with a
//! toolbar (‹ Today › title, Month / Week segmented control).
//!
//! - **Toolbar** (`mb-4`): `p-1.5` chevrons, a bordered "Today" button,
//!   the title (`MMMM yyyy` or `'Week of' MMM d, yyyy` of the cursor, 14/20
//!   semibold, 16 px after the chevron), the `sm` SegmentedControl right.
//! - **Month** (fills the rest, `border rounded-lg`, clipped): a `surface`
//!   header of `EEE` labels (`py-2`), then full weeks of day buttons with
//!   `border-b border-r` (not the very last), `p-1.5`, at least 80 px tall;
//!   every row as tall as the tallest (`auto-rows-fr`) and the content of a
//!   cell centred vertically, as a `<button>` centres it. A cell shows its
//!   day in a 24 px circle (today: accent fill) and up to 3 events as chips
//!   (`accent@.15`, accentBright; hover `accent@.25`), then "+N more".
//!   Rows that do not fit are clipped, as in the browser.
//! - **Week**: the same header with the date under each day (`3rem` gutter),
//!   then a scroller with a 24 × 44 px hour grid and the events placed by
//!   start and duration (`left-1 right-1`, `border-l-2 accent`,
//!   `accent@.20`, title medium + `HH:mm` 11 px).
//!
//! A click on a chip does not click its day (`stopPropagation`): nested
//! [`surface`]s. Custom event colours use `#rrggbb` + `26` / `33` alpha and
//! [`bato_theme::readable_text_for`].

use iced::widget::{Space, column, container, row, stack};
use iced::{Alignment, Length, Padding};

use bato_theme::{Rgb, readable_text_for};

use super::math::{HOUR_H, event_box, step};
use super::{nav_button, ua_ring};
use crate::El;
use crate::ctx::Ctx;
use crate::datefmt::{self, Days, NaiveDate, NaiveDateTime, start_of_week};
use crate::icon::Ico;
use crate::label::{Align, label};
use crate::layout::{Grid, Track, hline, vline};
use crate::segmented::{Segment, segmented};
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

/// An event (`CalendarEvent`).
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarEvent {
    pub id: String,
    pub title: String,
    pub start: NaiveDateTime,
    pub end: Option<NaiveDateTime>,
    /// Accent override.
    pub color: Option<Rgb>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Month,
    Week,
}

/// The view and the cursor date (React's `useState`s).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarViewState {
    pub view: View,
    pub cursor: NaiveDate,
}

impl Default for CalendarViewState {
    fn default() -> Self {
        Self {
            view: View::Month,
            cursor: datefmt::today(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CalendarViewMsg {
    /// ‹ (−1) or › (+1): a month or a week.
    Move(i32),
    Today,
    View(View),
    /// A day cell (`onDayClick`).
    Day(NaiveDate),
    /// An event, by id (`onEventClick`).
    Event(String),
}

impl CalendarViewState {
    /// Apply the toolbar messages; `Day` / `Event` are the app's.
    pub fn update(&mut self, msg: &CalendarViewMsg) {
        match msg {
            CalendarViewMsg::Move(dir) => {
                self.cursor = step(self.cursor, self.view == View::Week, *dir)
            }
            CalendarViewMsg::Today => self.cursor = datefmt::today(),
            CalendarViewMsg::View(v) => self.view = *v,
            CalendarViewMsg::Day(_) | CalendarViewMsg::Event(_) => {}
        }
    }
}

pub struct CalendarView<'a, M> {
    cx: Ctx,
    state: CalendarViewState,
    events: &'a [CalendarEvent],
    week_starts_on: u8,
    today: NaiveDate,
    on: std::rc::Rc<dyn Fn(CalendarViewMsg) -> M + 'a>,
}

/// A calendar view of `events`; it fills its container (give it a height).
pub fn calendar_view<'a, M: Clone + 'a>(
    cx: &Ctx,
    state: &CalendarViewState,
    events: &'a [CalendarEvent],
    on: impl Fn(CalendarViewMsg) -> M + 'a,
) -> CalendarView<'a, M> {
    CalendarView {
        cx: cx.clone(),
        state: *state,
        events,
        week_starts_on: 1,
        today: datefmt::today(),
        on: std::rc::Rc::new(on),
    }
}

impl<'a, M: Clone + 'a> CalendarView<'a, M> {
    pub fn week_starts_on(mut self, d: u8) -> Self {
        self.week_starts_on = d;
        self
    }
    /// Override "today" (tests, screenshots).
    pub fn today(mut self, d: NaiveDate) -> Self {
        self.today = d;
        self
    }
}

/// `eventsOn(day)`: the events starting that day, by start time.
pub fn events_on(events: &[CalendarEvent], day: NaiveDate) -> Vec<&CalendarEvent> {
    let mut v: Vec<&CalendarEvent> = events.iter().filter(|e| e.start.date() == day).collect();
    v.sort_by_key(|e| e.start);
    v
}

/// The height of a month cell's content: the day line (30: the 24 px
/// circle 2 px down, `mb-1`) and its events
/// (≤ 3 chips of 20 with 2 px gaps, "+N more" 16).
pub fn cell_content_height(n: usize) -> f32 {
    let shown = n.min(3);
    let mut h = 30.0;
    if shown > 0 {
        h += shown as f32 * 20.0 + (shown - 1) as f32 * 2.0;
    }
    if n > 3 {
        h += 2.0 + 16.0;
    }
    h
}

/// A month row's height: the tallest cell (padding 12, border 1), ≥ 80.
pub fn month_row_height(counts: impl IntoIterator<Item = usize>) -> f32 {
    counts
        .into_iter()
        .map(|n| cell_content_height(n) + 13.0)
        .fold(80.0, f32::max)
}

/// An event's colours: (fill alpha over the backdrop, text, border).
fn event_colors(
    cx: &Ctx,
    color: Option<Rgb>,
    alpha: f64,
) -> (iced::Color, iced::Color, iced::Color) {
    let p = cx.p();
    match color {
        Some(col) => (
            c(cx.blend(col, alpha)),
            c(readable_text_for(col, p)),
            c(col),
        ),
        None => (c(cx.tint(alpha)), c(p.accent_bright), c(p.accent)),
    }
}

/// A header band in the `surface` colour whose top corners follow the
/// box's 7 px inner radius.
fn header_band<'a, M: Clone + 'a>(cx: &Ctx, content: El<'a, M>, h: f32) -> El<'a, M> {
    let bg = c(cx.p().surface);
    stack![
        surface(Space::new().width(Length::Fill).height(h))
            .fixed(BoxStyle::default().bg(bg).radius(7.0)),
        container(
            surface(Space::new().width(Length::Fill).height(h - 7.0))
                .fixed(BoxStyle::default().bg(bg))
        )
        .padding(Padding::default().top(7)),
        content,
    ]
    .width(Length::Fill)
    .into()
}

impl<'a, M: Clone + 'a> CalendarView<'a, M> {
    fn toolbar(&self) -> El<'a, M> {
        let cx = &self.cx;
        let p = cx.p().clone();
        let on = &self.on;
        let title = datefmt::format_date(
            self.state.cursor,
            match self.state.view {
                View::Month => "MMMM yyyy",
                View::Week => "'Week of' MMM d, yyyy",
            },
        );
        let (rest_bg, hover_bg, border) = (c(cx.wash(0.04)), c(cx.wash(0.08)), c(cx.line_strong()));
        let today = surface(label("Today", 14.0, 20.0))
            .padding([7, 13])
            .transition_ms(0)
            .on_press(on(CalendarViewMsg::Today))
            .focus_ring(ua_ring())
            .style(move |s| {
                let b = BoxStyle::default().border(border).radius(6.0);
                match s {
                    Status::Hover => b.bg(hover_bg).fg(c(p.text)),
                    _ => b.bg(rest_bg).fg(c(p.subtext)),
                }
            });
        let p = cx.p();
        row![
            nav_button(cx, Ico::ChevronLeft, 6.0, on(CalendarViewMsg::Move(-1))),
            today,
            nav_button(cx, Ico::ChevronRight, 6.0, on(CalendarViewMsg::Move(1))),
            // `ml-2` on top of the `gap-2`.
            container(label(title, 14.0, 20.0).semibold().color(c(p.text)))
                .padding(Padding::default().left(8)),
            Space::new().width(Length::Fill),
            segmented(
                cx,
                self.state.view,
                [
                    Segment::new(View::Month, "Month"),
                    Segment::new(View::Week, "Week"),
                ],
                {
                    let on = on.clone();
                    move |v| on(CalendarViewMsg::View(v))
                },
            )
            .sm(),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    fn month(&self) -> El<'a, M> {
        let cx = &self.cx;
        let p = cx.p().clone();
        let line = c(p.border);
        let on = &self.on;
        let days = datefmt::month_grid(self.state.cursor, self.week_starts_on);

        let mut labels = Grid::columns(7);
        for w in datefmt::weekday_labels(self.week_starts_on, "EEE") {
            labels = labels.push(
                container(label(w, 12.0, 16.0).medium().color(c(p.muted)))
                    .center_x(Length::Fill)
                    .padding([8, 0]),
            );
        }
        let header = header_band(cx, labels.into(), 32.0);

        let per_day: Vec<Vec<&CalendarEvent>> =
            days.iter().map(|d| events_on(self.events, *d)).collect();
        let row_h = month_row_height(per_day.iter().map(Vec::len));
        let hover = c(cx.wash(0.06));
        let month = self.state.cursor;
        let mut grid = Grid::columns(7);
        let last = days.len() - 1;
        for (i, (day, evs)) in days.iter().zip(&per_day).enumerate() {
            let is_today = *day == self.today;
            let dim = datefmt::Datelike::month(day) != datefmt::Datelike::month(&month);
            let mut num = label(datefmt::format_date(*day, "d"), 12.0, 16.0);
            if is_today {
                num = num.medium();
            }
            let circle = surface(num)
                .width(24)
                .height(24)
                .center()
                .fixed(if is_today {
                    BoxStyle::default()
                        .bg(c(p.accent))
                        .fg(c(p.accent_ink))
                        .radius(12.0)
                } else {
                    BoxStyle::default().fg(c(if dim { p.muted } else { p.subtext }))
                });
            let mut list = column![].spacing(2).width(Length::Fill);
            for ev in evs.iter().take(3) {
                let (bg, fg, _) = event_colors(cx, ev.color, 0.149);
                let (hbg, _, _) = event_colors(cx, ev.color, 0.25);
                list = list.push(
                    surface(
                        label(ev.title.clone(), 12.0, 16.0)
                            .ellipsis()
                            .width(Length::Fill),
                    )
                    .width(Length::Fill)
                    .padding([2, 6])
                    .transition_ms(0)
                    .focusable(false)
                    .on_press(on(CalendarViewMsg::Event(ev.id.clone())))
                    .style(move |s| {
                        BoxStyle::default()
                            .bg(if s == Status::Hover { hbg } else { bg })
                            .fg(fg)
                            .radius(4.0)
                    }),
                );
            }
            if evs.len() > 3 {
                list = list.push(
                    container(
                        label(format!("+{} more", evs.len() - 3), 12.0, 16.0).color(c(p.muted)),
                    )
                    .padding(Padding::default().left(6)),
                );
            }
            let content = column![
                // The inline-flex circle in its 30 px line box (2 px above,
                // `mb-1` below).
                container(circle)
                    .height(30)
                    .padding(Padding::default().top(2)),
                list,
            ]
            .width(Length::Fill);
            let cell = surface(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(6)
                .center_y()
                .on_press(on(CalendarViewMsg::Day(*day)))
                .focus_ring(ua_ring())
                .style(move |s| {
                    if s == Status::Hover {
                        BoxStyle::default().bg(hover)
                    } else {
                        BoxStyle::default()
                    }
                });
            let mut top = row![cell].height(Length::Fill);
            if i != last {
                top = top.push(vline(line));
            }
            grid = grid.push(column![top, hline(line)].height(row_h));
        }
        column![header, hline(line), grid].into()
    }

    fn week(&self) -> El<'a, M> {
        let cx = &self.cx;
        let p = cx.p().clone();
        let line = c(p.border);
        let on = &self.on;
        let start = start_of_week(self.state.cursor, self.week_starts_on);
        let days: Vec<NaiveDate> = (0..7).map(|i| start + Days::new(i)).collect();
        let tracks = || {
            let mut t = vec![Track::Px(48.0)];
            t.extend([Track::Fr(1.0); 7]);
            t
        };

        let mut head = Grid::tracks(tracks()).push(Space::new());
        for d in &days {
            let today = *d == self.today;
            head = head.push(
                row![
                    vline(line),
                    container(
                        column![
                            label(datefmt::format_date(*d, "EEE"), 12.0, 16.0).color(c(p.muted)),
                            label(datefmt::format_date(*d, "d"), 14.0, 20.0)
                                .medium()
                                .color(c(if today { p.accent_bright } else { p.subtext })),
                        ]
                        .align_x(Alignment::Center)
                    )
                    .center_x(Length::Fill)
                    .padding([8, 0]),
                ]
                .height(52),
            );
        }
        let header = header_band(cx, head.into(), 52.0);

        let slot = |content: El<'a, M>| -> El<'a, M> {
            column![container(content).height(HOUR_H - 1.0), hline(line)].into()
        };
        let gutter = column((0..24).map(|h| {
            slot(
                container(if h > 0 {
                    El::from(
                        label(format!("{h:02}:00"), 12.0, 16.0)
                            .color(c(p.muted))
                            .align(Align::Right)
                            .width(Length::Fill),
                    )
                } else {
                    Space::new().into()
                })
                .padding(Padding::default().right(8))
                .width(Length::Fill)
                .into(),
            )
        }));
        let mut body = Grid::tracks(tracks()).push(gutter);
        for d in &days {
            let lines = column((0..24).map(|_| slot(Space::new().width(Length::Fill).into())));
            let mut layers = stack![lines].width(Length::Fill);
            for ev in self.events.iter().filter(|e| e.start.date() == *d) {
                let (top, h) = event_box(*d, ev.start, ev.end);
                let (bg, fg, edge) = event_colors(cx, ev.color, 0.2);
                let (hbg, _, _) = event_colors(cx, ev.color, 0.3);
                let body = column![
                    label(ev.title.clone(), 12.0, 16.0)
                        .medium()
                        .ellipsis()
                        .width(Length::Fill),
                    label(datefmt::format(&ev.start, "HH:mm"), 11.0, 16.0),
                ];
                let chip = stack![
                    surface(Space::new().width(Length::Fill).height(Length::Fill))
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .fixed(BoxStyle::default().bg(edge).radius(4.0)),
                    container(
                        surface(body)
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .padding([2, 6])
                            .clip()
                            .transition_ms(0)
                            .focusable(false)
                            .on_press(on(CalendarViewMsg::Event(ev.id.clone())))
                            .style(move |s| {
                                BoxStyle::default()
                                    .bg(if s == Status::Hover { hbg } else { bg })
                                    .fg(fg)
                                    .radius(2.0)
                            })
                    )
                    .padding(Padding::default().left(2)),
                ]
                .width(Length::Fill)
                .height(h);
                layers = layers.push(
                    column![Space::new().height(top), chip]
                        .padding([0, 4])
                        .width(Length::Fill),
                );
            }
            body = body.push(row![vline(line), layers].height(24.0 * HOUR_H));
        }
        let scroller = crate::scroller::scroll_area(cx, body).height(Length::Fill);
        column![header, hline(line), scroller]
            .height(Length::Fill)
            .into()
    }
}

impl<'a, M: Clone + 'a> From<CalendarView<'a, M>> for El<'a, M> {
    fn from(v: CalendarView<'a, M>) -> Self {
        let p = v.cx.p();
        let inner = match v.state.view {
            View::Month => v.month(),
            View::Week => v.week(),
        };
        // `overflow-hidden`: a layer clip inside the border (iced's container
        // clip only narrows the viewport; quads would still overflow).
        let boxed = surface(
            surface(inner)
                .width(Length::Fill)
                .height(Length::Fill)
                .clip(),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(1)
        .fixed(BoxStyle::default().border(c(p.border)).radius(8.0));
        column![v.toolbar(), Space::new().height(16), boxed]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, d)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap()
    }
    fn ev(d: u32, h: u32, title: &str) -> CalendarEvent {
        CalendarEvent {
            id: format!("{d}-{h}-{title}"),
            title: title.into(),
            start: at(d, h),
            end: Some(at(d, h + 1)),
            color: None,
        }
    }

    #[test]
    fn events_sorted_per_day() {
        let evs = vec![
            ev(14, 14, "Design review"),
            ev(14, 10, "Standup"),
            ev(13, 16, "Retro"),
        ];
        let on14 = events_on(&evs, NaiveDate::from_ymd_opt(2026, 11, 14).unwrap());
        assert_eq!(
            on14.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            ["Standup", "Design review"]
        );
    }

    #[test]
    fn month_rows_fit_the_tallest_cell() {
        // The reference: 2 events in a cell → 85 px rows.
        assert_eq!(month_row_height([0, 1, 2, 0]), 85.0);
        assert_eq!(month_row_height([0, 0]), 80.0);
        // 3 chips + "+2 more": 30 + 64 + 18 + 13.
        assert_eq!(month_row_height([5]), 125.0);
    }

    #[test]
    fn toolbar_moves_by_view() {
        let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        let mut s = CalendarViewState {
            view: View::Month,
            cursor: d(1, 31),
        };
        s.update(&CalendarViewMsg::Move(1));
        assert_eq!(s.cursor, d(2, 28));
        s.update(&CalendarViewMsg::View(View::Week));
        s.update(&CalendarViewMsg::Move(-1));
        assert_eq!(s.cursor, d(2, 21));
        s.update(&CalendarViewMsg::Day(d(2, 1)));
        assert_eq!(s.cursor, d(2, 21));
    }
}
