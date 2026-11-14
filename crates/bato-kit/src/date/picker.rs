//! `DatePicker`, `DateRangePicker` and `DateTimePicker`: a [`field`] whose
//! control is a button with the text-control look (calendar icon 15 muted,
//! the formatted value or a muted placeholder, ellipsized) that toggles a
//! Calendar in a Popover (`bottom` / `start`, 6 px, `p-3` card panel, scale-in).
//!
//! | Picker | Text | Closes |
//! |---|---|---|
//! | [`date_picker`] | `PP` (`Nov 14, 2026`); `clearable` adds an × that clears without opening | on a pick |
//! | [`date_range_picker`] | `PP → PP`, `…` while the end is open | when both ends are picked |
//! | [`date_time_picker`] | `PP · HH:mm` (`h:mm a`, `:ss` with the options); Calendar + TimePicker side by side | never on a pick |
//!
//! Every popover also closes on an outside press, Escape, a page scroll,
//! a resize or the window losing focus ([`Dismiss::POPOVER`]). The Calendar
//! remounts on every open, showing the value's month (or today's).
//!
//! ```ignore
//! date_picker(&cx, &state.p, state.date, Msg::Date).label("Date").clearable(true)
//! // update: Msg::Date(m) => state.p.update_date(m, &mut state.date),
//! ```

use iced::widget::{Space, container, row};
use iced::{Alignment, Length};

use super::calendar::{CalendarMsg, CalendarState, calendar, height as calendar_height};
use super::math::{DateRange, keep_time, trunc_seconds};
use super::popover_panel;
use super::time_picker::time_picker;
use super::trigger::trigger;
use crate::El;
use crate::ctx::Ctx;
use crate::datefmt::{self, NaiveDate, NaiveDateTime};
use crate::field::{FieldSize, field};
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::overlay::popover::press_probe;
use crate::overlay::{Dismiss, anchored};
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

/// A picker's transient state: open or not, and its Calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PickerState {
    pub open: bool,
    pub cal: CalendarState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickerMsg {
    /// The trigger: open or close.
    Toggle,
    /// A dismissal gesture.
    Close,
    /// The × of a clearable DatePicker.
    Clear,
    Cal(CalendarMsg),
    /// DateTimePicker's TimePicker.
    Time(NaiveDateTime),
}

impl PickerState {
    fn toggle(&mut self, month: Option<NaiveDate>) {
        self.open = !self.open;
        if self.open {
            // The Calendar mounts: `selected ?? range.start ?? new Date()`.
            self.cal = CalendarState::new(month.unwrap_or_else(datefmt::today));
        }
    }

    /// DatePicker: closes on a pick.
    pub fn update_date(&mut self, msg: PickerMsg, value: &mut Option<NaiveDate>) {
        match msg {
            PickerMsg::Toggle => self.toggle(*value),
            PickerMsg::Close => self.open = false,
            PickerMsg::Clear => *value = None,
            PickerMsg::Cal(m) => {
                if let Some(d) = self.cal.update(m) {
                    *value = Some(d);
                    self.open = false;
                }
            }
            PickerMsg::Time(_) => {}
        }
    }

    /// DateRangePicker: closes once both ends are picked.
    pub fn update_range(&mut self, msg: PickerMsg, value: &mut DateRange) {
        match msg {
            PickerMsg::Toggle => self.toggle(value.start),
            PickerMsg::Close => self.open = false,
            PickerMsg::Clear => *value = DateRange::EMPTY,
            PickerMsg::Cal(m) => {
                if let Some(d) = self.cal.update(m) {
                    *value = value.pick(d);
                    if value.is_complete() {
                        self.open = false;
                    }
                }
            }
            PickerMsg::Time(_) => {}
        }
    }

    /// DateTimePicker: a day keeps the time of day (or now's); stays open.
    pub fn update_datetime(&mut self, msg: PickerMsg, value: &mut Option<NaiveDateTime>) {
        match msg {
            PickerMsg::Toggle => self.toggle(value.map(|v| v.date())),
            PickerMsg::Close => self.open = false,
            PickerMsg::Clear => *value = None,
            PickerMsg::Cal(m) => {
                if let Some(d) = self.cal.update(m) {
                    let base = value.unwrap_or_else(|| trunc_seconds(datefmt::now()));
                    *value = Some(keep_time(d, base));
                }
            }
            PickerMsg::Time(t) => *value = Some(t),
        }
    }
}

