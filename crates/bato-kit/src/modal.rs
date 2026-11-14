//! The overlay stack (INF-6): one level per open modal, drawer or command
//! palette, each a full-window layer with its own scrim (`#000@.5`, drawers
//! `@.6`, plus the 4 px backdrop blur, see `blur.rs`) and panel. Levels are
//! stacked in order ([`crate::frame::Frame::view_with`]); the topmost one
//! takes the input:
//!
//! - **Escape** closes the top level only (`Modal.tsx`'s dialog stack). The
//!   level captures it, so the levels under it never see it.
//! - A press that **both starts and ends on the scrim** closes it (a drag out
//!   of the panel does not).
//! - **Scroll lock**: wheel, touch and the scrolling keys (arrows, Page
//!   Up/Down, Home/End, Space) never reach the page or a lower level;
//!   hover under the scrim is blocked.
//!
//! Kinds ([`Kind`]):
//!
//! | Kind | React | Panel | Enter |
//! |---|---|---|---|
//! | [`Kind::Center`] | `Modal.tsx` | centred, 16 px margin, max 85 % height | scrim fadeIn 150 ms, panel scaleIn .96→1 + opacity 120 ms ease-out |
//! | [`Kind::Top`] | `CommandPalette.tsx` | top-aligned at 12 vh, 16 px side margin | same |
//! | [`Kind::Drawer`] | `Drawer.tsx` | against one window edge, full height (width) | scrim fadeIn 150 ms, panel slides in from its edge 200 ms ease-out |
//!
//! No exit animations: React unmounts at once.
//!
//! **Group opacity**: iced has none, so a level over the plain page (the
//! frame's App Options and About) fades its panel in with a cover quad in the
//! scrim-over-page colour ([`modal`]'s `cover`); other levels use the kit's
//! opacity multiplier ([`crate::anim::with_opacity`]).
//!
//! **Focus**: each level's panel is a container with the level's
//! [`widget::Id`] ([`Modal::id`], [`level_id`]) so a focus trap can scope
//! `focus_next` to it, and [`Modal::wrap_panel`] lets the focus system wrap
//! the panel in its trap widget.

use std::time::Instant;

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::{
    Border, Color, Element, Event, Length, Point, Rectangle, Size, Transformation, Vector,
    keyboard, mouse, window,
};

use crate::anim::{ease_out, with_opacity};

type El<'a, M> = Element<'a, M, iced::Theme, iced::Renderer>;

/// A drawer's edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// Where a level's panel sits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// A dialog, centred.
    Center,
    /// The command palette, top-aligned at 12 % of the window height.
    Top,
    /// A drawer against `Side`, `size` px wide (left/right) or tall
    /// (top/bottom).
    Drawer(Side, f32),
}

/// The id of the `depth`-th level of a stack (0 = bottom).
pub fn level_id(depth: usize) -> widget::Id {
    widget::Id::from(format!("bato-overlay-level-{depth}"))
}

/// One level of the overlay stack.
pub struct Modal<'a, Message> {
    panel: El<'a, Message>,
    kind: Kind,
    max_width: f32,
    on_close: Message,
    scrim: f32,
    /// Opaque colour of the scrim over the page, for the panel fade cover.
    cover: Option<Color>,
    backdrop: Option<El<'a, Message>>,
    blur: bool,
    animate: bool,
    id: Option<widget::Id>,
}

/// A dialog level: `panel` centred over the window, at most `max_width` wide
/// (`max-w-md` = 448, `lg` = 512); `cover` is the opaque scrim-over-page
/// colour (`blend(#000, .5, bg)`) used for the panel's fade.
pub fn modal<'a, Message>(
    panel: impl Into<El<'a, Message>>,
    max_width: f32,
    on_close: Message,
    cover: Color,
) -> Modal<'a, Message> {
    Modal {
        panel: panel.into(),
        kind: Kind::Center,
        max_width,
        on_close,
        scrim: 0.5,
        cover: Some(cover),
        backdrop: None,
        blur: false,
        animate: true,
        id: None,
    }
}

