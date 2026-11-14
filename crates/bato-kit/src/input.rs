//! `Input`: the text input wrapper (`controlClasses`). iced's `text_input`
//! does the editing (caret, selection, clipboard); the wrapper draws the
//! focus halo `0 0 0 4px accent@.15` outside the box, which iced's style
//! cannot, snaps the field to the device-pixel grid (Chromium pixel-snaps
//! boxes, iced does not), and keeps middle presses for itself (primary paste;
//! `data-batoscroll-ignore`).
//!
//! The label/hint/error scaffold (`Field`, INF-10) builds on this in Phase 2.
//!
//! ```ignore
//! input(&cx.on_card(), "Type something…", &self.name, Msg::Name)          // 14/20, h 38
//! input(&cx, "#rrggbb", &self.hex, Msg::Hex).mono().text(12.0, 16.0).height(32.0)
//! ```

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::text::Renderer as TextRenderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::widget::text_input;
use iced::{Background, Border, Color, Element, Event, Length, Rectangle, Size, mouse};

use crate::ctx::Ctx;
use crate::fonts;
use crate::style::{c, rgba};

type P = <iced::Renderer as TextRenderer>::Paragraph;

/// The colours of a text field.
#[derive(Debug, Clone, Copy)]
pub struct InputColors {
    pub fill: Color,
    pub border: Color,
    pub border_focus: Color,
    pub halo: Color,
    pub text: Color,
    pub placeholder: Color,
    pub selection: Color,
}

impl InputColors {
    /// `bg-app-bg/50` fill, `border-app-control`, accentBright focus border
    /// with an `accent@.15` halo, `accent@.35` selection.
    pub fn of(cx: &Ctx) -> Self {
        let p = cx.p();
        Self {
            fill: c(cx.recessed()),
            border: c(p.control),
            border_focus: c(p.accent_bright),
            halo: c(cx.tint(0.15)),
            text: c(p.text),
            placeholder: c(p.muted),
            selection: rgba(p.accent, 0.35),
        }
    }
}

/// Builder; becomes the widget on `into()`.
pub struct Input<'a, Message> {
    placeholder: String,
    value: String,
    on_input: Option<Box<dyn Fn(String) -> Message + 'a>>,
    on_submit: Option<Message>,
    id: Option<widget::Id>,
    font: iced::Font,
    size: f32,
    line_height: f32,
    height: f32,
    width: Length,
    secure: bool,
    colors: InputColors,
}

/// A text input on `cx`'s backdrop: Inter 14/20, 38 px tall, full width.
pub fn input<'a, Message: Clone + 'a>(
    cx: &Ctx,
    placeholder: &str,
    value: &str,
    on_input: impl Fn(String) -> Message + 'a,
) -> Input<'a, Message> {
    Input {
        placeholder: placeholder.to_string(),
        value: value.to_string(),
        on_input: Some(Box::new(on_input)),
        on_submit: None,
        id: None,
        font: fonts::sans(),
        size: 14.0,
        line_height: 20.0,
        height: 38.0,
        width: Length::Fill,
        secure: false,
        colors: InputColors::of(cx),
    }
}

impl<'a, Message: Clone + 'a> Input<'a, Message> {
    /// JetBrains Mono.
    pub fn mono(mut self) -> Self {
        self.font = fonts::mono();
        self
    }
    /// Font size and line height.
    pub fn text(mut self, size: f32, line_height: f32) -> Self {
        self.size = size;
        self.line_height = line_height;
        self
    }
    /// Border-box height (`h-9` = 36 …).
    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }
    pub fn width(mut self, w: impl Into<Length>) -> Self {
        self.width = w.into();
        self
    }
    /// Password field.
    pub fn secure(mut self, on: bool) -> Self {
        self.secure = on;
        self
    }
    pub fn on_submit(mut self, m: Message) -> Self {
        self.on_submit = Some(m);
        self
    }
    /// Read-only/disabled: no `on_input`.
    pub fn disabled(mut self) -> Self {
        self.on_input = None;
        self
    }
    /// For `iced::widget::operation::focus(id)`.
    pub fn id(mut self, id: impl Into<widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }
    pub fn colors(mut self, colors: InputColors) -> Self {
        self.colors = colors;
        self
    }
}

impl<'a, Message: Clone + 'a> From<Input<'a, Message>>
    for Element<'a, Message, iced::Theme, iced::Renderer>
{
    fn from(f: Input<'a, Message>) -> Self {
        let colors = f.colors;
        // CSS: border 1px + padding-x 12; iced draws the border over its padding.
        let pad_y = (f.height - f.line_height) / 2.0;
        let mut inner = text_input(&f.placeholder, &f.value)
            .font(f.font)
            .size(f.size)
            .line_height(iced::widget::text::LineHeight::Absolute(
                f.line_height.into(),
            ))
            .padding([pad_y, 13.0])
            .width(Length::Fill)
            .secure(f.secure)
            .style(move |_theme, status| {
                let focused = matches!(status, text_input::Status::Focused { .. });
                text_input::Style {
                    background: Background::Color(colors.fill),
                    border: Border {
                        color: if focused {
                            colors.border_focus
                        } else {
                            colors.border
                        },
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    icon: colors.placeholder,
                    placeholder: colors.placeholder,
                    value: colors.text,
                    selection: colors.selection,
                }
            });
        if let Some(on_input) = f.on_input {
            inner = inner.on_input(on_input);
        }
        if let Some(m) = f.on_submit {
            inner = inner.on_submit(m);
        }
        if let Some(id) = f.id {
            inner = inner.id(id);
        }
        let halo = Halo {
            inner: inner.into(),
            halo: colors.halo,
            width: f.width,
        };
        Element::new(halo)
    }
}

/// The wrapper widget: halo, pixel snapping, middle-press capture.
struct Halo<'a, Message> {
    inner: Element<'a, Message, iced::Theme, iced::Renderer>,
    halo: Color,
    width: Length,
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for Halo<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.inner.as_widget().size().height)
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.inner)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.inner));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(self.width);
        let n = self
            .inner
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &limits);
        layout::Node::with_children(n.size(), vec![n])
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
        // `data-batoscroll-ignore`: a middle press on a text field is the
        // field's (primary paste), never an autoscroll.
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle))
        ) && cursor.is_over(layout.bounds())
        {
            shell.capture_event();
            return;
        }
        self.inner.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.inner.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        )
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.inner.as_widget_mut().operate(
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
        use iced::advanced::Renderer as _;
        let focused = tree.children[0]
            .state
            .downcast_ref::<text_input::State<P>>()
            .is_focused();
        let raw = layout.bounds();
        let b = crate::scale::snap_rect(raw);
        let shift = iced::Vector::new(b.x - raw.x, b.y - raw.y);
        if focused {
            renderer.fill_quad(
                Quad {
                    bounds: Rectangle {
                        x: b.x - 4.0,
                        y: b.y - 4.0,
                        width: b.width + 8.0,
                        height: b.height + 8.0,
                    },
                    border: Border {
                        radius: 12.0.into(),
                        ..Border::default()
                    },
                    ..Quad::default()
                },
                self.halo,
            );
        }
        renderer.with_translation(shift, |renderer| {
            self.inner.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout.children().next().unwrap(),
                cursor - shift,
                viewport,
            );
        });
    }
}
