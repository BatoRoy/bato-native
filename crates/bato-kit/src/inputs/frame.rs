//! The shared pieces of the B1 inputs.
//!
//! - [`control_frame`]: the `controlClasses` box around *any* content (a row
//!   of a text input and buttons, chips and an input, a text editor), with
//!   the focus look React gets from where the classes sit:
//!   [`Look::Halo`] (`focus:` on the focused element itself: TextArea),
//!   [`Look::Within`] (`focus-within:border-app-accent`: TagsInput) or
//!   [`Look::Plain`] (`focus:` on a wrapper `<div>`, which never matches:
//!   NumberInput, TimeInput). Focus is "anything inside is focused", found
//!   with an operation, so it works for `text_input`, `text_editor` and kit
//!   widgets alike. It also reports focus changes (`onFocus` / `onBlur`),
//!   lets a component see keys first ([`Frame::on_key`]: ↑/↓, Backspace on an
//!   empty field), keeps Esc and middle presses from the text widgets, and
//!   scrolls into view on keyboard focus.
//! - [`plain_input`]: an iced `text_input` styled to sit inside such a box.
//! - [`repeat_button`]: a `tabIndex=-1` icon button that steps on press and
//!   repeats while held ([`HoldRepeat`]): NumberInput's −/+ and TimeInput's
//!   spinners.
//! - [`AnyFocused`], [`focus_nth`]: subtree focus queries and moves.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::widget::operation::{Focusable, Operation};
use iced::advanced::widget::{self, Id, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::widget::text_input;
use iced::{
    Background, Border, Color, Element, Event, Length, Padding, Rectangle, Size, Vector, mouse,
    touch, window,
};

use crate::El;
use crate::anim::{HoldRepeat, ease_standard, with_opacity};
use crate::ctx::Ctx;
use crate::field::{ControlState, DISABLED_OPACITY, control_style};
use crate::fonts;
use crate::style::{c, lerp, rgba};
use crate::surface::{BoxStyle, paint};

/// The 150 ms `transition-[border-color,box-shadow]` / `transition-colors`.
pub const TRANSITION: Duration = Duration::from_millis(150);

/// What focus inside the box does to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Look {
    /// accentBright border + 4 px `accent@.15` halo (red with an error).
    #[default]
    Halo,
    /// `focus-within:border-app-accent`: the border only, no halo.
    Within,
    /// No focus look.
    Plain,
}

/// The box style of `look` at focus progress `t` (0..1).
pub fn frame_style(cx: &Ctx, look: Look, error: bool, t: f32) -> BoxStyle {
    match look {
        Look::Halo => control_style(
            cx,
            ControlState {
                error,
                disabled: false,
                focus: t,
            },
        ),
        Look::Plain => control_style(
            cx,
            ControlState {
                error,
                disabled: false,
                focus: 0.0,
            },
        ),
        Look::Within => {
            let mut s = control_style(
                cx,
                ControlState {
                    error,
                    disabled: false,
                    focus: 0.0,
                },
            );
            if !error && t > 0.0 {
                s.border = Some(lerp(c(cx.p().control), c(cx.p().accent), t));
            }
            s
        }
    }
}

/// Is anything in the subtree focused?
#[derive(Debug, Default)]
pub struct AnyFocused(pub bool);

impl Operation for AnyFocused {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
    fn focusable(&mut self, _id: Option<&Id>, _b: Rectangle, state: &mut dyn Focusable) {
        self.0 |= state.is_focused();
    }
}

/// Which focusable of the subtree is focused (index among its focusables)
/// and how many there are.
#[derive(Debug, Default)]
pub struct WhichFocused {
    pub focused: Option<usize>,
    pub count: usize,
}

impl Operation for WhichFocused {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
    fn focusable(&mut self, _id: Option<&Id>, _b: Rectangle, state: &mut dyn Focusable) {
        if state.is_focused() {
            self.focused = Some(self.count);
        }
        self.count += 1;
    }
}

