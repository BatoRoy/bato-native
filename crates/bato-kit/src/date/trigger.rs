//! The pickers' trigger: a `<button>` with `controlClasses` + `sizeClasses`.
//! It looks like a text control, and its focus look is `:focus` (not
//! `:focus-visible`): a click shows the accentBright border and the 4 px
//! halo too, with the 150 ms `transition-[border-color,box-shadow]`. A
//! button otherwise: focusable, Enter on press / Space on release, taps
//! click, `cursor: pointer`.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::{Alignment, Element, Event, Length, Padding, Rectangle, Size, mouse, touch, window};

use crate::anim::ease_standard;
use crate::ctx::Ctx;
use crate::field::{ControlState, FieldSize, control_style};
use crate::focus::FocusState;
use crate::scale::snap_rect;
use crate::style::c;
use crate::surface::paint;

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

const TRANSITION: Duration = Duration::from_millis(150);

pub(crate) struct Trigger<'a, M> {
    cx: Ctx,
    content: El<'a, M>,
    size: FieldSize,
    error: bool,
    on_press: Option<M>,
    id: Option<widget::Id>,
}

pub(crate) fn trigger<'a, M>(cx: &Ctx, content: impl Into<El<'a, M>>) -> Trigger<'a, M> {
    Trigger {
        cx: cx.clone(),
        content: content.into(),
        size: FieldSize::Md,
        error: false,
        on_press: None,
        id: None,
    }
}

impl<'a, M> Trigger<'a, M> {
    pub fn size(mut self, s: FieldSize) -> Self {
        self.size = s;
        self
    }
    pub fn error(mut self, on: bool) -> Self {
        self.error = on;
        self
    }
    pub fn on_press(mut self, m: M) -> Self {
        self.on_press = Some(m);
        self
    }
    pub fn id(mut self, id: Option<widget::Id>) -> Self {
        self.id = id;
        self
    }
}

#[derive(Debug, Default)]
struct State {
    focus: FocusState,
    /// The focus look's target and transition.
    shown: bool,
    from: f32,
    changed: Option<Instant>,
    pressed: bool,
    finger: Option<u64>,
}

impl State {
    fn progress(&self, now: Instant) -> f32 {
        let target = if self.shown { 1.0 } else { 0.0 };
        match self.changed {
            None => target,
            Some(t0) => {
                let k = (now.saturating_duration_since(t0).as_secs_f32()
                    / TRANSITION.as_secs_f32())
                .min(1.0);
                self.from + (target - self.from) * ease_standard(k)
            }
        }
    }
    /// Follow the focus; returns whether a transition started.
    fn sync(&mut self) -> bool {
        let focused = self.focus.is_focused();
        if focused == self.shown {
            return false;
        }
        let now = Instant::now();
        self.from = self.progress(now);
        self.shown = focused;
        self.changed = Some(now);
        true
    }
    fn running(&self, now: Instant) -> bool {
        self.changed
            .is_some_and(|t0| now.saturating_duration_since(t0) < TRANSITION)
    }
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for Trigger<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(self.size.metrics().height()))
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
        let m = self.size.metrics();
        layout::positioned(
            limits,
            Length::Fill,
            Length::Fixed(m.height()),
            Padding::from([0.0, m.pad_x + 1.0]),
            |limits| {
                self.content.as_widget_mut().layout(
                    &mut tree.children[0],
                    renderer,
                    &limits.loose(),
                )
            },
            |content, size| content.align(Alignment::Start, Alignment::Center, size),
        )
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
        let bounds = layout.bounds();
        let st = tree.state.downcast_mut::<State>();
        let over =
            cursor.is_over(bounds) && viewport.contains(cursor.position().unwrap_or_default());
        let inside = match event {
            Event::Touch(touch::Event::FingerPressed { position, .. }) => {
                bounds.contains(*position) && viewport.contains(*position)
            }
            _ => over,
        };
        if st.focus.pointer(event, inside, true) {
            shell.request_redraw();
        }
        st.focus.reveal(bounds, shell);
        if st.sync() {
            shell.request_redraw();
        }
        if let Event::Window(window::Event::RedrawRequested(now)) = event
            && st.running(*now)
        {
            shell.request_redraw();
        }
        if st.focus.activation(event, shell) {
            if let Some(m) = self.on_press.clone() {
                shell.publish(m);
            }
            return;
        }
        if shell.is_event_captured() {
            return;
        }
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if over => {
                st.pressed = true;
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if st.pressed => {
                st.pressed = false;
                if over {
                    if let Some(m) = self.on_press.clone() {
                        shell.publish(m);
                    }
                    shell.capture_event();
                }
            }
            Event::Touch(touch::Event::FingerPressed { id, .. })
                if inside && st.finger.is_none() =>
            {
                st.finger = Some(id.0);
                shell.capture_event();
            }
            Event::Touch(touch::Event::FingerLifted { id, position })
                if st.finger == Some(id.0) =>
            {
                st.finger = None;
                if bounds.contains(*position)
                    && let Some(m) = self.on_press.clone()
                {
                    shell.publish(m);
                }
                shell.capture_event();
            }
            Event::Touch(touch::Event::FingerLost { id, .. }) if st.finger == Some(id.0) => {
                st.finger = None;
            }
            _ => {}
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
        let inner = self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        );
        if inner != mouse::Interaction::None {
            inner
        } else if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_ref::<State>();
        let s = control_style(
            &self.cx,
            ControlState {
                error: self.error,
                disabled: false,
                focus: st.progress(Instant::now()),
            },
        );
        paint(renderer, snap_rect(layout.bounds()), &s);
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            &renderer::Style {
                text_color: c(self.cx.p().text),
            },
            layout.children().next().unwrap(),
            cursor,
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
        tree.state.downcast_mut::<State>().focus.report(
            operation,
            self.id.as_ref(),
            layout.bounds(),
            true,
            true,
        );
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: Clone + 'a> From<Trigger<'a, M>> for El<'a, M> {
    fn from(t: Trigger<'a, M>) -> Self {
        Element::new(t)
    }
}
