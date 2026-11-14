//! The anchored overlay (INF-3): content drawn in iced's overlay layer (the
//! React portal), placed against an anchor element's bounds measured on
//! layout. One widget backs Popover, Dropdown, ContextMenu and Tooltip; they
//! differ only in [`Position`], [`Enter`] and [`Dismiss`].
//!
//! ```ignore
//! // Popover (controlled, like React's `open` / `onClose`)
//! anchored(
//!     button(&cx).secondary().label("Popover").on_press(Msg::TogglePop),
//!     state.pop.then(|| panel(&cx, body)),
//! )
//! .position(Position::Float(floating::Options::default()))
//! .on_dismiss(|_| Msg::ClosePop)
//!
//! // Context menu: right-click anywhere in `area` opens at the pointer
//! context_area(area, Msg::OpenMenu)
//!     .menu(state.menu_at, menu_list(&cx, items).on_close(Msg::CloseMenu), |_| Msg::CloseMenu)
//! ```
//!
//! **Positioning** is recomputed on every layout from the anchor's current
//! window bounds ([`crate::floating`]), so the panel follows its anchor on
//! scroll and resize when it is not dismissed by them.
//!
//! **Dismissal** ([`Dismiss`], `useDismiss`): a press outside the panel (and
//! outside the anchor, unless [`Dismiss::ignore_anchor`] is off), Escape,
//! a scroll of an ancestor scroller (the anchor's scroll translation
//! changed), a window resize, the window losing focus. Every one is
//! reported as a [`DismissReason`] to `on_dismiss`. The press is not
//! swallowed: it still reaches what is under it, like the browser.
//! **Focus leaving** the panel ([`DismissReason::FocusOut`]) is the focus
//! system's call: the panel is a container with [`Anchored::id`], so a focus
//! operation can tell whether the focused widget is inside it, and the app
//! dismisses with that reason.
//!
//! **Enter animation**: [`Enter::ScaleIn`] (Popover, menus: scale .96→1 and
//! opacity 0→1, 120 ms ease-out, origin centre) or [`Enter::FadeIn`] (Tooltip:
//! 150 ms). No exit animation (React unmounts at once). Redraws are
//! requested only while the animation runs.

use std::time::Instant;

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::{
    Element, Event, Length, Point, Rectangle, Size, Transformation, Vector, keyboard, mouse, touch,
    window,
};

use crate::anim::{ease_out, with_opacity};
use crate::floating::{self, clamp_to_viewport, compute_position};
use crate::scale::device_scale;

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// Where the panel goes relative to the anchor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Position {
    /// `computePosition`: side, alignment, offset, flip, then clamp
    /// (Popover, Tooltip, pickers).
    Float(floating::Options),
    /// Dropdown: `gap` px under the anchor, its left (or right) edge aligned,
    /// clamped to the window; never flips.
    Below { align_right: bool, gap: f32 },
    /// ContextMenu: the top-left at this point, relative to the anchor's
    /// top-left (the pointer), clamped to the window.
    At(Point),
}

impl Default for Position {
    fn default() -> Self {
        Position::Float(floating::Options::default())
    }
}

/// The enter animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Enter {
    /// `animate-scale-in`: scale .96→1 + opacity 0→1, 120 ms ease-out.
    #[default]
    ScaleIn,
    /// `animate-fade-in`: opacity 0→1, 150 ms ease-out.
    FadeIn,
    None,
}

impl Enter {
    fn duration(self) -> f32 {
        match self {
            Enter::ScaleIn => 0.120,
            Enter::FadeIn => 0.150,
            Enter::None => 0.0,
        }
    }
}

/// Which gestures close the panel (`useDismiss` options).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dismiss {
    /// A press outside the panel.
    pub outside: bool,
    /// Presses on the anchor do not count as outside (the trigger toggles).
    pub ignore_anchor: bool,
    pub escape: bool,
    /// An ancestor scroller moved the anchor.
    pub scroll: bool,
    pub resize: bool,
    /// The window lost focus.
    pub blur: bool,
}

