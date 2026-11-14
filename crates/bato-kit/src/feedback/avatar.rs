//! `Avatar` (initials) and `AvatarGroup`.
//!
//! An avatar is a circle in `accent/20` with `accentBright` initials
//! (medium). The group overlaps them by 8 px (`-space-x-2`), each wrapped in
//! a 2 px `app-bg` ring, and ends with a `+N` chip (`bg-app-card`,
//! subtext) past `max`. Like CSS, the 20 % fill is translucent, so where two
//! avatars overlap the earlier one shows through the later.
//!
//! Images (`src`) are not supported yet: an avatar always shows initials,
//! which is also React's fallback when the image fails.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{Tree, Widget};
use iced::{Border, Color, Element, Length, Point, Rectangle, Size, mouse};

use crate::anim::fade;
use crate::scale::snap_rect;

use crate::El;
use crate::ctx::Ctx;
use crate::label::label;
use crate::style::{c, rgba};
use crate::surface::{BoxStyle, surface};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AvatarSize {
    /// 24 px, 10 px text
    Xs,
    /// 32 px, 12/16
    Sm,
    /// 40 px, 14/20
    #[default]
    Md,
    /// 48 px, 16/24
    Lg,
}

impl AvatarSize {
    /// (diameter, font size, line height)
    pub fn metrics(self) -> (f32, f32, f32) {
        match self {
            AvatarSize::Xs => (24.0, 10.0, 15.0),
            AvatarSize::Sm => (32.0, 12.0, 16.0),
            AvatarSize::Md => (40.0, 14.0, 20.0),
            AvatarSize::Lg => (48.0, 16.0, 24.0),
        }
    }
}

/// React's `initials`: the first letter of the first two words, upper-case,
/// or `?`.
pub fn initials(name: &str) -> String {
    let s: String = name
        .split_whitespace()
        .take(2)
        .filter_map(|w| w.chars().next())
        .flat_map(char::to_uppercase)
        .collect();
    if s.is_empty() { "?".into() } else { s }
}

fn circle<'a, M: Clone + 'a>(text: String, size: AvatarSize, bg: Color, fg: Color) -> El<'a, M> {
    let (d, fs, lh) = size.metrics();
    surface(label(text, fs, lh).medium().color(fg))
        .width(d)
        .height(d)
        .center()
        .fixed(BoxStyle::default().bg(bg).radius(d / 2.0))
        .into()
}

/// `<Avatar name size>`.
pub fn avatar<'a, M: Clone + 'a>(cx: &Ctx, name: &str, size: AvatarSize) -> El<'a, M> {
    let p = cx.p();
    circle(initials(name), size, c(cx.tint(0.20)), c(p.accent_bright))
}

/// `<AvatarGroup avatars max size>`.
pub fn avatar_group<'a, M: Clone + 'a>(
    cx: &Ctx,
    names: &[&str],
    max: usize,
    size: AvatarSize,
) -> El<'a, M> {
    let p = cx.p();
    let shown = names.len().min(max);
    let mut cells: Vec<El<'a, M>> = names[..shown]
        .iter()
        // Translucent like CSS: the avatar underneath shows through.
        .map(|n| circle(initials(n), size, rgba(p.accent, 0.20), c(p.accent_bright)))
        .collect();
    let extra = names.len() - shown;
    if extra > 0 {
        cells.push(circle(format!("+{extra}"), size, c(p.card), c(p.subtext)));
    }
    Element::new(Overlap {
        cells,
        overlap: 8.0,
        ring: (2.0, c(p.bg)),
    })
}

/// A row of circles overlapping by `overlap`, each with an outside ring
/// (`ring-2 ring-app-bg`) painted just before it, so it covers the previous
/// circle but not its own translucent fill.
struct Overlap<'a, Message> {
    cells: Vec<El<'a, Message>>,
    overlap: f32,
    ring: (f32, Color),
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for Overlap<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let mut x = 0.0f32;
        let mut h = 0.0f32;
        let mut nodes: Vec<layout::Node> = self
            .cells
            .iter_mut()
            .zip(tree.children.iter_mut())
            .enumerate()
            .map(|(i, (cell, t))| {
                if i > 0 {
                    x -= self.overlap;
                }
                let n = cell
                    .as_widget_mut()
                    .layout(t, renderer, &limits.loose())
                    .move_to(Point::new(x, 0.0));
                x += n.size().width;
                h = h.max(n.size().height);
                n
            })
            .collect();
        for n in &mut nodes {
            let y = (h - n.size().height) / 2.0;
            let px = n.bounds().x;
            *n = n.clone().move_to(Point::new(px, y));
        }
        layout::Node::with_children(Size::new(x.max(0.0), h), nodes)
    }

    fn children(&self) -> Vec<Tree> {
        self.cells.iter().map(Tree::new).collect()
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.cells);
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
        let (w, color) = self.ring;
        for ((cell, t), l) in self.cells.iter().zip(&tree.children).zip(layout.children()) {
            let b = snap_rect(l.bounds());
            let outer = b.expand(w);
            renderer.fill_quad(
                Quad {
                    bounds: outer,
                    border: Border {
                        radius: (outer.width / 2.0).into(),
                        width: w,
                        color: fade(color),
                    },
                    ..Quad::default()
                },
                Color::TRANSPARENT,
            );
            cell.as_widget()
                .draw(t, renderer, theme, style, l, cursor, viewport);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::initials;

    #[test]
    fn initials_like_react() {
        assert_eq!(initials("Ada Lovelace"), "AL");
        assert_eq!(initials("  cy   r  extra"), "CR");
        assert_eq!(initials("eve"), "E");
        assert_eq!(initials(""), "?");
        assert_eq!(initials("   "), "?");
    }
}
