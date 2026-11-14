//! `FileDropzone` (`inputs/FileDropzone.tsx`): a drop area with click-to-browse.
//!
//! `flex flex-col items-center justify-center gap-2 py-8 px-6 rounded-xl border-2 border-dashed`:
//! a 40 px round `bg-app-surface` badge with the Upload icon, "**Click to browse** or drag files
//! here" and an optional hint. The border is `app-control`, `accentBright` on hover and while
//! files are dragged over it (then also an `accent/5` fill), with a 150 ms colour transition.
//!
//! - **Click** (or tap) publishes [`FileDropzone::on_browse`]: the app opens the portal file
//!   chooser (bato-shell `iced_ext::open_files`, `multiple` from [`FileDropzone::is_multiple`])
//!   and hands the paths to its own handler (React's `onPaths`).
//! - **Drop**: files dragged from the desktop arrive as iced's `FileHovered` / `FileDropped` /
//!   `FilesHoveredLeft` window events (on Wayland through bato-native's winit patch). The zone
//!   highlights while files hover over it and publishes [`FileDropzone::on_drop`] once per drop
//!   with every path (React's `onFiles`). When the window does not know where the pointer is (X11
//!   during a drag), the whole window counts as over the zone.
//!
//! Not a Tab stop (a `<div onClick>` in React).
//!
//! The dashed border is Chromium's: 6 px dashes along the border's centreline with the gap
//! stretched so a whole number of dashes fits the closed path ([`dash_gap`]), drawn as an SVG
//! through the icon pipeline (iced has no dashed strokes).
//!
//! ```ignore
//! file_dropzone(&cx.on_card())
//!     .hint("PNG, JPG up to 10MB")
//!     .on_browse(Msg::Browse)
//!     .on_drop(Msg::Dropped)
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::svg::{self, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::widget::{column, container};
use iced::{
    Alignment, Border, Color, Event, Length, Padding, Radians, Rectangle, Size, mouse,
    touch, window,
};

use crate::El;
use crate::anim::{Tween, current_opacity, ease_standard};
use crate::ctx::Ctx;
use crate::icon::{Ico, icon};
use crate::label::{Span, label, rich};
use crate::style::{c, lerp, rgba};
use crate::surface::{BoxStyle, surface};

/// Builder; becomes the widget on `into()`.
pub struct FileDropzone<'a, M> {
    cx: Ctx,
    hint: Option<String>,
    multiple: bool,
    accept: Option<String>,
    on_browse: Option<M>,
    on_drop: Option<Box<dyn Fn(Vec<PathBuf>) -> M + 'a>>,
}

/// A drop zone on `cx`'s backdrop (a card: `cx.on_card()`).
pub fn file_dropzone<'a, M: Clone + 'a>(cx: &Ctx) -> FileDropzone<'a, M> {
    FileDropzone {
        cx: cx.clone(),
        hint: None,
        multiple: true,
        accept: None,
        on_browse: None,
        on_drop: None,
    }
}

impl<'a, M: Clone + 'a> FileDropzone<'a, M> {
    /// The muted line under the prompt.
    pub fn hint(mut self, text: impl Into<String>) -> Self {
        self.hint = Some(text.into());
        self
    }
    /// Let the browse dialog pick several files (default true).
    pub fn multiple(mut self, on: bool) -> Self {
        self.multiple = on;
        self
    }
    pub fn is_multiple(&self) -> bool {
        self.multiple
    }
    /// The HTML `accept` list (`.png,.jpg`, `image/*`), for the app's browse
    /// dialog filter ([`accept_extensions`]).
    pub fn accept(mut self, accept: impl Into<String>) -> Self {
        self.accept = Some(accept.into());
        self
    }
    /// A click or tap: open the file chooser.
    pub fn on_browse(mut self, m: M) -> Self {
        self.on_browse = Some(m);
        self
    }
    /// Files dropped on the zone.
    pub fn on_drop(mut self, f: impl Fn(Vec<PathBuf>) -> M + 'a) -> Self {
        self.on_drop = Some(Box::new(f));
        self
    }
}

/// The file extensions of an HTML `accept` list for a portal filter:
/// `.png, .JPG` → `png`, `jpg`; MIME types (`image/*`) are skipped (the
/// portal filter takes globs here).
pub fn accept_extensions(accept: &str) -> Vec<String> {
    accept
        .split(',')
        .map(str::trim)
        .filter_map(|s| s.strip_prefix('.'))
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect()
}

