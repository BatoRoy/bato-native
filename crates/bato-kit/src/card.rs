//! `Card` (§4.9): `bg-app-card`, 1 px line, radius 16, shadow md, padding 20,
//! an optional header (icon tile, title, action on the right).
//!
//! Build the body with `cx.on_card()`, since it sits on the card colour.
//!
//! ```ignore
//! card(&cx, "Backend").icon(Ico::Server).action(status_badge).body(body)
//! card_box(&cx, content)               // the box alone (Scroll lab text card)
//! ```

use iced::widget::{Column, Space, column, row};
use iced::{Alignment, Length};

use crate::El;
use crate::ctx::Ctx;
use crate::icon::{Icon, icon};
use crate::label::label;
use crate::layout::vgap;
use crate::style::{c, inset_of, shadows};
use crate::surface::{BoxStyle, Surface, surface};

/// The card box style on `cx`'s theme.
pub fn card_style(cx: &Ctx) -> BoxStyle {
    let p = cx.p();
    BoxStyle {
        bg: Some(c(p.card)),
        border: Some(c(cx.t.line_on(p.card))),
        radius: 16.0,
        shadows: shadows(&p.shadow_md),
        inset_top: inset_of(&p.shadow_md, p.card),
        ..BoxStyle::default()
    }
}

/// The card box around `content` (padding 20 + the 1 px border), full width.
pub fn card_box<'a, M: Clone + 'a>(cx: &Ctx, content: impl Into<El<'a, M>>) -> Surface<'a, M> {
    surface(content)
        .padding(21)
        .width(Length::Fill)
        .fixed(card_style(cx))
}

pub struct Card<'a, M> {
    cx: Ctx,
    title: String,
    icon: Option<Icon>,
    action: Option<El<'a, M>>,
    body: Option<El<'a, M>>,
}

pub fn card<'a, M: Clone + 'a>(cx: &Ctx, title: impl Into<String>) -> Card<'a, M> {
    Card {
        cx: cx.clone(),
        title: title.into(),
        icon: None,
        action: None,
        body: None,
    }
}

impl<'a, M: Clone + 'a> Card<'a, M> {
    /// The 28 px icon tile before the title.
    pub fn icon(mut self, ico: impl Into<Icon>) -> Self {
        self.icon = Some(ico.into());
        self
    }
    /// Content on the right of the title row.
    pub fn action(mut self, el: impl Into<El<'a, M>>) -> Self {
        self.action = Some(el.into());
        self
    }
    pub fn body(mut self, el: impl Into<El<'a, M>>) -> Self {
        self.body = Some(el.into());
        self
    }

    /// Header + body without the box.
    pub fn inner(self) -> Column<'a, M, iced::Theme, iced::Renderer> {
        let cx = self.cx.on_card();
        let p = cx.p();
        let mut left: Vec<El<'a, M>> = Vec::new();
        if let Some(i) = self.icon {
            let tile_bg = cx.wash(0.05);
            let tile = BoxStyle::default()
                .bg(c(tile_bg))
                .inner_ring(c(cx.t.line_on(tile_bg)))
                .radius(8.0);
            left.push(
                surface(icon(i, 14.0).color(c(p.subtext)))
                    .width(28)
                    .height(28)
                    .center()
                    .fixed(tile)
                    .into(),
            );
        }
        if !self.title.is_empty() {
            left.push(
                label(self.title, 14.0, 20.0)
                    .semibold()
                    .color(c(p.text))
                    .ellipsis()
                    .into(),
            );
        }
        let mut col = column![];
        if !left.is_empty() || self.action.is_some() {
            let mut header: Vec<El<'a, M>> =
                vec![row(left).spacing(10).align_y(Alignment::Center).into()];
            header.push(Space::new().width(Length::Fill).into());
            if let Some(a) = self.action {
                header.push(a);
            }
            col = col
                .push(row(header).spacing(12).align_y(Alignment::Center))
                .push(vgap(16.0));
        }
        if let Some(b) = self.body {
            col = col.push(b);
        }
        col
    }
}

impl<'a, M: Clone + 'a> From<Card<'a, M>> for El<'a, M> {
    fn from(card: Card<'a, M>) -> Self {
        let cx = card.cx.clone();
        card_box(&cx, card.inner()).into()
    }
}