/// A level of any kind (nested dialogs, drawers, the palette). The panel
/// fades with the group opacity.
pub fn level<'a, Message>(
    kind: Kind,
    panel: impl Into<El<'a, Message>>,
    on_close: Message,
) -> Modal<'a, Message> {
    Modal {
        panel: panel.into(),
        kind,
        max_width: match kind {
            Kind::Top => 512.0,
            _ => 448.0,
        },
        on_close,
        scrim: match kind {
            Kind::Drawer(..) => 0.6,
            _ => 0.5,
        },
        cover: None,
        backdrop: None,
        blur: false,
        animate: true,
        id: None,
    }
}

impl<'a, Message> Modal<'a, Message> {
    /// Drawn under the scrim (the blurred page).
    pub fn backdrop(mut self, b: impl Into<El<'a, Message>>) -> Self {
        self.backdrop = Some(b.into());
        self
    }
    /// Blur what is under the level (`backdrop-blur-sm`), fading in with
    /// the scrim. The frame turns it on for every level.
    pub fn blur(mut self, on: bool) -> Self {
        self.blur = on;
        self
    }
    pub fn animate(mut self, on: bool) -> Self {
        self.animate = on;
        self
    }
    /// `max-w-*` of a centred or top panel.
    pub fn max_width(mut self, w: f32) -> Self {
        self.max_width = w;
        self
    }
    /// Scrim alpha (`bg-black/50` = .5).
    pub fn scrim(mut self, a: f32) -> Self {
        self.scrim = a;
        self
    }
    /// The panel's container id (focus trap scope).
    pub fn id(mut self, id: widget::Id) -> Self {
        self.id = Some(id);
        self
    }
    /// Wrap the panel, e.g. in the focus system's trap.
    pub fn wrap_panel(mut self, f: impl FnOnce(El<'a, Message>) -> El<'a, Message>) -> Self {
        self.panel = f(self.panel);
        self
    }
    pub fn kind(&self) -> Kind {
        self.kind
    }
    /// Lift the level's messages (a page's levels into the app's).
    pub fn map<B: 'a>(self, f: impl Fn(Message) -> B + Clone + 'a) -> Modal<'a, B>
    where
        Message: 'a,
    {
        Modal {
            panel: self.panel.map(f.clone()),
            kind: self.kind,
            max_width: self.max_width,
            on_close: f(self.on_close),
            scrim: self.scrim,
            cover: self.cover,
            backdrop: self.backdrop.map(|b| b.map(f)),
            blur: self.blur,
            animate: self.animate,
            id: self.id,
        }
    }
}

#[derive(Default)]
struct State {
    opened: Option<Instant>,
    progress_overlay: f32,
    progress_panel: f32,
    down_on_backdrop: bool,
}

