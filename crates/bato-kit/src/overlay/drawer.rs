//! `Drawer` (`overlay/Drawer.tsx`): a panel against one window edge.
//!
//! A [`Kind::Drawer`] level of the overlay stack: scrim `#000@.6` with the
//! backdrop blur (fadeIn 150 ms), the panel sliding in from its edge
//! (200 ms ease-out, `animate-drawer-*`), Esc / a press on the scrim close
//! it, Tab is trapped inside, focus returns to the opener on close.
//!
//! The panel (`bg-app-card`, 1 px `lineStrong` on the inner edge, shadow
//! lg, `flex flex-col`): an optional header (`px-5 py-4`, 14 px semibold
//! title, a muted ✕ that turns `text` on hover, bottom border `app-border`)
//! over the body, which scrolls (`flex-1 overflow-y-auto p-5`; the padding
//! scrolls with the content). `size` is the width (left/right) or the height
//! (top/bottom), `20rem` by default.
//!
//! ```ignore
//! if self.drawer {
//!     levels.push(drawer(cx, body, Msg::CloseDrawer).title("Drawer").into_level());
//! }
//! ```

use iced::widget::{column, container, row};
use iced::{Alignment, Length, Padding};

use crate::El;
use crate::ctx::Ctx;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::layout::{hline, vline};
use crate::modal::{Kind, Modal, Side, level};
use crate::scroller::scroll_area;
use crate::style::{c, shadows};
use crate::surface::{BoxStyle, Status, surface};

/// Builder; [`Drawer::into_level`] makes the overlay level.
pub struct Drawer<'a, M> {
    cx: Ctx,
    body: El<'a, M>,
    on_close: M,
    side: Side,
    title: Option<String>,
    size: f32,
}

/// A right-side drawer, 320 px wide, without a header. Build `body` with
/// `cx.on_card()`.
pub fn drawer<'a, M: Clone + 'a>(
    cx: &Ctx,
    body: impl Into<El<'a, M>>,
    on_close: M,
) -> Drawer<'a, M> {
    Drawer {
        cx: cx.clone(),
        body: body.into(),
        on_close,
        side: Side::Right,
        title: None,
        size: 320.0,
    }
}

impl<'a, M: Clone + 'a> Drawer<'a, M> {
    pub fn side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }
    /// The header title (and its ✕).
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
    /// Width (left/right) or height (top/bottom), CSS px.
    pub fn size(mut self, px: f32) -> Self {
        self.size = px;
        self
    }

    /// The drawer as a level for [`crate::frame::Frame::view_with`].
    pub fn into_level(self) -> Modal<'a, M> {
        let side = self.side;
        let panel = panel(
            &self.cx,
            side,
            self.title.as_deref(),
            self.on_close.clone(),
            self.body,
        );
        level(Kind::Drawer(side, self.size), panel, self.on_close)
    }
}

/// The panel markup (see the module docs).
pub fn panel<'a, M: Clone + 'a>(
    cx: &Ctx,
    side: Side,
    title: Option<&str>,
    on_close: M,
    body: El<'a, M>,
) -> El<'a, M> {
    let on = cx.on_card();
    let p = on.p();
    let mut col: Vec<El<'a, M>> = Vec::new();
    if let Some(t) = title {
        let (muted, text) = (c(p.muted), c(p.text));
        let close = surface(icon(Ico::X, 16.0))
            .style(move |s| BoxStyle::default().fg(if s == Status::Hover { text } else { muted }))
            .transition_ms(150)
            .on_press(on_close);
        col.push(
            container(
                row![
                    label(t.to_string(), 14.0, 20.0)
                        .semibold()
                        .color(c(p.text))
                        .width(Length::Fill),
                    close
                ]
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([16.0, 20.0]))
            .into(),
        );
        col.push(hline(c(p.border)));
    }
    col.push(
        scroll_area(&on, container(body).padding(20).width(Length::Fill))
            .height(Length::Fill)
            .into(),
    );
    let edge = c(on.line_strong());
    let content: El<'a, M> = column(col).width(Length::Fill).height(Length::Fill).into();
    let framed: El<'a, M> = match side {
        Side::Right => row![vline(edge), content].into(),
        Side::Left => row![content, vline(edge)].into(),
        Side::Bottom => column![hline(edge), content].into(),
        Side::Top => column![content, hline(edge)].into(),
    };
    surface(framed)
        .width(Length::Fill)
        .height(Length::Fill)
        .fixed(BoxStyle {
            bg: Some(c(p.card)),
            shadows: shadows(&p.shadow_lg),
            ..BoxStyle::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_react() {
        let cx = Ctx::from_palette(bato_theme::palette(
            bato_theme::ThemeId::Dark,
            bato_theme::Rgb::new(0x8b, 0x5c, 0xf6),
        ));
        let d = drawer::<()>(&cx, iced::widget::Space::new(), ());
        assert_eq!(
            (d.side, d.size, d.title.is_none()),
            (Side::Right, 320.0, true)
        );
        for side in [Side::Left, Side::Right, Side::Top, Side::Bottom] {
            let l = drawer::<()>(&cx, iced::widget::Space::new(), ())
                .side(side)
                .size(240.0)
                .title("Drawer")
                .into_level();
            assert_eq!(l.kind(), Kind::Drawer(side, 240.0));
        }
    }
}
