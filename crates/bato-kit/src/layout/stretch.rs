//! A column as wide as its widest child, with every child stretched to that
//! width (`flex-col items-stretch`, shrink-wrapped; the toast stack).

use iced::advanced::layout;
use iced::advanced::widget::Widget;
use iced::{Element, Length, Point, Size};

use super::{forward_cells, into_element};

pub struct StretchColumn<'a, Message> {
    cells: Vec<Element<'a, Message, iced::Theme, iced::Renderer>>,
    gap: f32,
    max_width: f32,
}

pub fn stretch_column<'a, Message>(
    cells: Vec<Element<'a, Message, iced::Theme, iced::Renderer>>,
    gap: f32,
    max_width: f32,
) -> StretchColumn<'a, Message> {
    StretchColumn { cells, gap, max_width }
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for StretchColumn<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }
    fn layout(&mut self, tree: &mut iced::advanced::widget::Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        let maxw = limits.max().width.min(self.max_width);
        let mut w: f32 = 0.0;
        for (cell, t) in self.cells.iter_mut().zip(tree.children.iter_mut()) {
            let n = cell.as_widget_mut().layout(t, renderer, &layout::Limits::new(Size::ZERO, Size::new(maxw, f32::INFINITY)));
            w = w.max(n.size().width);
        }
        let mut y = 0.0;
        let mut nodes = Vec::new();
        for (cell, t) in self.cells.iter_mut().zip(tree.children.iter_mut()) {
            let n = cell.as_widget_mut().layout(t, renderer, &layout::Limits::new(Size::new(w, 0.0), Size::new(w, f32::INFINITY)));
            let h = n.size().height;
            nodes.push(n.move_to(Point::new(0.0, y)));
            y += h + self.gap;
        }
        layout::Node::with_children(Size::new(w, (y - self.gap).max(0.0)), nodes)
    }

    forward_cells!();
}

into_element!(StretchColumn);
