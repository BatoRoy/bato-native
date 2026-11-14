//! `rounded_clip`: `rounded-* overflow-hidden` (+ an optional 1 px
//! `border`) around content that paints its own backgrounds (panels, an
//! app shell, a disclosure's hover fill).
//!
//! iced clips to rectangles only, so the content is clipped to the box and
//! then the corners outside the radius are painted over in the backdrop
//! colour (a band around the box whose inner edge is the rounded shape),
//! and finally the border, on top like CSS draws it.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::{Border, Color, Element, Event, Length, Padding, Rectangle, Size, Vector, mouse};

use super::into_element;
use crate::anim::fade;
use crate::scale::snap_rect;

pub struct RoundedClip<'a, Message> {
    cells: Vec<Element<'a, Message, iced::Theme, iced::Renderer>>,
    radius: f32,
    backdrop: Color,
    border: Option<Color>,
    width: Length,
    height: Length,
}

/// `content` clipped to a box with corner `radius`, on `backdrop`.
pub fn rounded_clip<'a, Message>(
    radius: f32,
    backdrop: Color,
    content: impl Into<Element<'a, Message, iced::Theme, iced::Renderer>>,
) -> RoundedClip<'a, Message> {
    RoundedClip {
        cells: vec![content.into()],
        radius,
        backdrop,
        border: None,
        width: Length::Fill,
        height: Length::Shrink,
    }
}

impl<'a, Message> RoundedClip<'a, Message> {
    /// A 1 px border (inside the box; the content is inset by it).
    pub fn border(mut self, c: Color) -> Self {
        self.border = Some(c);
        self
    }
    pub fn width(mut self, w: impl Into<Length>) -> Self {
        self.width = w.into();
        self
    }
    pub fn height(mut self, h: impl Into<Length>) -> Self {
        self.height = h.into();
        self
    }
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for RoundedClip<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let pad = if self.border.is_some() { 1.0 } else { 0.0 };
        let content = &mut self.cells[0];
        let child_tree = &mut tree.children[0];
        layout::padded(limits, self.width, self.height, Padding::from(pad), |l| {
            content.as_widget_mut().layout(child_tree, renderer, l)
        })
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.cells[0])]
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
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let clip = layout.bounds().intersection(viewport).unwrap_or_default();
        self.cells[0].as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            &clip,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.cells[0].as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        )
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.cells[0].as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.cells[0].as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
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
        let b = snap_rect(layout.bounds());
        let Some(clip) = b.intersection(viewport) else {
            return;
        };
        let child = layout.children().next().unwrap();
        renderer.with_layer(clip, |renderer| {
            self.cells[0].as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                child,
                cursor,
                &clip,
            );
            let k = self.radius.max(1.0) + 2.0;
            renderer.fill_quad(
                Quad {
                    bounds: b.expand(k),
                    border: Border {
                        radius: (self.radius + k).into(),
                        width: k,
                        color: fade(self.backdrop),
                    },
                    ..Quad::default()
                },
                Color::TRANSPARENT,
            );
            if let Some(bc) = self.border {
                renderer.fill_quad(
                    Quad {
                        bounds: b,
                        border: Border {
                            radius: self.radius.into(),
                            width: 1.0,
                            color: fade(bc),
                        },
                        ..Quad::default()
                    },
                    Color::TRANSPARENT,
                );
            }
        });
    }
}

into_element!(RoundedClip);
