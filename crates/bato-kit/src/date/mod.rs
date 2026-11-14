//! Date and time (B5): Calendar, DatePicker, DateRangePicker, DateTimePicker, TimePicker,
//! CalendarView. Date maths and formatting: `crate::datefmt`.
//!
//! Phase 3 batches add one file per component here and one `mod` line below.
//!
//! | Module | React |
//! |---|---|
//! | [`calendar`] | `Calendar.tsx`: month grid, single or range selection |
//! | [`picker`] | `DatePicker`, `DateRangePicker`, `DateTimePicker`: a control-look trigger + Calendar popover |
//! | [`time_picker`] | `TimePicker.tsx`: scroll columns, the active item centred in its column |
//! | [`calendar_view`] | `CalendarView.tsx`: month and week event views |
//! | [`math`] | the date-fns maths of all of them (unit-tested) |
//!
//! State follows the kit rule: values come from the app; what React keeps in
//! `useState` (the shown month, the hovered day, open/closed, the view and
//! cursor) is a small `…State` struct the app stores and updates with the
//! component's `…Msg`.

pub mod calendar;
pub mod calendar_view;
pub mod math;
pub mod picker;
pub mod time_picker;
mod trigger;

pub use calendar::{Calendar, CalendarMsg, CalendarState, calendar};
pub use calendar_view::{
    CalendarEvent, CalendarView, CalendarViewMsg, CalendarViewState, View, calendar_view,
};
pub use math::DateRange;
pub use picker::{PickerMsg, PickerState, date_picker, date_range_picker, date_time_picker};
pub use time_picker::{TimePicker, time_picker};

use iced::Color;

use crate::El;
use crate::ctx::Ctx;
use crate::focus::Ring;
use crate::icon::{Ico, icon};
use crate::style::{c, inset_of, shadows};
use crate::surface::{BoxStyle, Status, surface};

/// Chromium's own `:focus-visible` outline (`outline: auto`) on a button
/// that sets no focus style (Calendar days, TimePicker items, CalendarView
/// cells): a 2 px ring in the dark color-scheme's focus colour `#99c8ff`
/// with a 1 px `#101010` contrast ring outside it.
pub(crate) fn ua_ring() -> Ring {
    Ring {
        width: 1.0,
        color: Color::from_rgb8(0x10, 0x10, 0x10),
        offset: 2.0,
        offset_color: Color::from_rgb8(0x99, 0xc8, 0xff),
    }
}

/// `Popover`'s panel: `bg-app-card border border-app-lineStrong rounded-xl
/// shadow-app-lg`, `p-3` (+ the border).
pub(crate) fn popover_panel<'a, M: Clone + 'a>(
    cx: &Ctx,
    content: impl Into<El<'a, M>>,
) -> El<'a, M> {
    let on = cx.on_card();
    let p = on.p();
    surface(content)
        .padding(13)
        .fixed(BoxStyle {
            bg: Some(c(p.card)),
            border: Some(c(on.line_strong())),
            radius: 12.0,
            shadows: shadows(&p.shadow_lg),
            inset_top: inset_of(&p.shadow_lg, p.card),
            ..BoxStyle::default()
        })
        .into()
}

/// A chevron button: `p-1 rounded-md text-app-muted hover:text-app-text
/// hover:bg-app-text/[0.06]` (no transition); `pad` = 4 (Calendar) or 6
/// (CalendarView's `p-1.5`).
pub(crate) fn nav_button<'a, M: Clone + 'a>(cx: &Ctx, ico: Ico, pad: f32, msg: M) -> El<'a, M> {
    let p = cx.p().clone();
    let hover = c(cx.wash(0.06));
    surface(icon(ico, 16.0))
        .padding(pad)
        .transition_ms(0)
        .on_press(msg)
        .focus_ring(ua_ring())
        .style(move |s| {
            let b = BoxStyle::default().radius(6.0);
            match s {
                Status::Hover => b.bg(hover).fg(c(p.text)),
                _ => b.fg(c(p.muted)),
            }
        })
        .into()
}
