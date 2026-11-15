//! Hover / focus-within reveal: Tailwind's
//! `group` + `opacity-0 group-hover:opacity-100 group-focus-within:opacity-100
//! transition-opacity` (card actions, row buttons).
//!
//! [`reveal_group`] wraps the whole card or row and tracks the pointer over it
//! and keyboard focus anywhere inside it; each [`revealed`] part inside fades
//! in (150 ms) while either holds. Like CSS opacity, a hidden part still takes
//! clicks and Tab stops, and tabbing onto it reveals it.
//!
//! ```ignore
//! reveal_group(card_box(cx, row![
//!     label(name, 14.0, 20.0),
//!     revealed(button(cx).ghost().icon(Ico::Trash2).on_press(Msg::Delete(id))),
//! ]))
//! ```

use std::cell::Cell;
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse, window};

use crate::anim::{Tween, ease_standard, with_opacity};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// `transition-opacity` (150 ms, the standard easing).
pub const TRANSITION: Duration = Duration::from_millis(150);

thread_local! {
    /// The innermost drawing group's reveal amount (1 outside any group).
    static AMOUNT: Cell<f32> = const { Cell::new(1.0) };
}

/// The reveal amount of the group being drawn (0 hidden … 1 shown).
pub fn current_reveal() -> f32 {
    AMOUNT.with(Cell::get)
}

/// Why a group is revealed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reasons {
    pub hover: bool,
    pub focus_within: bool,
    pub forced: bool,
}

impl Reasons {
    pub fn any(self) -> bool {
        self.hover || self.focus_within || self.forced
    }
}

/// A group's state: why it is revealed and the opacity tween.
#[derive(Debug, Clone, Copy)]
pub struct RevealState {
    pub reasons: Reasons,
    tween: Tween,
    /// `focus::revision()` last checked for focus-within.
    seen: u64,
}

impl Default for RevealState {
    fn default() -> Self {
        Self {
            reasons: Reasons::default(),
            tween: Tween::settled(0.0),
            seen: u64::MAX,
        }
    }
}

impl RevealState {
    /// Apply new reasons; returns whether the target changed.
    pub fn set(&mut self, reasons: Reasons, now: Instant) -> bool {
        let was = self.reasons.any();
        self.reasons = reasons;
        let on = reasons.any();
        if on != was {
            let from = self.tween.value(now);
            self.tween = Tween::new(from, if on { 1.0 } else { 0.0 }, TRANSITION, ease_standard);
            true
        } else {
            false
        }
    }
    pub fn value(&self, now: Instant) -> f32 {
        self.tween.value(now)
    }
    pub fn is_running(&self, now: Instant) -> bool {
        self.tween.is_running(now)
    }
}

/// A `group`: see the module docs.
pub struct RevealGroup<'a, M> {
    content: El<'a, M>,
    forced: bool,
    animate: bool,
}

/// Wrap a card or row whose [`revealed`] parts show on hover or focus-within.
pub fn reveal_group<'a, M>(content: impl Into<El<'a, M>>) -> RevealGroup<'a, M> {
    RevealGroup {
        content: content.into(),
        forced: false,
        animate: true,
    }
}

impl<M> RevealGroup<'_, M> {
    /// Keep the parts shown (the row's menu is open, it is selected).
    pub fn force(mut self, on: bool) -> Self {
        self.forced = on;
        self
    }
    /// `false` jumps instead of fading (screenshots).
    pub fn animate(mut self, on: bool) -> Self {
        self.animate = on;
        self
    }
}