impl<Message> Modal<'_, Message> {
    fn panel_duration(&self) -> f32 {
        match self.kind {
            Kind::Drawer(..) => 0.200,
            _ => 0.120,
        }
    }

    /// The panel's limits and top-left in a window of `size`.
    fn place(&self, size: Size, panel: Size) -> Point {
        match self.kind {
            Kind::Center => Point::new(
                ((size.width - panel.width) / 2.0).round(),
                ((size.height - panel.height) / 2.0).round(),
            ),
            Kind::Top => Point::new(
                ((size.width - panel.width) / 2.0).round(),
                (size.height * 0.12).round(),
            ),
            Kind::Drawer(side, _) => match side {
                Side::Left | Side::Top => Point::ORIGIN,
                Side::Right => Point::new(size.width - panel.width, 0.0),
                Side::Bottom => Point::new(0.0, size.height - panel.height),
            },
        }
    }

    fn limits(&self, size: Size) -> layout::Limits {
        match self.kind {
            Kind::Center => {
                let w = self.max_width.min(size.width - 32.0);
                let max_h = (size.height * 0.85).floor();
                layout::Limits::new(Size::new(w, 0.0), Size::new(w, max_h))
            }
            Kind::Top => {
                let w = self.max_width.min(size.width - 32.0);
                let top = (size.height * 0.12).round();
                layout::Limits::new(Size::new(w, 0.0), Size::new(w, size.height - top))
            }
            Kind::Drawer(side, px) => match side {
                Side::Left | Side::Right => {
                    let w = px.min(size.width);
                    layout::Limits::new(Size::new(w, size.height), Size::new(w, size.height))
                }
                Side::Top | Side::Bottom => {
                    let h = px.min(size.height);
                    layout::Limits::new(Size::new(size.width, h), Size::new(size.width, h))
                }
            },
        }
    }

    /// The panel's slide offset for drawer progress `p` (translate ±100 %).
    fn slide(&self, panel: Rectangle, p: f32) -> Vector {
        let k = 1.0 - p;
        match self.kind {
            Kind::Drawer(Side::Left, _) => Vector::new(-panel.width * k, 0.0),
            Kind::Drawer(Side::Right, _) => Vector::new(panel.width * k, 0.0),
            Kind::Drawer(Side::Top, _) => Vector::new(0.0, -panel.height * k),
            Kind::Drawer(Side::Bottom, _) => Vector::new(0.0, panel.height * k),
            _ => Vector::ZERO,
        }
    }
}

/// Keys that scroll the page: a level swallows them (body scroll lock).
fn scrolls_page(key: &keyboard::Key) -> bool {
    use keyboard::key::Named;
    matches!(
        key,
        keyboard::Key::Named(
            Named::ArrowUp
                | Named::ArrowDown
                | Named::PageUp
                | Named::PageDown
                | Named::Home
                | Named::End
                | Named::Space
        )
    )
}

