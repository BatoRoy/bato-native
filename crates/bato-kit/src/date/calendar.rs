//! `Calendar` (`Calendar.tsx`): a 256 px month grid with single or range
//! selection, `min` / `max` / `isDateDisabled`, `weekStartsOn` and event dots.
//!
//! - Header: ‹ `MMMM yyyy` › (`p-1` chevrons, 14/20 medium title), 12 px gap.
//! - Weekday initials (`EEEEE`, 12/16 medium muted, `py-1`), 4 px gap.
//! - Days: 7 columns × 32 px rows, 2 px row gap, `rounded-md`, 14/20:
//!   subtext (muted outside the month), hover `text@.06`, 150 ms colour
//!   transition; selected / range endpoints `accent` + `accentInk` medium;
//!   between `accent@.15` + `accentBright`, square; today `ring-1
//!   accentBright` unless selected; disabled `opacity-30`, `not-allowed`;
//!   an event dot 4 px above the bottom.
//! - Range mode previews the range up to the hovered day while the end is
//!   open.
//!
//! Keyboard is React's: every enabled day and both chevrons are buttons in
//! Tab order (Enter / Space pick), with Chromium's own focus outline. No
//! arrow-key grid (React has none).
//!
//! ```ignore
//! calendar(&cx, &state.cal, Msg::Cal).single(state.value)
//! // update: if let Some(d) = state.cal.update(m) { state.value = Some(d) }
//! ```

use iced::widget::{Space, column, container, mouse_area, row, stack};
use iced::{Alignment, Length, Padding};

use super::math::{DateRange, out_of_bounds};
use super::{nav_button, ua_ring};
use crate::El;
use crate::anim::opacity;
use crate::ctx::Ctx;
use crate::datefmt::{self, Datelike, NaiveDate};
use crate::icon::Ico;
use crate::label::label;
use crate::layout::{Grid, vgap};
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

/// Width (`w-64`).
pub const WIDTH: f32 = 256.0;
/// Day row height (`h-8`) and row gap (`gap-y-0.5`).
pub const DAY_H: f32 = 32.0;
pub const DAY_GAP: f32 = 2.0;

/// What the calendar keeps between renders (React's `internalMonth` and
/// `hovered`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarState {
    /// Any day of the shown month.
    pub month: NaiveDate,
    pub hovered: Option<NaiveDate>,
}

impl CalendarState {
    /// Showing `month` (React: `selected ?? range.start ?? new Date()`).
    pub fn new(month: NaiveDate) -> Self {
        Self {
            month,
            hovered: None,
        }
    }

    /// Apply a message; returns the picked day.
    pub fn update(&mut self, msg: CalendarMsg) -> Option<NaiveDate> {
        match msg {
            CalendarMsg::Month(m) => self.month = m,
            CalendarMsg::Hover(h) => self.hovered = h,
            CalendarMsg::Pick(d) => return Some(d),
        }
        None
    }
}

impl Default for CalendarState {
    fn default() -> Self {
        Self::new(datefmt::today())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarMsg {
    /// The chevrons: show this month.
    Month(NaiveDate),
    /// Pointer entered (`Some`) or left (`None`) a day (range mode only).
    Hover(Option<NaiveDate>),
    /// An enabled day was clicked.
    Pick(NaiveDate),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Single(Option<NaiveDate>),
    Range(DateRange),
}

pub struct Calendar<'a, M> {
    cx: Ctx,
    state: CalendarState,
    mode: Mode,
    min: Option<NaiveDate>,
    max: Option<NaiveDate>,
    is_disabled: Option<Box<dyn Fn(NaiveDate) -> bool + 'a>>,
    week_starts_on: u8,
    event_dates: Vec<NaiveDate>,
    today: NaiveDate,
    on: Box<dyn Fn(CalendarMsg) -> M + 'a>,
}

/// A calendar showing `state`'s month; `on` wraps its messages.
pub fn calendar<'a, M: Clone + 'a>(
    cx: &Ctx,
    state: &CalendarState,
    on: impl Fn(CalendarMsg) -> M + 'a,
) -> Calendar<'a, M> {
    Calendar {
        cx: cx.clone(),
        state: *state,
        mode: Mode::Single(None),
        min: None,
        max: None,
        is_disabled: None,
        week_starts_on: 1,
        event_dates: Vec::new(),
        today: datefmt::today(),
        on: Box::new(on),
    }
}

