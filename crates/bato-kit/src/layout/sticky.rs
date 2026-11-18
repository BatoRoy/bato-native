//! `position: sticky; top: 0` for one bar in a page column: a toolbar or a
//! progress bar that pins to the top of the enclosing scroller (the
//! frame's page scroller) while the content under it scrolls.
//!
//! [`sticky()`] lays out `before`, `bar` and `after` as a column (with `gap`
//! between them), then draws the bar shifted down by however far the
//! visible area starts below the bar's own place, clamped to the column's
//! end (CSS keeps a sticky box inside its parent). The bar is drawn last, in
//! its own layer, so what scrolls under it stays under it. With
//! [`Sticky::backdrop`] its background bleeds `bleed` px left and right
//! (`-mx-8 px-8`) and has a 1 px line at the bottom (`border-b`).
//!
//! Pointer events over the shifted bar go to the bar (with the cursor moved
//! into its layout coordinates) and not to what it covers.
//!
//! ```ignore
//! sticky(header, toolbar, grid, 16.0).backdrop(c(p.bg), c(cx.line()), 32.0)
//! ```

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::{Color, Element, Event, Length, Point, Rectangle, Size, Vector, mouse};

use crate::El;

pub struct Sticky<'a, M> {
    /// before, bar, after.
    cells: [El<'a, M>; 3],
    gap: f32,
    bleed: f32,
    bg: Color,
    line: Color,
}

pub fn sticky<'a, M>(
    before: impl Into<El<'a, M>>,
    bar: impl Into<El<'a, M>>,
    after: impl Into<El<'a, M>>,
    gap: f32,
) -> Sticky<'a, M> {
    Sticky {
        cells: [before.into(), bar.into(), after.into()],
        gap,
        bleed: 0.0,
        bg: Color::TRANSPARENT,
        line: Color::TRANSPARENT,
    }
}

impl<'a, M> Sticky<'a, M> {
    /// The bar's background, bleeding `bleed` px past each side, and its
    /// bottom line.
    pub fn backdrop(mut self, bg: Color, line: Color, bleed: f32) -> Self {
        self.bg = bg;
        self.line = line;
        self.bleed = bleed;
        self
    }
}

/// How far the bar at `bar` (inside a column of `total`) moves down so its
/// top meets the visible area's top `view_top`: never up, never past the
/// column's end.
fn shift(bar: Rectangle, total: Rectangle, view_top: f32) -> f32 {
    let max = (total.y + total.height - (bar.y + bar.height)).max(0.0);
    (view_top - bar.y).clamp(0.0, max)
}