impl<'a, Message: Clone + 'a> Widget<Message, iced::Theme, iced::Renderer> for Modal<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }
    fn children(&self) -> Vec<Tree> {
        let mut v = vec![Tree::new(&self.panel)];
        if let Some(b) = &self.backdrop {
            v.push(Tree::new(b));
        }
        v
    }
    fn diff(&self, tree: &mut Tree) {
        let mut v: Vec<&El<'a, Message>> = vec![&self.panel];
        if let Some(b) = &self.backdrop {
            v.push(b);
        }
        tree.diff_children(&v.into_iter().map(|e| e.as_widget()).collect::<Vec<_>>());
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = limits.max();
        let l = self.limits(size);
        let panel = self
            .panel
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &l);
        let at = self.place(size, panel.size());
        let mut children = vec![panel.move_to(at)];
        if let Some(b) = &mut self.backdrop {
            let full = layout::Limits::new(Size::ZERO, size);
            children.push(
                b.as_widget_mut()
                    .layout(&mut tree.children[1], renderer, &full),
            );
        }
        layout::Node::with_children(size, children)
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
        let panel_layout = layout.children().next().unwrap();
        self.panel.as_widget_mut().update(
            &mut tree.children[0],
            event,
            panel_layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        let st = tree.state.downcast_mut::<State>();
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            let opened = *st.opened.get_or_insert(*now);
            if !self.animate {
                st.progress_overlay = 1.0;
                st.progress_panel = 1.0;
            } else {
                let t = now.saturating_duration_since(opened).as_secs_f32();
                let d = self.panel_duration();
                st.progress_overlay = ease_out((t / 0.150).min(1.0));
                st.progress_panel = ease_out((t / d).min(1.0));
                if t < 0.150f32.max(d) {
                    shell.request_redraw();
                }
            }
        }
        if shell.is_event_captured() {
            return;
        }
        let over_panel = cursor.is_over(panel_layout.bounds());
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(_)) => {
                st.down_on_backdrop = !over_panel;
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(_)) => {
                if st.down_on_backdrop && !over_panel {
                    shell.publish(self.on_close.clone());
                }
                st.down_on_backdrop = false;
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::WheelScrolled { .. }) => shell.capture_event(),
            // Touch: same rule as the mouse (down and up both on the scrim),
            // and never let a finger reach the page underneath.
            Event::Touch(iced::touch::Event::FingerPressed { position, .. }) => {
                st.down_on_backdrop = !panel_layout.bounds().contains(*position);
                shell.capture_event();
            }
            Event::Touch(iced::touch::Event::FingerLifted { position, .. }) => {
                if st.down_on_backdrop && !panel_layout.bounds().contains(*position) {
                    shell.publish(self.on_close.clone());
                }
                st.down_on_backdrop = false;
                shell.capture_event();
            }
            Event::Touch(_) => shell.capture_event(),
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) => {
                shell.publish(self.on_close.clone());
                shell.capture_event();
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) if scrolls_page(key) => {
                shell.capture_event();
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
        let i = self.panel.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        );
        if i != mouse::Interaction::None {
            i
        } else {
            mouse::Interaction::Idle
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let panel_layout = layout.children().next().unwrap();
        operation.container(self.id.as_ref(), panel_layout.bounds());
        operation.traverse(&mut |operation| {
            self.panel.as_widget_mut().operate(
                &mut tree.children[0],
                panel_layout,
                renderer,
                operation,
            );
        });
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
        let st = tree.state.downcast_ref::<State>();
        let (po, pp) = if st.opened.is_none() && self.animate {
            (0.0, 0.0)
        } else {
            (st.progress_overlay, st.progress_panel)
        };
        let b = layout.bounds();
        let mut kids = layout.children();
        let panel_layout = kids.next().unwrap();
        if let (Some(bd), Some(bl)) = (&self.backdrop, kids.next()) {
            bd.as_widget().draw(
                &tree.children[1],
                renderer,
                theme,
                style,
                bl,
                cursor,
                viewport,
            );
        }
        if self.blur && po > 0.0 {
            iced_wgpu::primitive::Renderer::draw_primitive(renderer, b, crate::blur::primitive(po));
        }
        // The blur primitive (if any) samples everything drawn before it, so the
        // scrim must live in a later layer.
        renderer.with_layer(b, |renderer| {
            renderer.fill_quad(
                Quad {
                    bounds: b,
                    ..Quad::default()
                },
                Color::from_rgba(0.0, 0.0, 0.0, self.scrim * po),
            );
        });
        let pb = panel_layout.bounds();
        let draw_panel = |renderer: &mut iced::Renderer| {
            self.panel.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                panel_layout,
                cursor,
                viewport,
            );
        };
        if let Kind::Drawer(..) = self.kind {
            let v = self.slide(pb, pp);
            renderer.with_layer(b, |renderer| {
                if pp < 1.0 {
                    renderer.with_translation(v, draw_panel);
                } else {
                    draw_panel(renderer);
                }
            });
            return;
        }
        let k = 0.96 + 0.04 * pp;
        let c = pb.center();
        let tf = Transformation::translate(c.x, c.y)
            * Transformation::scale(k)
            * Transformation::translate(-c.x, -c.y);
        // Without a cover colour the panel fades with the group opacity.
        let alpha = if self.cover.is_some() { 1.0 } else { pp };
        renderer.with_layer(b, |renderer| {
            with_opacity(alpha, || {
                if pp < 1.0 {
                    renderer.with_transformation(tf, draw_panel);
                } else {
                    draw_panel(renderer);
                }
            });
        });
        if let Some(cover) = self.cover
            && pp < 1.0
        {
            renderer.with_layer(b, |renderer| {
                renderer.with_transformation(tf, |renderer| {
                    renderer.fill_quad(
                        Quad {
                            bounds: pb.expand(1.0),
                            border: Border {
                                radius: 17.0.into(),
                                ..Border::default()
                            },
                            ..Quad::default()
                        },
                        Color {
                            a: 1.0 - pp,
                            ..cover
                        },
                    );
                });
            });
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        // Popovers and menus inside the panel (z-50 over the modal's z-40).
        self.panel.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: Clone + 'a> From<Modal<'a, Message>> for El<'a, Message> {
    fn from(m: Modal<'a, Message>) -> Self {
        Element::new(m)
    }
}