/// Focus the `n`-th focusable of a subtree and unfocus the others in it
/// (`refs.current[n].focus()` between siblings).
pub fn focus_nth<M>(
    el: &mut El<'_, M>,
    tree: &mut Tree,
    layout: Layout<'_>,
    renderer: &iced::Renderer,
    n: usize,
) {
    let mut op = crate::focus::Apply::new(Some(n));
    el.as_widget_mut().operate(tree, layout, renderer, &mut op);
}

/// Move the caret of every text input in a subtree to the end (what
/// Chromium does when a script sets a different value).
pub struct CaretToEnd;

impl Operation for CaretToEnd {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
    fn text_input(
        &mut self,
        _id: Option<&Id>,
        _b: Rectangle,
        state: &mut dyn widget::operation::TextInput,
    ) {
        state.move_cursor_to_end();
    }
}

/// `true` for Esc presses (never passed to text widgets: Chromium keeps
/// focus and lets the key reach the dialog).
pub fn is_escape(event: &Event) -> bool {
    matches!(
        event,
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Named(Named::Escape),
            ..
        })
    )
}

fn is_middle_press(event: &Event) -> bool {
    matches!(
        event,
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle))
    )
}

/// Events after which focus may have moved.
fn may_move_focus(event: &Event) -> bool {
    !matches!(
        event,
        Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::WheelScrolled { .. })
            | Event::Touch(touch::Event::FingerMoved { .. })
    )
}

// ── Plain input ─────────────────────────────────────────────────────────

/// The colours of a text inside a control (`text_input` ignores group
/// opacity: disabled text is faded by hand over the backdrop).
pub fn text_style(cx: &Ctx, disabled: bool) -> text_input::Style {
    let p = cx.p();
    let fade = |rgb| {
        if disabled {
            c(cx.blend(rgb, DISABLED_OPACITY as f64))
        } else {
            c(rgb)
        }
    };
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border::default(),
        icon: fade(p.muted),
        placeholder: fade(p.muted),
        value: fade(p.text),
        selection: rgba(p.accent, 0.35),
    }
}

/// A borderless, transparent `text_input` (Inter, `size`/`line` px) for use
/// inside a [`control_frame`].
pub fn plain_input<'a, M: Clone + 'a>(
    cx: &Ctx,
    placeholder: &str,
    value: &str,
    size: f32,
    line: f32,
    disabled: bool,
) -> text_input::TextInput<'a, M, iced::Theme, iced::Renderer> {
    let style = text_style(cx, disabled);
    text_input::TextInput::new(placeholder, value)
        .font(fonts::sans())
        .size(size)
        .line_height(iced::widget::text::LineHeight::Absolute(line.into()))
        .padding(0)
        .style(move |_t, _s| style)
}

// ── Control frame ───────────────────────────────────────────────────────

/// Where focus is inside a frame when a key arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Within {
    /// The focused one among the frame's focusables (document order).
    pub index: usize,
    pub count: usize,
}

impl Within {
    /// The last focusable (the text input after a row of chips).
    pub fn is_last(&self) -> bool {
        self.index + 1 == self.count
    }
}

type KeyHook<'a, M> = Box<dyn Fn(&Key, Modifiers, Within) -> Option<M> + 'a>;

pub struct Frame<'a, M> {
    cx: Ctx,
    content: El<'a, M>,
    look: Look,
    error: bool,
    disabled: bool,
    disabled_opacity: f32,
    width: Length,
    padding: Padding,
    min_height: f32,
    on_focus: Option<Box<dyn Fn(bool) -> Option<M> + 'a>>,
    on_key: Option<KeyHook<'a, M>>,
}

