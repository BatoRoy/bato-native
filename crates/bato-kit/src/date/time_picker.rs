//! `TimePicker` (`TimePicker.tsx`): scroll columns Hour / Min (/ Sec) and
//! an AM/PM column on a 12-hour clock; the date part of the value is kept.
//!
//! - A number column is 48 px wide (`min-w-[3rem]`): a 12/16 medium muted
//!   title (`py-1`), then a list (`gap-0.5 px-1`, at most 160 px tall,
//!   scrolling with the classic 5 px scrollbar) of 28 px items (`px-2 py-1
//!   rounded-md`, 14/20): the active one `accent` + `accentInk`, the others
//!   subtext with a `text@.06` hover, 150 ms colour transition.
//! - The active item is **centred within its own column** when it mounts and
//!   whenever its value changes, by setting that column's offset (React sets
//!   `scrollTop`, never `scrollIntoView`, so the page does not move). A
//!   column whose value is not in its list (a minute step) does not move.
//!   Give the picker an [`TimePicker::id`] to recentre on change; without one
//!   it centres on mount only.
//! - Columns are 4 px apart; AM/PM has its title inside the 2 px-gap list.
//!
//! Keyboard: every item is a button in Tab order (React's), Enter / Space
//! pick.

use std::rc::Rc;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::widget::{Column, column, container, row};
use iced::{Element, Event, Length, Padding, Rectangle, Size, mouse};

use super::math::{
    centre_offset, display_hour12, hour_from12, hour_meridiem, hours, set_time, steps,
};
use super::ua_ring;
use crate::El;
use crate::ctx::Ctx;
use crate::datefmt::{NaiveDateTime, Timelike};
use crate::label::label;
use crate::scroller::{scroll_area, scroll_offset};
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

/// Item height (`py-1` + 20), gap (`gap-0.5`) and the list's `max-h-40`.
pub const ITEM_H: f32 = 28.0;
pub const ITEM_GAP: f32 = 2.0;
pub const LIST_MAX: f32 = 160.0;
/// A number column's width (`min-w-[3rem]`).
pub const COLUMN_W: f32 = 48.0;

type OnChange<'a, M> = Rc<dyn Fn(NaiveDateTime) -> M + 'a>;

pub struct TimePicker<'a, M> {
    cx: Ctx,
    value: NaiveDateTime,
    on_change: OnChange<'a, M>,
    hour12: bool,
    show_seconds: bool,
    minute_step: u32,
    second_step: u32,
    id: Option<String>,
}

/// A time picker for `value` (24-hour, minutes, steps of 1).
pub fn time_picker<'a, M: Clone + 'a>(
    cx: &Ctx,
    value: NaiveDateTime,
    on_change: impl Fn(NaiveDateTime) -> M + 'a,
) -> TimePicker<'a, M> {
    TimePicker {
        cx: cx.clone(),
        value,
        on_change: Rc::new(on_change),
        hour12: false,
        show_seconds: false,
        minute_step: 1,
        second_step: 1,
        id: None,
    }
}

impl<'a, M: Clone + 'a> TimePicker<'a, M> {
    pub fn hour12(mut self, on: bool) -> Self {
        self.hour12 = on;
        self
    }
    pub fn show_seconds(mut self, on: bool) -> Self {
        self.show_seconds = on;
        self
    }
    pub fn minute_step(mut self, step: u32) -> Self {
        self.minute_step = step.max(1);
        self
    }
    pub fn second_step(mut self, step: u32) -> Self {
        self.second_step = step.max(1);
        self
    }
    /// A unique prefix for the columns' scroller ids (`<id>:h`, `:m`, `:s`),
    /// needed to recentre a column when its value changes.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
}

/// One item: `px-2 py-1 rounded-md text-sm`, active = accent.
fn item<'a, M: Clone + 'a>(cx: &Ctx, text: String, active: bool, msg: M) -> El<'a, M> {
    let p = cx.p().clone();
    let hover = c(cx.wash(0.06));
    surface(label(text, 14.0, 20.0))
        .width(Length::Fill)
        .padding([4, 8])
        .center()
        .on_press(msg)
        .focus_ring(ua_ring())
        .style(move |s| {
            let b = BoxStyle::default().radius(6.0);
            if active {
                b.bg(c(p.accent)).fg(c(p.accent_ink))
            } else if s == Status::Hover {
                b.bg(hover).fg(c(p.subtext))
            } else {
                b.fg(c(p.subtext))
            }
        })
        .into()
}