impl Dismiss {
    /// `useDismiss` defaults: Popover.
    pub const POPOVER: Dismiss = Dismiss {
        outside: true,
        ignore_anchor: true,
        escape: true,
        scroll: true,
        resize: true,
        blur: true,
    };
    /// `Dropdown.tsx`: everything but window blur.
    pub const DROPDOWN: Dismiss = Dismiss {
        blur: false,
        ..Dismiss::POPOVER
    };
    /// `ContextMenu.tsx`: a press anywhere outside the menu, the area included.
    pub const CONTEXT_MENU: Dismiss = Dismiss {
        ignore_anchor: false,
        ..Dismiss::POPOVER
    };
    /// Tooltip: the owner opens and closes it.
    pub const NONE: Dismiss = Dismiss {
        outside: false,
        ignore_anchor: true,
        escape: false,
        scroll: false,
        resize: false,
        blur: false,
    };
}

impl Default for Dismiss {
    fn default() -> Self {
        Dismiss::POPOVER
    }
}

/// Why the panel asked to close.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DismissReason {
    OutsidePress,
    Escape,
    Scroll,
    Resize,
    Blur,
    /// Keyboard focus left the panel (reported by the focus system).
    FocusOut,
}

/// The pure dismissal rule for one event (unit-tested). `panel` and
/// `anchor` are window bounds; `pos` the pointer for presses.
pub fn dismissal(
    policy: &Dismiss,
    event: &Event,
    pos: Option<Point>,
    panel: Rectangle,
    anchor: Rectangle,
) -> Option<DismissReason> {
    let outside = |p: Point| !panel.contains(p) && !(policy.ignore_anchor && anchor.contains(p));
    match event {
        Event::Mouse(mouse::Event::ButtonPressed(_)) if policy.outside => pos
            .filter(|p| outside(*p))
            .map(|_| DismissReason::OutsidePress),
        Event::Touch(touch::Event::FingerPressed { position, .. })
            if policy.outside && outside(*position) =>
        {
            Some(DismissReason::OutsidePress)
        }
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            ..
        }) if policy.escape => Some(DismissReason::Escape),
        Event::Window(window::Event::Resized(_)) if policy.resize => Some(DismissReason::Resize),
        Event::Window(window::Event::Unfocused) if policy.blur => Some(DismissReason::Blur),
        _ => None,
    }
}

/// Animation and scroll-tracking state of an open panel.
#[derive(Debug, Default)]
pub struct FloatState {
    opened: Option<Instant>,
    progress: f32,
    /// The anchor's scroll translation when the panel opened.
    origin: Option<Vector>,
    /// A dismissal was reported; don't repeat it until the owner closes.
    dismissed: bool,
}

impl FloatState {
    fn reset(&mut self) {
        *self = FloatState::default();
    }
}

/// An element with a floating panel anchored to it.
pub struct Anchored<'a, M> {
    anchor: El<'a, M>,
    panel: Option<El<'a, M>>,
    position: Position,
    enter: Enter,
    dismiss: Dismiss,
    on_dismiss: Option<Box<dyn Fn(DismissReason) -> M + 'a>>,
    on_context: Option<Box<dyn Fn(Point) -> M + 'a>>,
    match_width: bool,
    hit_test: bool,
    animate: bool,
    id: Option<widget::Id>,
}

/// `anchor` with `panel` floating next to it while `panel` is `Some`.
pub fn anchored<'a, M>(anchor: impl Into<El<'a, M>>, panel: Option<El<'a, M>>) -> Anchored<'a, M> {
    Anchored {
        anchor: anchor.into(),
        panel,
        position: Position::default(),
        enter: Enter::ScaleIn,
        dismiss: Dismiss::POPOVER,
        on_dismiss: None,
        on_context: None,
        match_width: false,
        hit_test: true,
        animate: true,
        id: None,
    }
}