/// The control box around `content` (laid out inside `padding`, which
/// includes the 1 px border).
pub fn control_frame<'a, M: 'a>(cx: &Ctx, content: impl Into<El<'a, M>>) -> Frame<'a, M> {
    Frame {
        cx: cx.clone(),
        content: content.into(),
        look: Look::Halo,
        error: false,
        disabled: false,
        disabled_opacity: DISABLED_OPACITY,
        width: Length::Fill,
        padding: Padding::new(1.0),
        min_height: 0.0,
        on_focus: None,
        on_key: None,
    }
}

impl<'a, M: 'a> Frame<'a, M> {
    pub fn look(mut self, l: Look) -> Self {
        self.look = l;
        self
    }
    pub fn error(mut self, on: bool) -> Self {
        self.error = on;
        self
    }
    /// The whole box at `opacity` (default `.6`), `not-allowed`.
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn disabled_opacity(mut self, a: f32) -> Self {
        self.disabled_opacity = a;
        self
    }
    pub fn width(mut self, w: impl Into<Length>) -> Self {
        self.width = w.into();
        self
    }
    /// Border + padding around the content.
    pub fn padding(mut self, p: impl Into<Padding>) -> Self {
        self.padding = p.into();
        self
    }
    /// Border-box minimum height.
    pub fn min_height(mut self, h: f32) -> Self {
        self.min_height = h;
        self
    }
    /// Focus entered (`true`) or left (`false`) the box.
    pub fn on_focus(mut self, f: impl Fn(bool) -> Option<M> + 'a) -> Self {
        self.on_focus = Some(Box::new(f));
        self
    }
    /// Keys pressed while something inside is focused, before the content
    /// sees them; `Some` publishes and swallows the key.
    pub fn on_key(mut self, f: impl Fn(&Key, Modifiers, Within) -> Option<M> + 'a) -> Self {
        self.on_key = Some(Box::new(f));
        self
    }
}

#[derive(Debug)]
struct FrameState {
    focused: bool,
    from: f32,
    changed: Option<Instant>,
    token: u64,
}

impl Default for FrameState {
    fn default() -> Self {
        Self {
            focused: false,
            from: 0.0,
            changed: None,
            token: crate::focus::next_token(),
        }
    }
}

