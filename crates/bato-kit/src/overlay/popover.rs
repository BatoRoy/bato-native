//! `Popover` (`overlay/Popover.tsx`): a controlled, anchored panel, plus the
//! two pieces every picker built on it shares.
//!
//! - [`popover`] + [`popover_panel`]: the anchor with the styled panel
//!   (`bg-app-card border-app-lineStrong rounded-xl shadow-app-lg`) placed by
//!   `computePosition` (bottom, start, offset 6 by default), dismissed by
//!   `useDismiss` (outside press except on the anchor, Esc, scroll, resize,
//!   blur), `animate-scale-in`. The caller owns `open` / `onClose`, exactly
//!   like React.
//! - [`local`]: a widget with its own transient state (open, query, active
//!   row…) that renders an inner element from that state and turns inner
//!   events into app messages. React keeps that state in `useState`; the
//!   kit keeps it in the widget tree (INF-1), so Combobox, Dropdown,
//!   ColorPicker take only the React props.
//! - [`keys`]: named keys pressed while focus is inside a subtree (an
//!   input's `onKeyDown`: ↑ ↓ Enter in a Combobox search field).
//!
//! ```ignore
//! popover(
//!     button(&on).secondary().label("Popover").on_press(Msg::Toggle),
//!     state.open.then(|| popover_panel(cx, body).padding(13).width(192).into()),
//! )
//! .on_dismiss(|_| Msg::Close)
//! ```

use std::cell::RefCell;

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::keyboard::{self, key::Named};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};

use super::anchored::{Anchored, Dismiss, anchored};
use crate::ctx::Ctx;
use crate::floating;
use crate::focus::FocusedBounds;
use crate::style::{c, shadows};
use crate::surface::{BoxStyle, Surface, surface};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// The styled panel (`styled`): card fill, `lineStrong` border, radius 12,
/// `shadow-app-lg`. `cx` is the context the popover floats in.
pub fn panel_style(cx: &Ctx) -> BoxStyle {
    let on = cx.on_card();
    let p = on.p();
    BoxStyle {
        bg: Some(c(p.card)),
        border: Some(c(on.line_strong())),
        radius: 12.0,
        shadows: shadows(&p.shadow_lg),
        ..BoxStyle::default()
    }
}

/// The styled panel around `content` (`className="p-3 w-48"` → `.padding(13)
/// .width(192)`: the 1 px border is inside the box). Build the content on
/// `cx.on_card()`.
pub fn popover_panel<'a, M: Clone + 'a>(cx: &Ctx, content: impl Into<El<'a, M>>) -> Surface<'a, M> {
    surface(content).fixed(panel_style(cx))
}

/// `Popover`: `panel` floats next to `anchor` while it is `Some`, with
/// React's defaults (bottom, start, offset 6, `useDismiss`, scale-in). Tune
/// it with the [`Anchored`] builder: `.float(opts)` (placement, align,
/// offset), `.match_width(true)`, `.on_dismiss(..)`.
pub fn popover<'a, M>(anchor: impl Into<El<'a, M>>, panel: Option<El<'a, M>>) -> Anchored<'a, M> {
    anchored(anchor, panel)
        .float(floating::Options::default())
        .dismiss(Dismiss::POPOVER)
}

// ── Local state ─────────────────────────────────────────────────────────

type ViewFn<'a, S, E> = Box<dyn Fn(&S) -> El<'a, E> + 'a>;
type UpdateFn<'a, S, E, M> = Box<dyn Fn(&mut S, E) -> Vec<M> + 'a>;

/// A widget that owns a state `S` and shows `view(&S)`; the inner element
/// publishes events `E`, which `update` applies to `S`, optionally emitting
/// an app message. Both closures run during the app's view lifetime, so they
/// may borrow app data (the value, the options, `on_change`).
pub struct Local<'a, S, E, M> {
    view: ViewFn<'a, S, E>,
    update: UpdateFn<'a, S, E, M>,
    element: RefCell<Option<El<'a, E>>>,
    init: Option<Box<dyn Fn() -> S + 'a>>,
}

/// See [`Local`].
pub fn local<'a, S, E, M>(
    view: impl Fn(&S) -> El<'a, E> + 'a,
    update: impl Fn(&mut S, E) -> Option<M> + 'a,
) -> Local<'a, S, E, M>
where
    S: Default + 'static,
    E: 'a,
    M: 'a,
{
    local_many(view, move |s, e| update(s, e).into_iter().collect())
}

/// [`local`] whose events may publish several app messages.
pub fn local_many<'a, S, E, M>(
    view: impl Fn(&S) -> El<'a, E> + 'a,
    update: impl Fn(&mut S, E) -> Vec<M> + 'a,
) -> Local<'a, S, E, M>
where
    S: Default + 'static,
{
    Local {
        view: Box::new(view),
        update: Box::new(update),
        element: RefCell::new(None),
        init: None,
    }
}

