//! Masonry: CSS `columns-N gap-G` with `break-inside-avoid` children. Items
//! flow in order down the first column, then the next, with the column
//! height balanced the way CSS multi-column balancing does: the smallest
//! height at which the items fit into N columns.

use iced::advanced::layout;
use iced::advanced::widget::Widget;
use iced::{Element, Length, Point, Size};

use super::{forward_cells, into_element};

pub struct Masonry<'a, Message> {
    cells: Vec<Element<'a, Message, iced::Theme, iced::Renderer>>,
    columns: usize,
    gap: f32,
}

pub fn masonry<'a, Message>(
    columns: usize,
    gap: f32,
    cells: Vec<Element<'a, Message, iced::Theme, iced::Renderer>>,
) -> Masonry<'a, Message> {
    Masonry {
        cells,
        columns: columns.max(1),
        gap,
    }
}

/// Column index of each item for heights `hs`, balanced into `n` columns
/// (items keep their order; a column holds a contiguous run).
pub(crate) fn balance(hs: &[f32], n: usize, gap: f32) -> Vec<usize> {
    let fill = |limit: f32| -> Option<Vec<usize>> {
        let (mut col, mut used) = (0usize, 0.0f32);
        let mut out = Vec::with_capacity(hs.len());
        for &h in hs {
            let need = if used > 0.0 { used + gap + h } else { h };
            if need > limit + 0.01 && used > 0.0 {
                col += 1;
                used = h;
            } else {
                used = need;
            }
            if col >= n {
                return None;
            }
            out.push(col);
        }
        Some(out)
    };
    let (mut lo, mut hi) = (
        hs.iter().copied().fold(0.0, f32::max),
        hs.iter().sum::<f32>() + gap * hs.len() as f32,
    );
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if fill(mid).is_some() {
            hi = mid
        } else {
            lo = mid
        }
    }
    fill(hi).unwrap_or_else(|| vec![0; hs.len()])
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for Masonry<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }
    fn layout(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let total = limits.max().width;
        let n = self.columns as f32;
        let w = ((total - self.gap * (n - 1.0)) / n).max(0.0);
        let nodes: Vec<layout::Node> = self
            .cells
            .iter_mut()
            .zip(tree.children.iter_mut())
            .map(|(cell, t)| {
                cell.as_widget_mut().layout(
                    t,
                    renderer,
                    &layout::Limits::new(Size::new(w, 0.0), Size::new(w, f32::INFINITY)),
                )
            })
            .collect();
        let hs: Vec<f32> = nodes.iter().map(|n| n.size().height).collect();
        let cols = balance(&hs, self.columns, self.gap);
        let mut ys = vec![0.0f32; self.columns];
        let mut height: f32 = 0.0;
        let placed = nodes
            .into_iter()
            .zip(&cols)
            .map(|(node, &c)| {
                let y = ys[c];
                ys[c] += node.size().height + self.gap;
                height = height.max(y + node.size().height);
                node.move_to(Point::new(c as f32 * (w + self.gap), y))
            })
            .collect();
        layout::Node::with_children(Size::new(total, height), placed)
    }

    forward_cells!();
}

into_element!(Masonry);

#[cfg(test)]
mod tests {
    use super::balance;

    #[test]
    fn balances_in_order() {
        assert_eq!(balance(&[100.0, 100.0, 100.0], 3, 0.0), vec![0, 1, 2]);
        assert_eq!(balance(&[50.0, 50.0, 100.0], 2, 0.0), vec![0, 0, 1]);
        assert_eq!(balance(&[10.0; 6], 3, 4.0), vec![0, 0, 1, 1, 2, 2]);
    }
}
