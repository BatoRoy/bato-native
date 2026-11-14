//! `Modal` overlay (spec §4.3): full-window scrim `#000@.5` (+ backdrop blur,
//! see `blur.rs`), panel centred with 16 px margin, max 85% of the window
//! height. Enter: overlay fadeIn 150 ms ease-out, panel scaleIn .96→1 +
//! opacity 0→1, 120 ms ease-out. No exit animation. Closes when a press
//! both starts and ends on the scrim, or on Escape.
//!
//! iced has no group opacity, so the panel's fade is drawn as a cover quad in
//! the scrim's composite colour fading out over the panel.

use std::time::Instant;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::{
    Border, Color, Element, Event, Length, Rectangle, Size, Transformation, keyboard, mouse, window,
};

use crate::anim::ease_out;

pub struct Modal<'a, Message> {
    panel: Element<'a, Message, iced::Theme, iced::Renderer>,
    max_width: f32,
    on_close: Message,
    /// Opaque colour of the scrim over the page, for the panel fade cover.
    cover: Color,
    backdrop: Option<Element<'a, Message, iced::Theme, iced::Renderer>>,
    animate: bool,
}

/// `panel` centred over the window, at most `max_width` wide (`max-w-md` =
/// 448, `lg` = 512); `cover` is the opaque scrim-over-page colour
/// (`blend(#000, .5, bg)`) used for the panel's fade.
pub fn modal<'a, Message>(
    panel: impl Into<Element<'a, Message, iced::Theme, iced::Renderer>>,
    max_width: f32,
    on_close: Message,
    cover: Color,
) -> Modal<'a, Message> {
    Modal {
        panel: panel.into(),
        max_width,
        on_close,
        cover,
        backdrop: None,
        animate: true,
    }
}

impl<'a, Message> Modal<'a, Message> {
    /// Drawn under the scrim (the blurred page).
    pub fn backdrop(
        mut self,
        b: impl Into<Element<'a, Message, iced::Theme, iced::Renderer>>,
    ) -> Self {
        self.backdrop = Some(b.into());
        self
    }
    pub fn animate(mut self, on: bool) -> Self {
        self.animate = on;
        self
    }
}

#[derive(Default)]
struct State {
    opened: Option<Instant>,
    progress_overlay: f32,
    progress_panel: f32,
    down_on_backdrop: bool,
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
        let mut v: Vec<&Element<'a, Message, iced::Theme, iced::Renderer>> = vec![&self.panel];
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
        let w = self.max_width.min(size.width - 32.0);
        let max_h = (size.height * 0.85).floor();
        let l = layout::Limits::new(Size::new(w, 0.0), Size::new(w, max_h));
        let panel = self
            .panel
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &l);
        let ps = panel.size();
        let x = ((size.width - ps.width) / 2.0).round();
        let y = ((size.height - ps.height) / 2.0).round();
        let mut children = vec![panel.move_to(iced::Point::new(x, y))];
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
                st.progress_overlay = ease_out((t / 0.150).min(1.0));
                st.progress_panel = ease_out((t / 0.120).min(1.0));
                if t < 0.150 {
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
        self.panel.as_widget_mut().operate(
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
        // The blur primitive (if any) samples everything drawn before it, so the
        // scrim must live in a later layer.
        renderer.with_layer(b, |renderer| {
            renderer.fill_quad(
                Quad {
                    bounds: b,
                    ..Quad::default()
                },
                Color::from_rgba(0.0, 0.0, 0.0, 0.5 * po),
            );
        });
        let pb = panel_layout.bounds();
        let k = 0.96 + 0.04 * pp;
        let c = pb.center();
        let tf = Transformation::translate(c.x, c.y)
            * Transformation::scale(k)
            * Transformation::translate(-c.x, -c.y);
        renderer.with_layer(b, |renderer| {
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
            if pp < 1.0 {
                renderer.with_transformation(tf, draw_panel);
            } else {
                draw_panel(renderer);
            }
        });
        if pp < 1.0 {
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
                            ..self.cover
                        },
                    );
                });
            });
        }
    }
}

impl<'a, Message: Clone + 'a> From<Modal<'a, Message>>
    for Element<'a, Message, iced::Theme, iced::Renderer>
{
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
