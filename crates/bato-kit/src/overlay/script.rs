//! Scripted input for self-tests (`BATO_SELFTEST=…`): a root wrapper that
//! plays a list of [`Step`]s, feeding synthetic pointer and keyboard events
//! through the same path iced's `UserInterface` uses (the overlay layer
//! first, then the widget tree, the base cursor hidden when an overlay is
//! under it), with a virtual pointer that replaces the real one while the
//! script runs. Messages the widgets publish reach the app's `update` as
//! usual. [`Step::Shot`] prints `SELFTEST shot <name>` on stdout and holds,
//! so an outside script can take the screenshot; the end prints
//! `SELFTEST done`.
//!
//! Between steps the wrapper only asks for a redraw at the next step's
//! deadline; when the script is over it asks for nothing (idle stays idle).

use std::io::Write;
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::keyboard::{self, Key, Modifiers, key};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, mouse, window};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// Where a pointer step aims.
#[derive(Debug, Clone)]
pub enum Target {
    /// A window point.
    At(Point),
    /// The container with this id, at an offset from its top-left.
    Id(widget::Id, Vector),
    /// The centre of the container with this id.
    Center(widget::Id),
}

/// One step of a script.
#[derive(Debug, Clone)]
pub enum Step<M> {
    Wait(Duration),
    /// Move the virtual pointer (a `CursorMoved`).
    Move(Target),
    /// Move there, press and release `button`.
    Click(Target, mouse::Button),
    /// Move there and press `button` only.
    Press(Target, mouse::Button),
    /// Press and release a named key.
    Key(key::Named, Modifiers),
    /// Press and release a character key.
    Char(char, Modifiers),
    /// Any event, as is.
    Event(Event),
    /// Publish an app message.
    Message(M),
    /// Print `SELFTEST shot <name>` and hold for the screenshot.
    Shot(&'static str),
}

impl<M> Step<M> {
    pub fn map<N>(self, f: impl Fn(M) -> N) -> Step<N> {
        match self {
            Step::Wait(d) => Step::Wait(d),
            Step::Move(t) => Step::Move(t),
            Step::Click(t, b) => Step::Click(t, b),
            Step::Press(t, b) => Step::Press(t, b),
            Step::Key(k, m) => Step::Key(k, m),
            Step::Char(c, m) => Step::Char(c, m),
            Step::Event(e) => Step::Event(e),
            Step::Message(m) => Step::Message(f(m)),
            Step::Shot(s) => Step::Shot(s),
        }
    }

    /// How long to wait after this step before the next one.
    fn pause(&self) -> Duration {
        match self {
            Step::Wait(d) => *d,
            Step::Shot(_) => SHOT_HOLD,
            Step::Message(_) => Duration::from_millis(250),
            _ => Duration::from_millis(120),
        }
    }
}

/// How long a `Shot` holds the frame still.
pub const SHOT_HOLD: Duration = Duration::from_millis(1500);
/// Settle time before the first step.
const START: Duration = Duration::from_millis(800);

/// Key events for a named key or a character.
pub fn key_events(key: Key, modifiers: Modifiers) -> [Event; 2] {
    let text = match &key {
        Key::Character(c) => Some(c.clone()),
        _ => None,
    };
    let physical = key::Physical::Unidentified(key::NativeCode::Unidentified);
    [
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key.clone(),
            physical_key: physical,
            location: keyboard::Location::Standard,
            modifiers,
            text,
            repeat: false,
        }),
        Event::Keyboard(keyboard::Event::KeyReleased {
            key: key.clone(),
            modified_key: key,
            physical_key: physical,
            location: keyboard::Location::Standard,
            modifiers,
        }),
    ]
}

/// Wrap the whole window: `script(root, steps)`.
pub struct Script<'a, M> {
    content: El<'a, M>,
    steps: Vec<Step<M>>,
}

pub fn script<'a, M>(content: impl Into<El<'a, M>>, steps: Vec<Step<M>>) -> Script<'a, M> {
    Script {
        content: content.into(),
        steps,
    }
}

#[derive(Debug, Default)]
struct State {
    next: usize,
    due: Option<Instant>,
    pointer: Option<Point>,
    done: bool,
}

/// Finds a container's bounds by id.
struct Find<'a> {
    id: &'a widget::Id,
    found: Option<Rectangle>,
}

impl widget::Operation for Find<'_> {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation)) {
        operate(self);
    }
    fn container(&mut self, id: Option<&widget::Id>, bounds: Rectangle) {
        if self.found.is_none() && id == Some(self.id) {
            self.found = Some(bounds);
        }
    }
}

