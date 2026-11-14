//! `FocusOut`: publish a message when keyboard or programmatic focus leaves
//! a subtree it was in (`focusout` with the new target outside). Wrap an
//! anchored panel in it and dismiss with
//! [`DismissReason::FocusOut`](crate::overlay::anchored::DismissReason):
//!
//! ```ignore
//! anchored(trigger, open.then(|| on_focus_out(panel, Msg::Dismiss(DismissReason::FocusOut)).into()))
//! ```
//!
//! Pointer presses outside are the panel's own outside-press dismissal.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};

use super::{FocusedBounds, revision};

pub struct FocusOut<'a, M> {
    content: Element<'a, M, iced::Theme, iced::Renderer>,
    message: M,
}

/// Publish `message` when focus moves (by key or program) from inside
/// `content` to outside it.
pub fn on_focus_out<'a, M>(
    content: impl Into<Element<'a, M, iced::Theme, iced::Renderer>>,
    message: M,
) -> FocusOut<'a, M> {
    FocusOut {
        content: content.into(),
        message,
    }
}

#[derive(Default)]
struct State {
    seen: u64,
    inside: bool,
}

impl<'a, M: Clone> Widget<M, iced::Theme, iced::Renderer> for FocusOut<'a, M> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State {
            seen: revision(),
            inside: false,
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let n = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        layout::Node::with_children(n.size(), vec![n])
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
        let child = layout.children().next().unwrap();
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            child,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        let rev = revision();
        let st = tree.state.downcast_mut::<State>();
        let check = st.seen != rev || super::is_pointer_press(event);
        if !check {
            return;
        }
        let moved = st.seen != rev;
        st.seen = rev;
        let mut op = FocusedBounds::default();
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], child, renderer, &mut op);
        let st = tree.state.downcast_mut::<State>();
        let now = op.0.is_some();
        if st.inside && !now && moved {
            shell.publish(self.message.clone());
        }
        st.inside = now;
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
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
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
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
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: Clone + 'a> From<FocusOut<'a, M>> for Element<'a, M, iced::Theme, iced::Renderer> {
    fn from(w: FocusOut<'a, M>) -> Self {
        Element::new(w)
    }
}
