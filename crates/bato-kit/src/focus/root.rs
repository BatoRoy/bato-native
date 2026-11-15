//! The focus root: wraps the whole window. It tracks the input modality,
//! moves focus on Tab / Shift+Tab (after the widgets had their chance to use
//! the key), serves focus requests (scope autofocus and restore) and runs the
//! `BATO_SELFTEST=focus` script.
//!
//! **Self-test.** `BATO_SELFTEST=focus:<steps>` (steps separated by `,`)
//! plays keys and clicks through this widget's own `update` (the same path a
//! real event takes) once the window has been up for 800 ms, one step every
//! 150 ms, logging the focused widget after each step to stderr:
//!
//! | Step | Event |
//! |---|---|
//! | `tab`, `shift-tab`, `tab*N` | Tab key press and release |
//! | `space`, `enter`, `esc`, `left`, `right`, `up`, `down` | that key |
//! | `click=X;Y` | left press and release at window point X,Y, then the cursor leaves |
//! | `focus=ID` | programmatic focus of the widget with that id |
//! | `wait` | nothing, one step |
//!
//! `BATO_SELFTEST=focus` alone is `tab*3`.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Id, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::keyboard::{self, Key, Location, Modifiers, key::Code, key::Named, key::Physical};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, mouse, window};

use super::{
    Apply, Collect, Entry, Request, Target, begin_press, bump, end_press, is_modifier, plain, plan,
    set_keyboard_modality, take_requests,
};

pub struct FocusRoot<'a, M> {
    content: Element<'a, M, iced::Theme, iced::Renderer>,
    /// The window, for laying out overlays (anchored panels) to operate on.
    viewport: Rectangle,
}

/// Wrap the window's content (done by [`crate::frame::Frame::view`]).
pub fn focus_root<'a, M>(
    content: impl Into<Element<'a, M, iced::Theme, iced::Renderer>>,
) -> FocusRoot<'a, M> {
    FocusRoot {
        content: content.into(),
        viewport: Rectangle::with_size(Size::INFINITE),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Step {
    Key(Named, bool),
    Click(Point),
    Focus(String),
    Wait,
}

/// Parse a `BATO_SELFTEST=focus[:steps]` value.
pub(crate) fn parse_script(v: &str) -> Option<Vec<Step>> {
    let rest = v.strip_prefix("focus")?;
    let steps = match rest.strip_prefix(':') {
        Some(s) => s,
        None if rest.is_empty() => "tab*3",
        None => return None,
    };
    let mut out = Vec::new();
    for raw in steps.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (word, times) = match raw.split_once('*') {
            Some((w, n)) => (w, n.parse::<usize>().ok()?),
            None => (raw, 1),
        };
        let step = if let Some(id) = word.strip_prefix("focus=") {
            Step::Focus(id.to_string())
        } else if let Some(xy) = word.strip_prefix("click=") {
            let (x, y) = xy.split_once(';')?;
            Step::Click(Point::new(x.parse().ok()?, y.parse().ok()?))
        } else {
            match word {
                "tab" => Step::Key(Named::Tab, false),
                "shift-tab" => Step::Key(Named::Tab, true),
                "space" => Step::Key(Named::Space, false),
                "enter" => Step::Key(Named::Enter, false),
                "esc" => Step::Key(Named::Escape, false),
                "left" => Step::Key(Named::ArrowLeft, false),
                "right" => Step::Key(Named::ArrowRight, false),
                "up" => Step::Key(Named::ArrowUp, false),
                "down" => Step::Key(Named::ArrowDown, false),
                "wait" => Step::Wait,
                _ => return None,
            }
        };
        out.extend(std::iter::repeat_n(step, times));
    }
    Some(out)
}

struct Script {
    steps: std::collections::VecDeque<Step>,
    next: Option<Instant>,
}

struct State {
    script: Option<Script>,
    /// This window's identity for the scroll-key rules
    /// ([`crate::scroller::chain::in_surface`]).
    surface: u64,
}

fn key_event(named: Named, shift: bool, pressed: bool) -> Event {
    let key = Key::Named(named);
    let code = match named {
        Named::Tab => Code::Tab,
        Named::Space => Code::Space,
        Named::Enter => Code::Enter,
        Named::Escape => Code::Escape,
        Named::ArrowLeft => Code::ArrowLeft,
        Named::ArrowRight => Code::ArrowRight,
        Named::ArrowUp => Code::ArrowUp,
        _ => Code::ArrowDown,
    };
    let modifiers = if shift {
        Modifiers::SHIFT
    } else {
        Modifiers::empty()
    };
    let text = match named {
        Named::Tab => Some("\t".into()),
        Named::Space => Some(" ".into()),
        Named::Enter => Some("\r".into()),
        _ => None,
    };
    Event::Keyboard(if pressed {
        keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: Physical::Code(code),
            location: Location::Standard,
            modifiers,
            text,
            repeat: false,
        }
    } else {
        keyboard::Event::KeyReleased {
            key: key.clone(),
            modified_key: key,
            physical_key: Physical::Code(code),
            location: Location::Standard,
            modifiers,
        }
    })
}

