//! `Breadcrumbs`: a 14/20 trail with `ChevronRight` (14, muted) separators,
//! `gap-1.5`. Crumbs with an action are muted buttons that turn `text` on
//! hover (`transition-colors`); the last crumb is the current page
//! (`text` medium) and, like a crumb without an action, a disabled button:
//! no hover, not focusable.
//!
//! ```ignore
//! breadcrumbs(&cx, [Crumb::new("Home").on_press(Msg::Home), Crumb::new("Item")])
//! ```

use iced::Alignment;
use iced::widget::row;

use crate::El;
use crate::ctx::Ctx;
use crate::cursor::Cursor;
use crate::focus::Ring;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

#[derive(Debug, Clone)]
pub struct Crumb<M> {
    pub label: String,
    pub on_press: Option<M>,
}

impl<M> Crumb<M> {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            on_press: None,
        }
    }
    pub fn on_press(mut self, m: M) -> Self {
        self.on_press = Some(m);
        self
    }
}

pub fn breadcrumbs<'a, M: Clone + 'a>(
    cx: &Ctx,
    items: impl IntoIterator<Item = Crumb<M>>,
) -> El<'a, M> {
    let p = cx.p();
    let (muted, text) = (c(p.muted), c(p.text));
    let items: Vec<Crumb<M>> = items.into_iter().collect();
    let n = items.len();
    let mut out: Vec<El<'a, M>> = Vec::with_capacity(n * 2);
    for (i, item) in items.into_iter().enumerate() {
        let last = i + 1 == n;
        let enabled = !last && item.on_press.is_some();
        let l = label(item.label, 14.0, 20.0);
        let crumb = if last {
            surface(l.medium().color(text)).into()
        } else {
            surface(l)
                .on_press_maybe(item.on_press)
                .disabled(!enabled)
                .cursor(if enabled {
                    Cursor::Pointer
                } else {
                    Cursor::Default
                })
                .style(move |s| {
                    BoxStyle::default().fg(if s == Status::Hover && enabled {
                        text
                    } else {
                        muted
                    })
                })
                .focus_ring(Ring::plain(cx))
                .into()
        };
        out.push(crumb);
        if !last {
            out.push(icon(Ico::ChevronRight, 14.0).color(muted).into());
        }
    }
    row(out).spacing(6).align_y(Alignment::Center).into()
}