impl FrameState {
    fn progress(&self, now: Instant) -> f32 {
        let target = if self.focused { 1.0 } else { 0.0 };
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
    fn running(&self, now: Instant) -> bool {
        self.changed
            .is_some_and(|t0| now.saturating_duration_since(t0) < TRANSITION)
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for Frame<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Shrink)
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<FrameState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(FrameState::default())
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
        let pad = self.padding;
        let limits = limits.width(self.width);
        let inner = limits.shrink(pad);
        let n = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &inner);
        let w = match self.width {
            Length::Shrink => n.size().width + pad.left + pad.right,
            _ => limits.max().width,
        };
        let h = (n.size().height + pad.top + pad.bottom).max(self.min_height);
        let n = n.move_to(iced::Point::new(pad.left, pad.top));
        layout::Node::with_children(Size::new(w, h), vec![n])
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
        let bounds = layout.bounds();
        // `data-batoscroll-ignore`: a middle press on a text field is the
        // field's (primary paste), never an autoscroll.
        if is_middle_press(event) && cursor.is_over(bounds) {
            shell.capture_event();
            return;
        }
        if crate::focus::is_pointer_press(event) && cursor.is_over(bounds) {
            crate::focus::note_press(tree.state.downcast_ref::<FrameState>().token);
        }
        let child = layout.children().next().unwrap();
        if let (Some(hook), Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })) =
            (&self.on_key, event)
            && tree.state.downcast_ref::<FrameState>().focused
            && !shell.is_event_captured()
        {
            let mut w = WhichFocused::default();
            self.content
                .as_widget_mut()
                .operate(&mut tree.children[0], child, renderer, &mut w);
            if let Some(index) = w.focused
                && let Some(m) = hook(
                    key,
                    *modifiers,
                    Within {
                        index,
                        count: w.count,
                    },
                )
            {
                shell.publish(m);
                shell.capture_event();
                return;
            }
        }
        if !is_escape(event) {
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
        }
        let now = Instant::now();
        if may_move_focus(event) {
            let mut op = AnyFocused::default();
            self.content
                .as_widget_mut()
                .operate(&mut tree.children[0], child, renderer, &mut op);
            let st = tree.state.downcast_mut::<FrameState>();
            if op.0 != st.focused {
                if op.0 && !crate::focus::is_pointer_press(event) {
                    crate::scroller::focus_into_view(bounds);
                }
                st.from = st.progress(now);
                st.focused = op.0;
                st.changed = Some(now);
                shell.request_redraw();
                if let Some(m) = self.on_focus.as_ref().and_then(|f| f(op.0)) {
                    shell.publish(m);
                }
            }
        }
        if let Event::Window(window::Event::RedrawRequested(at)) = event
            && tree.state.downcast_ref::<FrameState>().running(*at)
        {
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
        if self.disabled {
            return if cursor.is_over(layout.bounds()) {
                mouse::Interaction::NotAllowed
            } else {
                mouse::Interaction::None
            };
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
        let token = tree.state.downcast_ref::<FrameState>().token;
        operation.custom(None, layout.bounds(), &mut crate::focus::Anchor(token));
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
        let raw = layout.bounds();
        let b = crate::scale::snap_rect(raw);
        let shift = Vector::new(b.x - raw.x, b.y - raw.y);
        let st = tree.state.downcast_ref::<FrameState>();
        let t = st.progress(Instant::now());
        let boxed = frame_style(&self.cx, self.look, self.error, t);
        let alpha = if self.disabled {
            self.disabled_opacity
        } else {
            1.0
        };
        with_opacity(alpha, || {
            renderer.with_translation(shift, |renderer| {
                paint(renderer, raw, &boxed);
                self.content.as_widget().draw(
                    &tree.children[0],
                    renderer,
                    theme,
                    style,
                    layout.children().next().unwrap(),
                    cursor - shift,
                    viewport,
                );
            });
        });
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
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

impl<'a, M: 'a> From<Frame<'a, M>> for El<'a, M> {
    fn from(f: Frame<'a, M>) -> Self {
        Element::new(f)
    }
}

// ── Repeat button ───────────────────────────────────────────────────────

/// A `tabIndex=-1` button that fires on press and repeats while held.
pub struct RepeatButton<'a, M> {
    content: El<'a, M>,
    on_step: Option<M>,
    rest: Color,
    hover: Color,
    width: Length,
    height: Length,
    transition: bool,
    repeat: bool,
    disabled_opacity: f32,
}

/// `content` (an icon drawn in the inherited colour) as a stepping button:
/// `rest` colour, `hover` colour under the pointer.
pub fn repeat_button<'a, M: Clone + 'a>(
    content: impl Into<El<'a, M>>,
    on_step: Option<M>,
    rest: Color,
    hover: Color,
) -> RepeatButton<'a, M> {
    RepeatButton {
        content: content.into(),
        on_step,
        rest,
        hover,
        width: Length::Shrink,
        height: Length::Shrink,
        transition: false,
        repeat: true,
        disabled_opacity: 1.0,
    }
}

impl<'a, M: Clone + 'a> RepeatButton<'a, M> {
    pub fn width(mut self, w: impl Into<Length>) -> Self {
        self.width = w.into();
        self
    }
    pub fn height(mut self, h: impl Into<Length>) -> Self {
        self.height = h.into();
        self
    }
    /// `transition-colors` (150 ms).
    pub fn transition(mut self) -> Self {
        self.transition = true;
        self
    }
    /// Hold-to-repeat (default on); off = one step per press.
    pub fn repeat(mut self, on: bool) -> Self {
        self.repeat = on;
        self
    }
    /// Opacity while disabled (`disabled:opacity-40`).
    pub fn disabled_opacity(mut self, a: f32) -> Self {
        self.disabled_opacity = a;
        self
    }
}

#[derive(Debug, Default)]
struct RepeatState {
    hovered: bool,
    t: f32,
    from: f32,
    started: Option<Instant>,
    held: Option<HoldRepeat>,
    finger: Option<u64>,
}