impl<'a, M> Sticky<'a, M> {
    fn bar_shift(&self, layout: Layout<'_>, viewport: &Rectangle) -> f32 {
        let bar = layout.children().nth(1).expect("bar").bounds();
        shift(bar, layout.bounds(), viewport.y)
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for Sticky<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn children(&self) -> Vec<Tree> {
        self.cells.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.cells);
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let w = limits.max().width;
        let loose = layout::Limits::new(Size::ZERO, Size::new(w, f32::INFINITY));
        let mut y = 0.0;
        let mut nodes = Vec::with_capacity(3);
        for (i, (cell, t)) in self.cells.iter_mut().zip(&mut tree.children).enumerate() {
            if i > 0 {
                y += self.gap;
            }
            let n = cell.as_widget_mut().layout(t, renderer, &loose);
            let h = n.size().height;
            nodes.push(n.move_to(Point::new(0.0, y)));
            y += h;
        }
        layout::Node::with_children(Size::new(w, y), nodes)
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
        let dy = self.bar_shift(layout, viewport);
        let bar = layout.children().nth(1).expect("bar").bounds();
        let over_bar = cursor.is_over(bar + Vector::new(0.0, dy));
        for (i, ((cell, t), l)) in self
            .cells
            .iter_mut()
            .zip(tree.children.iter_mut())
            .zip(layout.children())
            .enumerate()
        {
            if i == 1 {
                let c = cursor_shifted(cursor, -dy);
                let vp = *viewport - Vector::new(0.0, dy);
                cell.as_widget_mut()
                    .update(t, event, l, c, renderer, clipboard, shell, &vp);
            } else {
                let c = if over_bar {
                    mouse::Cursor::Unavailable
                } else {
                    cursor
                };
                cell.as_widget_mut()
                    .update(t, event, l, c, renderer, clipboard, shell, viewport);
            }
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
        let dy = self.bar_shift(layout, viewport);
        let mut ls = layout.children();
        let (before, bar, after) = (ls.next().unwrap(), ls.next().unwrap(), ls.next().unwrap());
        if cursor.is_over(bar.bounds() + Vector::new(0.0, dy)) {
            return self.cells[1].as_widget().mouse_interaction(
                &tree.children[1],
                bar,
                cursor_shifted(cursor, -dy),
                &(*viewport - Vector::new(0.0, dy)),
                renderer,
            );
        }
        [(0, before), (2, after)]
            .into_iter()
            .map(|(i, l)| {
                self.cells[i].as_widget().mouse_interaction(
                    &tree.children[i],
                    l,
                    cursor,
                    viewport,
                    renderer,
                )
            })
            .max()
            .unwrap_or_default()
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        operation.traverse(&mut |operation| {
            for ((cell, t), l) in self
                .cells
                .iter_mut()
                .zip(tree.children.iter_mut())
                .zip(layout.children())
            {
                cell.as_widget_mut().operate(t, l, renderer, operation);
            }
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
        use iced::advanced::Renderer as _;
        let dy = self.bar_shift(layout, viewport);
        let mut ls = layout.children();
        let (before, bar, after) = (ls.next().unwrap(), ls.next().unwrap(), ls.next().unwrap());
        let shifted = bar.bounds() + Vector::new(0.0, dy);
        let under = if cursor.is_over(shifted) {
            mouse::Cursor::Unavailable
        } else {
            cursor
        };
        for (i, l) in [(0, before), (2, after)] {
            self.cells[i].as_widget().draw(
                &tree.children[i],
                renderer,
                theme,
                style,
                l,
                under,
                viewport,
            );
        }
        let back = Rectangle {
            x: shifted.x - self.bleed,
            width: shifted.width + 2.0 * self.bleed,
            ..shifted
        };
        // Its own layer, created after everything above: on top of the cards.
        let clip = Rectangle {
            x: back.x,
            width: back.width,
            ..*viewport
        };
        renderer.with_layer(clip, |r| {
            r.fill_quad(
                Quad {
                    bounds: back,
                    ..Quad::default()
                },
                self.bg,
            );
            r.fill_quad(
                Quad {
                    bounds: Rectangle {
                        y: back.y + back.height - 1.0,
                        height: 1.0,
                        ..back
                    },
                    ..Quad::default()
                },
                self.line,
            );
            r.with_translation(Vector::new(0.0, dy), |r| {
                self.cells[1].as_widget().draw(
                    &tree.children[1],
                    r,
                    theme,
                    style,
                    bar,
                    cursor_shifted(cursor, -dy),
                    &(*viewport - Vector::new(0.0, dy)),
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
    ) -> Option<overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        overlay::from_children(
            &mut self.cells,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

fn cursor_shifted(cursor: mouse::Cursor, dy: f32) -> mouse::Cursor {
    match cursor {
        mouse::Cursor::Available(p) => mouse::Cursor::Available(p + Vector::new(0.0, dy)),
        other => other,
    }
}

impl<'a, M: 'a> From<Sticky<'a, M>> for El<'a, M> {
    fn from(w: Sticky<'a, M>) -> Self {
        Element::new(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(y: f32, h: f32) -> Rectangle {
        Rectangle {
            x: 0.0,
            y,
            width: 100.0,
            height: h,
        }
    }

    #[test]
    fn sticks_to_the_top_inside_its_column() {
        let total = r(40.0, 1000.0);
        let bar = r(120.0, 60.0);
        // Not scrolled past it yet: in place.
        assert_eq!(shift(bar, total, 0.0), 0.0);
        assert_eq!(shift(bar, total, 120.0), 0.0);
        // Scrolled past: pinned to the visible top.
        assert_eq!(shift(bar, total, 300.0), 180.0);
        // Never past the column's end (bar bottom at 1040).
        assert_eq!(shift(bar, total, 5000.0), 860.0);
    }
}
