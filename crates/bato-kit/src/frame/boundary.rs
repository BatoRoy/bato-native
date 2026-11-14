//! The `ErrorBoundary` analog (`ErrorBoundary.tsx`).
//!
//! Rust has no render-time exceptions, so the analog has two parts:
//!
//! - [`install_panic_hook`]: logs every panic (message and location, then
//!   the previous hook's report) and remembers the message for the fallback.
//! - [`error_boundary`]: a widget the frame wraps around the whole window.
//!   A panic inside its content's `layout`, `update`, `draw`, `operate` or
//!   `mouse_interaction` is caught with `catch_unwind`; the content is dropped
//!   and React's fallback takes its place: a centred card with the warning
//!   badge, "Something went wrong", the message in mono and **Reload**,
//!   which rebuilds the content from the app's current state (React's
//!   `window.location.reload()`; app state survives here).
//!
//! **This needs `panic = "unwind"`** (the default). With `panic = "abort"`
//! (bato-native's own release profile) nothing can be caught: the hook logs
//! the panic and the process ends, as any Rust program does. Panics in the
//! app's own `update` / `view` functions (outside widget calls) are not
//! caught either; iced lets those unwind out of the event loop.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, Once};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay};
use iced::widget::{column, container};
use iced::{Alignment, Event, Length, Padding, Rectangle, Size, Vector, mouse};

use crate::El;
use crate::ctx::Ctx;
use crate::icon::{Ico, icon};
use crate::label::{Align, label};
use crate::layout::vgap;
use crate::style::{c, shadows};
use crate::surface::{BoxStyle, Status, surface};

static LAST: Mutex<Option<String>> = Mutex::new(None);
static HOOK: Once = Once::new();

/// The text of a panic payload (`panic!("…")` / `.unwrap()` messages).
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown error".to_string()
    }
}

/// Log panics and remember the last message (idempotent; the frame calls
/// it). The previous hook still prints its report (and backtrace).
pub fn install_panic_hook() {
    HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let msg = panic_message(info.payload());
            let at = info
                .location()
                .map(|l| format!(" at {}:{}", l.file(), l.line()))
                .unwrap_or_default();
            eprintln!("bato: ErrorBoundary caught: {msg}{at}");
            if let Ok(mut last) = LAST.lock() {
                *last = Some(msg);
            }
            prev(info);
        }));
    });
}

/// Run `f`, turning a panic into its message.
pub fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|e| panic_message(e.as_ref()))
}

type Fallback<'a, M> = Box<dyn Fn(&str) -> El<'a, M> + 'a>;

/// Catch panics in `content` and show `fallback(message)` instead. `epoch`
/// is a counter the app bumps to retry (the fallback's Reload).
pub fn error_boundary<'a, M: 'a>(
    content: impl Into<El<'a, M>>,
    epoch: u64,
    fallback: impl Fn(&str) -> El<'a, M> + 'a,
) -> ErrorBoundary<'a, M> {
    ErrorBoundary {
        content: content.into(),
        epoch,
        fallback: Box::new(fallback),
        shown: None,
    }
}

/// React's default fallback, full window on the page background.
pub fn fallback_view<'a, M: Clone + 'a>(cx: &Ctx, message: &str, reload: M) -> El<'a, M> {
    let p = cx.on_card().p().clone();
    let badge = surface(icon(Ico::AlertTriangle, 22.0).color(c(p.red)))
        .width(48)
        .height(48)
        .center()
        .fixed(BoxStyle {
            bg: Some(c(bato_theme::blend(p.red, 0.15, p.card))),
            radius: 24.0,
            ..BoxStyle::default()
        });
    let t = cx.t.clone();
    let button = surface(label("Reload", 14.0, 20.0).medium())
        .padding(Padding::from([10.0, 16.0]))
        .on_press(reload)
        .focus_ring(crate::focus::Ring::focus(cx))
        .style(move |s| {
            let p = &t.p;
            BoxStyle::default()
                .bg(c(if s == Status::Hover {
                    p.accent_hover
                } else {
                    p.accent
                }))
                .fg(c(p.accent_ink))
                .radius(8.0)
        })
        .transition_ms(150);
    let card = surface(
        column![
            badge,
            vgap(16.0),
            label("Something went wrong", 14.0, 20.0)
                .semibold()
                .color(c(p.text))
                .align(Align::Center)
                .width(Length::Fill),
            vgap(6.0),
            label(message.to_string(), 12.0, 16.0)
                .mono()
                .wrap()
                .color(c(p.muted))
                .align(Align::Center)
                .width(Length::Fill),
            vgap(20.0),
            button,
        ]
        .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(24)
    .fixed(BoxStyle {
        bg: Some(c(p.card)),
        border: Some(c(cx.on_card().line_strong())),
        radius: 16.0,
        shadows: shadows(&p.shadow_lg),
        ..BoxStyle::default()
    });
    container(container(card).max_width(448))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(24)
        .center(Length::Fill)
        .into()
}

pub struct ErrorBoundary<'a, M> {
    content: El<'a, M>,
    epoch: u64,
    fallback: Fallback<'a, M>,
    /// The fallback element while an error is shown (built in `layout`).
    shown: Option<El<'a, M>>,
}

#[derive(Default)]
struct State {
    error: Option<String>,
    epoch: u64,
}

