//! Which scroller the scroll keys go to (the rules are in [`super::chain`]'s
//! keyboard section): the focused widget's scrollers ([`FocusChain`]), and
//! widgets that handle the keys themselves ([`key_owner`], [`own_keys`]).
//!
//! In Chromium a page's own key handler (a file list moving its cursor on
//! ↑/↓, a `window` `keydown` listener) runs before the browser scrolls and
//! `preventDefault`s it, wherever the focus or the last click is. A kit
//! scroller acts during the widget walk, possibly before such a widget
//! (an earlier sidebar), so the widget says up front which keys it takes:
//!
//! ```ignore
//! // A list with its own cursor: no scroller of this window scrolls on
//! // ↑/↓/Home/End/Page Up/Page Down/Space while it is on screen; the keys
//! // reach `list`'s own `update` (or the app's key subscription) uncaptured.
//! scroller::key_owner(list)
//! scroller::key_owner(list).keys([Named::ArrowUp, Named::ArrowDown])
//! // …and handle them right there (skipped while a text field has focus):
//! scroller::key_owner(list).on_key(|key, mods| match key {
//!     Named::ArrowDown => Some(Msg::Cursor(1)),
//!     Named::ArrowUp => Some(Msg::Cursor(-1)),
//!     _ => None,
//! })
//! // A custom widget can do the same from its `draw`:
//! scroller::own_keys(scroller::chain::ALL_KEYS);
//! ```

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Operation, Tree, Widget, operation::Focusable};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::keyboard::{self, Modifiers, key::Named};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};

use super::chain::{self, KeySet};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// What a kit scroller hands every operation (`Operation::custom`) before
/// its [`super::ScrollFrame`]: its identity, so [`FocusChain`] knows which
/// scrollers surround a widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollerMark(pub u64);

/// Finds the focused widget (a kit control or a text field) and the kit
/// scrollers around it, outer → inner.
#[derive(Debug, Default)]
pub struct FocusChain {
    stack: Vec<u64>,
    pending: Option<u64>,
    pub found: Option<Vec<u64>>,
}

impl Operation for FocusChain {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        let pushed = self.pending.take();
        if let Some(id) = pushed {
            self.stack.push(id);
        }
        operate(self);
        if pushed.is_some() {
            self.stack.pop();
        }
    }
    fn custom(
        &mut self,
        _id: Option<&widget::Id>,
        _bounds: Rectangle,
        state: &mut dyn std::any::Any,
    ) {
        if let Some(m) = state.downcast_ref::<ScrollerMark>() {
            self.pending = Some(m.0);
        }
    }
    fn focusable(
        &mut self,
        _id: Option<&widget::Id>,
        _bounds: Rectangle,
        state: &mut dyn Focusable,
    ) {
        if self.found.is_none() && state.is_focused() {
            self.found = Some(self.stack.clone());
        }
    }
}

/// Is `event` a press of a scroll key?
pub(crate) fn scroll_key(event: &Event) -> bool {
    matches!(
        event,
        Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(k),
            ..
        }) if chain::key_bit(*k).is_some()
    )
}

/// Reserve the scroll `keys` ([`chain::key_bit`] bits) for the widget
/// calling this from its `draw`: while it is drawn, no scroller of its window
/// acts on them.
pub fn own_keys(keys: KeySet) {
    chain::reserve_keys(keys);
}

type OnKey<'a, M> = Box<dyn Fn(Named, Modifiers) -> Option<M> + 'a>;

/// Wraps a widget that handles the scroll keys itself (see the module docs).
pub struct KeyOwner<'a, M> {
    content: El<'a, M>,
    keys: KeySet,
    on_key: Option<OnKey<'a, M>>,
}

/// [`KeyOwner`] over `content`, reserving every scroll key.
pub fn key_owner<'a, M>(content: impl Into<El<'a, M>>) -> KeyOwner<'a, M> {
    KeyOwner {
        content: content.into(),
        keys: chain::ALL_KEYS,
        on_key: None,
    }
}

impl<'a, M> KeyOwner<'a, M> {
    /// Reserve only these keys (the others still scroll).
    pub fn keys(mut self, keys: impl IntoIterator<Item = Named>) -> Self {
        self.keys = keys
            .into_iter()
            .filter_map(chain::key_bit)
            .fold(0, |a, b| a | b);
        self
    }

    /// Handle the reserved keys here: after `content` had them, a press
    /// nobody captured, outside text fields, publishes `f`'s message (and is
    /// captured) when it returns one.
    pub fn on_key(mut self, f: impl Fn(Named, Modifiers) -> Option<M> + 'a) -> Self {
        self.on_key = Some(Box::new(f));
        self
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for KeyOwner<'a, M> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
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
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
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
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        if let Some(f) = &self.on_key
            && let Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(k),
                modifiers,
                ..
            }) = event
            && chain::key_bit(*k).is_some_and(|b| b & self.keys != 0)
            && !shell.is_event_captured()
            && !crate::hotkeys::typing()
            && let Some(m) = f(*k, *modifiers)
        {
            shell.publish(m);
            shell.capture_event();
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
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
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
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
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
        own_keys(self.keys);
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
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
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: 'a> From<KeyOwner<'a, M>> for El<'a, M> {
    fn from(k: KeyOwner<'a, M>) -> Self {
        Element::new(k)
    }
}
