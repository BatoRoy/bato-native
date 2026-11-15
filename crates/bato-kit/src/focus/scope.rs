//! Focus scopes: the Modal focus trap (`Tab` wraps inside the topmost
//! trapping scope and pulls escaped focus back in), focus on mount and
//! restore on unmount, as React's `Modal` does.
//!
//! ```ignore
//! // A dialog level: trap, focus its first control, give focus back on close.
//! modal(focus_scope(panel), …)
//! // Only focus on mount (Combobox search, palette input):
//! autofocus(input(&cx, "Search…", &q, Msg::Q))
//! // Start on a given control instead of the first one:
//! focus_scope(form).initial("email")
//! ```
//!
//! Each scope's state is the restore token: when the scope leaves the tree
//! (its dialog closes) the state is dropped and asks the root to focus the
//! widget that had focus when it opened. A nested scope (a confirm dialog over
//! a dialog) restores into its parent, which is still trapping.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Id, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};

use super::{Request, ScopeEnd, Target, next_token, push};

pub(crate) struct ScopeState {
    pub key: u64,
    pub trap: bool,
    restore: bool,
    mounted: bool,
    /// What had focus when the scope took it.
    pub opener: Option<Target>,
}

impl Drop for ScopeState {
    fn drop(&mut self) {
        if self.restore
            && let Some(t) = self.opener.take()
        {
            push(Request::Restore(t));
        }
    }
}

pub struct FocusScope<'a, M> {
    content: Element<'a, M, iced::Theme, iced::Renderer>,
    id: Option<Id>,
    trap: bool,
    autofocus: bool,
    restore: bool,
    initial: Option<Id>,
    select: Option<(usize, usize)>,
}

/// A dialog level: traps Tab, focuses its first tab stop on mount, restores
/// focus to the opener when it goes away.
pub fn focus_scope<'a, M>(
    content: impl Into<Element<'a, M, iced::Theme, iced::Renderer>>,
) -> FocusScope<'a, M> {
    FocusScope {
        content: content.into(),
        id: None,
        trap: true,
        autofocus: true,
        restore: true,
        initial: None,
        select: None,
    }
}

/// Focus the first tab stop of `content` when it appears (React `autoFocus`);
/// no trap, no restore.
pub fn autofocus<'a, M>(
    content: impl Into<Element<'a, M, iced::Theme, iced::Renderer>>,
) -> FocusScope<'a, M> {
    focus_scope(content).trap(false).restore(false)
}

impl<'a, M> FocusScope<'a, M> {
    /// Confine Tab to this scope while it is the topmost trap (default on).
    pub fn trap(mut self, on: bool) -> Self {
        self.trap = on;
        self
    }
    /// Focus a control on mount (default on).
    pub fn autofocus(mut self, on: bool) -> Self {
        self.autofocus = on;
        self
    }
    /// Give focus back to the opener on unmount (default on).
    pub fn restore(mut self, on: bool) -> Self {
        self.restore = on;
        self
    }
    /// Focus this widget on mount instead of the first tab stop.
    pub fn initial(mut self, id: impl Into<Id>) -> Self {
        self.initial = Some(id.into());
        self
    }
    /// [`FocusScope::initial`] if `Some`.
    pub fn initial_maybe(self, id: Option<Id>) -> Self {
        match id {
            Some(id) => self.initial(id),
            None => self,
        }
    }
    /// After focusing the initial text input, select these cursor
    /// positions of it (graphemes; [`super::grapheme_range`]).
    pub fn select(mut self, range: Option<(usize, usize)>) -> Self {
        self.select = range;
        self
    }
    /// Reported to operations as a container id.
    pub fn id(mut self, id: impl Into<Id>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl<'a, M> Widget<M, iced::Theme, iced::Renderer> for FocusScope<'a, M> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<ScopeState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(ScopeState {
            key: next_token(),
            trap: self.trap,
            restore: self.restore,
            mounted: false,
            opener: None,
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        let s = tree.state.downcast_mut::<ScopeState>();
        s.trap = self.trap;
        s.restore = self.restore;
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
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        let s = tree.state.downcast_mut::<ScopeState>();
        if !s.mounted {
            s.mounted = true;
            if self.autofocus {
                push(Request::Autofocus {
                    scope: s.key,
                    initial: self.initial.clone(),
                    select: self.select,
                });
                shell.request_redraw();
            }
        }
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let b = layout.bounds();
        let s = tree.state.downcast_mut::<ScopeState>();
        operation.container(self.id.as_ref(), b);
        operation.custom(self.id.as_ref(), b, s);
        let child = &mut tree.children[0];
        let content = &mut self.content;
        operation.traverse(&mut |op| {
            content
                .as_widget_mut()
                .operate(child, layout.children().next().unwrap(), renderer, op);
        });
        operation.custom(None, b, &mut ScopeEnd);
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

impl<'a, M: 'a> From<FocusScope<'a, M>> for Element<'a, M, iced::Theme, iced::Renderer> {
    fn from(s: FocusScope<'a, M>) -> Self {
        Element::new(s)
    }
}
