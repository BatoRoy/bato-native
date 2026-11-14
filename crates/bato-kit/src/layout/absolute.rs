//! Absolute placement (`position: absolute` with `left/top/width/height` in
//! px): calendar events, timeline bars, dashboard widgets. The box is as wide
//! as offered and `height` tall (or as tall as its lowest child).

use iced::advanced::layout;
use iced::advanced::widget::Widget;
use iced::{Element, Length, Point, Rectangle, Size};

use super::{forward_cells, into_element};

pub struct Absolute<'a, Message> {
    cells: Vec<Element<'a, Message, iced::Theme, iced::Renderer>>,
    rects: Vec<Rectangle>,
    height: Option<f32>,
}

/// An empty absolute box; `height: None` = as tall as the lowest child.
pub fn absolute<'a, Message>(height: Option<f32>) -> Absolute<'a, Message> {
    Absolute { cells: Vec::new(), rects: Vec::new(), height }
}

impl<'a, Message> Absolute<'a, Message> {
    /// A child at `rect` (relative to the box's top-left), laid out at
    /// exactly that size.
    pub fn push(mut self, rect: Rectangle, child: impl Into<Element<'a, Message, iced::Theme, iced::Renderer>>) -> Self {
        self.cells.push(child.into());
        self.rects.push(rect);
        self
    }
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for Absolute<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, self.height.map_or(Length::Shrink, Length::Fixed))
    }
    fn layout(&mut self, tree: &mut iced::advanced::widget::Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        let w = limits.max().width;
        let mut bottom: f32 = 0.0;
        let nodes = self
            .cells
            .iter_mut()
            .zip(tree.children.iter_mut())
            .zip(&self.rects)
            .map(|((cell, t), r)| {
                bottom = bottom.max(r.y + r.height);
                let size = Size::new(r.width, r.height);
                cell.as_widget_mut().layout(t, renderer, &layout::Limits::new(size, size)).move_to(Point::new(r.x, r.y))
            })
            .collect();
        layout::Node::with_children(Size::new(w, self.height.unwrap_or(bottom)), nodes)
    }

    forward_cells!();
}

into_element!(Absolute);
