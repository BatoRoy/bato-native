//! `Skeleton`: `animate-pulse bg-app-text/[0.07] rounded`, a placeholder bar.
//! The pulse (opacity 1 → .5 → 1 every 2 s) is folded into the colour
//! against the backdrop, so the bar stays opaque. Asks for frames only while
//! drawn, in phase with every other pulse ([`crate::anim::epoch`]).

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{Tree, Widget};
use iced::{Border, Color, Element, Length, Rectangle, Size, mouse};

use crate::anim::{epoch, fade, pulse};
use crate::ctx::Ctx;
use crate::scale::snap_rect;
use crate::style::{c, lerp};

pub struct Skeleton {
    fill: Color,
    over: Color,
    width: Length,
    fraction: f32,
    height: f32,
    radius: f32,
    animate: bool,
}

/// A full-width, 16 px tall (`h-4`) skeleton bar on `cx`'s backdrop.
pub fn skeleton(cx: &Ctx) -> Skeleton {
    Skeleton {
        fill: c(cx.wash(0.07)),
        over: c(cx.over),
        width: Length::Fill,
        fraction: 1.0,
        height: 16.0,
        radius: 4.0,
        animate: true,
    }
}

impl Skeleton {
    /// A share of the available width (`w-3/4` → 0.75).
    pub fn fraction(mut self, f: f32) -> Self {
        self.width = Length::Fill;
        self.fraction = f.clamp(0.0, 1.0);
        self
    }
    /// A fixed width in px.
    pub fn width(mut self, w: f32) -> Self {
        self.width = Length::Fixed(w);
        self.fraction = 1.0;
        self
    }
    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }
    /// Corner radius (default 4, `rounded`; `rounded-full` = height / 2).
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
    /// Turn the pulse off (reduced motion, screenshots).
    pub fn still(mut self) -> Self {
        self.animate = false;
        self
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Skeleton {
    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Fixed(self.height))
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let w = match self.width {
            Length::Fixed(w) => w,
            _ => limits.max().width * self.fraction,
        };
        layout::Node::new(Size::new(w, self.height))
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &iced::Event,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if self.animate
            && matches!(
                event,
                iced::Event::Window(iced::window::Event::RedrawRequested(_))
            )
            && layout.bounds().intersects(viewport)
        {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let o = if self.animate {
            pulse(epoch().elapsed())
        } else {
            1.0
        };
        renderer.fill_quad(
            Quad {
                bounds: snap_rect(layout.bounds()),
                border: Border {
                    radius: self.radius.into(),
                    ..Border::default()
                },
                ..Quad::default()
            },
            fade(lerp(self.over, self.fill, o)),
        );
    }
}

impl<'a, Message: 'a> From<Skeleton> for Element<'a, Message, iced::Theme, iced::Renderer> {
    fn from(s: Skeleton) -> Self {
        Element::new(s)
    }
}