/// The dialog panel (`Modal.tsx` markup): header with an optional AppMark
/// (40 px), the 16 px semibold title and 13 px description, a ghost close
/// button; a hairline; the body with `py-5 px-6`. `bg-app-card`, radius 16,
/// shadow lg. Build `body` with `cx.on_card()`.
pub fn modal_panel<'a, Message: Clone + 'a>(
    cx: &crate::Ctx,
    title: &str,
    description: Option<&str>,
    mark: Option<crate::icon::Ico>,
    on_close: Message,
    body: impl Into<Element<'a, Message, iced::Theme, iced::Renderer>>,
) -> Element<'a, Message, iced::Theme, iced::Renderer> {
    use iced::widget::{column, container, row};
    use iced::{Alignment, Padding};

    use crate::label::label;
    use crate::layout::{hline, vgap};
    use crate::style::{c, shadows};
    use crate::surface::{BoxStyle, surface};

    let cx = cx.on_card();
    let p = cx.p();
    let mut head: Vec<Element<'a, Message, iced::Theme, iced::Renderer>> = Vec::new();
    if let Some(glyph) = mark {
        head.push(crate::kbd::app_mark(&cx, glyph, 40.0));
    }
    // The title block is `self-center` in a row as tall as its tallest item.
    let title_h: f32 = 24.0 + if description.is_some() { 21.5 } else { 0.0 };
    let head_h = title_h
        .max(if mark.is_some() { 40.0 } else { 0.0 })
        .max(32.0);
    let mut titles: Vec<Element<'a, Message, iced::Theme, iced::Renderer>> =
        vec![label(title, 16.0, 24.0).semibold().color(c(p.text)).into()];
    if let Some(d) = description {
        titles.push(vgap(2.0));
        titles.push(label(d, 13.0, 19.5).color(c(p.subtext)).into());
    }
    head.push(
        container(column(titles))
            .width(Length::Fill)
            .height(head_h)
            .align_y(Alignment::Center)
            .into(),
    );
    head.push(
        crate::button::button(&cx)
            .ghost()
            .square(32)
            .icon_sized(crate::icon::Ico::X, 16.0)
            .on_press(on_close)
            .into(),
    );
    let header = container(row(head).spacing(16).align_y(Alignment::Start)).padding(Padding {
        top: 20.0,
        right: 16.0,
        bottom: 16.0,
        left: 24.0,
    });
    let lg = BoxStyle {
        bg: Some(c(p.card)),
        radius: 16.0,
        shadows: shadows(&p.shadow_lg),
        ..BoxStyle::default()
    };
    surface(column![
        header,
        hline(c(cx.line())),
        container(body)
            .padding(Padding::from([20.0, 24.0]))
            .width(Length::Fill)
    ])
    .width(Length::Fill)
    .fixed(lg)
    .into()
}

