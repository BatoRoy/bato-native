//! `Switch` (spec §4.4): 36×20 pill, 16 px white knob with
//! `0 1px 3px #000@.3`, track colour and knob slide over 200 ms. Taps toggle
//! like clicks. Focusable (a click focuses it, Tab reaches it, Enter / Space
//! toggle) with `focusRing`.
//!
//! `.label(…)` makes the row a `<label>`: 14 px `subtext` text 10 px after
//! the switch; a click on it toggles and focuses the switch. `.disabled(…)`:
//! `opacity .5` on the row, no toggling, not focusable, default cursor.

use std::time::Instant;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::{Border, Color, Element, Event, Length, Rectangle, Shadow, Size, Vector, mouse, window};

use crate::anim::{ease_standard, fade};
use crate::focus::{FocusState, Ring, paint_ring};
use crate::style::lerp;

pub struct Switch<'a, Message> {
    on: bool,
    on_toggle: Box<dyn Fn(bool) -> Message + 'a>,
    off_color: Color,
    on_color: Color,
    ring: Option<Ring>,
    id: Option<widget::Id>,
    label: Option<(String, Color)>,
    disabled: bool,
    /// The opaque backdrop, so a faded (disabled) knob composites once.
    over: Option<Color>,
}

/// A switch in the theme's colours (`app-control` off, `app-accent` on).
pub fn switch<'a, Message>(
    cx: &crate::Ctx,
    on: bool,
    f: impl Fn(bool) -> Message + 'a,
) -> Switch<'a, Message> {
    Switch::new(
        on,
        crate::style::c(cx.p().control),
        crate::style::c(cx.p().accent),
        f,
    )
    .focus_ring(Ring::focus(cx))
    .label_color(crate::style::c(cx.p().subtext))
    .over(crate::style::c(cx.over))
}

impl<'a, Message> Switch<'a, Message> {
    pub fn new(
        on: bool,
        off_color: Color,
        on_color: Color,
        f: impl Fn(bool) -> Message + 'a,
    ) -> Self {
        Switch {
            on,
            on_toggle: Box::new(f),
            off_color,
            on_color,
            ring: None,
            id: None,
            label: None,
            disabled: false,
            over: None,
        }
    }
    fn over(mut self, c: Color) -> Self {
        self.over = Some(c);
        self
    }
    /// The `<label>` text after the switch.
    pub fn label(mut self, text: impl Into<String>) -> Self {
        let color = self.label.take().map_or(Color::WHITE, |(_, c)| c);
        self.label = Some((text.into(), color));
        self
    }
    fn label_color(mut self, color: Color) -> Self {
        self.label = Some((String::new(), color));
        self
    }
    /// `disabled`: inert, `opacity .5`.
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    /// The `:focus-visible` ring.
    pub fn focus_ring(mut self, ring: Ring) -> Self {
        self.ring = Some(ring);
        self
    }
    /// For [`crate::focus::focus`].
    pub fn id(mut self, id: impl Into<widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }
}

