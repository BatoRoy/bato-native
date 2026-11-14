//! `Tooltip` (`Tooltip.tsx`): a small bubble next to its child after
//! `delay` (400 ms) of hover **or keyboard focus** inside the child; leaving
//! or blurring hides it at once. Placed by `computePosition` on `side`
//! (default top), centred, 8 px away: it **flips** to the opposite side when
//! cramped and is clamped 8 px inside the window. `animate-fade-in` (150 ms),
//! no exit animation, `pointer-events: none`.
//!
//! Bubble: `px-2 py-1 rounded-md bg-app-card border-app-lineStrong
//! shadow-app-lg text-xs text-app-text whitespace-nowrap`.
//!
//! The open state is the widget's own: no app message, no timer thread (a
//! redraw is requested at the deadline).
//!
//! ```ignore
//! tooltip(&cx, button(&on).secondary().label("Hover me"), "A helpful hint")
//! tooltip(&cx, icon_button, "Collapse").side(Placement::Right)   // RailTip
//! ```

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::{Element, Event, Length, Padding, Rectangle, Size, Vector, mouse, window};

use super::anchored::{Dismiss, Enter, FloatState, Floating, Position};
use crate::ctx::Ctx;
use crate::floating::{Align, Options, Placement};
use crate::focus::{FocusedBounds, is_pointer_press, revision};
use crate::label::label;
use crate::style::{c, shadows};
use crate::surface::{BoxStyle, surface};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// React's default `delay`.
pub const DELAY: Duration = Duration::from_millis(400);

pub struct Tooltip<'a, M> {
    anchor: El<'a, M>,
    bubble: El<'a, M>,
    side: Placement,
    delay: Duration,
    animate: bool,
}

/// The bubble style on `cx`'s theme (it floats on nothing in particular:
/// card fill, `lineStrong` over the card).
pub fn bubble_style(cx: &Ctx) -> BoxStyle {
    let p = cx.p();
    BoxStyle {
        bg: Some(c(p.card)),
        border: Some(c(cx.t.line_strong_on(p.card))),
        radius: 6.0,
        shadows: shadows(&p.shadow_lg),
        ..BoxStyle::default()
    }
}

/// The bubble around any content (`content: ReactNode`).
pub fn bubble<'a, M: Clone + 'a>(cx: &Ctx, content: impl Into<El<'a, M>>) -> El<'a, M> {
    surface(content)
        .padding(Padding::from([5.0, 9.0]))
        .fixed(bubble_style(cx))
        .into()
}

/// `<Tooltip content={text}>{anchor}</Tooltip>`.
pub fn tooltip<'a, M: Clone + 'a>(
    cx: &Ctx,
    anchor: impl Into<El<'a, M>>,
    text: impl Into<String>,
) -> Tooltip<'a, M> {
    let p = cx.p();
    tooltip_with(
        anchor,
        bubble(cx, label(text.into(), 12.0, 16.0).color(c(p.text))),
    )
}

/// A tooltip whose bubble is built by the caller (see [`bubble`]).
pub fn tooltip_with<'a, M>(
    anchor: impl Into<El<'a, M>>,
    bubble: impl Into<El<'a, M>>,
) -> Tooltip<'a, M> {
    Tooltip {
        anchor: anchor.into(),
        bubble: bubble.into(),
        side: Placement::Top,
        delay: DELAY,
        animate: true,
    }
}

impl<'a, M> Tooltip<'a, M> {
    /// The preferred side (flips when there isn't room).
    pub fn side(mut self, side: Placement) -> Self {
        self.side = side;
        self
    }
    pub fn delay(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }
    /// `false` skips the fade (screenshots).
    pub fn animate(mut self, on: bool) -> Self {
        self.animate = on;
        self
    }
}

/// Hover and focus bookkeeping (the timer of `show` / `hide`).
#[derive(Debug, Default)]
pub struct Tip {
    hovered: bool,
    focused: bool,
    /// `setTimeout(open, delay)` pending.
    due: Option<Instant>,
    open: bool,
}

impl Tip {
    /// `onMouseEnter` / `onFocus`.
    pub fn show(&mut self, now: Instant, delay: Duration) {
        self.due = Some(now + delay);
    }
    /// `onMouseLeave` / `onBlur`.
    pub fn hide(&mut self) {
        self.due = None;
        self.open = false;
    }
    /// The timer fires; returns whether it opened.
    pub fn tick(&mut self, now: Instant) -> bool {
        match self.due {
            Some(t) if now >= t => {
                self.due = None;
                self.open = true;
                true
            }
            _ => false,
        }
    }
    /// Pointer over the anchor or not; returns whether anything changed.
    pub fn hover(&mut self, over: bool, now: Instant, delay: Duration) -> bool {
        if over == self.hovered {
            return false;
        }
        self.hovered = over;
        if over {
            self.show(now, delay);
        } else {
            self.hide();
        }
        true
    }
    /// Focus inside the anchor or not.
    pub fn focus(&mut self, inside: bool, now: Instant, delay: Duration) -> bool {
        if inside == self.focused {
            return false;
        }
        self.focused = inside;
        if inside {
            self.show(now, delay);
        } else {
            self.hide();
        }
        true
    }
    pub fn is_open(&self) -> bool {
        self.open
    }
}

