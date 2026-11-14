//! `Collapsible` and `Accordion`.
//!
//! A collapsible is a `border border-app-border rounded-lg overflow-hidden`
//! box: a full-width header button (`px-4 py-3`, 14/20 medium, hover
//! `text@.06` with `transition-colors`) with a 16 px muted `ChevronDown`
//! that turns 180° (`transition-transform`, 150 ms), over a
//! [`crate::layout::collapse`] holding the body (`px-4 py-3`, 14/20
//! subtext, `border-t`). An accordion stacks them `space-y-2`; with
//! `multiple` off, opening one closes the others ([`toggle`]).
//!
//! Both are controlled: the app keeps which ids are open.
//!
//! ```ignore
//! accordion(&cx, items, &state.open, Msg::Toggle)        // Msg::Toggle(i) → toggle(&mut open, i, false)
//! collapsible(&cx, "Details", open, Msg::Flip, body)
//! ```

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Tree, Widget, tree};
use iced::widget::{Column, Space, column, container, row};
use iced::{Alignment, Color, Element, Length, Padding, Rectangle, Size, mouse};

use crate::El;
use crate::anim::{Tween, ease_standard};
use crate::ctx::Ctx;
use crate::focus::Ring;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::layout::{collapse, hline, rounded_clip};
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

/// React's `toggle`: close `id` if open; otherwise open it, alone unless
/// `multiple`.
pub fn toggle<T: PartialEq + Copy>(open: &mut Vec<T>, id: T, multiple: bool) {
    if let Some(i) = open.iter().position(|x| *x == id) {
        open.remove(i);
    } else if multiple {
        open.push(id);
    } else {
        open.clear();
        open.push(id);
    }
}

/// One collapsible section: `title`, `open`, `on_toggle` for the header,
/// and its body.
pub fn collapsible<'a, M: Clone + 'a>(
    cx: &Ctx,
    title: impl Into<String>,
    open: bool,
    on_toggle: M,
    body: impl Into<El<'a, M>>,
) -> El<'a, M> {
    let p = cx.p();
    let rest = BoxStyle::default();
    let hover = rest.clone().bg(c(cx.wash(0.06)));
    let header = surface(
        row![
            label(title, 14.0, 20.0).medium().color(c(p.text)),
            Space::new().width(Length::Fill),
            Element::new(Chevron {
                flipped: open,
                color: c(p.muted),
            }),
        ]
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(Padding::from([12.0, 16.0]))
    .on_press(on_toggle)
    .style(move |s| {
        if s == Status::Hover {
            hover.clone()
        } else {
            rest.clone()
        }
    })
    .focus_ring(Ring::plain(cx));
    let content = column![
        hline(c(p.border)),
        container(body).padding([12.0, 16.0]).width(Length::Fill),
    ];
    rounded_clip(8.0, c(cx.over), column![header, collapse(open, content)])
        .border(c(p.border))
        .into()
}

/// A body paragraph as React renders `content` text (14/20 subtext).
pub fn body_text<'a, M: 'a>(cx: &Ctx, text: impl Into<String>) -> El<'a, M> {
    label(text, 14.0, 20.0)
        .color(c(cx.p().subtext))
        .wrap()
        .into()
}

/// `<Accordion items>`: `(title, body)` pairs; `open` lists the open
/// indices; `on_toggle(i)` fires on a header (apply [`toggle`]).
pub fn accordion<'a, M: Clone + 'a>(
    cx: &Ctx,
    items: impl IntoIterator<Item = (String, El<'a, M>)>,
    open: &[usize],
    on_toggle: impl Fn(usize) -> M,
) -> El<'a, M> {
    let mut col: Column<'a, M, iced::Theme, iced::Renderer> = Column::new().spacing(8);
    for (i, (title, body)) in items.into_iter().enumerate() {
        col = col.push(collapsible(
            cx,
            title,
            open.contains(&i),
            on_toggle(i),
            body,
        ));
    }
    col.width(Length::Fill).into()
}

/// The 16 px chevron, turning 180° over 150 ms when `flipped` changes.
struct Chevron {
    flipped: bool,
    color: Color,
}

const TURN: Duration = Duration::from_millis(150);

impl<M> Widget<M, iced::Theme, iced::Renderer> for Chevron {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(16.0), Length::Fixed(16.0))
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Tween>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(Tween::settled(if self.flipped { 1.0 } else { 0.0 }))
    }
    fn diff(&self, tree: &mut Tree) {
        let tw = tree.state.downcast_mut::<Tween>();
        let target = if self.flipped { 1.0 } else { 0.0 };
        if tw.to != target {
            let now = Instant::now();
            let mut n = Tween::new(tw.value(now), target, TURN, ease_standard);
            n.start = now;
            *tw = n;
        }
    }
    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(Size::new(16.0, 16.0))
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced::Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        if let iced::Event::Window(iced::window::Event::RedrawRequested(now)) = event
            && tree.state.downcast_ref::<Tween>().is_running(*now)
        {
            shell.request_redraw();
        }
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
        let t = tree.state.downcast_ref::<Tween>().value(Instant::now());
        let i = icon(Ico::ChevronDown, 16.0)
            .color(self.color)
            .rotation(t * std::f32::consts::PI);
        <crate::icon::Icon as Widget<M, iced::Theme, iced::Renderer>>::draw(
            &i,
            &Tree::empty(),
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::toggle;

    #[test]
    fn toggle_single_and_multiple() {
        let mut open = vec![];
        toggle(&mut open, 1, false);
        assert_eq!(open, vec![1]);
        toggle(&mut open, 2, false);
        assert_eq!(open, vec![2], "single: opening one closes the other");
        toggle(&mut open, 2, false);
        assert!(open.is_empty(), "the open one closes");
        toggle(&mut open, 1, true);
        toggle(&mut open, 3, true);
        assert_eq!(open, vec![1, 3]);
        toggle(&mut open, 1, true);
        assert_eq!(open, vec![3]);
    }
}