/// The options every picker shares (`Field` props, size, limits).
pub struct Picker<'a, M> {
    cx: Ctx,
    kind: Kind,
    state: PickerState,
    on: std::rc::Rc<dyn Fn(PickerMsg) -> M + 'a>,
    label: Option<String>,
    hint: Option<String>,
    error: Option<String>,
    placeholder: Option<String>,
    date_format: String,
    clearable: bool,
    size: FieldSize,
    min: Option<NaiveDate>,
    max: Option<NaiveDate>,
    week_starts_on: u8,
    hour12: bool,
    show_seconds: bool,
    minute_step: u32,
    id: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Date(Option<NaiveDate>),
    Range(DateRange),
    DateTime(Option<NaiveDateTime>),
}

fn picker<'a, M: Clone + 'a>(
    cx: &Ctx,
    state: &PickerState,
    kind: Kind,
    on: impl Fn(PickerMsg) -> M + 'a,
) -> Picker<'a, M> {
    Picker {
        cx: cx.clone(),
        kind,
        state: *state,
        on: std::rc::Rc::new(on),
        label: None,
        hint: None,
        error: None,
        placeholder: None,
        date_format: "PP".into(),
        clearable: false,
        size: FieldSize::Md,
        min: None,
        max: None,
        week_starts_on: 1,
        hour12: false,
        show_seconds: false,
        minute_step: 5,
        id: None,
    }
}

/// `DatePicker` for `value`.
pub fn date_picker<'a, M: Clone + 'a>(
    cx: &Ctx,
    state: &PickerState,
    value: Option<NaiveDate>,
    on: impl Fn(PickerMsg) -> M + 'a,
) -> Picker<'a, M> {
    picker(cx, state, Kind::Date(value), on)
}

/// `DateRangePicker` for `value`.
pub fn date_range_picker<'a, M: Clone + 'a>(
    cx: &Ctx,
    state: &PickerState,
    value: DateRange,
    on: impl Fn(PickerMsg) -> M + 'a,
) -> Picker<'a, M> {
    picker(cx, state, Kind::Range(value), on)
}

/// `DateTimePicker` for `value` (minute step 5).
pub fn date_time_picker<'a, M: Clone + 'a>(
    cx: &Ctx,
    state: &PickerState,
    value: Option<NaiveDateTime>,
    on: impl Fn(PickerMsg) -> M + 'a,
) -> Picker<'a, M> {
    picker(cx, state, Kind::DateTime(value), on)
}

impl<'a, M: Clone + 'a> Picker<'a, M> {
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = Some(text.into());
        self
    }
    pub fn hint(mut self, text: impl Into<String>) -> Self {
        self.hint = Some(text.into());
        self
    }
    pub fn error(mut self, text: Option<&str>) -> Self {
        self.error = text.map(str::to_string);
        self
    }
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }
    /// A date-fns pattern (default `PP`; DateTimePicker ignores it).
    pub fn date_format(mut self, f: impl Into<String>) -> Self {
        self.date_format = f.into();
        self
    }
    /// DatePicker: an × that clears the value.
    pub fn clearable(mut self, on: bool) -> Self {
        self.clearable = on;
        self
    }
    pub fn size(mut self, s: FieldSize) -> Self {
        self.size = s;
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
    pub fn week_starts_on(mut self, d: u8) -> Self {
        self.week_starts_on = d;
        self
    }
    /// DateTimePicker: 12-hour clock with AM/PM.
    pub fn hour12(mut self, on: bool) -> Self {
        self.hour12 = on;
        self
    }
    /// DateTimePicker: a seconds column and `:ss`.
    pub fn show_seconds(mut self, on: bool) -> Self {
        self.show_seconds = on;
        self
    }
    /// DateTimePicker: the minute column's step (default 5).
    pub fn minute_step(mut self, step: u32) -> Self {
        self.minute_step = step;
        self
    }
    /// A unique id: the trigger's focus id, the popover body's container id
    /// (`<id>:panel`) and the TimePicker columns' scroller ids
    /// (`<id>:time:h` …).
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// The trigger text and whether it is the placeholder.
    fn text(&self) -> (String, bool) {
        let f = &self.date_format;
        match self.kind {
            Kind::Date(Some(d)) => (datefmt::format_date(d, f), false),
            Kind::Date(None) => (self.placeholder_or("Pick a date"), true),
            Kind::Range(r) => match r.start {
                Some(s) => (
                    format!(
                        "{} → {}",
                        datefmt::format_date(s, f),
                        r.end
                            .map_or("…".to_string(), |e| datefmt::format_date(e, f))
                    ),
                    false,
                ),
                None => (self.placeholder_or("Pick a range"), true),
            },
            Kind::DateTime(Some(t)) => (datefmt::format(&t, &self.datetime_format()), false),
            Kind::DateTime(None) => (self.placeholder_or("Pick date & time"), true),
        }
    }

    fn placeholder_or(&self, default: &str) -> String {
        self.placeholder.clone().unwrap_or_else(|| default.into())
    }

    /// `` `PP '·' ${timeFmt}` ``.
    fn datetime_format(&self) -> String {
        format!(
            "PP '·' {}:mm{}{}",
            if self.hour12 { "h" } else { "HH" },
            if self.show_seconds { ":ss" } else { "" },
            if self.hour12 { " a" } else { "" },
        )
    }
}