/// `useContextMenu`: a right press anywhere in `area` publishes
/// `on_open(point)`, the pointer relative to the area's top-left. Show the
/// menu with [`Anchored::menu`]; it opens at that point, clamped to the
/// window, and closes on [`Dismiss::CONTEXT_MENU`] gestures.
pub fn context_area<'a, M>(
    area: impl Into<El<'a, M>>,
    on_open: impl Fn(Point) -> M + 'a,
) -> Anchored<'a, M> {
    let mut a = anchored(area, None);
    a.dismiss = Dismiss::CONTEXT_MENU;
    a.on_context = Some(Box::new(on_open));
    a
}

impl<'a, M> Anchored<'a, M> {
    pub fn position(mut self, p: Position) -> Self {
        self.position = p;
        self
    }
    /// Shorthand for `Position::Float(opts)`.
    pub fn float(self, opts: floating::Options) -> Self {
        self.position(Position::Float(opts))
    }
    pub fn enter(mut self, e: Enter) -> Self {
        self.enter = e;
        self
    }
    pub fn dismiss(mut self, d: Dismiss) -> Self {
        self.dismiss = d;
        self
    }
    pub fn on_dismiss(mut self, f: impl Fn(DismissReason) -> M + 'a) -> Self {
        self.on_dismiss = Some(Box::new(f));
        self
    }
    /// `matchWidth`: the panel is at least as wide as the anchor.
    pub fn match_width(mut self, on: bool) -> Self {
        self.match_width = on;
        self
    }
    /// `pointer-events: none` (Tooltip): the panel never takes the pointer.
    pub fn hit_test(mut self, on: bool) -> Self {
        self.hit_test = on;
        self
    }
    /// `false` skips the enter animation (screenshots).
    pub fn animate(mut self, on: bool) -> Self {
        self.animate = on;
        self
    }
    /// The panel's container id (focus scoping, tests).
    pub fn id(mut self, id: widget::Id) -> Self {
        self.id = Some(id);
        self
    }
    /// For a [`context_area`]: the open menu at the area-relative point
    /// `at` (`None` = closed).
    pub fn menu(
        mut self,
        at: Option<Point>,
        menu: impl Into<El<'a, M>>,
        on_dismiss: impl Fn(DismissReason) -> M + 'a,
    ) -> Self {
        if let Some(p) = at {
            self.position = Position::At(p);
            self.panel = Some(menu.into());
        } else {
            self.panel = None;
        }
        self.on_dismiss = Some(Box::new(on_dismiss));
        self
    }
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for Anchored<'a, M> {
    fn size(&self) -> Size<Length> {
        self.anchor.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.anchor.as_widget().size_hint()
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<FloatState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(FloatState::default())
    }
    fn children(&self) -> Vec<Tree> {
        let mut v = vec![Tree::new(&self.anchor)];
        if let Some(p) = &self.panel {
            v.push(Tree::new(p));
        }
        v
    }
    fn diff(&self, tree: &mut Tree) {
        match &self.panel {
            Some(p) => tree.diff_children(&[&self.anchor, p]),
            None => {
                tree.diff_children(std::slice::from_ref(&self.anchor));
                tree.state.downcast_mut::<FloatState>().reset();
            }
        }
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
        self.anchor.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        if shell.is_event_captured() {
            return;
        }
        let Some(on_context) = &self.on_context else {
            return;
        };
        let b = layout.bounds();
        let at = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                cursor.position_over(b)
            }
            _ => None,
        };
        if let Some(p) = at {
            shell.publish(on_context(Point::new(p.x - b.x, p.y - b.y)));
            shell.capture_event();
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
        let anchor_bounds = layout.bounds() + translation;
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
        let mine = match (&mut self.panel, rest.first_mut()) {
            (Some(panel), Some(panel_tree)) => {
                let on_dismiss = self.on_dismiss.as_deref();
                Some(overlay::Element::new(Box::new(Floating {
                    content: panel,
                    tree: panel_tree,
                    state: state.downcast_mut::<FloatState>(),
                    anchor: anchor_bounds,
                    translation,
                    position: self.position,
                    enter: if self.animate {
                        self.enter
                    } else {
                        Enter::None
                    },
                    dismiss: self.dismiss,
                    on_dismiss: on_dismiss.map(|f| f as &dyn Fn(DismissReason) -> M),
                    match_width: self.match_width,
                    hit_test: self.hit_test,
                    id: self.id.as_ref(),
                })))
            }
            _ => None,
        };
        match (inner, mine) {
            (Some(a), Some(b)) => Some(overlay::Group::with_children(vec![a, b]).overlay()),
            (a, b) => a.or(b),
        }
    }
}

