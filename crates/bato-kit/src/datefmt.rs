//! Date and time (INF-14) on `chrono`: a formatter for the date-fns tokens
//! the components use, and calendar week math with `weekStartsOn`.
//!
//! | date-fns | example |
//! |---|---|
//! | `PP` | `Nov 14, 2026` |
//! | `MMM d` · `MMMM yyyy` | `Nov 14` · `November 2026` |
//! | `EEE` · `EEEEE` · `EEEE` | `Sat` · `S` · `Saturday` |
//! | `d` · `dd` · `M` · `MM` · `yyyy` · `yy` | `4` · `04` · … |
//! | `HH:mm` · `h:mm a` · `p` | `09:41` · `9:41 AM` · `9:41 AM` |
//! | `'Week of' MMM d, yyyy` | `Week of Nov 8, 2026` |
//!
//! English only (the template is English; locale tables are out of scope).

pub use chrono::{
    Datelike, Days, Local, Months, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Weekday,
};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// Format `dt` with a date-fns pattern.
pub fn format(dt: &NaiveDateTime, pattern: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\'' {
            // 'literal', '' = a quote
            if chars.get(i + 1) == Some(&'\'') {
                out.push('\'');
                i += 2;
                continue;
            }
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                out.push(chars[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        if !ch.is_ascii_alphabetic() {
            out.push(ch);
            i += 1;
            continue;
        }
        let mut n = 1;
        while chars.get(i + n) == Some(&ch) {
            n += 1;
        }
        i += n;
        let month0 = dt.month0() as usize;
        let wd = dt.weekday().num_days_from_monday() as usize;
        let h12 = match dt.hour() % 12 {
            0 => 12,
            h => h,
        };
        match (ch, n) {
            ('y', 2) => out.push_str(&format!("{:02}", dt.year().rem_euclid(100))),
            ('y', _) => out.push_str(&format!("{:0w$}", dt.year(), w = n)),
            ('M', 1) => out.push_str(&dt.month().to_string()),
            ('M', 2) => out.push_str(&format!("{:02}", dt.month())),
            ('M', 3) => out.push_str(&MONTHS[month0][..3]),
            ('M', _) => out.push_str(MONTHS[month0]),
            ('d', 1) => out.push_str(&dt.day().to_string()),
            ('d', _) => out.push_str(&format!("{:02}", dt.day())),
            ('E', 1..=3) => out.push_str(&DAYS[wd][..3]),
            ('E', 4) => out.push_str(DAYS[wd]),
            ('E', _) => out.push_str(&DAYS[wd][..1]),
            ('H', 1) => out.push_str(&dt.hour().to_string()),
            ('H', _) => out.push_str(&format!("{:02}", dt.hour())),
            ('h', 1) => out.push_str(&h12.to_string()),
            ('h', _) => out.push_str(&format!("{h12:02}")),
            ('m', 1) => out.push_str(&dt.minute().to_string()),
            ('m', _) => out.push_str(&format!("{:02}", dt.minute())),
            ('s', 1) => out.push_str(&dt.second().to_string()),
            ('s', _) => out.push_str(&format!("{:02}", dt.second())),
            ('a', _) => out.push_str(if dt.hour() < 12 { "AM" } else { "PM" }),
            ('P', 2) => out.push_str(&format(dt, "MMM d, yyyy")),
            ('P', 1) => out.push_str(&format(dt, "MM/dd/yyyy")),
            ('P', 3) => out.push_str(&format(dt, "MMMM d, yyyy")),
            ('P', _) => out.push_str(&format(dt, "EEEE, MMMM d, yyyy")),
            ('p', _) => out.push_str(&format(dt, "h:mm a")),
            _ => {
                for _ in 0..n {
                    out.push(ch);
                }
            }
        }
    }
    out
}

/// Format a date (midnight) with a date-fns pattern.
pub fn format_date(d: NaiveDate, pattern: &str) -> String {
    format(&d.and_time(NaiveTime::MIN), pattern)
}

/// Now, local time.
pub fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

/// Today, local time.
pub fn today() -> NaiveDate {
    now().date()
}

/// `weekStartsOn` as date-fns numbers it: 0 = Sunday … 6 = Saturday.
pub fn weekday_from_index(week_starts_on: u8) -> Weekday {
    match week_starts_on % 7 {
        0 => Weekday::Sun,
        1 => Weekday::Mon,
        2 => Weekday::Tue,
        3 => Weekday::Wed,
        4 => Weekday::Thu,
        5 => Weekday::Fri,
        _ => Weekday::Sat,
    }
}

/// `startOfWeek(d, { weekStartsOn })`.
pub fn start_of_week(d: NaiveDate, week_starts_on: u8) -> NaiveDate {
    let start = weekday_from_index(week_starts_on);
    let back = (7 + d.weekday().num_days_from_monday() - start.num_days_from_monday()) % 7;
    d - Days::new(back as u64)
}

/// `startOfMonth(d)`.
pub fn start_of_month(d: NaiveDate) -> NaiveDate {
    d.with_day(1).expect("day 1 exists")
}

/// `endOfMonth(d)` (the last day).
pub fn end_of_month(d: NaiveDate) -> NaiveDate {
    start_of_month(d) + Months::new(1) - Days::new(1)
}

/// `addMonths(d, n)` (clamped to the month's last day, like date-fns).
pub fn add_months(d: NaiveDate, n: i32) -> NaiveDate {
    if n >= 0 {
        d + Months::new(n as u32)
    } else {
        d - Months::new(n.unsigned_abs())
    }
}

/// The days of a month grid: from the start of the week of the 1st to the end
/// of the week of the last day (`eachDayOfInterval` over full weeks).
pub fn month_grid(month: NaiveDate, week_starts_on: u8) -> Vec<NaiveDate> {
    let first = start_of_week(start_of_month(month), week_starts_on);
    let last = start_of_week(end_of_month(month), week_starts_on) + Days::new(6);
    first.iter_days().take_while(|d| *d <= last).collect()
}

/// The 7 weekday labels of a week starting on `week_starts_on`, in `EEEEE`
/// (or any weekday pattern).
pub fn weekday_labels(week_starts_on: u8, pattern: &str) -> Vec<String> {
    let start = start_of_week(
        NaiveDate::from_ymd_opt(2024, 1, 10).expect("valid"),
        week_starts_on,
    );
    start
        .iter_days()
        .take(7)
        .map(|d| format_date(d, pattern))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(y: i32, m: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, mi, 5)
            .unwrap()
    }

    #[test]
    fn date_fns_tokens() {
        let t = dt(2026, 11, 14, 9, 41);
        assert_eq!(format(&t, "PP"), "Nov 14, 2026");
        assert_eq!(format(&t, "MMM d"), "Nov 14");
        assert_eq!(format(&t, "MMMM yyyy"), "November 2026");
        assert_eq!(format(&t, "EEE"), "Sat");
        assert_eq!(format(&t, "EEEEE"), "S");
        assert_eq!(format(&t, "EEEE"), "Saturday");
        assert_eq!(format(&t, "d"), "14");
        assert_eq!(format(&t, "HH:mm"), "09:41");
        assert_eq!(format(&t, "h:mm a"), "9:41 AM");
        assert_eq!(format(&dt(2026, 1, 3, 0, 5), "h:mm a"), "12:05 AM");
        assert_eq!(format(&dt(2026, 1, 3, 13, 5), "hh:mm:ss a"), "01:05:05 PM");
        assert_eq!(format(&t, "'Week of' MMM d, yyyy"), "Week of Nov 14, 2026");
        assert_eq!(format(&t, "'It''s' p"), "It's 9:41 AM");
        assert_eq!(format(&t, "dd/MM/yy"), "14/11/26");
    }

    #[test]
    fn weeks_and_months() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 14).unwrap(); // Saturday
        assert_eq!(
            start_of_week(d, 1),
            NaiveDate::from_ymd_opt(2026, 11, 9).unwrap()
        );
        assert_eq!(
            start_of_week(d, 0),
            NaiveDate::from_ymd_opt(2026, 11, 8).unwrap()
        );
        assert_eq!(
            end_of_month(d),
            NaiveDate::from_ymd_opt(2026, 11, 30).unwrap()
        );
        assert_eq!(
            add_months(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(), 1),
            NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()
        );
        assert_eq!(
            add_months(d, -11),
            NaiveDate::from_ymd_opt(2025, 12, 14).unwrap()
        );
        let g = month_grid(d, 1);
        assert_eq!(g.len() % 7, 0);
        assert_eq!(g[0], NaiveDate::from_ymd_opt(2026, 10, 26).unwrap());
        assert_eq!(
            *g.last().unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 6).unwrap()
        );
        assert_eq!(weekday_labels(1, "EEEEE").join(""), "MTWTFSS");
        assert_eq!(weekday_labels(0, "EEE")[0], "Sun");
    }
}