impl<'a, M: Clone + 'a> From<Picker<'a, M>> for El<'a, M> {
    fn from(pk: Picker<'a, M>) -> Self {
        let cx = &pk.cx;
        let p = cx.p().clone();
        let m = pk.size.metrics();
        let (text, placeholder) = pk.text();
        let ico = match pk.kind {
            Kind::DateTime(_) => Ico::CalendarClock,
            _ => Ico::Calendar,
        };
        let mut content = row![
            icon(ico, 15.0).color(c(p.muted)),
            label(text, m.text, m.line)
                .ellipsis()
                .width(Length::Fill)
                .color_maybe(placeholder.then_some(c(p.muted))),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if pk.clearable && matches!(pk.kind, Kind::Date(Some(_))) {
            // `<span role="button">`: not focusable, no transition.
            let p = p.clone();
            content = content.push(
                surface(icon(Ico::X, 14.0))
                    .on_press((pk.on)(PickerMsg::Clear))
                    .focusable(false)
                    .transition_ms(0)
                    .style(move |s| {
                        BoxStyle::default().fg(c(if s == Status::Hover { p.text } else { p.muted }))
                    }),
            );
        }
        let button = trigger(cx, content)
            .size(pk.size)
            .error(pk.error.is_some())
            .on_press((pk.on)(PickerMsg::Toggle))
            .id(pk.id.clone().map(iced::widget::Id::from));

        let panel = pk.state.open.then(|| {
            let on = pk.on.clone();
            let mut cal = calendar(cx, &pk.state.cal, move |m| on(PickerMsg::Cal(m)))
                .min(pk.min)
                .max(pk.max)
                .week_starts_on(pk.week_starts_on);
            cal = match pk.kind {
                Kind::Date(v) => cal.single(v),
                Kind::Range(r) => cal.range(r),
                Kind::DateTime(v) => cal.single(v.map(|t| t.date())),
            };
            let body: El<'a, M> = match pk.kind {
                Kind::DateTime(v) => {
                    let on = pk.on.clone();
                    let now = v.unwrap_or_else(|| trunc_seconds(datefmt::now()));
                    let mut tp = time_picker(cx, now, move |t| on(PickerMsg::Time(t)))
                        .hour12(pk.hour12)
                        .show_seconds(pk.show_seconds)
                        .minute_step(pk.minute_step);
                    if let Some(id) = &pk.id {
                        tp = tp.id(format!("{id}:time"));
                    }
                    // `flex gap-3`; the time side is `border-l pl-3`,
                    // stretched to the row's height.
                    let h = calendar_height(pk.state.cal.month, pk.week_starts_on).max(184.0);
                    let border = c(p.border);
                    let line =
                        container(Space::new().width(1).height(Length::Fill)).style(move |_| {
                            iced::widget::container::Style::default().background(border)
                        });
                    row![
                        El::from(cal),
                        row![line, Space::new().width(12), El::from(tp)].height(h),
                    ]
                    .spacing(12)
                    .into()
                }
                _ => cal.into(),
            };
            let body = container(body).id(iced::widget::Id::from(format!(
                "{}:panel",
                pk.id.as_deref().unwrap_or("picker")
            )));
            press_probe(popover_panel(cx, body))
        });
        let on = pk.on;
        let pop = anchored(button, panel)
            .dismiss(Dismiss::POPOVER)
            .on_dismiss(move |_| on(PickerMsg::Close));

        let mut f = field(cx, pop).error(pk.error.as_deref());
        if let Some(l) = pk.label {
            f = f.label(l);
        }
        if let Some(h) = pk.hint {
            f = f.hint(h);
        }
        f.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn date_picker_opens_on_the_value_and_closes_on_pick() {
        let mut st = PickerState::default();
        let mut v = Some(d(2024, 2, 29));
        st.update_date(PickerMsg::Toggle, &mut v);
        assert!(st.open);
        assert_eq!(st.cal.month, d(2024, 2, 29));
        st.update_date(PickerMsg::Cal(CalendarMsg::Month(d(2024, 3, 29))), &mut v);
        assert!(st.open);
        st.update_date(PickerMsg::Cal(CalendarMsg::Pick(d(2024, 3, 2))), &mut v);
        assert_eq!((st.open, v), (false, Some(d(2024, 3, 2))));
        st.update_date(PickerMsg::Clear, &mut v);
        assert_eq!(v, None);
        // Toggle twice closes.
        st.update_date(PickerMsg::Toggle, &mut v);
        st.update_date(PickerMsg::Toggle, &mut v);
        assert!(!st.open);
    }

    #[test]
    fn range_picker_closes_when_complete() {
        let mut st = PickerState::default();
        let mut r = DateRange::EMPTY;
        st.update_range(PickerMsg::Toggle, &mut r);
        st.update_range(PickerMsg::Cal(CalendarMsg::Pick(d(2026, 11, 20))), &mut r);
        assert!(st.open && !r.is_complete());
        st.update_range(PickerMsg::Cal(CalendarMsg::Pick(d(2026, 11, 3))), &mut r);
        assert!(!st.open);
        assert_eq!(r, DateRange::new(d(2026, 11, 3), Some(d(2026, 11, 20))));
        // Reopening shows the start's month.
        st.update_range(PickerMsg::Toggle, &mut r);
        assert_eq!(st.cal.month, d(2026, 11, 3));
    }

    #[test]
    fn datetime_keeps_the_time_of_day() {
        let mut st = PickerState::default();
        let t = d(2026, 11, 14).and_hms_opt(8, 23, 41).unwrap();
        let mut v = Some(t);
        st.update_datetime(PickerMsg::Toggle, &mut v);
        st.update_datetime(PickerMsg::Cal(CalendarMsg::Pick(d(2026, 12, 1))), &mut v);
        assert!(st.open, "stays open");
        assert_eq!(v, Some(d(2026, 12, 1).and_hms_opt(8, 23, 41).unwrap()));
        let t2 = d(2026, 12, 1).and_hms_opt(20, 5, 0).unwrap();
        st.update_datetime(PickerMsg::Time(t2), &mut v);
        assert_eq!(v, Some(t2));
    }

    #[test]
    fn trigger_texts() {
        let cx = Ctx::from_palette(bato_theme::palette(
            bato_theme::ThemeId::Dark,
            bato_theme::Rgb::hex("#8b5cf6").unwrap(),
        ));
        let st = PickerState::default();
        let t = d(2026, 11, 14).and_hms_opt(20, 5, 9).unwrap();
        let text = |p: Picker<'_, PickerMsg>| p.text();
        assert_eq!(
            text(date_picker(&cx, &st, Some(d(2026, 11, 14)), |m| m)),
            ("Nov 14, 2026".into(), false)
        );
        assert_eq!(
            text(date_picker(&cx, &st, None, |m| m)),
            ("Pick a date".into(), true)
        );
        assert_eq!(
            text(date_range_picker(
                &cx,
                &st,
                DateRange::new(d(2026, 11, 3), None),
                |m| m
            ))
            .0,
            "Nov 3, 2026 → …"
        );
        assert_eq!(
            text(date_range_picker(
                &cx,
                &st,
                DateRange::new(d(2026, 11, 3), Some(d(2026, 12, 1))),
                |m| m
            ))
            .0,
            "Nov 3, 2026 → Dec 1, 2026"
        );
        assert_eq!(
            text(date_time_picker(&cx, &st, Some(t), |m| m)).0,
            "Nov 14, 2026 · 20:05"
        );
        assert_eq!(
            text(
                date_time_picker(&cx, &st, Some(t), |m| m)
                    .hour12(true)
                    .show_seconds(true)
            )
            .0,
            "Nov 14, 2026 · 8:05:09 PM"
        );
    }
}
