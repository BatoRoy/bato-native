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

use std::any::Any;
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

/// Finds a container's bounds by id, in window coordinates (through every
/// kit scroller around it).
struct Find<'a> {
    id: &'a widget::Id,
    found: Option<Rectangle>,
    translation: crate::scroller::Translation,
}

impl widget::Operation for Find<'_> {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation)) {
        if self.found.is_some() {
            return;
        }
        let entered = self.translation.enter();
        operate(self);
        self.translation.leave(entered);
    }
    fn container(&mut self, id: Option<&widget::Id>, bounds: Rectangle) {
        if self.found.is_none() && id == Some(self.id) {
            self.found = Some(self.translation.to_window(bounds));
        }
    }
    fn custom(&mut self, _id: Option<&widget::Id>, _bounds: Rectangle, state: &mut dyn Any) {
        self.translation.custom(state);
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
        let mut op = Find {
            id,
            found: None,
            translation: Default::default(),
        };
        // The top layer first, like a hit test.
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
            op.translation = Default::default();
            self.content
                .as_widget_mut()
                .operate(&mut tree.children[0], layout, renderer, &mut op);
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

    // ── Targets and reveals through real scrollers ──────────────────────

    use crate::scroller::{scroll_offset, scroller};
    use iced::Color;
    use iced::advanced::clipboard;
    use iced::advanced::renderer::Headless;
    use iced::widget::{Space, column, container, row};

    const VIEW: Size = Size::new(800.0, 600.0);

    fn renderer() -> iced::Renderer {
        let fut = <iced::Renderer as Headless>::new(
            iced::Font::DEFAULT,
            iced::Pixels(16.0),
            Some("tiny-skia"),
        );
        let mut fut = std::pin::pin!(fut);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            if let std::task::Poll::Ready(r) = fut.as_mut().poll(&mut cx) {
                return r.expect("tiny-skia renderer");
            }
        }
    }

    fn id(name: &'static str) -> widget::Id {
        widget::Id::new(name)
    }

    fn boxed(name: &'static str, w: f32, h: f32) -> El<'static, ()> {
        container(Space::new().width(w).height(h))
            .id(id(name))
            .into()
    }

    fn page(content: El<'static, ()>) -> crate::scroller::Scroller<'static, ()> {
        scroller(content, Color::BLACK, Color::WHITE)
    }

    struct Ui {
        s: Script<'static, ()>,
        tree: Tree,
        r: iced::Renderer,
    }

    impl Ui {
        fn new(content: El<'static, ()>) -> Self {
            let s = script(content, Vec::new());
            let tree = Tree::new(&s as &dyn Widget<(), iced::Theme, iced::Renderer>);
            Self {
                s,
                tree,
                r: renderer(),
            }
        }
        fn layout(&mut self) -> layout::Node {
            self.tree
                .diff(&self.s as &dyn Widget<(), iced::Theme, iced::Renderer>);
            self.s.layout(
                &mut self.tree,
                &self.r,
                &layout::Limits::new(Size::ZERO, VIEW),
            )
        }
        fn send(&mut self, event: Event) {
            let node = self.layout();
            let mut msgs = Vec::new();
            let mut shell = Shell::new(&mut msgs);
            self.s.content.as_widget_mut().update(
                &mut self.tree.children[0],
                &event,
                Layout::new(&node),
                mouse::Cursor::Available(Point::new(5.0, 5.0)),
                &self.r,
                &mut clipboard::Null,
                &mut shell,
                &Rectangle::with_size(VIEW),
            );
        }
        fn moved(&mut self) {
            self.send(Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(5.0, 5.0),
            }));
        }
        /// Window bounds, as a `Target::Id` resolves them.
        fn window(&mut self, name: &'static str) -> Rectangle {
            let node = self.layout();
            let viewport = Rectangle::with_size(VIEW);
            self.s
                .find(
                    &mut self.tree,
                    Layout::new(&node),
                    &self.r,
                    &viewport,
                    &id(name),
                )
                .expect("found")
        }
        /// Layout bounds (no scroll applied).
        fn laid_out(&mut self, name: &'static str) -> Rectangle {
            struct Raw(widget::Id, Option<Rectangle>);
            impl widget::Operation for Raw {
                fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation)) {
                    operate(self);
                }
                fn container(&mut self, id: Option<&widget::Id>, bounds: Rectangle) {
                    if id == Some(&self.0) {
                        self.1 = Some(bounds);
                    }
                }
            }
            let node = self.layout();
            let mut op = Raw(id(name), None);
            self.s.content.as_widget_mut().operate(
                &mut self.tree.children[0],
                Layout::new(&node),
                &self.r,
                &mut op,
            );
            op.1.expect("laid out")
        }
    }

    #[test]
    fn id_targets_are_window_points_inside_any_scroller() {
        let nested = page(row![Space::new().width(900.0), boxed("in-h", 40.0, 20.0)].into())
            .horizontal()
            .id(id("h"))
            .width(300.0)
            .height(40.0);
        let content = column![
            Space::new().height(500.0),
            boxed("in-page", 50.0, 20.0),
            nested,
            Space::new().height(1000.0),
        ];
        let mut ui = Ui::new(
            column![
                boxed("chrome", 100.0, 40.0),
                page(content.into()).initial(Some(200.0)).height(400.0),
            ]
            .into(),
        );
        ui.moved();
        scroll_offset(id("h"), Some(150.0), None, false);
        ui.moved();

        // Outside any scroller: as laid out (B4's page-offset shift moved it).
        assert_eq!(ui.window("chrome"), ui.laid_out("chrome"));
        // In the page: 200 px higher.
        assert_eq!(
            ui.window("in-page"),
            ui.laid_out("in-page") - Vector::new(0.0, 200.0)
        );
        // In a horizontal scroller inside the page: both offsets.
        assert_eq!(
            ui.window("in-h"),
            ui.laid_out("in-h") - Vector::new(150.0, 200.0)
        );
    }

    /// Calls `focus_into_view` with its own bounds on every press.
    struct Revealer;

    impl Widget<(), iced::Theme, iced::Renderer> for Revealer {
        fn size(&self) -> Size<Length> {
            Size::new(Length::Fill, Length::Fixed(40.0))
        }
        fn layout(
            &mut self,
            _tree: &mut Tree,
            _renderer: &iced::Renderer,
            limits: &layout::Limits,
        ) -> layout::Node {
            layout::Node::new(Size::new(limits.max().width, 40.0))
        }
        fn draw(
            &self,
            _tree: &Tree,
            _renderer: &mut iced::Renderer,
            _theme: &iced::Theme,
            _style: &renderer::Style,
            _layout: Layout<'_>,
            _cursor: mouse::Cursor,
            _viewport: &Rectangle,
        ) {
        }
        fn update(
            &mut self,
            _tree: &mut Tree,
            event: &Event,
            layout: Layout<'_>,
            _cursor: mouse::Cursor,
            _renderer: &iced::Renderer,
            _clipboard: &mut dyn Clipboard,
            _shell: &mut Shell<'_, ()>,
            _viewport: &Rectangle,
        ) {
            if let Event::Keyboard(keyboard::Event::KeyPressed { .. }) = event {
                crate::scroller::focus_into_view(layout.bounds());
            }
        }
    }

    /// A page at 1000 px, and a widget outside it (a fixed bar, a modal
    /// layer) whose window bounds fall inside the page's content range.
    fn bar_under_page(rooted: bool) -> Vector {
        let content = column![
            Space::new().height(1000.0),
            boxed("in-page", 50.0, 20.0),
            Space::new().height(1000.0),
        ];
        let tree = column![
            page(content.into()).initial(Some(1000.0)).height(400.0),
            El::new(Revealer),
        ];
        let el: El<'static, ()> = if rooted {
            crate::focus::focus_root(tree).into()
        } else {
            tree.into()
        };
        let mut ui = Ui::new(el);
        ui.moved();
        let before = ui.window("in-page");
        for e in key_events(Key::Named(key::Named::Tab), Modifiers::default()) {
            ui.send(e);
        }
        ui.moved();
        ui.moved();
        ui.window("in-page").position() - before.position()
    }

    #[test]
    fn reveal_with_no_scroller_around_never_moves_the_page() {
        // Under the focus root (every app): nothing to scroll.
        assert_eq!(bar_under_page(true), Vector::ZERO);
        // Without it (an app's `update`), the guess still runs: this layout
        // is exactly the one it got wrong.
        assert_ne!(bar_under_page(false), Vector::ZERO);
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