fn describe(e: Option<&Entry>) -> String {
    match e {
        None => "nothing focused".into(),
        Some(e) => format!(
            "focused {}{} at ({:.0},{:.0} {:.0}x{:.0})",
            e.id.as_ref().map(|i| format!("{i:?} ")).unwrap_or_default(),
            e.token
                .map(|t| format!("token {t}"))
                .unwrap_or_else(|| "(iced widget)".into()),
            e.bounds.x,
            e.bounds.y,
            e.bounds.width,
            e.bounds.height
        ),
    }
}

impl<'a, M> FocusRoot<'a, M> {
    /// [`Widget::update`], inside the window's surface.
    #[allow(clippy::too_many_arguments)]
    fn update_in(
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
        self.viewport = *viewport;
        // Where the focus is, for the scroll keys (the overlays had theirs).
        if crate::scroller::keys::scroll_key(event) {
            let chain = self.focus_chain(tree, layout, renderer, viewport);
            crate::scroller::chain::set_key_focus(match chain {
                Some(c) => crate::scroller::chain::KeyFocus::In(c),
                None => crate::scroller::chain::KeyFocus::Nothing,
            });
        }
        // Modality first, so widgets see the new value while handling it.
        let press = super::is_pointer_press(event);
        match event {
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })
                if !is_modifier(key) && plain(*modifiers) =>
            {
                if set_keyboard_modality(true) {
                    shell.request_redraw();
                }
            }
            _ if press => {
                if set_keyboard_modality(false) {
                    shell.request_redraw();
                }
                begin_press();
            }
            _ => {}
        }
        crate::scroller::chain::in_ui(|| {
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
        });
        if press {
            end_press();
        }
        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Named(Named::Tab),
            modifiers,
            ..
        }) = event
            && plain(*modifiers)
            && !shell.is_event_captured()
        {
            self.tab(tree, layout, renderer, modifiers.shift());
            shell.capture_event();
            shell.request_redraw();
        }
        let requests = take_requests();
        if !requests.is_empty() {
            self.serve(tree, layout, renderer, requests);
            shell.request_redraw();
        }
        // The self-test script, on redraws.
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            let st = tree.state.downcast_mut::<State>();
            let Some(script) = &mut st.script else {
                return;
            };
            let due = *script.next.get_or_insert(*now + Duration::from_millis(800));
            if *now < due {
                shell.request_redraw_at(due);
                return;
            }
            let Some(step) = script.steps.pop_front() else {
                st.script = None;
                eprintln!("focus selftest: done");
                return;
            };
            script.next = Some(*now + Duration::from_millis(150));
            let at = script.next.unwrap();
            self.play(tree, step, layout, renderer, clipboard, shell, viewport);
            shell.request_redraw_at(at);
        }
    }

    /// The kit scrollers around the focused widget (base layer or an
    /// overlay); `None` when nothing has focus.
    fn focus_chain(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
    ) -> Option<Vec<u64>> {
        let mut op = crate::scroller::FocusChain::default();
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, &mut op);
        if op.found.is_some() {
            return op.found;
        }
        let ov = self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            Vector::ZERO,
        )?;
        let mut nested = overlay::Nested::new(ov);
        let node = nested.layout(renderer, layout.bounds().size());
        let mut op = crate::scroller::FocusChain::default();
        nested.operate(Layout::new(&node), renderer, &mut op);
        op.found
    }
    /// Run `op` on the content, then on its overlay (anchored panels come
    /// last in document order, like React portals).
    fn each(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        op: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, op);
        let viewport = self.viewport;
        if let Some(mut ov) = self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            &viewport,
            Vector::ZERO,
        ) {
            let node = ov.as_overlay_mut().layout(renderer, viewport.size());
            ov.as_overlay_mut()
                .operate(Layout::new(&node), renderer, op);
        }
    }

    fn collect(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
    ) -> Collect {
        let mut col = Collect::new(super::ANCHOR.get());
        self.each(tree, layout, renderer, &mut col);
        col
    }

    fn apply(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer, op: Apply) {
        let mut op = op;
        self.each(tree, layout, renderer, &mut op);
    }

    /// Tab / Shift+Tab. Returns whether focus moved.
    fn tab(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        back: bool,
    ) -> bool {
        let col = self.collect(tree, layout, renderer);
        let Some(to) = plan(&col.entries, &col.scopes, col.anchor.as_ref(), back) else {
            return false;
        };
        self.apply(tree, layout, renderer, Apply::new(Some(to)));
        super::ANCHOR.set(None);
        bump();
        true
    }

    fn serve(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        requests: Vec<Request>,
    ) {
        for r in requests {
            let col = self.collect(tree, layout, renderer);
            match r {
                Request::Autofocus {
                    scope,
                    initial,
                    select,
                } => {
                    let opener = col
                        .entries
                        .iter()
                        .rfind(|e| e.focused)
                        .and_then(Entry::target);
                    let inside = |e: &Entry| e.scopes.contains(&scope);
                    let to = match &initial {
                        Some(id) => col
                            .entries
                            .iter()
                            .position(|e| inside(e) && e.id.as_ref() == Some(id)),
                        None => None,
                    }
                    .or_else(|| col.entries.iter().position(|e| inside(e) && e.tab_stop));
                    // Nothing to focus: React focuses the panel itself,
                    // which shows no ring; here that is "nothing focused".
                    self.apply(
                        tree,
                        layout,
                        renderer,
                        Apply::new(to).with_opener(scope, opener),
                    );
                    // Focusing a text input put its cursor at the end;
                    // now the dialog's preselection (rename: the stem).
                    if let (Some(id), Some((start, end))) = (initial, select) {
                        let mut op = super::SelectRange {
                            id,
                            start,
                            end,
                            focus: false,
                        };
                        self.each(tree, layout, renderer, &mut op);
                    }
                    super::ANCHOR.set(None);
                    bump();
                }
                Request::Restore(t) | Request::Focus(t) => {
                    if let Some(i) = col.entries.iter().position(|e| e.matches(&t)) {
                        self.apply(tree, layout, renderer, Apply::new(Some(i)));
                        bump();
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn play(
        &mut self,
        tree: &mut Tree,
        step: Step,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let mut send = |this: &mut Self, tree: &mut Tree, e: Event, cursor: mouse::Cursor| {
            this.update(
                tree, &e, layout, cursor, renderer, clipboard, shell, viewport,
            );
        };
        match &step {
            Step::Key(k, shift) => {
                send(
                    self,
                    tree,
                    key_event(*k, *shift, true),
                    mouse::Cursor::Unavailable,
                );
                send(
                    self,
                    tree,
                    key_event(*k, *shift, false),
                    mouse::Cursor::Unavailable,
                );
            }
            Step::Click(p) => {
                let at = mouse::Cursor::Available(*p);
                send(
                    self,
                    tree,
                    Event::Mouse(mouse::Event::CursorMoved { position: *p }),
                    at,
                );
                send(
                    self,
                    tree,
                    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                    at,
                );
                send(
                    self,
                    tree,
                    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                    at,
                );
                send(
                    self,
                    tree,
                    Event::Mouse(mouse::Event::CursorLeft),
                    mouse::Cursor::Unavailable,
                );
            }
            Step::Focus(id) => {
                let reqs = vec![Request::Focus(Target::Id(Id::from(id.clone())))];
                self.serve(tree, layout, renderer, reqs);
            }
            Step::Wait => {}
        }
        let col = self.collect(tree, layout, renderer);
        eprintln!(
            "focus selftest: {step:?} -> {} (keyboard modality: {})",
            describe(col.entries.iter().rfind(|e| e.focused)),
            super::focus_visible()
        );
        shell.request_redraw();
    }
}

impl<'a, M> Widget<M, iced::Theme, iced::Renderer> for FocusRoot<'a, M> {
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
        let script = std::env::var("BATO_SELFTEST")
            .ok()
            .and_then(|v| parse_script(&v))
            .map(|steps| Script {
                steps: steps.into(),
                next: None,
            });
        tree::State::new(State {
            script,
            surface: crate::scroller::chain::next_surface(),
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
        let surface = tree.state.downcast_ref::<State>().surface;
        crate::scroller::chain::in_surface(surface, || {
            self.update_in(
                tree, event, layout, cursor, renderer, clipboard, shell, viewport,
            );
        });
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
        let surface = tree.state.downcast_ref::<State>().surface;
        crate::scroller::chain::in_surface(surface, || {
            crate::scroller::chain::begin_draw();
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout,
                cursor,
                viewport,
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
    ) -> Option<overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        let surface = tree.state.downcast_ref::<State>().surface;
        self.content
            .as_widget_mut()
            .overlay(
                &mut tree.children[0],
                layout,
                renderer,
                viewport,
                translation,
            )
            .map(|o| crate::scroller::ui_overlay(o, surface))
    }
}

impl<'a, M: 'a> From<FocusRoot<'a, M>> for Element<'a, M, iced::Theme, iced::Renderer> {
    fn from(r: FocusRoot<'a, M>) -> Self {
        Element::new(r)
    }
}
