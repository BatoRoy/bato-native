//! The vim status bar (`.cm-vim-panel` in `index.css`): `--NORMAL--` on
//! the left, the pending keys on the right, or the `:` / `/` prompt.
//! `padding: 1px 8px`, JetBrains Mono 12 px (line-height 1.5 from the
//! page), `border-top: 1px border`, `surface` background, `subtext` text.
//! It reads the [`Content`](super::Content) when it draws, so mode changes
//! show without a message.

use std::cell::RefCell;

use iced::advanced::graphics::text::cosmic_text as ct;
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{Tree, Widget};
use iced::{Element, Length, Rectangle, Size, mouse};

use bato_theme::Palette;

use super::Content;
use super::engine::Doc;
use crate::label::{Shaped, label};
use crate::scale::device_scale;
use crate::style::c;

/// 1 px border + 1 px padding × 2 + an 18 px line.
pub const HEIGHT: f32 = 21.0;
const PAD_X: f32 = 8.0;

/// `--NORMAL--` and the like (`updateStatus`).
pub fn mode_label(doc: &Doc) -> Option<String> {
    let mode = doc.vi_mode()?;
    Some(match mode {
        ct::ViMode::Insert => "--INSERT--".into(),
        ct::ViMode::Replace => "--REPLACE--".into(),
        ct::ViMode::Visual => "--VISUAL--".into(),
        ct::ViMode::VisualLine => "--VISUAL LINE--".into(),
        ct::ViMode::Command { value } => format!(":{value}"),
        ct::ViMode::Search { value, forwards } => {
            format!("{}{value}", if forwards { '/' } else { '?' })
        }
        ct::ViMode::Normal | ct::ViMode::Extra(_) => "--NORMAL--".into(),
    })
}

#[derive(Default)]
pub(crate) struct Cache {
    key: Option<(String, String, u32)>,
    left: Option<Shaped>,
    right: Option<Shaped>,
}

pub(crate) struct StatusBar<'a> {
    pub content: &'a Content,
    pub palette: Palette,
    pub cache: RefCell<Cache>,
}

impl<M> Widget<M, iced::Theme, iced::Renderer> for StatusBar<'_> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(HEIGHT))
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.resolve(Length::Fill, HEIGHT, Size::ZERO))
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let b = layout.bounds();
        let p = &self.palette;
        let s = device_scale();
        renderer.fill_quad(
            Quad {
                bounds: Rectangle {
                    height: s.round().max(1.0) / s,
                    ..b
                },
                ..Quad::default()
            },
            c(p.border),
        );
        let (left, right) = {
            let doc = self.content.0.borrow();
            (mode_label(&doc).unwrap_or_default(), doc.vi_pending())
        };
        let key = (left.clone(), right.clone(), s.to_bits());
        let mut cache = self.cache.borrow_mut();
        if cache.key.as_ref() != Some(&key) {
            let shape = |t: &str| {
                (!t.is_empty())
                    .then(|| label(t.to_string(), 12.0, 18.0).mono().shape(f32::INFINITY))
            };
            cache.left = shape(&left);
            cache.right = shape(&right);
            cache.key = Some(key);
        }
        let text_y = b.y + 2.0;
        let color = c(p.subtext);
        if let Some(l) = &cache.left {
            l.draw(
                renderer,
                Rectangle::new(
                    iced::Point::new(b.x + PAD_X, text_y),
                    Size::new(l.width(), 18.0),
                ),
                color,
                viewport,
            );
        }
        if let Some(r) = &cache.right {
            r.draw(
                renderer,
                Rectangle::new(
                    iced::Point::new(b.x + b.width - PAD_X - r.width(), text_y),
                    Size::new(r.width(), 18.0),
                ),
                color,
                viewport,
            );
        }
    }
}

impl<'a, M: 'a> From<StatusBar<'a>> for Element<'a, M, iced::Theme, iced::Renderer> {
    fn from(s: StatusBar<'a>) -> Self {
        Element::new(s)
    }
}
