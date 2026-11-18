//! `flex flex-col` with a `flex-1` body (`mt-auto` on the last child):
//! `top` at the top, `bottom` pushed to the end of whatever height the
//! parent settles on, e.g. a card's button row level with its neighbours'
//! in a stretched [`Grid`](super::Grid) row.
//!
//! The grid lays a cell out twice, first with an unbounded height to
//! measure the row, then with the row's height as the maximum, and a card's
//! surface passes that on (less its padding). So a finite maximum height is
//! this widget's own height and an unbounded one means "as tall as the
//! content". A `Length::Fill` spacer can't do that: it would make the
//! measuring pass infinite.
//!
//! ```ignore
//! card(cx, "Plugin").body(fill_end(column![title, blurb], row![install], 16.0))
//! ```

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell, overlay, renderer};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, mouse};

use crate::El;

pub struct FillEnd<'a, M> {
    cells: [El<'a, M>; 2],
    gap: f32,
}

/// `top`, at least `gap`, then `bottom` at the end.
pub fn fill_end<'a, M>(
    top: impl Into<El<'a, M>>,
    bottom: impl Into<El<'a, M>>,
    gap: f32,
) -> FillEnd<'a, M> {
    FillEnd {
        cells: [top.into(), bottom.into()],
        gap,
    }
}

/// The height and the bottom cell's y for natural heights `top`, `bottom`
/// in a parent offering at most `max`.
fn arrange(top: f32, bottom: f32, gap: f32, max: f32) -> (f32, f32) {
    let natural = top + gap + bottom;
    let h = if max.is_finite() {
        natural.max(max)
    } else {
        natural
    };
    (h, h - bottom)
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for FillEnd<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let w = limits.max().width;
        let loose = layout::Limits::new(Size::ZERO, Size::new(w, f32::INFINITY));
        let [top, bottom] = &mut self.cells;
        let t = top
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &loose);
        let b = bottom
            .as_widget_mut()
            .layout(&mut tree.children[1], renderer, &loose);
        let (h, by) = arrange(
            t.size().height,
            b.size().height,
            self.gap,
            limits.max().height,
        );
        layout::Node::with_children(Size::new(w, h), vec![t, b.move_to(Point::new(0.0, by))])
    }

    fn children(&self) -> Vec<Tree> {
        self.cells.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.cells);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        for ((cell, t), l) in self
            .cells
            .iter_mut()
            .zip(tree.children.iter_mut())
            .zip(layout.children())
        {
            cell.as_widget_mut()
                .update(t, event, l, cursor, renderer, clipboard, shell, viewport);
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.cells
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((c, t), l)| {
                c.as_widget()
                    .mouse_interaction(t, l, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default()
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        operation.traverse(&mut |operation| {
            for ((cell, t), l) in self
                .cells
                .iter_mut()
                .zip(tree.children.iter_mut())
                .zip(layout.children())
            {
                cell.as_widget_mut().operate(t, l, renderer, operation);
            }
        });
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        for ((cell, t), l) in self.cells.iter().zip(&tree.children).zip(layout.children()) {
            cell.as_widget()
                .draw(t, renderer, theme, style, l, cursor, viewport);
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        overlay::from_children(
            &mut self.cells,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: 'a> From<FillEnd<'a, M>> for El<'a, M> {
    fn from(w: FillEnd<'a, M>) -> Self {
        Element::new(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_naturally_then_pins_the_bottom() {
        // The grid's measuring pass: unbounded.
        assert_eq!(arrange(60.0, 36.0, 16.0, f32::INFINITY), (112.0, 76.0));
        // The row is taller (a neighbour's blurb wraps): the button moves down.
        assert_eq!(arrange(60.0, 36.0, 16.0, 132.0), (132.0, 96.0));
        // Never squeezed below the content.
        assert_eq!(arrange(60.0, 36.0, 16.0, 50.0), (112.0, 76.0));
    }
}
