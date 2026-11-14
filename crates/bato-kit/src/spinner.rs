//! `Spinner`: `border-2 border-current border-t-transparent rounded-full
//! animate-spin`. The visible border is a 270° arc whose ends are the
//! diagonal colour joins of a CSS border (radial at ±45° from the top); it is
//! rasterized by resvg like the icons and spun once per second. Asks for
//! frames only while it is drawn.

use std::cell::RefCell;
use std::collections::HashMap;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::svg::{self, Renderer as _};
use iced::advanced::widget::{Tree, Widget};
use iced::{Color, Element, Length, Radians, Rectangle, Size, mouse};

use crate::anim::{current_opacity, epoch};
use crate::scale::snap_rect;

/// The SVG of a `size` px spinner (border 2).
pub fn spinner_svg(size: f32) -> String {
    let r = size / 2.0 - 1.0;
    let (cx, cy) = (size / 2.0, size / 2.0);
    let at = |deg: f32| {
        let t = deg.to_radians();
        (cx + r * t.sin(), cy - r * t.cos())
    };
    let (x0, y0) = at(45.0);
    let (x1, y1) = at(315.0);
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}"><path d="M{x0:.4} {y0:.4}A{r} {r} 0 1 1 {x1:.4} {y1:.4}" fill="none" stroke="#000000" stroke-width="2"/></svg>"##
    )
}

thread_local! {
    static HANDLES: RefCell<HashMap<u32, svg::Handle>> = RefCell::new(HashMap::new());
}

pub struct Spinner {
    size: f32,
    color: Option<Color>,
}

/// A `size` px spinner (default in React: 16) in the inherited text colour.
pub fn spinner(size: f32) -> Spinner {
    Spinner { size, color: None }
}

impl Spinner {
    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }
}

impl<Message, Theme> Widget<Message, Theme, iced::Renderer> for Spinner {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.size), Length::Fixed(self.size))
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &iced::Renderer, _limits: &layout::Limits) -> layout::Node {
        layout::Node::new(Size::new(self.size, self.size))
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &iced::Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        if matches!(event, iced::Event::Window(iced::window::Event::RedrawRequested(_))) {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let color = self.color.unwrap_or(style.text_color);
        let handle = HANDLES.with(|h| {
            h.borrow_mut()
                .entry(self.size.to_bits())
                .or_insert_with(|| svg::Handle::from_memory(spinner_svg(self.size).into_bytes()))
                .clone()
        });
        let rotation = (epoch().elapsed().as_secs_f32() % 1.0) * std::f32::consts::TAU;
        renderer.draw_svg(
            svg::Svg { handle, color: Some(Color { a: 1.0, ..color }), rotation: Radians(rotation), opacity: color.a * current_opacity() },
            snap_rect(layout.bounds()),
            viewport.expand(2.0),
        );
    }
}

impl<'a, Message: 'a, Theme: 'a> From<Spinner> for Element<'a, Message, Theme, iced::Renderer> {
    fn from(s: Spinner) -> Self {
        Element::new(s)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn arc_leaves_the_top_quarter_open() {
        let s = super::spinner_svg(16.0);
        // r = 7 around (8, 8): from 45° (12.95, 3.05) the long way to 315°.
        assert!(s.contains("M12.9497 3.0503A7 7 0 1 1 3.0503 3.0503"), "{s}");
    }
}