/// The drawer panel (`Drawer.tsx` markup): `bg-app-card`, a 1 px
/// `lineStrong` border on the inner edge, shadow lg; an optional header (14/20
/// semibold title, a muted ✕ that turns `text` on hover, `px-5 py-4`,
/// bottom border `app-border`) over the body with `p-5`. Put it in
/// `level(Kind::Drawer(side, 320.0), …)`; build `body` with `cx.on_card()`.
pub fn drawer_panel<'a, Message: Clone + 'a>(
    cx: &crate::Ctx,
    side: Side,
    title: Option<&str>,
    on_close: Message,
    body: impl Into<El<'a, Message>>,
) -> El<'a, Message> {
    use iced::widget::{column, container, row};
    use iced::{Alignment, Padding};

    use crate::icon::{Ico, icon};
    use crate::label::label;
    use crate::layout::{hline, vline};
    use crate::style::{c, shadows};
    use crate::surface::{BoxStyle, Status, surface};

    let cx = cx.on_card();
    let p = cx.p();
    let mut col: Vec<El<'a, Message>> = Vec::new();
    if let Some(t) = title {
        let (muted, text) = (c(p.muted), c(p.text));
        let close = surface(icon(Ico::X, 16.0))
            .style(move |s| BoxStyle::default().fg(if s == Status::Hover { text } else { muted }))
            .on_press(on_close);
        col.push(
            container(
                row![
                    label(t.to_string(), 14.0, 20.0)
                        .semibold()
                        .color(c(p.text))
                        .width(Length::Fill),
                    close
                ]
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([16.0, 20.0]))
            .into(),
        );
        col.push(hline(c(p.border)));
    }
    col.push(
        container(body)
            .padding(20)
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
    );
    let edge = c(cx.line_strong());
    let content: El<'a, Message> = column(col).width(Length::Fill).height(Length::Fill).into();
    let framed: El<'a, Message> = match side {
        Side::Right => row![vline(edge), content].into(),
        Side::Left => row![content, vline(edge)].into(),
        Side::Bottom => column![hline(edge), content].into(),
        Side::Top => column![content, hline(edge)].into(),
    };
    surface(framed)
        .width(Length::Fill)
        .height(Length::Fill)
        .fixed(BoxStyle {
            bg: Some(c(p.card)),
            shadows: shadows(&p.shadow_lg),
            ..BoxStyle::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lvl(kind: Kind) -> Modal<'static, ()> {
        level(kind, iced::widget::Space::new(), ())
    }

    #[test]
    fn placement_per_kind() {
        let win = Size::new(1280.0, 800.0);
        let m = lvl(Kind::Center);
        let l = m.limits(win);
        assert_eq!(l.max(), Size::new(448.0, 680.0));
        assert_eq!(
            m.place(win, Size::new(448.0, 300.0)),
            Point::new(416.0, 250.0)
        );
        let p = lvl(Kind::Top);
        assert_eq!(p.limits(win).max().width, 512.0);
        assert_eq!(
            p.place(win, Size::new(512.0, 200.0)),
            Point::new(384.0, 96.0)
        );
        let d = lvl(Kind::Drawer(Side::Right, 320.0));
        assert_eq!(d.limits(win).max(), Size::new(320.0, 800.0));
        assert_eq!(
            d.place(win, Size::new(320.0, 800.0)),
            Point::new(960.0, 0.0)
        );
        let b = lvl(Kind::Drawer(Side::Bottom, 240.0));
        assert_eq!(
            b.place(win, Size::new(1280.0, 240.0)),
            Point::new(0.0, 560.0)
        );
        // A narrow window keeps the 16 px margins.
        assert_eq!(m.limits(Size::new(400.0, 800.0)).max().width, 368.0);
    }

    #[test]
    fn scrim_and_drawer_slide() {
        assert_eq!(lvl(Kind::Center).scrim, 0.5);
        assert_eq!(lvl(Kind::Drawer(Side::Left, 320.0)).scrim, 0.6);
        let d = lvl(Kind::Drawer(Side::Right, 320.0));
        let pb = Rectangle::new(Point::new(960.0, 0.0), Size::new(320.0, 800.0));
        assert_eq!(d.slide(pb, 0.0), Vector::new(320.0, 0.0));
        assert_eq!(d.slide(pb, 1.0), Vector::new(0.0, 0.0));
        let t = lvl(Kind::Drawer(Side::Top, 200.0));
        assert_eq!(t.slide(pb, 0.5), Vector::new(0.0, -400.0));
        assert_eq!(lvl(Kind::Center).slide(pb, 0.0), Vector::ZERO);
    }

    #[test]
    fn scroll_keys_are_locked() {
        use keyboard::key::Named;
        assert!(scrolls_page(&keyboard::Key::Named(Named::PageDown)));
        assert!(scrolls_page(&keyboard::Key::Named(Named::Space)));
        assert!(!scrolls_page(&keyboard::Key::Named(Named::Tab)));
        assert!(!scrolls_page(&keyboard::Key::Character("a".into())));
    }
}