impl<'a, M: 'a> ErrorBoundary<'a, M> {
    fn caught(tree: &mut Tree, e: String) {
        let st = tree.state.downcast_mut::<State>();
        st.error = Some(e);
    }
}

impl<'a, M: 'a> Widget<M, iced::Theme, iced::Renderer> for ErrorBoundary<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State {
            error: None,
            epoch: self.epoch,
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        let st = tree.state.downcast_mut::<State>();
        if st.epoch != self.epoch {
            // Reload: start over with a fresh content tree.
            st.epoch = self.epoch;
            st.error = None;
            tree.children = vec![Tree::new(&self.content)];
            return;
        }
        if st.error.is_none() {
            let content = &self.content;
            if let Err(e) = guarded(|| tree.children[0].diff(content)) {
                tree.state.downcast_mut::<State>().error = Some(e);
            }
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = limits.max();
        if tree.state.downcast_ref::<State>().error.is_none() {
            let content = &mut self.content;
            let child = &mut tree.children[0];
            match guarded(|| content.as_widget_mut().layout(child, renderer, limits)) {
                Ok(n) => return layout::Node::with_children(size, vec![n]),
                Err(e) => Self::caught(tree, e),
            }
        }
        let msg = tree
            .state
            .downcast_ref::<State>()
            .error
            .clone()
            .unwrap_or_default();
        let mut fb = (self.fallback)(&msg);
        if tree.children.len() < 2 {
            tree.children.push(Tree::new(&fb));
        } else {
            tree.children[1].diff(&fb);
        }
        let n = fb
            .as_widget_mut()
            .layout(&mut tree.children[1], renderer, limits);
        self.shown = Some(fb);
        layout::Node::with_children(size, vec![n])
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
        let child = layout.children().next().unwrap();
        if let Some(fb) = &mut self.shown {
            fb.as_widget_mut().update(
                &mut tree.children[1],
                event,
                child,
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
            return;
        }
        if let Some(e) = take_draw_panic() {
            Self::caught(tree, e);
            shell.invalidate_layout();
            shell.request_redraw();
            return;
        }
        let content = &mut self.content;
        let t = &mut tree.children[0];
        let r = guarded(|| {
            content.as_widget_mut().update(
                t, event, child, cursor, renderer, clipboard, shell, viewport,
            )
        });
        if let Err(e) = r {
            Self::caught(tree, e);
            shell.invalidate_layout();
            shell.request_redraw();
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
        let child = layout.children().next().unwrap();
        if let Some(fb) = &self.shown {
            return fb.as_widget().mouse_interaction(
                &tree.children[1],
                child,
                cursor,
                viewport,
                renderer,
            );
        }
        guarded(|| {
            self.content.as_widget().mouse_interaction(
                &tree.children[0],
                child,
                cursor,
                viewport,
                renderer,
            )
        })
        .unwrap_or_default()
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let child = layout.children().next().unwrap();
        if let Some(fb) = &mut self.shown {
            fb.as_widget_mut()
                .operate(&mut tree.children[1], child, renderer, operation);
            return;
        }
        let content = &mut self.content;
        let t = &mut tree.children[0];
        if let Err(e) = guarded(|| {
            content
                .as_widget_mut()
                .operate(t, child, renderer, operation)
        }) {
            Self::caught(tree, e);
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
        let child = layout.children().next().unwrap();
        if let Some(fb) = &self.shown {
            fb.as_widget().draw(
                &tree.children[1],
                renderer,
                theme,
                style,
                child,
                cursor,
                viewport,
            );
            return;
        }
        // `draw` cannot touch the tree state: the next `update` picks the
        // panic up and shows the fallback.
        let r = guarded(|| {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                child,
                cursor,
                viewport,
            )
        });
        if let Err(e) = r {
            DRAW_PANIC.with(|d| *d.borrow_mut() = Some(e));
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        let child = layout.children().next().unwrap();
        if let Some(fb) = &mut self.shown {
            return fb.as_widget_mut().overlay(
                &mut tree.children[1],
                child,
                renderer,
                viewport,
                translation,
            );
        }
        if tree.state.downcast_ref::<State>().error.is_some() {
            return None;
        }
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            child,
            renderer,
            viewport,
            translation,
        )
    }
}

thread_local! {
    /// A panic caught in `draw` (which cannot touch the tree state), picked
    /// up by the next `update`.
    static DRAW_PANIC: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// A panic caught while drawing, if any (cleared).
fn take_draw_panic() -> Option<String> {
    DRAW_PANIC.with(|d| d.borrow_mut().take())
}

impl<'a, M: 'a> From<ErrorBoundary<'a, M>> for El<'a, M> {
    fn from(b: ErrorBoundary<'a, M>) -> Self {
        El::new(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_turns_panics_into_messages() {
        install_panic_hook();
        assert_eq!(guarded(|| 2 + 2), Ok(4));
        assert_eq!(guarded(|| -> u8 { panic!("boom") }), Err("boom".into()));
        let n: Option<u8> = std::hint::black_box(None);
        let e = guarded(|| n.expect("no value")).unwrap_err();
        assert_eq!(e, "no value");
        assert_eq!(
            guarded(|| -> u8 { std::panic::panic_any(7u8) }),
            Err("unknown error".into())
        );
        assert_eq!(LAST.lock().unwrap().as_deref(), Some("unknown error"));
    }
}
