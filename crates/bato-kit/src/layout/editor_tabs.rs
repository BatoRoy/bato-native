//! `EditorTabs`: the VS Code tab strip. `flex items-stretch bg-app-surface
//! border-b border-app-border overflow-x-auto` of tabs (`px-3 py-2 text-sm
//! border-r`, an optional 14 px icon, the label, then a 16 px slot holding
//! the dirty dot and the close ×), then the `+` button (`px-2.5`).
//!
//! - Active tab: `bg-app-bg text-app-text` with a 2 px `bg-app-accent` bar
//!   on top. Others: `text-app-muted`, hover `text-app-text
//!   bg-app-text/[0.06]` (`transition-colors`).
//! - The × shows on the active tab and on hover (`transition-opacity`); a
//!   dirty tab shows `bg-app-subtext` dot instead, which hides on hover. (On
//!   the active dirty tab both show, as React's classes resolve.)
//! - Drag to reorder (4 px, then the tab follows the pointer at
//!   `opacity-60`, clipped to the strip, the others slide 200 ms `ease`); the
//!   × never starts a drag. A click activates; the strip scrolls sideways
//!   when the tabs overflow, and auto-scrolls while dragging near its ends.
//!
//! ```ignore
//! editor_tabs(&cx, &tabs, &active, Msg::Activate)
//!     .on_close(Msg::Close).on_reorder(Msg::Reorder).on_add(Msg::Add)
//! ```

use std::cell::Cell;
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::widget::{Space, column, container, row, stack};
use iced::{Alignment, Color, Element, Event, Length, Rectangle, Size, Vector, mouse, window};

use super::hline;
use crate::anim::{ease_standard, fade, with_opacity};
use crate::ctx::Ctx;
use crate::date::ua_ring;
use crate::dnd::{self, Axis, PreviewStyle};
use crate::icon::{Ico, Icon, icon};
use crate::label::label;
use crate::scroller::scroll_area;
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorTab {
    pub id: String,
    pub label: String,
    pub icon: Option<Icon>,
    pub dirty: bool,
}

impl EditorTab {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            dirty: false,
        }
    }
    pub fn icon(mut self, i: impl Into<Icon>) -> Self {
        self.icon = Some(i.into());
        self
    }
    pub fn dirty(mut self, d: bool) -> Self {
        self.dirty = d;
        self
    }
}

/// A dragged tab: `opacity-60`.
pub const DRAG_OPACITY: f32 = 0.6;
/// A tab's height: `py-2` around the 20 px `text-sm` line.
pub const TAB_H: f32 = 36.0;
const FADE: Duration = Duration::from_millis(150);

pub struct EditorTabs<'a, M> {
    cx: Ctx,
    tabs: Vec<EditorTab>,
    active: String,
    on_activate: Box<dyn Fn(String) -> M + 'a>,
    on_close: Option<Box<dyn Fn(String) -> M + 'a>>,
    on_reorder: Option<Box<dyn Fn(usize, usize) -> M + 'a>>,
    on_add: Option<M>,
}

/// The strip for `tabs`, `active` highlighted; a click calls `on_activate`.
pub fn editor_tabs<'a, M>(
    cx: &Ctx,
    tabs: &[EditorTab],
    active: &str,
    on_activate: impl Fn(String) -> M + 'a,
) -> EditorTabs<'a, M> {
    EditorTabs {
        cx: cx.clone(),
        tabs: tabs.to_vec(),
        active: active.to_string(),
        on_activate: Box::new(on_activate),
        on_close: None,
        on_reorder: None,
        on_add: None,
    }
}

impl<'a, M: Clone + 'a> EditorTabs<'a, M> {
    /// Show the close slot; the × calls this.
    pub fn on_close(mut self, f: impl Fn(String) -> M + 'a) -> Self {
        self.on_close = Some(Box::new(f));
        self
    }
    /// Drag to reorder: `(from, to)` ([`dnd::array_move`]).
    pub fn on_reorder(mut self, f: impl Fn(usize, usize) -> M + 'a) -> Self {
        self.on_reorder = Some(Box::new(f));
        self
    }
    /// The `+` button.
    pub fn on_add(mut self, m: M) -> Self {
        self.on_add = Some(m);
        self
    }
}