impl<'a, S: Default + 'static, E, M> Local<'a, S, E, M> {
    /// The state a new widget starts from (default: `S::default()`).
    pub fn init(mut self, f: impl Fn() -> S + 'a) -> Self {
        self.init = Some(Box::new(f));
        self
    }

    fn fresh(&self) -> S {
        self.init.as_ref().map_or_else(S::default, |f| f())
    }

    /// The inner element, built from `S::default()` when no state is known
    /// yet (a brand-new tree).
    fn ensure(&self) {
        let mut el = self.element.borrow_mut();
        if el.is_none() {
            *el = Some((self.view)(&self.fresh()));
        }
    }

    /// Rebuild from `inner` when it changed, and diff the child tree.
    fn refresh(&mut self, tree: &mut Tree) {
        let inner = tree.state.downcast_mut::<Inner<S>>();
        if !inner.dirty && self.element.get_mut().is_some() {
            return;
        }
        inner.dirty = false;
        let el = (self.view)(&inner.s);
        tree.diff_children(std::slice::from_ref(&el));
        *self.element.get_mut() = Some(el);
    }
}

/// The tree state of a [`Local`].
pub struct Inner<S> {
    pub s: S,
    /// `s` changed since the element was built.
    dirty: bool,
}

/// Copy everything but the messages from a sub-shell.
fn merge_flags<A, B>(shell: &mut Shell<'_, A>, sub: &Shell<'_, B>) {
    if sub.is_event_captured() {
        shell.capture_event();
    }
    shell.request_redraw_at(sub.redraw_request());
    if sub.is_layout_invalid() {
        shell.invalidate_layout();
    }
    if sub.are_widgets_invalid() {
        shell.invalidate_widgets();
    }
    shell.request_input_method(sub.input_method());
}

/// Apply inner events; returns whether the state changed.
fn apply<S, E, M>(
    inner: &mut Inner<S>,
    update: &dyn Fn(&mut S, E) -> Vec<M>,
    events: Vec<E>,
    shell: &mut Shell<'_, M>,
) -> bool {
    let any = !events.is_empty();
    for e in events {
        for m in update(&mut inner.s, e) {
            shell.publish(m);
        }
    }
    if any {
        inner.dirty = true;
        shell.invalidate_layout();
        shell.request_redraw();
    }
    any
}

impl<'a, S, E, M> Widget<M, iced::Theme, iced::Renderer> for Local<'a, S, E, M>
where
    S: Default + 'static,
    E: 'a,
    M: 'a,
{
    fn size(&self) -> Size<Length> {
        self.ensure();
        self.element.borrow().as_ref().unwrap().as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.ensure();
        self.element
            .borrow()
            .as_ref()
            .unwrap()
            .as_widget()
            .size_hint()
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Inner<S>>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(Inner {
            s: self.fresh(),
            dirty: false,
        })
    }
    fn children(&self) -> Vec<Tree> {
        self.ensure();
        vec![Tree::new(self.element.borrow().as_ref().unwrap())]
    }
    fn diff(&self, tree: &mut Tree) {
        let inner = tree.state.downcast_mut::<Inner<S>>();
        inner.dirty = false;
        let el = (self.view)(&inner.s);
        tree.diff_children(std::slice::from_ref(&el));
        *self.element.borrow_mut() = Some(el);
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.refresh(tree);
        let el = self.element.get_mut().as_mut().unwrap();
        let n = el
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
        self.refresh(tree);
        let mut events = Vec::new();
        {
            let mut sub = Shell::new(&mut events);
            let el = self.element.get_mut().as_mut().unwrap();
            el.as_widget_mut().update(
                &mut tree.children[0],
                event,
                layout.children().next().unwrap(),
                cursor,
                renderer,
                clipboard,
                &mut sub,
                viewport,
            );
            merge_flags(shell, &sub);
        }
        let inner = tree.state.downcast_mut::<Inner<S>>();
        if apply(inner, &*self.update, events, shell) {
            self.refresh(tree);
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
        self.ensure();
        self.element
            .borrow()
            .as_ref()
            .unwrap()
            .as_widget()
            .mouse_interaction(
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
        self.refresh(tree);
        self.element
            .get_mut()
            .as_mut()
            .unwrap()
            .as_widget_mut()
            .operate(
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
        self.ensure();
        self.element.borrow().as_ref().unwrap().as_widget().draw(
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
        self.refresh(tree);
        let Tree {
            state, children, ..
        } = tree;
        let inner = state.downcast_mut::<Inner<S>>();
        let el = self.element.get_mut().as_mut()?;
        let ov = el.as_widget_mut().overlay(
            &mut children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )?;
        Some(overlay::Element::new(Box::new(LocalOverlay {
            inner: ov,
            state: inner,
            update: &*self.update,
        })))
    }
}

impl<'a, S, E, M> From<Local<'a, S, E, M>> for El<'a, M>
where
    S: Default + 'static,
    E: 'a,
    M: 'a,
{
    fn from(l: Local<'a, S, E, M>) -> Self {
        Element::new(l)
    }
}

/// The inner element's overlay, its events turned into app messages.
struct LocalOverlay<'b, S, E, M> {
    inner: overlay::Element<'b, E, iced::Theme, iced::Renderer>,
    state: &'b mut Inner<S>,
    update: &'b dyn Fn(&mut S, E) -> Vec<M>,
}

impl<S, E, M> overlay::Overlay<M, iced::Theme, iced::Renderer> for LocalOverlay<'_, S, E, M> {
    fn layout(&mut self, renderer: &iced::Renderer, bounds: Size) -> layout::Node {
        self.inner.as_overlay_mut().layout(renderer, bounds)
    }
    fn draw(
        &self,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.inner
            .as_overlay()
            .draw(renderer, theme, style, layout, cursor);
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
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
    ) {
        let mut events = Vec::new();
        {
            let mut sub = Shell::new(&mut events);
            self.inner
                .as_overlay_mut()
                .update(event, layout, cursor, renderer, clipboard, &mut sub);
            merge_flags(shell, &sub);
        }
        // The element is rebuilt on the relayout this asks for.
        apply(self.state, self.update, events, shell);
    }
    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.inner
            .as_overlay()
            .mouse_interaction(layout, cursor, renderer)
    }
    fn overlay<'c>(
        &'c mut self,
        layout: Layout<'c>,
        renderer: &iced::Renderer,
    ) -> Option<overlay::Element<'c, M, iced::Theme, iced::Renderer>> {
        let nested = self.inner.as_overlay_mut().overlay(layout, renderer)?;
        Some(overlay::Element::new(Box::new(LocalOverlay {
            inner: nested,
            state: &mut *self.state,
            update: self.update,
        })))
    }
    fn index(&self) -> f32 {
        self.inner.as_overlay().index()
    }
}