impl<'a, M: Clone + 'a> From<Anchored<'a, M>> for El<'a, M> {
    fn from(a: Anchored<'a, M>) -> Self {
        Element::new(a)
    }
}

/// Where a panel of `size` goes, in window coordinates, snapped to device
/// pixels.
pub fn place(position: Position, anchor: Rectangle, size: Size, window: Size) -> Point {
    let p = match position {
        Position::Float(o) => compute_position(anchor, size, o, window).point(),
        Position::Below { align_right, gap } => {
            let x = if align_right {
                anchor.x + anchor.width - size.width
            } else {
                anchor.x
            };
            clamp_to_viewport(
                x,
                anchor.y + anchor.height + gap,
                size.width,
                size.height,
                8.0,
                window,
            )
        }
        Position::At(p) => clamp_to_viewport(
            anchor.x + p.x,
            anchor.y + p.y,
            size.width,
            size.height,
            8.0,
            window,
        ),
    };
    let s = device_scale();
    Point::new((p.x * s).round() / s, (p.y * s).round() / s)
}

/// The panel in the overlay layer.
pub struct Floating<'a, 'b, M> {
    pub content: &'b mut El<'a, M>,
    pub tree: &'b mut Tree,
    pub state: &'b mut FloatState,
    /// The anchor in window coordinates.
    pub anchor: Rectangle,
    pub translation: Vector,
    pub position: Position,
    pub enter: Enter,
    pub dismiss: Dismiss,
    pub on_dismiss: Option<&'b dyn Fn(DismissReason) -> M>,
    pub match_width: bool,
    pub hit_test: bool,
    pub id: Option<&'b widget::Id>,
}

impl<M> Floating<'_, '_, M> {
    fn report(&mut self, reason: DismissReason, shell: &mut Shell<'_, M>) {
        if self.state.dismissed {
            return;
        }
        if let Some(f) = self.on_dismiss {
            self.state.dismissed = true;
            shell.publish(f(reason));
        }
    }

    /// Opacity and scale of the enter animation.
    fn look(&self) -> (f32, f32) {
        let p = if self.enter == Enter::None {
            1.0
        } else if self.state.opened.is_none() {
            0.0
        } else {
            self.state.progress
        };
        match self.enter {
            Enter::ScaleIn => (p, 0.96 + 0.04 * p),
            Enter::FadeIn => (p, 1.0),
            Enter::None => (1.0, 1.0),
        }
    }
}