#[derive(Default)]
struct State {
    tip: Tip,
    float: FloatState,
    seen: u64,
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for Tooltip<'a, M> {
    fn size(&self) -> Size<Length> {
        self.anchor.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.anchor.as_widget().size_hint()
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State {
            seen: revision(),
            ..State::default()
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.anchor), Tree::new(&self.bubble)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.anchor, &self.bubble]);
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let n = self
            .anchor
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
        self.anchor.as_widget_mut().update(
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
        let was_open = tree.state.downcast_ref::<State>().tip.is_open();
        // Focus inside the anchor (`focusin` / `focusout` on the span).
        let rev = revision();
        if tree.state.downcast_ref::<State>().seen != rev || is_pointer_press(event) {
            let mut op = FocusedBounds::default();
            self.anchor
                .as_widget_mut()
                .operate(&mut tree.children[0], child, renderer, &mut op);
            let st = tree.state.downcast_mut::<State>();
            st.seen = rev;
            st.tip.focus(op.0.is_some(), now, self.delay);
        }
        let st = tree.state.downcast_mut::<State>();
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                st.tip
                    .hover(cursor.is_over(layout.bounds()), now, self.delay);
            }
            Event::Mouse(mouse::Event::CursorLeft) => {
                st.tip.hover(false, now, self.delay);
            }
            Event::Window(window::Event::RedrawRequested(at)) => {
                st.tip.tick(*at);
            }
            _ => {}
        }
        if let Some(due) = st.tip.due {
            shell.request_redraw_at(due);
        }
        if st.tip.is_open() != was_open {
            st.float = FloatState::default();
            shell.invalidate_layout();
            shell.request_redraw();
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
        self.anchor.as_widget().mouse_interaction(
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
        self.anchor.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
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
        self.anchor.as_widget().draw(
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
        let anchor = layout.bounds() + translation;
        let Tree {
            state, children, ..
        } = tree;
        let (anchor_tree, rest) = children.split_first_mut()?;
        let inner = self.anchor.as_widget_mut().overlay(
            anchor_tree,
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        );
        let st = state.downcast_mut::<State>();
        let mine = st.tip.is_open().then(|| {
            overlay::Element::new(Box::new(Floating {
                content: &mut self.bubble,
                tree: &mut rest[0],
                state: &mut st.float,
                anchor,
                translation,
                position: Position::Float(
                    Options::default()
                        .placement(self.side)
                        .align(Align::Center)
                        .offset(8.0),
                ),
                enter: if self.animate {
                    Enter::FadeIn
                } else {
                    Enter::None
                },
                dismiss: Dismiss::NONE,
                on_dismiss: None,
                match_width: false,
                hit_test: false,
                id: None,
            }))
        });
        match (inner, mine) {
            (Some(a), Some(b)) => Some(overlay::Group::with_children(vec![a, b]).overlay()),
            (a, b) => a.or(b),
        }
    }
}

impl<'a, M: Clone + 'a> From<Tooltip<'a, M>> for El<'a, M> {
    fn from(t: Tooltip<'a, M>) -> Self {
        Element::new(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_after_the_delay_and_hides_at_once() {
        let t0 = Instant::now();
        let mut tip = Tip::default();
        assert!(tip.hover(true, t0, DELAY));
        assert!(!tip.tick(t0 + Duration::from_millis(399)));
        assert!(!tip.is_open());
        assert!(tip.tick(t0 + DELAY));
        assert!(tip.is_open());
        // Moving inside changes nothing.
        assert!(!tip.hover(true, t0 + DELAY, DELAY));
        assert!(tip.hover(false, t0 + DELAY, DELAY));
        assert!(!tip.is_open());
        assert!(!tip.tick(t0 + DELAY * 2), "leaving clears the timer");
    }

    #[test]
    fn leaving_before_the_delay_cancels() {
        let t0 = Instant::now();
        let mut tip = Tip::default();
        tip.hover(true, t0, DELAY);
        tip.hover(false, t0 + Duration::from_millis(100), DELAY);
        assert!(!tip.tick(t0 + DELAY));
        assert!(!tip.is_open());
    }

    #[test]
    fn focus_shows_and_blur_hides() {
        let t0 = Instant::now();
        let mut tip = Tip::default();
        assert!(tip.focus(true, t0, DELAY));
        assert!(tip.tick(t0 + DELAY));
        assert!(tip.is_open());
        // The pointer leaving hides it even while focused (React).
        tip.hover(true, t0 + DELAY, DELAY);
        tip.hover(false, t0 + DELAY, DELAY);
        assert!(!tip.is_open());
        tip.focus(false, t0 + DELAY, DELAY);
        assert!(!tip.tick(t0 + DELAY * 3));
    }
}
