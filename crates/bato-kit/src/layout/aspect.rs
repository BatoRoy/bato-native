//! `aspect-video` / `aspect-[w/h]`: full width, height = width / ratio.

use iced::advanced::layout;
use iced::advanced::widget::Widget;
use iced::{Element, Length, Size};

use super::{forward_cells, into_element};

pub struct AspectRatio<'a, Message> {
    cells: Vec<Element<'a, Message, iced::Theme, iced::Renderer>>,
    ratio: f32,
}

/// `content` filling a box of `ratio` = width / height (16:9 → 16.0 / 9.0).
pub fn aspect_ratio<'a, Message>(
    ratio: f32,
    content: impl Into<Element<'a, Message, iced::Theme, iced::Renderer>>,
) -> AspectRatio<'a, Message> {
    AspectRatio {
        cells: vec![content.into()],
        ratio: ratio.max(1e-3),
    }
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for AspectRatio<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }
    fn layout(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let w = limits.max().width;
        let h = (w / self.ratio).round();
        let n = self.cells[0].as_widget_mut().layout(
            &mut tree.children[0],
            renderer,
            &layout::Limits::new(Size::new(w, h), Size::new(w, h)),
        );
        layout::Node::with_children(Size::new(w, h), vec![n])
    }

    forward_cells!();
}

into_element!(AspectRatio);