impl<M: Clone> overlay::Overlay<M, iced::Theme, iced::Renderer> for Floating<'_, '_, M> {
    fn layout(&mut self, renderer: &iced::Renderer, bounds: Size) -> layout::Node {
        let min_w = if self.match_width {
            self.anchor.width
        } else {
            0.0
        };
        let limits = layout::Limits::new(Size::new(min_w.min(bounds.width), 0.0), bounds);
        let node = self
            .content
            .as_widget_mut()
            .layout(self.tree, renderer, &limits);
        let size = node.size();
        let at = place(self.position, self.anchor, size, bounds);
        layout::Node::with_children(size, vec![node]).move_to(at)
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
    ) {
        let panel = layout.bounds();
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            let opened = *self.state.opened.get_or_insert(*now);
            let d = self.enter.duration();
            let t = now.saturating_duration_since(opened).as_secs_f32();
            self.state.progress = if d <= 0.0 {
                1.0
            } else {
                ease_out((t / d).min(1.0))
            };
            if t < d {
                shell.request_redraw();
            }
        }
        // An ancestor scrolled: the anchor's translation moved.
        match self.state.origin {
            None => self.state.origin = Some(self.translation),
            Some(o) if self.dismiss.scroll && o != self.translation => {
                self.report(DismissReason::Scroll, shell);
            }
            _ => {}
        }
        let is_pointer = matches!(event, Event::Mouse(_) | Event::Touch(_));
        if self.hit_test || !is_pointer {
            self.content.as_widget_mut().update(
                self.tree,
                event,
                layout.children().next().unwrap(),
                cursor,
                renderer,
                clipboard,
                shell,
                &layout.bounds(),
            );
        }
        if shell.is_event_captured() {
            return;
        }
        if let Some(reason) = dismissal(&self.dismiss, event, cursor.position(), panel, self.anchor)
        {
            self.report(reason, shell);
            if reason == DismissReason::Escape {
                // Only the innermost open layer closes on one Escape.
                shell.capture_event();
            }
            return;
        }
        if !self.hit_test {
            return;
        }
        // The panel is opaque to the pointer: nothing under it reacts.
        let over = cursor.is_over(panel);
        match event {
            Event::Mouse(
                mouse::Event::ButtonPressed(_)
                | mouse::Event::ButtonReleased(_)
                | mouse::Event::WheelScrolled { .. },
            ) if over => shell.capture_event(),
            Event::Touch(
                touch::Event::FingerPressed { position, .. }
                | touch::Event::FingerMoved { position, .. }
                | touch::Event::FingerLifted { position, .. },
            ) if panel.contains(*position) => shell.capture_event(),
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if !self.hit_test || !cursor.is_over(layout.bounds()) {
            return mouse::Interaction::None;
        }
        let i = self.content.as_widget().mouse_interaction(
            self.tree,
            layout.children().next().unwrap(),
            cursor,
            &layout.bounds(),
            renderer,
        );
        if i == mouse::Interaction::None {
            mouse::Interaction::Idle
        } else {
            i
        }
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        operation.container(self.id, layout.bounds());
        operation.traverse(&mut |operation| {
            self.content.as_widget_mut().operate(
                self.tree,
                layout.children().next().unwrap(),
                renderer,
                operation,
            );
        });
    }

    fn draw(
        &self,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let (alpha, k) = self.look();
        if alpha <= 0.0 {
            return;
        }
        let b = layout.bounds();
        let cursor = if self.hit_test {
            cursor
        } else {
            mouse::Cursor::Unavailable
        };
        let style = renderer::Style {
            text_color: iced::Color {
                a: style.text_color.a * alpha,
                ..style.text_color
            },
        };
        let draw = |renderer: &mut iced::Renderer| {
            with_opacity(alpha, || {
                self.content.as_widget().draw(
                    self.tree,
                    renderer,
                    theme,
                    &style,
                    layout.children().next().unwrap(),
                    cursor,
                    &b,
                );
            });
        };
        if k < 1.0 {
            let c = b.center();
            let tf = Transformation::translate(c.x, c.y)
                * Transformation::scale(k)
                * Transformation::translate(-c.x, -c.y);
            renderer.with_transformation(tf, draw);
        } else {
            draw(renderer);
        }
    }

