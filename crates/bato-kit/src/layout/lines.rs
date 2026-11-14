//! Hairlines, fixed gaps and the page container.

use iced::widget::{Space, container};
use iced::{Alignment, Color, Element, Length, Padding};

use crate::surface::{BoxStyle, surface};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// A fixed horizontal gap (`w-N` spacer in a row).
pub fn gap<'a, M: 'a>(w: f32) -> El<'a, M> {
    Space::new().width(w).into()
}

/// A fixed vertical gap (`mt-N` between column items).
pub fn vgap<'a, M: 'a>(h: f32) -> El<'a, M> {
    Space::new().height(h).into()
}

/// A full-width 1 px line (`border-t`), `color` composited by the caller
/// (`cx.line()`).
pub fn hline<'a, M: Clone + 'a>(color: Color) -> El<'a, M> {
    surface(Space::new().width(Length::Fill).height(1)).width(Length::Fill).height(1).fixed(BoxStyle::default().bg(color)).into()
}

/// A full-height 1 px line (`border-l`).
pub fn vline<'a, M: Clone + 'a>(color: Color) -> El<'a, M> {
    surface(Space::new().width(1).height(Length::Fill)).width(1).height(Length::Fill).fixed(BoxStyle::default().bg(color)).into()
}

/// The page container: `padding`, and unless `wide`, centred at
/// `--app-content-width` (896 px). Home and Scroll lab use `[48, 32]`
/// (`py-12 px-8`), the Showcase `24` (`p-6`).
pub fn page<'a, M: 'a>(content: impl Into<El<'a, M>>, wide: bool, padding: impl Into<Padding>) -> El<'a, M> {
    let inner = container(content).padding(padding).width(Length::Fill);
    let inner = if wide { inner } else { inner.max_width(bato_theme::CONTENT_WIDTH) };
    container(inner).width(Length::Fill).align_x(Alignment::Center).into()
}
