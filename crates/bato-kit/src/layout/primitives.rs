//! `Primitives.tsx`: Stack / HStack / VStack, Center, Container, Divider,
//! Spacer. Gaps are Tailwind units (1 = 4 px) as in React.
//!
//! | React | Kit |
//! |---|---|
//! | `<HStack gap={2}>` / `<VStack>` / `<Stack direction="row" align="center">` | [`hstack`] / [`vstack`] (iced `Row`/`Column`: `.align_y(..)`, `.wrap()`) |
//! | `<Center>` | [`center`] |
//! | `<Container size="sm" padded={false}>` | [`sized_container`] |
//! | `<Divider />`, `orientation="vertical"`, `label="…"` | [`divider`], [`divider_v`], [`divider_label`] |
//! | `<Spacer />` | [`spacer`] |
//! | `<Grid cols={4} gap={2}>`, `<AspectRatio>` | [`super::grid`], [`super::aspect_ratio`] |

use iced::widget::{Column, Row, Space, container, row};
use iced::{Alignment, Element, Length};

use super::{hline, vline};
use crate::ctx::Ctx;
use crate::label::label;
use crate::style::c;

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// Tailwind spacing units to px (`gap={2}` → 8).
pub fn units(n: f32) -> f32 {
    n * 4.0
}

/// `<HStack gap>`: a row with `gap` units between children.
pub fn hstack<'a, M: 'a>(
    gap: f32,
    children: Vec<El<'a, M>>,
) -> Row<'a, M, iced::Theme, iced::Renderer> {
    Row::with_children(children).spacing(units(gap))
}

/// `<VStack gap>`: a column with `gap` units between children.
pub fn vstack<'a, M: 'a>(
    gap: f32,
    children: Vec<El<'a, M>>,
) -> Column<'a, M, iced::Theme, iced::Renderer> {
    Column::with_children(children).spacing(units(gap))
}

/// `<Center>`: `grid place-items-center` (fills the space it is given).
pub fn center<'a, M: 'a>(content: impl Into<El<'a, M>>) -> El<'a, M> {
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .into()
}

/// `<Spacer />`: `flex-1`.
pub fn spacer<'a, M: 'a>() -> El<'a, M> {
    Space::new().width(Length::Fill).height(Length::Fill).into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContainerSize {
    /// 24rem
    Sm,
    /// 42rem
    Md,
    /// 64rem
    #[default]
    Lg,
    /// 80rem
    Xl,
    Full,
}

impl ContainerSize {
    pub fn max_width(self) -> f32 {
        match self {
            ContainerSize::Sm => 384.0,
            ContainerSize::Md => 672.0,
            ContainerSize::Lg => 1024.0,
            ContainerSize::Xl => 1280.0,
            ContainerSize::Full => f32::INFINITY,
        }
    }
}

/// `<Container size padded>`: `mx-auto w-full`, at most the size's width,
/// `px-6` when padded.
pub fn sized_container<'a, M: 'a>(
    size: ContainerSize,
    padded: bool,
    content: impl Into<El<'a, M>>,
) -> El<'a, M> {
    let inner = container(content)
        .width(Length::Fill)
        .max_width(size.max_width())
        .padding([0.0, if padded { 24.0 } else { 0.0 }]);
    container(inner)
        .width(Length::Fill)
        .align_x(Alignment::Center)
        .into()
}

/// `<Divider />`: a full-width 1 px `bg-app-border` line.
pub fn divider<'a, M: Clone + 'a>(cx: &Ctx) -> El<'a, M> {
    hline(c(cx.p().border))
}

/// `<Divider orientation="vertical" />`: `w-px self-stretch`, as tall as
/// the row it is in.
pub fn divider_v<'a, M: Clone + 'a>(cx: &Ctx) -> El<'a, M> {
    vline(c(cx.p().border))
}

/// `<Divider label>`: lines either side of a 12/16 muted label, `gap-3`.
pub fn divider_label<'a, M: Clone + 'a>(cx: &Ctx, text: impl Into<String>) -> El<'a, M> {
    let p = cx.p();
    row![
        hline(c(p.border)),
        label(text, 12.0, 16.0).color(c(p.muted)),
        hline(c(p.border)),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .width(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_sizes_and_units() {
        assert_eq!(ContainerSize::Sm.max_width(), 384.0);
        assert_eq!(ContainerSize::default(), ContainerSize::Lg);
        assert_eq!(units(3.0), 12.0);
    }
}