    fn overlay<'c>(
        &'c mut self,
        layout: Layout<'c>,
        renderer: &iced::Renderer,
    ) -> Option<overlay::Element<'c, M, iced::Theme, iced::Renderer>> {
        let b = layout.bounds();
        self.content.as_widget_mut().overlay(
            self.tree,
            layout.children().next().unwrap(),
            renderer,
            &b,
            Vector::ZERO,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIN: Size = Size {
        width: 1280.0,
        height: 800.0,
    };

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rectangle {
        Rectangle::new(Point::new(x, y), Size::new(w, h))
    }

    fn press() -> Event {
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
    }

    fn key(named: keyboard::key::Named) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(named),
            modified_key: keyboard::Key::Named(named),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::default(),
            text: None,
            repeat: false,
        })
    }

    #[test]
    fn outside_press_rules() {
        let panel = rect(100.0, 140.0, 200.0, 100.0);
        let anchor = rect(100.0, 100.0, 80.0, 34.0);
        let d = |pol: &Dismiss, p: Point| dismissal(pol, &press(), Some(p), panel, anchor);
        // Inside the panel: never.
        assert_eq!(d(&Dismiss::POPOVER, Point::new(150.0, 200.0)), None);
        // On the anchor: ignored by Popover/Dropdown (the trigger toggles)…
        assert_eq!(d(&Dismiss::POPOVER, Point::new(120.0, 110.0)), None);
        // …but not by a context menu, whose "anchor" is the whole area.
        assert_eq!(
            d(&Dismiss::CONTEXT_MENU, Point::new(120.0, 110.0)),
            Some(DismissReason::OutsidePress)
        );
        assert_eq!(
            d(&Dismiss::POPOVER, Point::new(600.0, 600.0)),
            Some(DismissReason::OutsidePress)
        );
        assert_eq!(d(&Dismiss::NONE, Point::new(600.0, 600.0)), None);
        // Right presses count (pointerdown fires for every button).
        let right = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right));
        assert_eq!(
            dismissal(
                &Dismiss::CONTEXT_MENU,
                &right,
                Some(Point::new(5.0, 5.0)),
                panel,
                anchor
            ),
            Some(DismissReason::OutsidePress)
        );
    }

    #[test]
    fn keys_resize_and_blur() {
        let r = Rectangle::default();
        let esc = key(keyboard::key::Named::Escape);
        assert_eq!(
            dismissal(&Dismiss::POPOVER, &esc, None, r, r),
            Some(DismissReason::Escape)
        );
        let resized = Event::Window(window::Event::Resized(WIN));
        assert_eq!(
            dismissal(&Dismiss::DROPDOWN, &resized, None, r, r),
            Some(DismissReason::Resize)
        );
        let blur = Event::Window(window::Event::Unfocused);
        assert_eq!(
            dismissal(&Dismiss::POPOVER, &blur, None, r, r),
            Some(DismissReason::Blur)
        );
        assert_eq!(dismissal(&Dismiss::DROPDOWN, &blur, None, r, r), None);
        let down = key(keyboard::key::Named::ArrowDown);
        assert_eq!(dismissal(&Dismiss::POPOVER, &down, None, r, r), None);
    }

    #[test]
    fn placement_strategies() {
        let anchor = rect(1100.0, 700.0, 100.0, 36.0);
        let size = Size::new(192.0, 120.0);
        // Popover flips above the anchor near the bottom edge.
        let p = place(Position::default(), anchor, size, WIN);
        assert_eq!(p, Point::new(1080.0, 574.0));
        // Dropdown never flips: clamped to the bottom padding instead.
        let p = place(
            Position::Below {
                align_right: true,
                gap: 4.0,
            },
            anchor,
            size,
            WIN,
        );
        assert_eq!(p, Point::new(1008.0, 672.0));
        // Context menu at the pointer, relative to the area.
        let area = rect(300.0, 200.0, 600.0, 96.0);
        let p = place(Position::At(Point::new(40.0, 30.0)), area, size, WIN);
        assert_eq!(p, Point::new(340.0, 230.0));
    }
}
