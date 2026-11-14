//! `collapse`: the animated height clip of `Collapsible` (`grid
//! transition-all duration-200` from `grid-rows-[0fr]` to `[1fr]` around an
//! `overflow-hidden` child). The content keeps its full layout; the box
//! shows `height × t`, `t` easing 0 ↔ 1 over 200 ms when `open` flips.
//!
//! Closed content takes no events and is out of the Tab order; while the
//! height moves the widget relayouts every frame, and asks for nothing once
//! it settles.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse, window};

use crate::anim::{Tween, ease_standard};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// React's `duration-200`.
pub const COLLAPSE: Duration = Duration::from_millis(200);

pub struct Collapse<'a, Message> {
    content: El<'a, Message>,
    open: bool,
    duration: Duration,
}

/// `content`, shown when `open`, its height animated on every change.
pub fn collapse<'a, Message>(
    open: bool,
    content: impl Into<El<'a, Message>>,
) -> Collapse<'a, Message> {
    Collapse {
        content: content.into(),
        open,
        duration: COLLAPSE,
    }
}

impl<'a, Message> Collapse<'a, Message> {
    pub fn duration(mut self, d: Duration) -> Self {
        self.duration = d;
        self
    }
}

#[derive(Debug, Clone, Copy)]
struct State {
    open: bool,
    tween: Tween,
    /// The shown fraction for this frame (set on each redraw).
    t: f32,
}

impl State {
    fn new(open: bool) -> Self {
        let v = if open { 1.0 } else { 0.0 };
        Self {
            open,
            tween: Tween::settled(v),
            t: v,
        }
    }
    fn toggle(&mut self, open: bool, dur: Duration, now: Instant) {
        if open == self.open {
            return;
        }
        self.open = open;
        let from = self.tween.value(now);
        let mut tw = Tween::new(from, if open { 1.0 } else { 0.0 }, dur, ease_standard);
        tw.start = now;
        self.tween = tw;
    }
    fn shown(&self) -> bool {
        self.t > 0.0
    }
    fn settled_open(&self) -> bool {
        self.open && self.t >= 1.0
    }
}

impl<'a, Message> Widget<Message, iced::Theme, iced::Renderer> for Collapse<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::new(self.open))
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.state
            .downcast_mut::<State>()
            .toggle(self.open, self.duration, Instant::now());
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let w = limits.max().width;
        let child = self.content.as_widget_mut().layout(
            &mut tree.children[0],
            renderer,
            &layout::Limits::new(Size::new(w, 0.0), Size::new(w, f32::INFINITY)),
        );
        let t = tree.state.downcast_ref::<State>().t;
        let h = child.size().height * t;
        layout::Node::with_children(Size::new(w, h), vec![child])
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
        let st = tree.state.downcast_mut::<State>();
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            let t = st.tween.value(*now);
            if (t - st.t).abs() > f32::EPSILON {
                st.t = t;
                shell.invalidate_layout();
            }
            if st.tween.is_running(*now) {
                shell.request_redraw();
            }
        }
        if !st.settled_open() {
            return;
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            &layout.bounds().intersection(viewport).unwrap_or_default(),
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
        if !tree.state.downcast_ref::<State>().settled_open() {
            return mouse::Interaction::None;
        }
        self.content.as_widget().mouse_interaction(
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
        if !tree.state.downcast_ref::<State>().shown() {
            return;
        }
        self.content.as_widget_mut().operate(
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
        let b = layout.bounds();
        if b.height <= 0.0 {
            return;
        }
        let Some(clip) = b.intersection(viewport) else {
            return;
        };
        renderer.with_layer(clip, |renderer| {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout.children().next().unwrap(),
                cursor,
                &clip,
            );
        });
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        if !tree.state.downcast_ref::<State>().settled_open() {
            return None;
        }
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: 'a> From<Collapse<'a, Message>> for El<'a, Message> {
    fn from(c: Collapse<'a, Message>) -> Self {
        Element::new(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_and_closes_over_200ms() {
        let t0 = Instant::now();
        let mut s = State::new(false);
        assert!(!s.shown());
        s.toggle(true, COLLAPSE, t0);
        let mid = s.tween.value(t0 + Duration::from_millis(100));
        assert!(mid > 0.5 && mid < 1.0, "eased ahead at half time: {mid}");
        assert_eq!(s.tween.value(t0 + COLLAPSE), 1.0);
        // Reversing mid-way starts from where it is.
        s.toggle(false, COLLAPSE, t0 + Duration::from_millis(100));
        assert!((s.tween.from - mid).abs() < 1e-4);
        assert_eq!(s.tween.to, 0.0);
        // The same state again changes nothing.
        let before = s.tween.start;
        s.toggle(false, COLLAPSE, t0 + Duration::from_millis(150));
        assert_eq!(s.tween.start, before);
    }
}