impl<'a, M: Clone + 'a> Calendar<'a, M> {
    /// Single mode with this selection (the default, nothing selected).
    pub fn single(mut self, selected: Option<NaiveDate>) -> Self {
        self.mode = Mode::Single(selected);
        self
    }
    /// Range mode with this range.
    pub fn range(mut self, range: DateRange) -> Self {
        self.mode = Mode::Range(range);
        self
    }
    pub fn min(mut self, d: Option<NaiveDate>) -> Self {
        self.min = d;
        self
    }
    pub fn max(mut self, d: Option<NaiveDate>) -> Self {
        self.max = d;
        self
    }
    pub fn is_date_disabled(mut self, f: impl Fn(NaiveDate) -> bool + 'a) -> Self {
        self.is_disabled = Some(Box::new(f));
        self
    }
    /// 0 = Sunday … 6 = Saturday (default 1, Monday).
    pub fn week_starts_on(mut self, d: u8) -> Self {
        self.week_starts_on = d;
        self
    }
    /// Days that show an event dot.
    pub fn event_dates(mut self, dates: Vec<NaiveDate>) -> Self {
        self.event_dates = dates;
        self
    }
    /// Override "today" (tests, screenshots).
    pub fn today(mut self, d: NaiveDate) -> Self {
        self.today = d;
        self
    }

    fn disabled(&self, d: NaiveDate) -> bool {
        out_of_bounds(d, self.min, self.max) || self.is_disabled.as_ref().is_some_and(|f| f(d))
    }
}

/// How one day cell looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayLook {
    pub dim: bool,
    pub disabled: bool,
    pub selected: bool,
    pub between: bool,
    pub today: bool,
    pub dot: bool,
}

impl<'a, M: Clone + 'a> Calendar<'a, M> {
    /// The cell flags of `d` (React's class logic).
    pub fn look(&self, d: NaiveDate) -> DayLook {
        let (selected, between) = match self.mode {
            Mode::Single(sel) => (sel == Some(d), false),
            Mode::Range(r) => {
                let end = r.is_endpoint(d);
                (end, !end && r.contains(d, self.state.hovered))
            }
        };
        DayLook {
            dim: d.month() != self.state.month.month() || d.year() != self.state.month.year(),
            disabled: self.disabled(d),
            selected,
            between,
            today: d == self.today,
            dot: self.event_dates.contains(&d),
        }
    }
}

fn day_cell<'a, M: Clone + 'a>(cx: &Ctx, d: NaiveDate, look: DayLook, msg: M) -> El<'a, M> {
    let p = cx.p().clone();
    let mut text = label(d.day().to_string(), 14.0, 20.0);
    if look.selected {
        text = text.medium();
    }
    let content: El<'a, M> = if look.dot && !look.selected {
        let dot = surface(Space::new().width(4).height(4))
            .fixed(BoxStyle::default().bg(c(p.accent_bright)).radius(2.0));
        stack![
            container(text).center(Length::Fill),
            container(dot)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::End)
                .padding(Padding::default().bottom(4)),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    } else {
        text.into()
    };
    let hover = c(cx.wash(0.06));
    let between_bg = c(cx.tint(0.15));
    let style = move |s: Status| {
        let mut b = BoxStyle::default().radius(6.0);
        if look.selected {
            b = b.bg(c(p.accent)).fg(c(p.accent_ink));
        } else if look.between {
            b = b.bg(between_bg).fg(c(p.accent_bright)).radius(0.0);
        } else {
            b = b.fg(c(if look.dim { p.muted } else { p.subtext }));
            if s == Status::Hover && !look.disabled {
                b = b.bg(hover);
            }
        }
        if look.today && !look.selected {
            b = b.ring(1.0, c(p.accent_bright));
        }
        b
    };
    let cell = surface(content)
        .width(Length::Fill)
        .height(DAY_H)
        .center()
        .on_press(msg)
        .disabled(look.disabled)
        .focus_ring(ua_ring())
        .style(style);
    if look.disabled {
        opacity(0.3, cell).into()
    } else {
        cell.into()
    }
}