impl<'a, M: Clone + 'a> Script<'a, M> {
    fn cursor(state: &State, real: mouse::Cursor) -> mouse::Cursor {
        match state.pointer {
            Some(p) => mouse::Cursor::Available(p),
            None => real,
        }
    }

    fn find(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        id: &widget::Id,
    ) -> Option<Rectangle> {
        let mut op = Find { id, found: None };
        // Containers in the base tree report content coordinates: inside
        // the page scroller they sit `page_offset` lower than on screen.
        // (Assumes base-tree targets are page content.)
        let page = crate::scroller::page_offset();
        if let Some(ov) = self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            Vector::ZERO,
        ) {
            let mut nested = overlay::Nested::new(ov);
            let node = nested.layout(renderer, layout.bounds().size());
            nested.operate(Layout::new(&node), renderer, &mut op);
        }
        if op.found.is_none() {
            self.content
                .as_widget_mut()
                .operate(&mut tree.children[0], layout, renderer, &mut op);
            return op.found.map(|b| b - page);
        }
        op.found
    }

    fn resolve(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        t: &Target,
    ) -> Option<Point> {
        match t {
            Target::At(p) => Some(*p),
            Target::Id(id, off) => self
                .find(tree, layout, renderer, viewport, id)
                .map(|b| b.position() + *off),
            Target::Center(id) => self
                .find(tree, layout, renderer, viewport, id)
                .map(|b| b.center()),
        }
    }

    /// Dispatch one synthetic event like `UserInterface::update` does.
    #[allow(clippy::too_many_arguments)]
    fn dispatch(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let cursor = Self::cursor(
            tree.state.downcast_ref::<State>(),
            mouse::Cursor::Unavailable,
        );
        let mut messages = Vec::new();
        let mut base_cursor = cursor;
        let mut captured = false;
        let mut layout_invalid = false;
        let mut widgets_invalid = false;
        let mut redraw = false;
        {
            let mut sub = Shell::new(&mut messages);
            if let Some(ov) = self.content.as_widget_mut().overlay(
                &mut tree.children[0],
                layout,
                renderer,
                viewport,
                Vector::ZERO,
            ) {
                let mut nested = overlay::Nested::new(ov);
                let node = nested.layout(renderer, layout.bounds().size());
                nested.update(
                    event,
                    Layout::new(&node),
                    cursor,
                    renderer,
                    clipboard,
                    &mut sub,
                );
                if nested.mouse_interaction(Layout::new(&node), cursor, renderer)
                    != mouse::Interaction::None
                {
                    base_cursor = mouse::Cursor::Unavailable;
                }
                captured = sub.is_event_captured();
            }
            layout_invalid |= sub.is_layout_invalid();
            widgets_invalid |= sub.are_widgets_invalid();
            redraw |= sub.redraw_request() != window::RedrawRequest::Wait;
        }
        if !captured {
            let mut sub = Shell::new(&mut messages);
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                layout,
                base_cursor,
                renderer,
                clipboard,
                &mut sub,
                viewport,
            );
            layout_invalid |= sub.is_layout_invalid();
            widgets_invalid |= sub.are_widgets_invalid();
            redraw |= sub.redraw_request() != window::RedrawRequest::Wait;
        }
        for m in messages {
            shell.publish(m);
        }
        if layout_invalid {
            shell.invalidate_layout();
        }
        if widgets_invalid {
            shell.invalidate_widgets();
        }
        if redraw {
            shell.request_redraw();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        i: usize,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let step = self.steps[i].clone();
        let mut events: Vec<Event> = Vec::new();
        let aim = |this: &mut Self, tree: &mut Tree, t: &Target| -> bool {
            match this.resolve(tree, layout, renderer, viewport, t) {
                Some(p) => {
                    if std::env::var_os("BATO_SELFTEST_TRACE").is_some() {
                        eprintln!("SELFTEST aim {t:?} -> {p:?}");
                    }
                    tree.state.downcast_mut::<State>().pointer = Some(p);
                    true
                }
                None => {
                    eprintln!("SELFTEST target not found: {t:?}");
                    false
                }
            }
        };
        let moved = |p: Point| Event::Mouse(mouse::Event::CursorMoved { position: p });
        match &step {
            Step::Wait(_) => {}
            Step::Move(t) => {
                if aim(self, tree, t) {
                    let p = tree.state.downcast_ref::<State>().pointer.unwrap();
                    events.push(moved(p));
                }
            }
            Step::Click(t, b) | Step::Press(t, b) => {
                if aim(self, tree, t) {
                    let p = tree.state.downcast_ref::<State>().pointer.unwrap();
                    events.push(moved(p));
                    events.push(Event::Mouse(mouse::Event::ButtonPressed(*b)));
                    if matches!(step, Step::Click(..)) {
                        events.push(Event::Mouse(mouse::Event::ButtonReleased(*b)));
                    }
                }
            }
            Step::Key(k, m) => events.extend(key_events(Key::Named(*k), *m)),
            Step::Char(c, m) => events.extend(key_events(Key::Character(c.to_string().into()), *m)),
            Step::Event(e) => events.push(e.clone()),
            Step::Message(m) => shell.publish(m.clone()),
            Step::Shot(name) => {
                println!("SELFTEST shot {name}");
                let _ = std::io::stdout().flush();
            }
        }
        for e in &events {
            self.dispatch(tree, e, layout, renderer, clipboard, shell, viewport);
        }
    }
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for Script<'a, M> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::default())
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
        let c = Self::cursor(tree.state.downcast_ref::<State>(), cursor);
        // While the script owns the pointer, the real one is ignored.
        let real_pointer = matches!(event, Event::Mouse(_))
            && tree.state.downcast_ref::<State>().pointer.is_some();
        if !real_pointer {
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                layout,
                c,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        }
        let Event::Window(window::Event::RedrawRequested(now)) = event else {
            return;
        };
        let now = *now;
        let st = tree.state.downcast_mut::<State>();
        if st.done {
            return;
        }
        let due = *st.due.get_or_insert(now + START);
        if now < due {
            shell.request_redraw_at(due);
            return;
        }
        let i = st.next;
        if i >= self.steps.len() {
            st.done = true;
            println!("SELFTEST done");
            let _ = std::io::stdout().flush();
            return;
        }
        st.next += 1;
        st.due = Some(now + self.steps[i].pause());
        self.run(i, tree, layout, renderer, clipboard, shell, viewport);
        let due = tree.state.downcast_ref::<State>().due.unwrap();
        shell.request_redraw_at(due);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let c = Self::cursor(tree.state.downcast_ref::<State>(), cursor);
        self.content
            .as_widget()
            .mouse_interaction(&tree.children[0], layout, c, viewport, renderer)
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
        let c = Self::cursor(tree.state.downcast_ref::<State>(), cursor);
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            c,
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
        let pointer = tree.state.downcast_ref::<State>().pointer;
        let inner = self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )?;
        Some(match pointer {
            Some(p) => overlay::Element::new(Box::new(Pointer { inner, at: p })),
            None => inner,
        })
    }
}

