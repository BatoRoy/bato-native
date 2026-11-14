//! `SortableList`: a vertical drag-to-reorder list (`space-y-2`) of themed
//! rows: `bg-app-card border border-app-border rounded-lg px-3 py-2.5`,
//! a `GripVertical` handle (`text-app-muted hover:text-app-text
//! cursor-grab active:cursor-grabbing`), then the item's content.
//!
//! While dragged a row follows the pointer at `opacity-50 shadow-app-lg`
//! above everything; the others slide (200 ms `ease`). With the handle only
//! the grip starts a drag; without it the whole row does.
//!
//! Keyboard: every row is a Tab stop (Chromium's own focus ring, as React's
//! row sets no focus style). Space / Enter picks the focused row up, ↑ / ↓
//! move it, Space / Enter drops, Esc cancels. React wires these keys to the
//! grip (`tabIndex=-1`), so there they only work after clicking it; here the
//! focused row takes them.
//!
//! ```ignore
//! sortable_list(&cx.on_card(), items.iter().map(|s| (s.clone(), label(s, 14.0, 20.0))))
//!     .on_reorder(Msg::Reorder)   // (from, to): dnd::array_move(&mut items, from, to)
//! ```

use iced::widget::{container, row};
use iced::{Alignment, Length, Padding};

use crate::El;
use crate::ctx::Ctx;
use crate::cursor::Cursor;
use crate::date::ua_ring;
use crate::dnd::{self, Axis, PreviewStyle};
use crate::icon::{Ico, icon};
use crate::style::{c, inset_of, shadows};
use crate::surface::{BoxStyle, Status, surface};

/// Space between rows (`space-y-2`).
pub const GAP: f32 = 8.0;
/// The row's line box: the content sits in a 16 px / 1.5 block, so the
/// `text-sm` item text still gets a 24 px line.
pub const LINE: f32 = 24.0;

pub struct SortableList<'a, K, M> {
    cx: Ctx,
    items: Vec<(K, El<'a, M>)>,
    handle: bool,
    on_reorder: Option<Box<dyn Fn(usize, usize) -> M + 'a>>,
}

/// A SortableList of `(id, content)` rows; `cx` is what the list sits on.
pub fn sortable_list<'a, K, M>(
    cx: &Ctx,
    items: impl IntoIterator<Item = (K, impl Into<El<'a, M>>)>,
) -> SortableList<'a, K, M> {
    SortableList {
        cx: cx.clone(),
        items: items.into_iter().map(|(k, e)| (k, e.into())).collect(),
        handle: true,
        on_reorder: None,
    }
}

impl<'a, K: Clone + PartialEq + 'static, M: Clone + 'a> SortableList<'a, K, M> {
    /// Show the grip (default); without it the whole row drags.
    pub fn handle(mut self, on: bool) -> Self {
        self.handle = on;
        self
    }
    /// `(from, to)` after a drop (React's `onReorder(arrayMove(..))`).
    pub fn on_reorder(mut self, f: impl Fn(usize, usize) -> M + 'a) -> Self {
        self.on_reorder = Some(Box::new(f));
        self
    }
}

/// The row box.
pub fn row_style(cx: &Ctx) -> BoxStyle {
    let p = cx.p();
    BoxStyle::default()
        .bg(c(p.card))
        .border(c(p.border))
        .radius(8.0)
}

/// `shadow-app-lg` under / over a dragged box with radius `r`.
pub fn lifted(cx: &Ctx, r: f32, opacity: f32) -> PreviewStyle {
    let p = cx.p();
    PreviewStyle {
        opacity,
        under: Some(BoxStyle {
            shadows: shadows(&p.shadow_lg),
            radius: r,
            ..BoxStyle::default()
        }),
        over: inset_of(&p.shadow_lg, p.card).map(|i| BoxStyle {
            inset_top: Some(i),
            radius: r,
            ..BoxStyle::default()
        }),
    }
}

impl<'a, K: Clone + PartialEq + 'static, M: Clone + 'a> From<SortableList<'a, K, M>> for El<'a, M> {
    fn from(s: SortableList<'a, K, M>) -> Self {
        let cx = s.cx.on_card();
        let p = cx.p().clone();
        let mut list = dnd::sortable::<K, M>(Axis::Vertical)
            .gap(GAP)
            .preview(lifted(&cx, 8.0, 0.5))
            .focusable(ua_ring(), 8.0)
            .keyboard(true);
        list = if s.handle {
            list.cursors(Some(Cursor::Grabbing), Some(Cursor::Grabbing))
        } else {
            list
        };
        for (k, content) in s.items {
            // The 16 px strut puts the 14 px text's baseline 0.7 px lower
            // than centring its 20 px line would; Chromium rounds it to 1.
            let body: El<'a, M> = container(content)
                .width(Length::Fill)
                .height(LINE)
                .padding(Padding::ZERO.top(3))
                .into();
            let inner: El<'a, M> = if s.handle {
                let (muted, text) = (c(p.muted), c(p.text));
                let grip = surface(icon(Ico::GripVertical, 16.0))
                    .style(move |st| {
                        BoxStyle::default().fg(if st == Status::Hover { text } else { muted })
                    })
                    .transition_ms(0)
                    .cursor(Cursor::Grab);
                row![dnd::handle(grip), body]
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .into()
            } else {
                row![body].into()
            };
            let rowbox = surface(inner)
                .padding([11, 13])
                .width(Length::Fill)
                .fixed(row_style(&cx));
            list = list.item(k, rowbox);
        }
        if let Some(f) = s.on_reorder {
            list = list.on_reorder(f);
        }
        list.into()
    }
}