impl<'a, M: Clone + 'a> From<Calendar<'a, M>> for El<'a, M> {
    fn from(cal: Calendar<'a, M>) -> Self {
        let cx = &cal.cx;
        let p = cx.p();
        let month = cal.state.month;
        let header = row![
            nav_button(
                cx,
                Ico::ChevronLeft,
                4.0,
                (cal.on)(CalendarMsg::Month(datefmt::add_months(month, -1)))
            ),
            container(
                label(datefmt::format_date(month, "MMMM yyyy"), 14.0, 20.0)
                    .medium()
                    .color(c(p.text))
            )
            .center_x(Length::Fill),
            nav_button(
                cx,
                Ico::ChevronRight,
                4.0,
                (cal.on)(CalendarMsg::Month(datefmt::add_months(month, 1)))
            ),
        ]
        .align_y(Alignment::Center);

        let mut weekdays = Grid::columns(7);
        for w in datefmt::weekday_labels(cal.week_starts_on, "EEEEE") {
            weekdays = weekdays.push(
                container(label(w, 12.0, 16.0).medium().color(c(p.muted)))
                    .center_x(Length::Fill)
                    .padding([4, 0]),
            );
        }

        let range_mode = matches!(cal.mode, Mode::Range(_));
        let mut days = Grid::columns(7).gap_y(DAY_GAP);
        for d in datefmt::month_grid(month, cal.week_starts_on) {
            let look = cal.look(d);
            let cell = day_cell(cx, d, look, (cal.on)(CalendarMsg::Pick(d)));
            days = days.push(if range_mode {
                // React's onMouseEnter / onMouseLeave (only range mode reads
                // `hovered`).
                El::from(
                    mouse_area(cell)
                        .on_enter((cal.on)(CalendarMsg::Hover(Some(d))))
                        .on_exit((cal.on)(CalendarMsg::Hover(None))),
                )
            } else {
                cell
            });
        }

        column![header, vgap(12.0), weekdays, vgap(4.0), days]
            .width(WIDTH)
            .into()
    }
}

/// The calendar's height for a month (5 or 6 weeks): header 24 + 12 +
/// weekdays 24 + 4 + rows.
pub fn height(month: NaiveDate, week_starts_on: u8) -> f32 {
    let rows = (datefmt::month_grid(month, week_starts_on).len() / 7) as f32;
    24.0 + 12.0 + 24.0 + 4.0 + rows * DAY_H + (rows - 1.0) * DAY_GAP
}

#[cfg(test)]
mod tests {
    use super::*;
    use bato_theme::{Rgb, ThemeId, palette};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }
    fn cx() -> Ctx {
        Ctx::from_palette(palette(ThemeId::Dark, Rgb::hex("#8b5cf6").unwrap()))
    }

    #[test]
    fn state_navigates_and_picks() {
        let mut s = CalendarState::new(d(2026, 11, 14));
        assert_eq!(s.update(CalendarMsg::Month(d(2026, 12, 14))), None);
        assert_eq!(s.month, d(2026, 12, 14));
        assert_eq!(s.update(CalendarMsg::Hover(Some(d(2026, 12, 3)))), None);
        assert_eq!(s.hovered, Some(d(2026, 12, 3)));
        assert_eq!(
            s.update(CalendarMsg::Pick(d(2026, 12, 9))),
            Some(d(2026, 12, 9))
        );
    }

    #[test]
    fn cell_looks() {
        let cx = cx();
        let st = CalendarState::new(d(2026, 11, 14));
        let cal = calendar(&cx, &st, |m| m)
            .single(Some(d(2026, 11, 20)))
            .today(d(2026, 11, 14))
            .min(Some(d(2026, 11, 3)))
            .is_date_disabled(|x| x == d(2026, 11, 25))
            .event_dates(vec![d(2026, 11, 14)]);
        let l = cal.look(d(2026, 11, 14));
        assert!(l.today && l.dot && !l.selected && !l.dim);
        assert!(cal.look(d(2026, 11, 20)).selected);
        assert!(cal.look(d(2026, 10, 30)).dim && cal.look(d(2026, 10, 30)).disabled);
        assert!(!cal.look(d(2026, 11, 3)).disabled);
        assert!(cal.look(d(2026, 11, 25)).disabled);
        // Range with hover preview.
        let mut st = st;
        st.hovered = Some(d(2026, 11, 12));
        let cal = calendar(&cx, &st, |m| m).range(DateRange::new(d(2026, 11, 10), None));
        assert!(cal.look(d(2026, 11, 10)).selected);
        assert!(cal.look(d(2026, 11, 11)).between);
        assert!(cal.look(d(2026, 11, 12)).between);
        assert!(!cal.look(d(2026, 11, 13)).between);
    }

    #[test]
    fn heights() {
        assert_eq!(height(d(2026, 11, 1), 1), 64.0 + 6.0 * 32.0 + 10.0);
        assert_eq!(height(d(2026, 2, 1), 0), 64.0 + 4.0 * 32.0 + 6.0);
    }
}