// ── Keys while focused ──────────────────────────────────────────────────

/// Named keys pressed while keyboard focus is inside `content` (React's
/// `onKeyDown` on an input): `on_key` maps a key to a message, which is
/// published and the key consumed (`preventDefault`). Keys the content
/// itself consumed are not seen.
pub struct Keys<'a, M> {
    content: El<'a, M>,
    on_key: Box<dyn Fn(Named, keyboard::Modifiers) -> Option<M> + 'a>,
    /// Before the content (a key the content would otherwise use, e.g. → at
    /// the end of a field accepting a completion).
    first: bool,
}

pub fn keys<'a, M>(
    content: impl Into<El<'a, M>>,
    on_key: impl Fn(Named, keyboard::Modifiers) -> Option<M> + 'a,
) -> Keys<'a, M> {
    Keys {
        content: content.into(),
        on_key: Box::new(on_key),
        first: false,
    }
}

impl<'a, M> Keys<'a, M> {
    /// See the keys before the content does.
    pub fn first(mut self) -> Self {
        self.first = true;
        self
    }
}

impl<'a, M: 'a> Keys<'a, M> {
    fn handle(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, M>,
    ) {
        let Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(named),
            modifiers,
            ..
        }) = event
        else {
            return;
        };
        if shell.is_event_captured() {
            return;
        }
        let Some(m) = (self.on_key)(*named, *modifiers) else {
            return;
        };
        let mut op = FocusedBounds::default();
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            &mut op,
        );
        if op.0.is_some() {
            shell.publish(m);
            shell.capture_event();
        }
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for Keys<'a, M> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
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
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        if self.first {
            self.handle(tree, event, layout, renderer, shell);
            if shell.is_event_captured() {
                return;
            }
        }
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
        if !self.first {
            self.handle(tree, event, layout, renderer, shell);
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
        self.content.as_widget().draw(
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
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: 'a> From<Keys<'a, M>> for El<'a, M> {
    fn from(k: Keys<'a, M>) -> Self {
        Element::new(k)
    }
}

// ── matchWidth panels ───────────────────────────────────────────────────

/// A `matchWidth` panel whose rows are `w-full`: when the floating layer
/// passes a minimum width (the anchor's), the content is laid out exactly
/// that wide, so `Length::Fill` children fill the anchor's width instead of
/// the window's. (React grows the panel past the anchor for rows wider than
/// it; the kit's pickers truncate them instead.)
pub struct MinFill<'a, M> {
    content: El<'a, M>,
}

pub fn min_fill<'a, M>(content: impl Into<El<'a, M>>) -> MinFill<'a, M> {
    MinFill {
        content: content.into(),
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for MinFill<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
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
        let w = limits.min().width;
        let limits = if w > 0.0 {
            layout::Limits::new(Size::new(w, 0.0), Size::new(w, limits.max().height))
        } else {
            *limits
        };
        let n = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &limits);
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
        self.content.as_widget().draw(
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
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: 'a> From<MinFill<'a, M>> for El<'a, M> {
    fn from(m: MinFill<'a, M>) -> Self {
        Element::new(m)
    }
}
