//! `Timer`: fires a message at a deadline without a timer thread. The widget
//! asks the runtime for a redraw at the deadline (`request_redraw_at`) and
//! publishes when that redraw arrives, so an idle app stays idle. Optionally
//! plays the toast `slideIn` (−8 px → 0, 200 ms) on its content.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse, window};

use crate::anim::ease_out;

pub struct Timer<'a, Message> {
    content: Element<'a, Message, iced::Theme, iced::Renderer>,
    born: Instant,
    after: Duration,
    message: Message,
    slide_in: bool,
}

pub fn timer<'a, Message>(
    content: impl Into<Element<'a, Message, iced::Theme, iced::Renderer>>,
    born: Instant,
    after: Duration,
    message: Message,
) -> Timer<'a, Message> {
    Timer {
        content: content.into(),
        born,
        after,
        message,
        slide_in: false,
    }
}

impl<Message> Timer<'_, Message> {
    /// Toast `slideIn`: the content moves in from 8 px left over 200 ms.
    pub fn slide_in(mut self) -> Self {
        self.slide_in = true;
        self
    }
}

impl<'a, Message: Clone + 'a> Widget<Message, iced::Theme, iced::Renderer> for Timer<'a, Message> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
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
        shell: &mut Shell<'_, Message>,
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
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            let deadline = self.born + self.after;
            if *now >= deadline {
                shell.publish(self.message.clone());
            } else if self.slide_in
                && now.saturating_duration_since(self.born) < Duration::from_millis(200)
            {
                shell.request_redraw();
            } else {
                shell.request_redraw_at(deadline);
            }
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
        use iced::advanced::Renderer as _;
        let f = if self.slide_in {
            (self.born.elapsed().as_secs_f32() / 0.2).min(1.0)
        } else {
            1.0
        };
        let dx = -8.0 * (1.0 - ease_out(f));
        renderer.with_translation(Vector::new(dx, 0.0), |renderer| {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout.children().next().unwrap(),
                cursor,
                viewport,
            );
        });
    }
}

impl<'a, Message: Clone + 'a> From<Timer<'a, Message>>
    for Element<'a, Message, iced::Theme, iced::Renderer>
{
    fn from(t: Timer<'a, Message>) -> Self {
        Element::new(t)
    }
}

/// A tick every `period` while subscribed: `Subscription::none()` when the
/// app does not need it, so nothing runs (idle stays idle). iced's
/// `time::every` needs an async runtime the kit does not enable; this is a
/// sleeping thread that ends once the subscription is dropped (within one
/// period). Changing `period` restarts it.
///
/// ```ignore
/// fn subscription(&self) -> Subscription<Msg> {
///     if self.polling { timer::every(Duration::from_secs(2)).map(Msg::Tick) }
///     else { Subscription::none() }
/// }
/// ```
pub fn every(period: Duration) -> iced::Subscription<Instant> {
    iced::Subscription::run_with(period, |period| ticker(*period))
}

fn ticker(period: Duration) -> iced::futures::channel::mpsc::UnboundedReceiver<Instant> {
    let (tx, rx) = iced::futures::channel::mpsc::unbounded();
    let period = period.max(Duration::from_millis(1));
    let _ = std::thread::Builder::new()
        .name("bato-every".into())
        .spawn(move || {
            let mut next = Instant::now() + period;
            loop {
                std::thread::sleep(next.saturating_duration_since(Instant::now()));
                if tx.unbounded_send(Instant::now()).is_err() {
                    break;
                }
                // No drift, no catch-up burst after a stall (suspend).
                next = (next + period).max(Instant::now());
            }
        });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::StreamExt;

    #[test]
    fn ticks_until_dropped() {
        let mut rx = ticker(Duration::from_millis(5));
        let t0 = Instant::now();
        let a = iced::futures::executor::block_on(rx.next()).unwrap();
        let b = iced::futures::executor::block_on(rx.next()).unwrap();
        assert!(a >= t0 + Duration::from_millis(5));
        assert!(b > a);
        // Dropping the receiver ends the thread at its next tick.
        drop(rx);
    }
}