fn tab<'a, M: Clone + 'a>(
    cx: &Ctx,
    t: &EditorTab,
    active: bool,
    activate: M,
    close: Option<M>,
) -> El<'a, M> {
    let p = cx.p();
    let mut content: Vec<El<'a, M>> = Vec::new();
    if let Some(i) = t.icon {
        content.push(icon(i, 14.0).into());
    }
    content.push(label(t.label.clone(), 14.0, 20.0).into());
    if let Some(close) = close {
        let dot: El<'a, M> = surface(Space::new().width(8).height(8))
            .fixed(BoxStyle::default().bg(c(p.subtext)).radius(4.0))
            .into();
        let (muted, text) = (c(p.muted), c(p.text));
        let x = surface(icon(Ico::X, 13.0))
            .style(move |s| {
                BoxStyle::default()
                    .fg(if s == Status::Hover { text } else { muted })
                    .radius(4.0)
            })
            .transition_ms(0)
            .on_press(close)
            .focus_ring(ua_ring());
        // `opacity-100` on the active tab wins over the dirty tab's
        // `opacity-0` (Tailwind orders opacity-0 first).
        let x_rest = if active { 1.0 } else { 0.0 };
        let mut layers: Vec<El<'a, M>> = Vec::new();
        if t.dirty {
            layers.push(center(group_fade(1.0, 0.0, false, dot)));
        }
        layers.push(center(dnd::no_drag(group_fade(x_rest, 1.0, true, x))));
        // `ml-1`: 4 more than the gap.
        content.push(
            container(container(stack(layers)).width(16).height(16))
                .padding(iced::Padding::ZERO.left(4))
                .into(),
        );
    }
    let body = row(content).spacing(8).align_y(Alignment::Center);
    let (bg, text, muted) = (c(p.bg), c(p.text), c(p.muted));
    let hover = c(cx.wash(0.06));
    let inner = surface(container(body).padding(iced::Padding {
        top: 8.0,
        right: 12.0,
        bottom: 8.0,
        left: 12.0,
    }))
    .style(move |s| {
        if active {
            BoxStyle::default().bg(bg).fg(text)
        } else if s == Status::Hover {
            BoxStyle::default().bg(hover).fg(text)
        } else {
            BoxStyle::default().fg(muted)
        }
    })
    .on_press(activate)
    .focus_ring(ua_ring());
    let rule =
        surface(Space::new().width(1).height(TAB_H)).fixed(BoxStyle::default().bg(c(p.border)));
    Group {
        content: row![inner, rule].into(),
        bar: active.then(|| c(p.accent)),
    }
    .into()
}

fn center<'a, M: 'a>(el: impl Into<El<'a, M>>) -> El<'a, M> {
    container(el)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .into()
}

impl<'a, M: Clone + 'a> From<EditorTabs<'a, M>> for El<'a, M> {
    fn from(e: EditorTabs<'a, M>) -> Self {
        let cx = e.cx.on(e.cx.p().surface);
        let p = cx.p().clone();
        let mut list = dnd::sortable::<String, M>(Axis::Horizontal).preview(PreviewStyle {
            opacity: DRAG_OPACITY,
            ..PreviewStyle::default()
        });
        for t in &e.tabs {
            let active = t.id == e.active;
            let close = e.on_close.as_ref().map(|f| f(t.id.clone()));
            list = list.item(
                t.id.clone(),
                tab(&cx, t, active, (e.on_activate)(t.id.clone()), close),
            );
        }
        if let Some(f) = e.on_reorder {
            list = list.on_reorder(f);
        }
        let mut strip: Vec<El<'a, M>> = vec![list.into()];
        if let Some(add) = e.on_add {
            let (muted, text, hover) = (c(p.muted), c(p.text), c(cx.wash(0.06)));
            strip.push(
                surface(icon(Ico::Plus, 15.0))
                    .padding([0, 10])
                    .height(if e.tabs.is_empty() {
                        Length::Shrink
                    } else {
                        Length::Fixed(TAB_H)
                    })
                    .center_y()
                    .style(move |s| {
                        if s == Status::Hover {
                            BoxStyle::default().bg(hover).fg(text)
                        } else {
                            BoxStyle::default().fg(muted)
                        }
                    })
                    .on_press(add)
                    .focus_ring(ua_ring())
                    .into(),
            );
        }
        let bar = surface(
            scroll_area(&cx, row(strip).height(Length::Shrink))
                .horizontal()
                .width(Length::Fill),
        )
        .width(Length::Fill)
        .fixed(BoxStyle::default().bg(c(p.surface)));
        column![bar, hline(c(p.border))].into()
    }
}

// ── Group hover (`group` / `group-hover:`) ──────────────────────────────

thread_local! {
    /// While a [`Group`] draws: (hovered, eased 150 ms progress).
    static GROUP: Cell<(bool, f32)> = const { Cell::new((false, 0.0)) };
}

/// A tab: hover for its `group-hover:` children, and the active tab's
/// `absolute top-0 inset-x-0 h-0.5` accent bar over its padding box.
struct Group<'a, M> {
    content: El<'a, M>,
    bar: Option<Color>,
}