impl RepeatState {
    fn advance(&mut self, now: Instant) -> bool {
        let Some(s) = self.started else { return false };
        let target = if self.hovered { 1.0 } else { 0.0 };
        let k =
            (now.saturating_duration_since(s).as_secs_f32() / TRANSITION.as_secs_f32()).min(1.0);
        self.t = self.from + (target - self.from) * ease_standard(k);
        if k >= 1.0 {
            self.started = None;
            false
        } else {
            true
        }
    }
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for RepeatButton<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<RepeatState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(RepeatState::default())
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
        layout::positioned(
            limits,
            self.width,
            self.height,
            Padding::ZERO,
            |l| {
                self.content
                    .as_widget_mut()
                    .layout(&mut tree.children[0], renderer, &l.loose())
            },
            |n, size| n.align(iced::Alignment::Center, iced::Alignment::Center, size),
        )
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_mut::<RepeatState>();
        let b = layout.bounds();
        let enabled = self.on_step.is_some();
        let over =
            enabled && cursor.is_over(b) && cursor.position().is_some_and(|p| viewport.contains(p));
        let start = |st: &mut RepeatState, shell: &mut Shell<'_, M>| {
            if let Some(m) = &self.on_step {
                shell.publish(m.clone());
            }
            if self.repeat {
                let now = Instant::now();
                st.held = Some(HoldRepeat::press(now));
                shell.request_redraw_at(now + HoldRepeat::DELAY);
            }
            shell.capture_event();
        };
        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                if self.transition && st.advance(*now) {
                    shell.request_redraw();
                }
                if let Some(h) = &mut st.held {
                    let (n, next) = h.due(*now);
                    if let Some(m) = &self.on_step {
                        for _ in 0..n {
                            shell.publish(m.clone());
                        }
                    } else {
                        st.held = None;
                    }
                    if st.held.is_some() {
                        shell.request_redraw_at(next);
                    }
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                if over != st.hovered {
                    st.hovered = over;
                    st.from = st.t;
                    if self.transition {
                        st.started = Some(Instant::now());
                    } else {
                        st.t = if over { 1.0 } else { 0.0 };
                    }
                    shell.request_redraw();
                }
                // `onPointerLeave` stops the repeat.
                if !over && st.finger.is_none() {
                    st.held = None;
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if over => {
                start(st, shell);
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if st.held.take().is_some() && over {
                    shell.capture_event();
                }
            }
            Event::Touch(touch::Event::FingerPressed { id, position })
                if enabled && b.contains(*position) && viewport.contains(*position) =>
            {
                st.finger = Some(id.0);
                start(st, shell);
            }
            Event::Touch(
                touch::Event::FingerLifted { id, .. } | touch::Event::FingerLost { id, .. },
            ) if st.finger == Some(id.0) => {
                st.finger = None;
                st.held = None;
                shell.capture_event();
            }
            _ => {}
        }
        if !enabled {
            st.held = None;
        }
    }
    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Idle
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
        let st = tree.state.downcast_ref::<RepeatState>();
        let enabled = self.on_step.is_some();
        let color = if enabled {
            lerp(self.rest, self.hover, st.t)
        } else {
            self.rest
        };
        let alpha = if enabled { 1.0 } else { self.disabled_opacity };
        with_opacity(alpha, || {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                &renderer::Style { text_color: color },
                layout.children().next().unwrap(),
                cursor,
                viewport,
            );
        });
    }
}

impl<'a, M: Clone + 'a> From<RepeatButton<'a, M>> for El<'a, M> {
    fn from(b: RepeatButton<'a, M>) -> Self {
        Element::new(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bato_theme::{Rgb, ThemeId, palette};

    fn cx() -> Ctx {
        Ctx::from_palette(palette(ThemeId::Dark, Rgb::hex("#8b5cf6").unwrap())).on_card()
    }

    #[test]
    fn looks() {
        let cx = cx();
        let p = cx.p().clone();
        let halo = frame_style(&cx, Look::Halo, false, 1.0);
        assert_eq!(halo.border, Some(c(p.accent_bright)));
        assert_eq!(halo.rings.len(), 1);
        let within = frame_style(&cx, Look::Within, false, 1.0);
        assert_eq!(
            within.border,
            Some(c(p.accent)),
            "focus-within:border-app-accent"
        );
        assert!(within.rings.is_empty(), "no halo on focus-within");
        let plain = frame_style(&cx, Look::Plain, false, 1.0);
        assert_eq!(plain.border, Some(c(p.control)));
        assert!(plain.rings.is_empty());
        let err = frame_style(&cx, Look::Within, true, 1.0);
        assert_eq!(err.border, Some(c(p.red)));
    }
}