impl<'a, M: Clone + 'a> From<Script<'a, M>> for El<'a, M> {
    fn from(s: Script<'a, M>) -> Self {
        Element::new(s)
    }
}

/// The overlay layer seen through the virtual pointer.
struct Pointer<'b, M> {
    inner: overlay::Element<'b, M, iced::Theme, iced::Renderer>,
    at: Point,
}

impl<M> overlay::Overlay<M, iced::Theme, iced::Renderer> for Pointer<'_, M> {
    fn layout(&mut self, renderer: &iced::Renderer, bounds: Size) -> layout::Node {
        self.inner.as_overlay_mut().layout(renderer, bounds)
    }
    fn draw(
        &self,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
    ) {
        self.inner.as_overlay().draw(
            renderer,
            theme,
            style,
            layout,
            mouse::Cursor::Available(self.at),
        );
    }
    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.inner
            .as_overlay_mut()
            .operate(layout, renderer, operation);
    }
    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
    ) {
        if matches!(event, Event::Mouse(_)) {
            return;
        }
        self.inner.as_overlay_mut().update(
            event,
            layout,
            mouse::Cursor::Available(self.at),
            renderer,
            clipboard,
            shell,
        );
    }
    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.inner.as_overlay().mouse_interaction(
            layout,
            mouse::Cursor::Available(self.at),
            renderer,
        )
    }
    fn overlay<'c>(
        &'c mut self,
        layout: Layout<'c>,
        renderer: &iced::Renderer,
    ) -> Option<overlay::Element<'c, M, iced::Theme, iced::Renderer>> {
        let at = self.at;
        self.inner
            .as_overlay_mut()
            .overlay(layout, renderer)
            .map(|inner| overlay::Element::new(Box::new(Pointer { inner, at })))
    }
    fn index(&self) -> f32 {
        self.inner.as_overlay().index()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_events_press_then_release() {
        let [p, r] = key_events(Key::Named(key::Named::ArrowDown), Modifiers::default());
        assert!(matches!(
            p,
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Named(key::Named::ArrowDown),
                ..
            })
        ));
        assert!(matches!(
            r,
            Event::Keyboard(keyboard::Event::KeyReleased { .. })
        ));
        let [p, _] = key_events(Key::Character("k".into()), Modifiers::CTRL);
        let Event::Keyboard(keyboard::Event::KeyPressed {
            text, modifiers, ..
        }) = p
        else {
            panic!()
        };
        assert_eq!(text.as_deref(), Some("k"));
        assert!(modifiers.command());
    }

    #[test]
    fn pauses() {
        assert_eq!(Step::<()>::Shot("x").pause(), SHOT_HOLD);
        assert_eq!(
            Step::<()>::Wait(Duration::from_secs(2)).pause(),
            Duration::from_secs(2)
        );
        let s: Step<u8> = Step::Message(1u8);
        assert!(matches!(s.map(|m| m as u32 + 1), Step::Message(2)));
    }
}