/// `text-center text-xs font-medium text-app-muted py-1`.
fn title<'a, M: 'a>(cx: &Ctx, text: &str) -> El<'a, M> {
    container(label(text, 12.0, 16.0).medium().color(c(cx.p().muted)))
        .center_x(Length::Fill)
        .padding([4, 0])
        .into()
}

/// A number column (Hour / Min / Sec).
fn number_column<'a, M: Clone + 'a>(
    cx: &Ctx,
    name: &str,
    items: Vec<u32>,
    value: u32,
    pick: impl Fn(u32) -> M,
    id: Option<String>,
) -> El<'a, M> {
    let index = items.iter().position(|v| *v == value);
    let target = index.map(|i| centre_offset(i, items.len(), ITEM_H, ITEM_GAP, LIST_MAX));
    let list = Column::with_children(
        items
            .iter()
            .map(|v| item(cx, format!("{v:02}"), *v == value, pick(*v))),
    )
    .spacing(ITEM_GAP)
    .padding([0, 4]);
    let mut area = scroll_area(cx, list).max_height(LIST_MAX).initial(target);
    if let Some(id) = &id {
        area = area.id(widget::Id::from(id.clone()));
    }
    column![
        title(cx, name),
        Recentre {
            content: area.into(),
            value,
            target,
            id: id.map(widget::Id::from),
        }
    ]
    .width(COLUMN_W)
    .into()
}

impl<'a, M: Clone + 'a> From<TimePicker<'a, M>> for El<'a, M> {
    fn from(tp: TimePicker<'a, M>) -> Self {
        let cx = &tp.cx;
        let v = tp.value;
        let h = v.hour();
        let is_pm = h >= 12;
        let sub = |s: &str| tp.id.as_ref().map(|id| format!("{id}:{s}"));

        let on = tp.on_change.clone();
        let hour12 = tp.hour12;
        let hour_col = number_column(
            cx,
            "Hour",
            hours(hour12),
            if hour12 { display_hour12(h) } else { h },
            move |x| {
                let hh = if hour12 { hour_from12(h, x) } else { x };
                on(set_time(v, Some(hh), None, None))
            },
            sub("h"),
        );
        let on = tp.on_change.clone();
        let min_col = number_column(
            cx,
            "Min",
            steps(tp.minute_step),
            v.minute(),
            move |x| on(set_time(v, None, Some(x), None)),
            sub("m"),
        );
        let mut cols: Vec<El<'a, M>> = vec![hour_col, min_col];
        if tp.show_seconds {
            let on = tp.on_change.clone();
            cols.push(number_column(
                cx,
                "Sec",
                steps(tp.second_step),
                v.second(),
                move |x| on(set_time(v, None, None, Some(x))),
                sub("s"),
            ));
        }
        if hour12 {
            // `flex flex-col gap-0.5 px-1`, as wide as its widest child.
            let w = ["AM", "PM"]
                .iter()
                .map(|t| label(*t, 14.0, 20.0).shape(f32::INFINITY).width() + 16.0)
                .chain([label("AM/PM", 12.0, 16.0)
                    .medium()
                    .shape(f32::INFINITY)
                    .width()])
                .fold(0.0, f32::max);
            let mut col = column![title(cx, "AM/PM")].spacing(ITEM_GAP);
            for (text, pm) in [("AM", false), ("PM", true)] {
                col = col.push(item(
                    cx,
                    text.to_string(),
                    pm == is_pm,
                    (tp.on_change)(set_time(v, Some(hour_meridiem(h, pm)), None, None)),
                ));
            }
            cols.push(
                container(col.width(w.ceil()))
                    .padding(Padding::from([0, 4]))
                    .into(),
            );
        }
        row(cols).spacing(4).into()
    }
}

/// React's `useEffect(…, [value])` on a column: when the column's value
/// changes between renders, set its scroller's offset to centre the new
/// active item (instantly). Mounting is covered by the scroller's
/// `initial` offset.
struct Recentre<'a, M> {
    content: El<'a, M>,
    value: u32,
    target: Option<f32>,
    id: Option<widget::Id>,
}

struct Seen(u32);

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for Recentre<'a, M> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Seen>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(Seen(self.value))
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        let seen = tree.state.downcast_mut::<Seen>();
        if seen.0 != self.value {
            seen.0 = self.value;
            if let (Some(id), Some(y)) = (&self.id, self.target) {
                scroll_offset(id.clone(), None, Some(y), false);
            }
        }
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: 'a> From<Recentre<'a, M>> for El<'a, M> {
    fn from(r: Recentre<'a, M>) -> Self {
        Element::new(r)
    }
}