impl<'a, M: Clone + 'a> From<FileDropzone<'a, M>> for El<'a, M> {
    fn from(f: FileDropzone<'a, M>) -> Self {
        let p = f.cx.p();
        let badge = surface(icon(Ico::Upload, 18.0).color(c(p.muted)))
            .width(40)
            .height(40)
            .center()
            .fixed(BoxStyle {
                bg: Some(c(p.surface)),
                border: Some(c(p.border)),
                radius: 20.0,
                ..BoxStyle::default()
            });
        let prompt = rich(
            vec![
                Span::new("Click to browse")
                    .weight(500)
                    .color(c(p.accent_bright)),
                Span::new(" or drag files here"),
            ],
            14.0,
            20.0,
        )
        .color(c(p.subtext));
        let mut items: Vec<El<'a, M>> = vec![badge.into(), prompt.into()];
        if let Some(h) = f.hint {
            items.push(label(h, 12.0, 16.0).color(c(p.muted)).into());
        }
        let content = container(column(items).spacing(8).align_x(Alignment::Center))
            .width(Length::Fill)
            .align_x(Alignment::Center)
            .padding(Padding::from([34.0, 26.0]));
        El::new(Zone {
            content: content.into(),
            idle: c(p.control),
            active: c(p.accent_bright),
            fill: rgba(p.accent, 0.05),
            on_browse: f.on_browse,
            on_drop: f.on_drop,
        })
    }
}

struct Zone<'a, M> {
    content: El<'a, M>,
    idle: Color,
    active: Color,
    fill: Color,
    on_browse: Option<M>,
    on_drop: Option<Box<dyn Fn(Vec<PathBuf>) -> M + 'a>>,
}

#[derive(Debug)]
struct State {
    /// Files are being dragged over the window.
    dragging: bool,
    /// Dropped on the zone, published at the next redraw (one message per drop).
    dropped: Vec<PathBuf>,
    pressed: bool,
    finger: Option<touch::Finger>,
    highlighted: bool,
    /// The border colour: 0 = `control`, 1 = `accentBright`.
    tween: Tween,
}

impl Default for State {
    fn default() -> Self {
        Self {
            dragging: false,
            dropped: Vec::new(),
            pressed: false,
            finger: None,
            highlighted: false,
            tween: Tween::settled(0.0),
        }
    }
}