#[derive(Debug, Default)]
struct GroupState {
    hovered: bool,
    from: f32,
    start: Option<Instant>,
}

impl GroupState {
    fn t(&self, now: Instant) -> f32 {
        let to = if self.hovered { 1.0 } else { 0.0 };
        match self.start {
            Some(s) => {
                let f =
                    (now.saturating_duration_since(s).as_secs_f32() / FADE.as_secs_f32()).min(1.0);
                self.from + (to - self.from) * ease_standard(f)
            }
            None => to,
        }
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for Group<'a, M> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<GroupState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(GroupState::default())
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
        let now = Instant::now();
        let st = tree.state.downcast_mut::<GroupState>();
        let over = cursor.is_over(layout.bounds()) && cursor.is_over(*viewport);
        match event {
            Event::Mouse(_) | Event::Touch(_) if over != st.hovered => {
                st.from = st.t(now);
                st.hovered = over;
                st.start = Some(now);
                shell.request_redraw();
            }
            Event::Window(window::Event::RedrawRequested(at)) => {
                if st.start.is_some_and(|s| *at < s + FADE) {
                    shell.request_redraw();
                } else {
                    st.start = None;
                }
            }
            _ => {}
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
        let st = tree.state.downcast_ref::<GroupState>();
        let prev = GROUP.get();
        GROUP.set((st.hovered, st.t(Instant::now())));
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
        GROUP.set(prev);
        if let Some(bar) = self.bar {
            let b = layout.bounds();
            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle {
                        height: 2.0,
                        width: b.width - 1.0,
                        ..b
                    },
                    ..renderer::Quad::default()
                },
                fade(bar),
            );
        }
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

impl<'a, M: 'a> From<Group<'a, M>> for El<'a, M> {
    fn from(g: Group<'a, M>) -> Self {
        Element::new(g)
    }
}

/// Opacity `rest`, or `hover` while the enclosing [`Group`] is hovered
/// (eased over 150 ms when `animated`, else at once).
fn group_fade<'a, M: 'a>(
    rest: f32,
    hover: f32,
    animated: bool,
    content: impl Into<El<'a, M>>,
) -> El<'a, M> {
    Element::new(GroupFade {
        content: content.into(),
        rest,
        hover,
        animated,
    })
}

struct GroupFade<'a, M> {
    content: El<'a, M>,
    rest: f32,
    hover: f32,
    animated: bool,
}

impl<'a, M: 'a> GroupFade<'a, M> {
    fn alpha(&self) -> f32 {
        let (h, t) = GROUP.get();
        let k = if self.animated {
            t
        } else if h {
            1.0
        } else {
            0.0
        };
        self.rest + (self.hover - self.rest) * k
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for GroupFade<'a, M> {
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
        let a = self.alpha();
        if a <= 0.0 {
            return;
        }
        let style = renderer::Style {
            text_color: Color {
                a: style.text_color.a * a,
                ..style.text_color
            },
        };
        with_opacity(a, || {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                &style,
                layout.children().next().unwrap(),
                cursor,
                viewport,
            );
        });
    }
}
