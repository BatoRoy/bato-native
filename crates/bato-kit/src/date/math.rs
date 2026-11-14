//! The date and time maths of the date components, ported 1:1 from the React
//! code (date-fns calls) so it can be unit-tested: ranges, limits, the
//! 12-hour clock, the time columns and the week view's event boxes.
//! Calendar grids and formatting live in [`crate::datefmt`].

use crate::datefmt::{Days, NaiveDate, NaiveDateTime, NaiveTime, Timelike, add_months};

/// A start/end selection (`DateRange`); `end` is `None` while picking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DateRange {
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
}

impl DateRange {
    pub const EMPTY: DateRange = DateRange {
        start: None,
        end: None,
    };

    pub fn new(start: NaiveDate, end: Option<NaiveDate>) -> Self {
        Self {
            start: Some(start),
            end,
        }
    }

    /// Calendar `pick` in range mode: a click starts a new range when there is
    /// none or it is complete, else completes it (a day before the start
    /// becomes the start).
    pub fn pick(self, d: NaiveDate) -> DateRange {
        match (self.start, self.end) {
            (Some(start), None) if d < start => DateRange::new(d, Some(start)),
            (Some(start), None) => DateRange::new(start, Some(d)),
            _ => DateRange::new(d, None),
        }
    }

    /// `inRange`: inside the range, or between the start and the hovered day
    /// while the end is still open (either direction), endpoints included.
    pub fn contains(&self, d: NaiveDate, hovered: Option<NaiveDate>) -> bool {
        let Some(start) = self.start else {
            return false;
        };
        let Some(end) = self.end.or(hovered) else {
            return false;
        };
        let (a, b) = if end < start {
            (end, start)
        } else {
            (start, end)
        };
        a <= d && d <= b
    }

    /// `isEndpoint`.
    pub fn is_endpoint(&self, d: NaiveDate) -> bool {
        self.start == Some(d) || self.end == Some(d)
    }

    pub fn is_complete(&self) -> bool {
        self.start.is_some() && self.end.is_some()
    }
}

/// `min` / `max` / `isDateDisabled` (day granularity: `min` itself is
/// allowed, like `isBefore(d, min) && !isSameDay(d, min)`).
pub fn out_of_bounds(d: NaiveDate, min: Option<NaiveDate>, max: Option<NaiveDate>) -> bool {
    min.is_some_and(|m| d < m) || max.is_some_and(|m| d > m)
}

/// `addMonths` / `addWeeks` for the CalendarView toolbar.
pub fn step(cursor: NaiveDate, week: bool, dir: i32) -> NaiveDate {
    if week {
        if dir >= 0 {
            cursor + Days::new(7 * dir as u64)
        } else {
            cursor - Days::new(7 * dir.unsigned_abs() as u64)
        }
    } else {
        add_months(cursor, dir)
    }
}

/// DateTimePicker `pickDate`: the picked day at `base`'s time of day
/// (milliseconds dropped).
pub fn keep_time(day: NaiveDate, base: NaiveDateTime) -> NaiveDateTime {
    day.and_hms_opt(base.hour(), base.minute(), base.second())
        .expect("valid time")
}

/// Whole seconds (`new Date()` shown without milliseconds).
pub fn trunc_seconds(t: NaiveDateTime) -> NaiveDateTime {
    t.with_nanosecond(0).expect("0 ns is valid")
}

/// `TimePicker`'s `set`: replace the given parts, keep the date.
pub fn set_time(t: NaiveDateTime, h: Option<u32>, m: Option<u32>, s: Option<u32>) -> NaiveDateTime {
    let time = NaiveTime::from_hms_opt(
        h.unwrap_or(t.hour()),
        m.unwrap_or(t.minute()),
        s.unwrap_or(t.second()),
    )
    .expect("valid time");
    t.date().and_time(time)
}

/// The hour a 12-hour column shows (`h % 12 === 0 ? 12 : h % 12`).
pub fn display_hour12(h: u32) -> u32 {
    match h % 12 {
        0 => 12,
        x => x,
    }
}

/// `setHour12`: a 12-hour pick keeps AM/PM.
pub fn hour_from12(current: u32, picked: u32) -> u32 {
    let base = picked % 12;
    if current >= 12 { base + 12 } else { base }
}

/// `setMeridiem`.
pub fn hour_meridiem(current: u32, pm: bool) -> u32 {
    if pm { current % 12 + 12 } else { current % 12 }
}

/// A minute or second column: `ceil(60 / step)` items, `i × step`.
pub fn steps(step: u32) -> Vec<u32> {
    let step = step.max(1);
    (0..60u32.div_ceil(step)).map(|i| i * step).collect()
}

/// The hour column: 1…12 or 0…23.
pub fn hours(hour12: bool) -> Vec<u32> {
    if hour12 {
        (1..=12).collect()
    } else {
        (0..24).collect()
    }
}

