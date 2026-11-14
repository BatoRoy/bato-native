//! Roving focus: arrow keys move focus between the focusable widgets of a
//! group (wrapping) and report the new index, so the group can select it.
//! Members other than the selected one are usually `tab_stop(false)`, so Tab
//! enters and leaves the group in one step (the ARIA radio-group pattern).
//!
//! The template's React `SegmentedControl` and `RadioGroup` do **not** rove
//! (every segment is a Tab stop and arrows scroll the page), so the kit's
//! default follows React; `segmented(…).roving()` opts in.
//!
//! ```ignore
//! roving(row(items)).on_move(|i| Msg::Pick(values[i]))
//! ```

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::keyboard::{self, Key, key::Named};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};

use super::{Apply, Collect, bump, plain};

/// Which arrows move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Axis {
    /// Left / Right.
    Horizontal,
    /// Up / Down.
    Vertical,
    /// All four (radio groups in Chromium).
    #[default]
    Both,
}

pub struct Roving<'a, M> {
    content: Element<'a, M, iced::Theme, iced::Renderer>,
    on_move: Option<Box<dyn Fn(usize) -> M + 'a>>,
    axis: Axis,
    wrap: bool,
}

/// Arrow keys move focus among `content`'s focusable widgets.
pub fn roving<'a, M>(
    content: impl Into<Element<'a, M, iced::Theme, iced::Renderer>>,
) -> Roving<'a, M> {
    Roving {
        content: content.into(),
        on_move: None,
        axis: Axis::Both,
        wrap: true,
    }
}

impl<'a, M> Roving<'a, M> {
    /// Publish this with the new member's index after each move.
    pub fn on_move(mut self, f: impl Fn(usize) -> M + 'a) -> Self {
        self.on_move = Some(Box::new(f));
        self
    }
    pub fn axis(mut self, a: Axis) -> Self {
        self.axis = a;
        self
    }
    /// Wrap from the last member to the first (default on).
    pub fn wrap(mut self, on: bool) -> Self {
        self.wrap = on;
        self
    }
}

/// The step an arrow key makes on `axis`, if any.
pub(crate) fn step(key: &Key, axis: Axis) -> Option<isize> {
    let (h, v) = match axis {
        Axis::Horizontal => (true, false),
        Axis::Vertical => (false, true),
        Axis::Both => (true, true),
    };
    match key {
        Key::Named(Named::ArrowLeft) if h => Some(-1),
        Key::Named(Named::ArrowRight) if h => Some(1),
        Key::Named(Named::ArrowUp) if v => Some(-1),
        Key::Named(Named::ArrowDown) if v => Some(1),
        _ => None,
    }
}

/// The member after `from` by `d` among `n`.
pub(crate) fn next(from: usize, d: isize, n: usize, wrap: bool) -> usize {
    let i = from as isize + d;
    if wrap {
        i.rem_euclid(n as isize) as usize
    } else {
        i.clamp(0, n as isize - 1) as usize
    }
}

impl<'a, M> Widget<M, iced::Theme, iced::Renderer> for Roving<'a, M> {
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
        if shell.is_event_captured() {
            return;
        }
        let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event else {
            return;
        };
        let Some(d) = step(key, self.axis) else {
            return;
        };
        if !plain(*modifiers) || modifiers.shift() {
            return;
        }
        let mut col = Collect::default();
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], child, renderer, &mut col);
        let n = col.entries.len();
        let Some(cur) = col.entries.iter().rposition(|e| e.focused) else {
            return;
        };
        let to = next(cur, d, n, self.wrap);
        let mut apply = Apply::new(Some(to));
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], child, renderer, &mut apply);
        bump();
        if let Some(f) = &self.on_move {
            shell.publish(f(to));
        }
        shell.capture_event();
        shell.request_redraw();
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

impl<'a, M: 'a> From<Roving<'a, M>> for Element<'a, M, iced::Theme, iced::Renderer> {
    fn from(r: Roving<'a, M>) -> Self {
        Element::new(r)
    }
}