impl<M> Widget<M, iced::Theme, iced::Renderer> for RevealGroup<'_, M> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<RevealState>()
    }
    fn state(&self) -> tree::State {
        let mut s = RevealState::default();
        if self.forced {
            s.reasons.forced = true;
            s.tween = Tween::settled(1.0);
        }
        tree::State::new(s)
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
        let now = Instant::now();
        // Focus-within: ask the content again only when focus moved.
        let rev = crate::focus::revision();
        let st = tree.state.downcast_mut::<RevealState>();
        let mut reasons = st.reasons;
        if st.seen != rev {
            st.seen = rev;
            let mut op = crate::focus::FocusedBounds::default();
            self.content
                .as_widget_mut()
                .operate(&mut tree.children[0], child, renderer, &mut op);
            reasons.focus_within = op.0.is_some();
        }
        let st = tree.state.downcast_mut::<RevealState>();
        if matches!(
            event,
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft)
                | Event::Touch(_)
        ) {
            reasons.hover = cursor.is_over(layout.bounds());
        }
        reasons.forced = self.forced;
        if st.set(reasons, now) {
            if !self.animate {
                st.tween = Tween::settled(if reasons.any() { 1.0 } else { 0.0 });
            }
            shell.request_redraw();
        }
        if let Event::Window(window::Event::RedrawRequested(at)) = event
            && st.is_running(*at)
        {
            shell.request_redraw();
        }
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
        let k = tree
            .state
            .downcast_ref::<RevealState>()
            .value(Instant::now());
        let prev = AMOUNT.with(|a| a.replace(k));
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
        AMOUNT.with(|a| a.set(prev));
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

impl<'a, M: 'a> From<RevealGroup<'a, M>> for El<'a, M> {
    fn from(g: RevealGroup<'a, M>) -> Self {
        Element::new(g)
    }
}

/// A part of a [`reveal_group`] that is hidden until the group is hovered
/// or has focus inside. Outside a group it is always shown.
pub fn revealed<'a, M: 'a>(content: impl Into<El<'a, M>>) -> El<'a, M> {
    Element::new(Revealed {
        content: content.into(),
    })
}

struct Revealed<'a, M> {
    content: El<'a, M>,
}

impl<M> Widget<M, iced::Theme, iced::Renderer> for Revealed<'_, M> {
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
        let k = current_reveal();
        if k <= 0.0 {
            return;
        }
        let style = renderer::Style {
            text_color: iced::Color {
                a: style.text_color.a * k,
                ..style.text_color
            },
        };
        with_opacity(k, || {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                &style,
                layout.children().next().unwrap(),
                cursor,
                viewport,
            );
        });
    }
}

/// [`with_opacity`] by the current group's reveal amount (custom widgets
/// that draw their own hover-only parts).
pub fn with_reveal<R>(f: impl FnOnce() -> R) -> R {
    with_opacity(current_reveal(), f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_or_focus_reveals_with_a_fade() {
        let t0 = Instant::now();
        let mut s = RevealState::default();
        assert_eq!(s.value(t0), 0.0);
        let hover = Reasons {
            hover: true,
            ..Reasons::default()
        };
        assert!(s.set(hover, t0));
        assert!(s.is_running(t0 + TRANSITION / 2));
        assert_eq!(s.value(t0 + TRANSITION), 1.0);
        // Focus arriving while hovered changes nothing visible.
        let both = Reasons {
            focus_within: true,
            ..hover
        };
        assert!(!s.set(both, t0 + TRANSITION));
        // The pointer leaves; focus keeps it shown.
        let focus = Reasons {
            focus_within: true,
            ..Reasons::default()
        };
        assert!(!s.set(focus, t0 + TRANSITION));
        assert_eq!(s.value(t0 + TRANSITION * 3), 1.0);
        // Focus leaves too: fades out from where it is.
        let t1 = t0 + TRANSITION * 3;
        assert!(s.set(Reasons::default(), t1));
        assert_eq!(s.value(t1 + TRANSITION), 0.0);
    }

    #[test]
    fn amount_is_scoped_to_the_group_being_drawn() {
        assert_eq!(current_reveal(), 1.0, "outside a group: shown");
        let prev = AMOUNT.with(|a| a.replace(0.25));
        assert_eq!(current_reveal(), 0.25);
        AMOUNT.with(|a| a.set(prev));
        assert_eq!(current_reveal(), 1.0);
    }
}
