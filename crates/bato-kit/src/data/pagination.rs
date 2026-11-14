//! `Pagination` (`data/Pagination.tsx`): Prev, an ellipsis page list
//! (`1 … 4 5 [6] 7 8 … 20`, [`page_list`]) and Next, `gap-1`.
//!
//! Buttons are `min-w-8 h-8 px-2 rounded-lg text-sm` with
//! `transition-colors`: the current page is an accent fill with
//! `accent_ink` medium text; other pages are `subtext`, Prev / Next `muted`,
//! all turning `text` on a `text@.06` hover. Prev / Next at an end are
//! disabled (`opacity-40`, no hover). A gap is a 32 px `MoreHorizontal` (14,
//! muted). Nothing is drawn for a single page (React returns `null`).
//!
//! ```ignore
//! pagination(&cx, self.page, 12, Msg::Page)            // siblings 1
//! pagination(&cx, page, count, Msg::Page).siblings(2)
//! ```

use iced::Alignment;
use iced::widget::{Space, row};

use crate::El;
use crate::ctx::Ctx;
use crate::focus::Ring;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

/// One entry of the page list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageItem {
    Page(usize),
    Gap,
}

/// The page list with ellipses (`pageList`): the first and last page,
/// `siblings` pages either side of `page`, and a gap where pages are left
/// out. Pages are 1-based.
pub fn page_list(page: usize, count: usize, siblings: usize) -> Vec<PageItem> {
    let mut out = vec![PageItem::Page(1)];
    let lo = page.saturating_sub(siblings).max(2);
    let hi = (page + siblings).min(count.saturating_sub(1));
    if lo > 2 {
        out.push(PageItem::Gap);
    }
    for i in lo..=hi {
        out.push(PageItem::Page(i));
    }
    if hi < count.saturating_sub(1) {
        out.push(PageItem::Gap);
    }
    if count > 1 {
        out.push(PageItem::Page(count));
    }
    out
}

pub struct Pagination<'a, M> {
    cx: Ctx,
    page: usize,
    count: usize,
    siblings: usize,
    on_change: Box<dyn Fn(usize) -> M + 'a>,
}

/// A pager over `count` pages at `page` (1-based); `on_change` gets the new
/// page.
pub fn pagination<'a, M: Clone + 'a>(
    cx: &Ctx,
    page: usize,
    count: usize,
    on_change: impl Fn(usize) -> M + 'a,
) -> Pagination<'a, M> {
    Pagination {
        cx: cx.clone(),
        page,
        count,
        siblings: 1,
        on_change: Box::new(on_change),
    }
}

impl<'a, M: Clone + 'a> Pagination<'a, M> {
    /// Pages shown around the current one (default 1).
    pub fn siblings(mut self, n: usize) -> Self {
        self.siblings = n;
        self
    }
}

/// The style of a page / arrow button: `rest` text colour, active fill.
fn btn_style(cx: &Ctx, active: bool, rest: bato_theme::Rgb) -> impl Fn(Status) -> BoxStyle + use<> {
    let t = cx.t.clone();
    let over = cx.over;
    move |st| {
        let p = &t.p;
        if active {
            BoxStyle::default()
                .bg(c(p.accent))
                .fg(c(p.accent_ink))
                .radius(8.0)
        } else if st == Status::Hover {
            BoxStyle::default()
                .bg(c(t.wash(0.06, over)))
                .fg(c(p.text))
                .radius(8.0)
        } else {
            BoxStyle::default().fg(c(rest)).radius(8.0)
        }
    }
}

impl<'a, M: Clone + 'a> From<Pagination<'a, M>> for El<'a, M> {
    fn from(pg: Pagination<'a, M>) -> Self {
        if pg.count <= 1 {
            return Space::new().into();
        }
        let cx = &pg.cx;
        let p = cx.p();
        let ring = Ring::plain(cx);
        let arrow = |ico: Ico, target: Option<usize>| -> El<'a, M> {
            let disabled = target.is_none();
            let s = if disabled {
                // `disabled:opacity-40 disabled:hover:bg-transparent`.
                let col = c(bato_theme::blend(p.muted, 0.4, cx.over));
                surface(icon(ico, 15.0).color(col))
                    .fixed(BoxStyle::default().radius(8.0))
                    .disabled(true)
            } else {
                surface(icon(ico, 15.0))
                    .style(btn_style(cx, false, p.muted))
                    .on_press_maybe(target.map(|t| (pg.on_change)(t)))
            };
            s.width(32).height(32).center().focus_ring(ring).into()
        };
        let mut items: Vec<El<'a, M>> = Vec::new();
        items.push(arrow(Ico::ChevronLeft, (pg.page > 1).then(|| pg.page - 1)));
        for it in page_list(pg.page, pg.count, pg.siblings) {
            items.push(match it {
                PageItem::Gap => surface(icon(Ico::MoreHorizontal, 14.0).color(c(p.muted)))
                    .width(32)
                    .height(32)
                    .center()
                    .into(),
                PageItem::Page(n) => {
                    let active = n == pg.page;
                    let mut l = label(n.to_string(), 14.0, 20.0);
                    if active {
                        l = l.medium();
                    }
                    // `min-w-8 px-2`: at least 32, else the text + 16.
                    let w = (l.shape(f32::INFINITY).width() + 16.0).max(32.0);
                    surface(l)
                        .width(w)
                        .height(32)
                        .center()
                        .style(btn_style(cx, active, p.subtext))
                        .on_press((pg.on_change)(n))
                        .focus_ring(ring)
                        .into()
                }
            });
        }
        items.push(arrow(
            Ico::ChevronRight,
            (pg.page < pg.count).then(|| pg.page + 1),
        ));
        row(items).spacing(4).align_y(Alignment::Center).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PageItem::{Gap, Page};

    #[test]
    fn page_lists_match_react() {
        assert_eq!(
            page_list(3, 12, 1),
            [Page(1), Page(2), Page(3), Page(4), Gap, Page(12)]
        );
        assert_eq!(page_list(1, 4, 1), [Page(1), Page(2), Gap, Page(4)]);
        assert_eq!(
            page_list(6, 20, 2),
            [
                Page(1),
                Gap,
                Page(4),
                Page(5),
                Page(6),
                Page(7),
                Page(8),
                Gap,
                Page(20)
            ]
        );
        assert_eq!(page_list(12, 12, 1), [Page(1), Gap, Page(11), Page(12)]);
        assert_eq!(page_list(1, 2, 1), [Page(1), Page(2)]);
        assert_eq!(page_list(1, 1, 1), [Page(1)]);
        assert_eq!(page_list(2, 3, 1), [Page(1), Page(2), Page(3)]);
    }
}
