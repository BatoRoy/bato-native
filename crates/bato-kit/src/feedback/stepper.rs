//! `Stepper`: wizard steps. Steps before `current` are complete (an accent
//! disc with a check), the current one has an accent ring and number, later
//! ones are muted. The 32 px discs have a 2 px border and change colour with
//! `transition-colors` (150 ms); they are buttons when `on_step` is set.
//!
//! Horizontal: each step takes an equal share; disc, label (`mt-2 px-1`)
//! and a 2 px connector (`mx-2 mt-4`, accent once complete) fill it, in a
//! row whose items are centred on the disc. Vertical: discs in a column
//! joined by 2 px lines, labels beside them (`pb-6 pt-1`).

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{Tree, Widget, tree};
use iced::widget::{Column, Space, column, row};
use iced::{Border, Color, Element, Length, Padding, Rectangle, Size, mouse};

use crate::El;
use crate::anim::{ease_standard, fade};
use crate::ctx::Ctx;
use crate::focus::Ring;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::scale::snap_rect;
use crate::style::{c, lerp};
use crate::surface::{BoxStyle, surface};

#[derive(Debug, Clone)]
pub struct Step {
    pub label: String,
    pub description: Option<String>,
}

impl Step {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: None,
        }
    }
    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }
}

/// The state of step `i` when `current` is in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Complete,
    Active,
    Pending,
}

impl StepState {
    pub fn of(i: usize, current: usize) -> Self {
        use std::cmp::Ordering::*;
        match i.cmp(&current) {
            Less => StepState::Complete,
            Equal => StepState::Active,
            Greater => StepState::Pending,
        }
    }
}

pub struct Stepper<'a, M> {
    cx: Ctx,
    steps: Vec<Step>,
    current: usize,
    vertical: bool,
    on_step: Option<Box<dyn Fn(usize) -> M + 'a>>,
}

pub fn stepper<'a, M: Clone + 'a>(
    cx: &Ctx,
    steps: impl IntoIterator<Item = Step>,
    current: usize,
) -> Stepper<'a, M> {
    Stepper {
        cx: cx.clone(),
        steps: steps.into_iter().collect(),
        current,
        vertical: false,
        on_step: None,
    }
}

impl<'a, M: Clone + 'a> Stepper<'a, M> {
    pub fn vertical(mut self) -> Self {
        self.vertical = true;
        self
    }
    /// `onStepClick`: the discs become buttons.
    pub fn on_step(mut self, f: impl Fn(usize) -> M + 'a) -> Self {
        self.on_step = Some(Box::new(f));
        self
    }
}

/// (fill, border, ink) of a disc.
fn disc_colors(cx: &Ctx, s: StepState) -> (Color, Color, Color) {
    let p = cx.p();
    match s {
        StepState::Complete => (c(p.accent), c(p.accent), c(p.accent_ink)),
        StepState::Active => (Color::TRANSPARENT, c(p.accent), c(p.accent_bright)),
        StepState::Pending => (Color::TRANSPARENT, c(p.border), c(p.muted)),
    }
}

impl<'a, M: Clone + 'a> From<Stepper<'a, M>> for El<'a, M> {
    fn from(s: Stepper<'a, M>) -> Self {
        let cx = &s.cx;
        let p = cx.p();
        let n = s.steps.len();
        let mut steps: Vec<El<'a, M>> = Vec::with_capacity(n);
        for (i, step) in s.steps.into_iter().enumerate() {
            let st = StepState::of(i, s.current);
            let last = i + 1 == n;
            let (bg, border, ink) = disc_colors(cx, st);
            let content: El<'a, M> = if st == StepState::Complete {
                icon(Ico::Check, 15.0).into()
            } else {
                label(format!("{}", i + 1), 12.0, 16.0).semibold().into()
            };
            let disc = Element::new(Disc {
                content,
                colors: [bg, border, ink],
            });
            let disc: El<'a, M> = surface(disc)
                .on_press_maybe(s.on_step.as_ref().map(|f| f(i)))
                .style(|_| BoxStyle::default().radius(16.0))
                .focus_ring(Ring::plain(cx))
                .into();
            let lc = if st == StepState::Pending {
                p.muted
            } else {
                p.text
            };
            let has_desc = step.description.is_some();
            let mut text: Column<'a, M, iced::Theme, iced::Renderer> =
                column![label(step.label, 14.0, 20.0).medium().color(c(lc))];
            if let Some(d) = step.description {
                text = text
                    .push(Space::new().height(2))
                    .push(label(d, 12.0, 16.0).color(c(p.muted)));
            }
            if s.vertical {
                // `w-0.5 flex-1 my-1 min-h-[1.5rem]` beside a `pt-1 pb-6`
                // text block: as tall as the text minus the disc and the
                // margins, at least 24 (labels on one line).
                let text_h = 4.0 + 20.0 + 24.0 + if has_desc { 18.0 } else { 0.0 };
                let line_h = (text_h - DISC - 8.0).max(24.0);
                let mut rail = column![disc];
                if !last {
                    rail = rail.push(Space::new().height(4)).push(
                        surface(Space::new().width(2).height(line_h))
                            .width(2)
                            .height(line_h)
                            .fixed(BoxStyle::default().bg(c(p.border))),
                    );
                    rail = rail.push(Space::new().height(4));
                }
                let rail = rail.align_x(iced::Alignment::Center);
                steps.push(
                    row![
                        rail,
                        iced::widget::container(text).padding(Padding {
                            top: 4.0,
                            bottom: 24.0,
                            ..Padding::ZERO
                        }),
                    ]
                    .spacing(12)
                    .into(),
                );
            } else {
                // `items-center` on a 32 px disc: the label's margin box
                // (8 + 20) starts 2 px down, the connector's (16 + 2) 7 px.
                let mut r = row![
                    disc,
                    iced::widget::container(text).padding(Padding {
                        top: 10.0,
                        left: 4.0,
                        right: 4.0,
                        bottom: 0.0
                    }),
                ];
                if !last {
                    let lc = if st == StepState::Complete {
                        p.accent
                    } else {
                        p.border
                    };
                    r = r.push(
                        iced::widget::container(
                            surface(Space::new().width(Length::Fill).height(2))
                                .width(Length::Fill)
                                .height(2)
                                .fixed(BoxStyle::default().bg(c(lc))),
                        )
                        .padding(Padding {
                            top: 23.0,
                            left: 8.0,
                            right: 8.0,
                            bottom: 0.0,
                        })
                        .width(Length::Fill),
                    );
                }
                steps.push(r.width(Length::Fill).into());
            }
        }
        if s.vertical {
            Column::with_children(steps).into()
        } else {
            row(steps).width(Length::Fill).into()
        }
    }
}