/// Whether a drag hovers the zone: over its bounds, or anywhere when the
/// window has no pointer position during the drag.
fn drag_over(cursor: mouse::Cursor, bounds: Rectangle) -> bool {
    match cursor.position() {
        Some(p) => bounds.contains(p),
        None => true,
    }
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for Zone<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let n = self.content.as_widget_mut().layout(
            &mut tree.children[0],
            renderer,
            &limits.width(Length::Fill),
        );
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
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        let b = layout.bounds();
        let st = tree.state.downcast_mut::<State>();
        match event {
            Event::Window(window::Event::FileHovered(_)) => {
                st.dragging = true;
                shell.request_redraw();
            }
            Event::Window(window::Event::FilesHoveredLeft) => {
                st.dragging = false;
                shell.request_redraw();
            }
            Event::Window(window::Event::FileDropped(path)) => {
                if drag_over(cursor, b) {
                    st.dropped.push(path.clone());
                }
                st.dragging = false;
                shell.request_redraw();
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                if !st.dropped.is_empty()
                    && let Some(f) = &self.on_drop
                {
                    shell.publish(f(std::mem::take(&mut st.dropped)));
                }
                st.dropped.clear();
                if st.tween.is_running(*now) {
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor.is_over(b) && !shell.is_event_captured() =>
            {
                st.pressed = true;
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if st.pressed => {
                st.pressed = false;
                if cursor.is_over(b)
                    && let Some(m) = &self.on_browse
                {
                    shell.publish(m.clone());
                }
                shell.capture_event();
            }
            Event::Touch(touch::Event::FingerPressed { id, position })
                if b.contains(*position) && !shell.is_event_captured() =>
            {
                st.finger = Some(*id);
                shell.capture_event();
            }
            Event::Touch(touch::Event::FingerLifted { id, position }) if st.finger == Some(*id) => {
                st.finger = None;
                if b.contains(*position)
                    && let Some(m) = &self.on_browse
                {
                    shell.publish(m.clone());
                }
                shell.capture_event();
            }
            Event::Touch(touch::Event::FingerLost { id, .. }) if st.finger == Some(*id) => {
                st.finger = None;
            }
            _ => {}
        }
        // Hover (and a drag over the zone) turn the border accentBright.
        let hover = cursor.is_over(b) || (st.dragging && drag_over(cursor, b));
        if hover != st.highlighted {
            st.highlighted = hover;
            let now = Instant::now();
            let from = st.tween.value(now);
            st.tween = Tween::new(from, if hover { 1.0 } else { 0.0 }, TRANSITION, ease_standard);
            shell.request_redraw();
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content.as_widget_mut().operate(
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
        let st = tree.state.downcast_ref::<State>();
        let b = crate::scale::snap_rect(layout.bounds());
        let over = st.dragging && drag_over(cursor, layout.bounds());
        if over {
            renderer.fill_quad(
                Quad {
                    bounds: b,
                    border: Border {
                        radius: 12.0.into(),
                        ..Border::default()
                    },
                    ..Quad::default()
                },
                crate::anim::fade(self.fill),
            );
        }
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
        let t = st.tween.value(Instant::now());
        let color = lerp(self.idle, self.active, t);
        renderer.draw_svg(
            svg::Svg {
                handle: dashed_border(b.width, b.height),
                color: Some(Color { a: 1.0, ..color }),
                rotation: Radians(0.0),
                opacity: color.a * current_opacity(),
            },
            b,
            viewport.expand(2.0),
        );
    }
}

/// `transition-colors` (150 ms, Tailwind's default curve).
const TRANSITION: Duration = Duration::from_millis(150);

// ── The dashed border ───────────────────────────────────────────────────

/// Chromium's dash length for a 2 px dashed border (3 × width).
pub const DASH: f32 = 6.0;
/// Its nominal gap (2 × width).
pub const GAP: f32 = 4.0;

/// The perimeter of the border's centreline: a `w`×`h` box, 2 px border,
/// 12 px outer radius (11 px on the centreline).
pub fn perimeter(w: f32, h: f32) -> f32 {
    let r = 11.0;
    2.0 * (w - 2.0 - 2.0 * r) + 2.0 * (h - 2.0 - 2.0 * r) + 2.0 * std::f32::consts::PI * r
}

/// The gap that fits a whole number of dashes around the closed path
/// (Skia/Blink `SelectBestDashGap`, rounded down: measured on the Electron
/// reference, 806×160 → 190 dashes).
pub fn dash_gap(perimeter: f32) -> f32 {
    let n = (perimeter / (DASH + GAP)).floor().max(1.0);
    perimeter / n - DASH
}

/// The border as SVG: the centreline rounded rect, stroked black (the
/// colour comes from the icon pipeline's filter), dashes starting half a
/// pixel before the top-left corner's end (measured).
pub fn dashed_svg(w: f32, h: f32) -> String {
    let gap = dash_gap(perimeter(w, h));
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><rect x="1" y="1" width="{}" height="{}" rx="11" fill="none" stroke="#000" stroke-width="2" stroke-dasharray="{DASH} {gap:.4}" stroke-dashoffset="0.53"/></svg>"##,
        w - 2.0,
        h - 2.0,
    )
}

thread_local! {
    static BORDERS: RefCell<HashMap<(u32, u32), svg::Handle>> = RefCell::new(HashMap::new());
}

fn dashed_border(w: f32, h: f32) -> svg::Handle {
    BORDERS.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() > 32 {
            m.clear();
        }
        m.entry((w.to_bits(), h.to_bits()))
            .or_insert_with(|| svg::Handle::from_memory(dashed_svg(w, h).into_bytes()))
            .clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dash_count_matches_the_electron_reference() {
        // The Showcase dropzone is 806×160 at 1280×800.
        let p = perimeter(806.0, 160.0);
        assert!((p - 1905.1).abs() < 0.1, "{p}");
        let gap = dash_gap(p);
        assert!((DASH + gap - 10.0269).abs() < 0.001, "{gap}");
        // A whole number of dashes always fits.
        for w in [100.0, 333.0, 806.0, 1200.0] {
            let p = perimeter(w, 160.0);
            let n = p / (DASH + dash_gap(p));
            assert!((n - n.round()).abs() < 1e-3, "{w}: {n}");
            assert!(dash_gap(p) >= GAP - 1e-3, "rounding down never shrinks the gap");
        }
    }

    #[test]
    fn svg_has_the_measured_pattern() {
        let s = dashed_svg(806.0, 160.0);
        assert!(s.contains(r#"width="804" height="158" rx="11""#), "{s}");
        assert!(s.contains(r#"stroke-dasharray="6 4.0269""#), "{s}");
    }

    #[test]
    fn accept_list_to_extensions() {
        assert_eq!(accept_extensions(".png, .JPG,image/*,"), vec!["png", "jpg"]);
        assert!(accept_extensions("").is_empty());
    }

    #[test]
    fn drag_position_rules() {
        let b = Rectangle::new(iced::Point::new(10.0, 10.0), Size::new(100.0, 50.0));
        assert!(drag_over(mouse::Cursor::Available(iced::Point::new(20.0, 20.0)), b));
        assert!(!drag_over(mouse::Cursor::Available(iced::Point::new(0.0, 0.0)), b));
        assert!(drag_over(mouse::Cursor::Unavailable, b), "no position: the window");
    }
}
