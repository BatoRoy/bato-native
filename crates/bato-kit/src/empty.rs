//! `EmptyState` (`Feedback.tsx`): centred column, `py-12 px-6`; an optional
//! 48 px tile (radius 16, `text@.05`, inset line ring) with a 22 px muted
//! icon; 14 px medium title; 12 px muted subtitle (max 320 px, wrapped); an
//! optional action 16 px below.
//!
//! ```ignore
//! empty_state(&cx, "No results").icon(Ico::Search).subtitle("Try another filter.")
//! ```

use iced::widget::{column, container};
use iced::{Alignment, Length, Padding};

use crate::El;
use crate::ctx::Ctx;
use crate::icon::{Ico, icon};
use crate::label::{Align, label};
use crate::layout::vgap;
use crate::style::c;
use crate::surface::{BoxStyle, surface};

pub struct EmptyState<'a, M> {
    cx: Ctx,
    title: String,
    icon: Option<Ico>,
    subtitle: Option<String>,
    action: Option<El<'a, M>>,
}

pub fn empty_state<'a, M: Clone + 'a>(cx: &Ctx, title: impl Into<String>) -> EmptyState<'a, M> {
    EmptyState {
        cx: cx.clone(),
        title: title.into(),
        icon: None,
        subtitle: None,
        action: None,
    }
}

impl<'a, M: Clone + 'a> EmptyState<'a, M> {
    pub fn icon(mut self, ico: Ico) -> Self {
        self.icon = Some(ico);
        self
    }
    pub fn subtitle(mut self, text: impl Into<String>) -> Self {
        self.subtitle = Some(text.into());
        self
    }
    pub fn action(mut self, el: impl Into<El<'a, M>>) -> Self {
        self.action = Some(el.into());
        self
    }
}

impl<'a, M: Clone + 'a> From<EmptyState<'a, M>> for El<'a, M> {
    fn from(e: EmptyState<'a, M>) -> Self {
        let cx = &e.cx;
        let p = cx.p();
        let mut col = column![].align_x(Alignment::Center).width(Length::Fill);
        if let Some(i) = e.icon {
            let bg = cx.wash(0.05);
            let tile = BoxStyle::default()
                .bg(c(bg))
                .inner_ring(c(cx.t.line_on(bg)))
                .radius(16.0);
            col = col
                .push(
                    surface(icon(i, 22.0).color(c(p.muted)))
                        .width(48)
                        .height(48)
                        .center()
                        .fixed(tile),
                )
                .push(vgap(12.0));
        }
        col = col.push(
            label(e.title, 14.0, 20.0)
                .medium()
                .color(c(p.text))
                .align(Align::Center),
        );
        if let Some(s) = e.subtitle {
            col = col.push(vgap(4.0)).push(
                label(s, 12.0, 16.0)
                    .color(c(p.muted))
                    .align(Align::Center)
                    .wrap()
                    .max_width(320.0),
            );
        }
        if let Some(a) = e.action {
            col = col.push(vgap(16.0)).push(a);
        }
        container(col)
            .width(Length::Fill)
            .padding(Padding::from([48.0, 24.0]))
            .into()
    }
}