/// The 32 px disc: a 2 px border, a fill and an ink colour that tween over
/// 150 ms when the step's state changes.
struct Disc<'a, M> {
    content: El<'a, M>,
    colors: [Color; 3],
}

const DISC: f32 = 32.0;
const FADE: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Copy)]
struct DiscState {
    from: [Color; 3],
    to: [Color; 3],
    start: Option<Instant>,
}

impl DiscState {
    fn at(&self, now: Instant) -> ([Color; 3], bool) {
        let Some(start) = self.start else {
            return (self.to, false);
        };
        let f = (now.saturating_duration_since(start).as_secs_f32() / FADE.as_secs_f32()).min(1.0);
        if f >= 1.0 {
            return (self.to, false);
        }
        let e = ease_standard(f);
        let (a, b) = tween_endpoints(self.from, self.to);
        let mix = |i: usize| lerp(a[i], b[i], e);
        ([mix(0), mix(1), mix(2)], f < 1.0)
    }
}

/// `lerp` for a colour that may be transparent: fade the other colour's
/// alpha instead of passing through black.
fn tween_endpoints(from: [Color; 3], to: [Color; 3]) -> ([Color; 3], [Color; 3]) {
    let mut f = from;
    let mut t = to;
    for i in 0..3 {
        if f[i].a == 0.0 {
            f[i] = Color { a: 0.0, ..t[i] };
        }
        if t[i].a == 0.0 {
            t[i] = Color { a: 0.0, ..f[i] };
        }
    }
    (f, t)
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for Disc<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(DISC), Length::Fixed(DISC))
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<DiscState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(DiscState {
            from: self.colors,
            to: self.colors,
            start: None,
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        let st = tree.state.downcast_mut::<DiscState>();
        if st.to != self.colors {
            let now = Instant::now();
            let (cur, _) = st.at(now);
            *st = DiscState {
                from: cur,
                to: self.colors,
                start: Some(now),
            };
        }
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        let size = Size::new(DISC, DISC);
        let child = self.content.as_widget_mut().layout(
            &mut tree.children[0],
            renderer,
            &layout::Limits::new(Size::ZERO, size),
        );
        let cs = child.size();
        let child = child.move_to(iced::Point::new(
            (DISC - cs.width) / 2.0,
            (DISC - cs.height) / 2.0,
        ));
        layout::Node::with_children(size, vec![child])
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
        if let iced::Event::Window(iced::window::Event::RedrawRequested(now)) = event {
            let st = tree.state.downcast_mut::<DiscState>();
            if st.at(*now).1 {
                shell.request_redraw();
            } else {
                st.start = None;
            }
        }
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_ref::<DiscState>();
        let ([bg, border, ink], _) = st.at(Instant::now());
        let b = snap_rect(layout.bounds());
        renderer.fill_quad(
            Quad {
                bounds: b,
                border: Border {
                    radius: (DISC / 2.0).into(),
                    width: 2.0,
                    color: fade(border),
                },
                ..Quad::default()
            },
            fade(bg),
        );
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            &renderer::Style { text_color: ink },
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_follow_current() {
        assert_eq!(StepState::of(0, 1), StepState::Complete);
        assert_eq!(StepState::of(1, 1), StepState::Active);
        assert_eq!(StepState::of(2, 1), StepState::Pending);
    }

    #[test]
    fn disc_tween_reaches_the_target() {
        let a = [Color::TRANSPARENT, Color::WHITE, Color::BLACK];
        let b = [Color::WHITE, Color::BLACK, Color::WHITE];
        let t0 = Instant::now();
        let (f, _) = tween_endpoints(a, b);
        assert_eq!(
            f[0],
            Color {
                a: 0.0,
                ..Color::WHITE
            },
            "fades in, not via black"
        );
        let st = DiscState {
            from: a,
            to: b,
            start: Some(t0),
        };
        assert!(st.at(t0 + Duration::from_millis(75)).1);
        let (end, running) = st.at(t0 + FADE);
        assert!(!running);
        assert_eq!(end, b);
    }
}