/// A TimePicker column's `scrollTop` that centres item `index`
/// (`(a.top − c.top) − (clientHeight / 2 − a.height / 2)`), clamped like the
/// browser does. `item` and `gap` in px, `view` = the column's max height.
pub fn centre_offset(index: usize, count: usize, item: f32, gap: f32, view: f32) -> f32 {
    let content = count as f32 * item + count.saturating_sub(1) as f32 * gap;
    let client = content.min(view);
    let want = index as f32 * (item + gap) - (client / 2.0 - item / 2.0);
    want.clamp(0.0, (content - client).max(0.0))
}

/// Week view: px per hour.
pub const HOUR_H: f32 = 44.0;

/// Week view: an event's `top` and `height` in its day column
/// (`differenceInMinutes` truncates; no end = 24 px; at least 20 px).
pub fn event_box(day: NaiveDate, start: NaiveDateTime, end: Option<NaiveDateTime>) -> (f32, f32) {
    let day0 = day.and_time(NaiveTime::MIN);
    let top = (start - day0).num_minutes() as f32 / 60.0 * HOUR_H;
    let h = match end {
        Some(e) => ((e - start).num_minutes() as f32 / 60.0 * HOUR_H).max(20.0),
        None => 24.0,
    };
    (top, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datefmt::{month_grid, start_of_week};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }
    fn dt(y: i32, m: u32, day: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
        d(y, m, day).and_hms_opt(h, mi, s).unwrap()
    }

    #[test]
    fn month_grids_cover_full_weeks() {
        // Feb 2026 starts on a Sunday: Monday weeks start on Jan 26, Sunday
        // weeks on Feb 1; Feb 2026 is exactly 4 Sunday-weeks.
        let g = month_grid(d(2026, 2, 10), 1);
        assert_eq!((g[0], g.len()), (d(2026, 1, 26), 35));
        let g = month_grid(d(2026, 2, 10), 0);
        assert_eq!((g[0], g.len()), (d(2026, 2, 1), 28));
        // Leap year: Feb 2024 has a 29th.
        let g = month_grid(d(2024, 2, 1), 1);
        assert!(g.contains(&d(2024, 2, 29)));
        assert_eq!(*g.last().unwrap(), d(2024, 3, 3));
        // Six weeks: Nov 2026 (Mon start) runs Oct 26 – Dec 6.
        let g = month_grid(d(2026, 11, 14), 1);
        assert_eq!(
            (g[0], *g.last().unwrap(), g.len()),
            (d(2026, 10, 26), d(2026, 12, 6), 42)
        );
        // Saturday weeks (weekStartsOn 6).
        assert_eq!(start_of_week(d(2026, 11, 14), 6), d(2026, 11, 14));
        assert_eq!(start_of_week(d(2026, 11, 13), 6), d(2026, 11, 7));
        // Year boundary.
        let g = month_grid(d(2027, 1, 1), 1);
        assert_eq!(g[0], d(2026, 12, 28));
    }

    #[test]
    fn month_steps_clamp_to_the_last_day() {
        assert_eq!(step(d(2026, 1, 31), false, 1), d(2026, 2, 28));
        assert_eq!(step(d(2024, 1, 31), false, 1), d(2024, 2, 29));
        assert_eq!(step(d(2024, 3, 31), false, -1), d(2024, 2, 29));
        assert_eq!(step(d(2026, 12, 15), false, 1), d(2027, 1, 15));
        assert_eq!(step(d(2026, 11, 14), true, 1), d(2026, 11, 21));
        assert_eq!(step(d(2026, 1, 2), true, -1), d(2025, 12, 26));
    }

    #[test]
    fn range_selection() {
        let r = DateRange::EMPTY.pick(d(2026, 11, 10));
        assert_eq!(r, DateRange::new(d(2026, 11, 10), None));
        assert!(!r.is_complete());
        // Earlier second pick swaps.
        assert_eq!(
            r.pick(d(2026, 11, 3)),
            DateRange::new(d(2026, 11, 3), Some(d(2026, 11, 10)))
        );
        // Same day is a one-day range.
        assert_eq!(
            r.pick(d(2026, 11, 10)),
            DateRange::new(d(2026, 11, 10), Some(d(2026, 11, 10)))
        );
        let full = r.pick(d(2026, 11, 20));
        assert!(full.is_complete());
        // A pick on a complete range starts over.
        assert_eq!(
            full.pick(d(2026, 12, 1)),
            DateRange::new(d(2026, 12, 1), None)
        );
        // Across a month boundary.
        let r = DateRange::new(d(2026, 10, 30), Some(d(2026, 11, 2)));
        assert!(r.contains(d(2026, 10, 31), None));
        assert!(r.contains(d(2026, 11, 2), None));
        assert!(!r.contains(d(2026, 11, 3), None));
        assert!(r.is_endpoint(d(2026, 10, 30)) && !r.is_endpoint(d(2026, 11, 1)));
    }

    #[test]
    fn range_hover_preview() {
        let r = DateRange::new(d(2026, 11, 10), None);
        assert!(!r.contains(d(2026, 11, 12), None), "no hover, no preview");
        assert!(r.contains(d(2026, 11, 12), Some(d(2026, 11, 14))));
        // Hovering before the start previews backwards.
        assert!(r.contains(d(2026, 11, 8), Some(d(2026, 11, 5))));
        assert!(!r.contains(d(2026, 11, 11), Some(d(2026, 11, 5))));
        // A complete range ignores the hover.
        let full = DateRange::new(d(2026, 11, 10), Some(d(2026, 11, 12)));
        assert!(!full.contains(d(2026, 11, 13), Some(d(2026, 11, 20))));
        assert!(!DateRange::EMPTY.contains(d(2026, 11, 13), Some(d(2026, 11, 13))));
    }

    #[test]
    fn limits_are_inclusive_days() {
        let (min, max) = (Some(d(2026, 11, 5)), Some(d(2026, 11, 20)));
        assert!(out_of_bounds(d(2026, 11, 4), min, max));
        assert!(!out_of_bounds(d(2026, 11, 5), min, max));
        assert!(!out_of_bounds(d(2026, 11, 20), min, max));
        assert!(out_of_bounds(d(2026, 11, 21), min, max));
        assert!(!out_of_bounds(d(1990, 1, 1), None, None));
    }

    #[test]
    fn twelve_hour_clock() {
        assert_eq!(display_hour12(0), 12);
        assert_eq!(display_hour12(12), 12);
        assert_eq!(display_hour12(13), 1);
        assert_eq!(display_hour12(11), 11);
        // Picking 12 in the AM is midnight, in the PM noon.
        assert_eq!(hour_from12(3, 12), 0);
        assert_eq!(hour_from12(15, 12), 12);
        assert_eq!(hour_from12(15, 7), 19);
        assert_eq!(hour_from12(9, 7), 7);
        assert_eq!(hour_meridiem(9, true), 21);
        assert_eq!(hour_meridiem(21, false), 9);
        assert_eq!(hour_meridiem(0, true), 12);
        assert_eq!(hour_meridiem(12, false), 0);
        assert_eq!(hours(true), (1..=12).collect::<Vec<_>>());
        assert_eq!(hours(false).len(), 24);
    }

    #[test]
    fn time_columns_and_setters() {
        assert_eq!(steps(1).len(), 60);
        assert_eq!(steps(5), (0..12).map(|i| i * 5).collect::<Vec<_>>());
        assert_eq!(steps(7), vec![0, 7, 14, 21, 28, 35, 42, 49, 56]);
        let t = dt(2024, 2, 29, 23, 59, 30);
        assert_eq!(set_time(t, Some(0), None, None), dt(2024, 2, 29, 0, 59, 30));
        assert_eq!(
            set_time(t, None, Some(5), Some(0)),
            dt(2024, 2, 29, 23, 5, 0)
        );
        assert_eq!(keep_time(d(2026, 3, 1), t), dt(2026, 3, 1, 23, 59, 30));
    }

    #[test]
    fn time_columns_centre_like_the_browser() {
        // 24 items of 28 px, 2 px gaps, 160 px tall: item 8 → 8×30 − 66.
        assert_eq!(centre_offset(8, 24, 28.0, 2.0, 160.0), 174.0);
        assert_eq!(centre_offset(0, 24, 28.0, 2.0, 160.0), 0.0);
        assert_eq!(centre_offset(1, 24, 28.0, 2.0, 160.0), 0.0);
        // Clamped at the end: content 718, max 558.
        assert_eq!(centre_offset(23, 24, 28.0, 2.0, 160.0), 558.0);
        // A short column never scrolls.
        assert_eq!(centre_offset(1, 2, 28.0, 2.0, 160.0), 0.0);
    }

    #[test]
    fn week_event_boxes() {
        let day = d(2026, 11, 14);
        assert_eq!(
            event_box(
                day,
                dt(2026, 11, 14, 10, 0, 0),
                Some(dt(2026, 11, 14, 11, 0, 0))
            ),
            (440.0, 44.0)
        );
        assert_eq!(
            event_box(
                day,
                dt(2026, 11, 14, 14, 30, 0),
                Some(dt(2026, 11, 14, 14, 40, 0))
            ),
            (638.0, 20.0)
        );
        assert_eq!(event_box(day, dt(2026, 11, 14, 0, 0, 0), None), (0.0, 24.0));
    }
}