#[derive(Default)]
struct State {
    initialized: bool,
    shown_on: bool,
    t: f32,
    from: f32,
    started: Option<Instant>,
    pressed: bool,
    finger: Option<u64>,
    focus: FocusState,
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Switch<'_, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(36.0), Length::Fixed(20.0))
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        _r: &iced::Renderer,
        _l: &layout::Limits,
    ) -> layout::Node {
        let st = tree.state.downcast_mut::<State>();
        if !st.initialized {
            st.initialized = true;
            st.shown_on = self.on;
            st.t = if self.on { 1.0 } else { 0.0 };
        }
        layout::Node::new(Size::new(36.0, 20.0))
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _r: &iced::Renderer,
        _c: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_mut::<State>();
        let inside = match event {
            Event::Touch(iced::touch::Event::FingerPressed { position, .. }) => {
                layout.bounds().contains(*position) && viewport.contains(*position)
            }
            _ => cursor.is_over(layout.bounds()),
        };
        let enabled = !self.disabled;
        if st.focus.sync(enabled) | st.focus.pointer(event, inside, enabled) {
            shell.request_redraw();
        }
        st.focus.reveal(layout.bounds(), shell);
        if enabled && st.focus.activation(event, shell) {
            shell.publish((self.on_toggle)(!self.on));
            return;
        }
        if st.shown_on != self.on {
            st.shown_on = self.on;
            st.from = st.t;
            st.started = Some(Instant::now());
            shell.request_redraw();
        }
        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                if let Some(s) = st.started {
                    let target = if self.on { 1.0 } else { 0.0 };
                    let f = (now.saturating_duration_since(s).as_secs_f32() / 0.2).min(1.0);
                    st.t = st.from + (target - st.from) * ease_standard(f);
                    if f >= 1.0 {
                        st.started = None;
                    } else {
                        shell.request_redraw();
                    }
                }
            }
            _ if !enabled => {}
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor.is_over(layout.bounds()) =>
            {
                st.pressed = true;
                shell.capture_event();
            }
            Event::Touch(iced::touch::Event::FingerPressed { id, position })
                if st.finger.is_none() && layout.bounds().contains(*position) =>
            {
                st.finger = Some(id.0);
                shell.capture_event();
            }
            Event::Touch(iced::touch::Event::FingerLifted { id, position })
                if st.finger == Some(id.0) =>
            {
                st.finger = None;
                if layout.bounds().contains(*position) {
                    shell.publish((self.on_toggle)(!self.on));
                }
                shell.capture_event();
            }
            Event::Touch(iced::touch::Event::FingerLost { id, .. }) if st.finger == Some(id.0) => {
                st.finger = None
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if st.pressed => {
                st.pressed = false;
                if cursor.is_over(layout.bounds()) {
                    shell.publish((self.on_toggle)(!self.on));
                    shell.capture_event();
                }
            }
            _ => {}
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        tree.state.downcast_mut::<State>().focus.report(
            operation,
            self.id.as_ref(),
            layout.bounds(),
            !self.disabled,
            true,
        );
    }

    fn mouse_interaction(
        &self,
        _t: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _v: &Rectangle,
        _r: &iced::Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) && !self.disabled {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_ref::<State>();
        let b = crate::scale::snap_rect(layout.bounds());
        if let Some(ring) = &self.ring
            && st.focus.ring_visible()
        {
            paint_ring(renderer, b, 10.0, ring);
        }
        renderer.fill_quad(
            Quad {
                bounds: b,
                border: Border {
                    radius: 10.0.into(),
                    ..Border::default()
                },
                ..Quad::default()
            },
            fade(lerp(self.off_color, self.on_color, st.t)),
        );
        let knob = Rectangle {
            x: b.x + 2.0 + 16.0 * st.t,
            y: b.y + 2.0,
            width: 16.0,
            height: 16.0,
        };
        renderer.fill_quad(
            Quad {
                bounds: knob,
                border: Border {
                    radius: 8.0.into(),
                    ..Border::default()
                },
                shadow: Shadow {
                    color: fade(Color::from_rgba(0.0, 0.0, 0.0, 0.3)),
                    offset: Vector::new(0.0, 1.0),
                    blur_radius: 3.0,
                },
                snap: false,
            },
            // CSS fades the switch as one layer: the knob is white at the
            // group opacity over the backdrop, not over the faded track.
            match self.over {
                Some(bg) if crate::anim::current_opacity() < 1.0 => {
                    lerp(bg, Color::WHITE, crate::anim::current_opacity())
                }
                _ => fade(Color::WHITE),
            },
        );
    }
}

impl<'a, Message: Clone + 'a> From<Switch<'a, Message>>
    for Element<'a, Message, iced::Theme, iced::Renderer>
{
    fn from(mut s: Switch<'a, Message>) -> Self {
        let label = s.label.take().filter(|(t, _)| !t.is_empty());
        let disabled = s.disabled;
        let Some((text, color)) = label else {
            return if disabled {
                crate::anim::opacity(0.5, Element::new(s)).into()
            } else {
                Element::new(s)
            };
        };
        let toggle = (!disabled).then(|| (s.on_toggle)(!s.on));
        let row = crate::surface::surface(
            iced::widget::row![
                Element::new(s),
                crate::label::label(text, 14.0, 20.0).color(color)
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center),
        )
        .on_press_maybe(toggle)
        .focusable(false)
        .labels_child()
        .cursor(if disabled {
            crate::cursor::Cursor::Default
        } else {
            crate::cursor::Cursor::Pointer
        });
        if disabled {
            crate::anim::opacity(0.5, row).into()
        } else {
            row.into()
        }
    }
}
