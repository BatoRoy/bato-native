//! OS drags of files to other apps (Electron's `webContents.startDrag`):
//! drag a file row out of the window and drop it on Discord, a browser or a
//! file manager.
//!
//! The drag offers `text/uri-list` (and the paths as `text/plain`) with the
//! copy action (and move, if asked), and an icon from RGBA pixels. It runs on
//! patched winit (`winit::bato_drag`): on Wayland a `wl_data_source` on the
//! window's data device, on X11 an XDND source (a pointer grab with a
//! copy / no-drop cursor, Escape cancels).
//!
//! **Start it during a press.** The compositor ties the drag to the held
//! pointer button, so start it from a press-and-move ([`drag_source`] fires
//! once the press travels past [`ACTIVATION`](super::ACTIVATION), or when
//! it leaves the window: [`Start::LeaveWindow`], for rows that also drag
//! in-app). After the drag starts the window gets `CursorLeft` and never the
//! button release: reset any in-app drag state when you start one.
//!
//! ```ignore
//! // view: a file row that drags out
//! os_drag::drag_source(row, Msg::DragOut(i))
//! // update
//! Msg::DragOut(i) => os_drag::Drag::new(vec![path(i)])
//!     .icon(os_drag::file_icon(1, accent, 2))
//!     .start()
//!     .map(Msg::DragEnded),
//! ```
//!
//! **Our own drag coming back.** When the drag is over one of this app's
//! windows, the usual `FileHovered` / `FileDropped` events arrive and
//! [`own_drop`] names the drag, so a drop zone can ignore (or reorder
//! instead of import) its own files.

use std::path::PathBuf;

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay, renderer};
use iced::{Color, Element, Event, Length, Point, Rectangle, Size, Task, Vector, mouse, window};

use crate::El;

pub use winit::bato_drag::{Action, Icon, Outcome};

/// A drag of local files, to start with [`Drag::start`].
#[derive(Clone, Debug, Default)]
pub struct Drag {
    paths: Vec<PathBuf>,
    icon: Option<Icon>,
    allow_move: bool,
    window: Option<window::Id>,
}

impl Drag {
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            paths: paths.into_iter().collect(),
            ..Self::default()
        }
    }

    /// The image under the pointer (see [`file_icon`]).
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Offer "move" besides "copy": a file manager may then move the files
    /// away. Off by default.
    pub fn allow_move(mut self, allow: bool) -> Self {
        self.allow_move = allow;
        self
    }

    /// The window the press is in (default: the oldest, the main window).
    pub fn window(mut self, id: window::Id) -> Self {
        self.window = Some(id);
        self
    }

    /// Start the drag; resolves when it ends.
    pub fn start(self) -> Task<Outcome> {
        let window = match self.window {
            Some(id) => Task::done(Some(id)),
            None => window::oldest(),
        };
        window.then(move |id| {
            let Some(id) = id else {
                return Task::done(Outcome::Failed("no window".into()));
            };
            let drag = self.clone();
            window::raw_id::<()>(id).then(move |raw| Task::future(begin(raw, drag.clone())))
        })
    }
}

/// Start `drag` from the winit window `raw`.
fn begin(raw: u64, drag: Drag) -> impl Future<Output = Outcome> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    winit::bato_drag::start(winit::bato_drag::Request {
        window: raw,
        paths: drag.paths,
        icon: drag.icon,
        allow_move: drag.allow_move,
        done: Box::new(move |outcome| {
            let _ = tx.send(outcome);
        }),
    });
    async move { rx.await.unwrap_or(Outcome::Cancelled) }
}

/// The id of this app's own OS drag that the latest `FileHovered` /
/// `FileDropped` came from; `None` when another app's drag brought them.
pub fn own_drop() -> Option<u64> {
    winit::bato_drag::incoming_drag()
}

/// When a [`drag_source`] starts the OS drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Start {
    /// Once the press travels this far (logical px).
    Distance(f32),
    /// Once the pressed pointer leaves the window (its logical size): the
    /// hand-over for an item that also drags inside the window.
    LeaveWindow(Size),
}

impl Default for Start {
    fn default() -> Self {
        Start::Distance(super::ACTIVATION)
    }
}

/// Is `at` (window coordinates) outside a window of `size`?
pub fn left_window(at: Point, size: Size) -> bool {
    at.x < 0.0 || at.y < 0.0 || at.x >= size.width || at.y >= size.height
}

/// Press tracking for [`drag_source`]: fires once per press.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tracker {
    press: Option<Point>,
    fired: bool,
}

impl Tracker {
    pub fn press(&mut self, at: Point) {
        *self = Tracker {
            press: Some(at),
            fired: false,
        };
    }

    /// The press ended (release, or the pointer left).
    pub fn release(&mut self) {
        *self = Tracker::default();
    }

    /// The pointer moved to `at`: start the OS drag now?
    pub fn moved(&mut self, at: Point, start: Start) -> bool {
        let Some(from) = self.press.filter(|_| !self.fired) else {
            return false;
        };
        let go = match start {
            Start::Distance(d) => (at - from).x.hypot((at - from).y) > d,
            Start::LeaveWindow(size) => left_window(at, size),
        };
        self.fired |= go;
        go
    }
}

/// Wraps content so a left press dragged out of it publishes `on_drag` (once
/// per press); answer it with [`Drag::start`]. Clicks still reach the content.
pub fn drag_source<'a, M: Clone + 'a>(
    content: impl Into<El<'a, M>>,
    on_drag: M,
) -> DragSource<'a, M> {
    DragSource {
        content: content.into(),
        on_drag,
        start: Start::default(),
    }
}

pub struct DragSource<'a, M> {
    content: El<'a, M>,
    on_drag: M,
    start: Start,
}

impl<'a, M> DragSource<'a, M> {
    pub fn start(mut self, start: Start) -> Self {
        self.start = start;
        self
    }
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for DragSource<'a, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Tracker>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(Tracker::default())
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
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
        let n = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
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
        let tracker = tree.state.downcast_mut::<Tracker>();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                match cursor.position_over(layout.bounds()) {
                    Some(at) => tracker.press(at),
                    None => tracker.release(),
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if tracker.moved(*position, self.start) {
                    shell.publish(self.on_drag.clone());
                }
            }
            Event::Mouse(
                mouse::Event::ButtonReleased(mouse::Button::Left) | mouse::Event::CursorLeft,
            ) => tracker.release(),
            _ => {}
        }
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
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        match self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        ) {
            mouse::Interaction::None if cursor.is_over(layout.bounds()) => mouse::Interaction::Grab,
            i => i,
        }
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
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
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: Clone + 'a> From<DragSource<'a, M>> for El<'a, M> {
    fn from(d: DragSource<'a, M>) -> Self {
        Element::new(d)
    }
}

/// The logical size of [`file_icon`].
pub const FILE_ICON: f32 = 44.0;

/// A generated drag image: a document (a stack of two for several files) in
/// `accent`, with a count badge past one file. `scale`: pixels per logical
/// pixel (the window's, rounded up). The pointer sits on the document's
/// upper left.
pub fn file_icon(count: usize, accent: Color, scale: u32) -> Icon {
    let scale = scale.clamp(1, 4);
    let side = (FILE_ICON as u32) * scale;
    let s = scale as f32;
    let mut rgba = vec![0u8; (side * side * 4) as usize];
    let white = Color::WHITE;
    let paper = Color { a: 0.97, ..white };
    let doc = |dx: f32, dy: f32| {
        move |x: f32, y: f32| {
            let (x, y) = (x - dx, y - dy);
            // 26 × 32 with a 9 px folded corner (top right).
            (0.0..26.0).contains(&x) && (0.0..32.0).contains(&y) && x - 17.0 <= y
        }
    };
    let fold = |dx: f32, dy: f32| {
        move |x: f32, y: f32| {
            let (x, y) = (x - dx, y - dy);
            (17.0..26.0).contains(&x) && (0.0..9.0).contains(&y) && x - 17.0 <= y
        }
    };
    let inset = |f: &dyn Fn(f32, f32) -> bool, x: f32, y: f32| {
        f(x - 1.5, y - 1.5) && f(x + 1.5, y + 1.5) && f(x - 1.5, y + 1.5) && f(x + 1.5, y - 1.5)
    };
    type Shape = Box<dyn Fn(f32, f32) -> bool>;
    let mut layers: Vec<(Shape, Color)> = Vec::new();
    if count > 1 {
        let back = doc(12.0, 9.0);
        layers.push((Box::new(back), accent));
        layers.push((
            Box::new(move |x, y| inset(&back, x, y)),
            Color { a: 0.85, ..paper },
        ));
    }
    let front = doc(6.0, 4.0);
    layers.push((Box::new(front), accent));
    layers.push((Box::new(move |x, y| inset(&front, x, y)), paper));
    layers.push((Box::new(fold(6.0, 4.0)), Color { a: 0.55, ..accent }));
    for i in 0..3 {
        let y0 = 18.0 + i as f32 * 5.0;
        let w = if i == 2 { 9.0 } else { 14.0 };
        layers.push((
            Box::new(move |x, y| (11.0..11.0 + w).contains(&x) && (y0..y0 + 2.0).contains(&y)),
            Color { a: 0.5, ..accent },
        ));
    }
    if count > 1 {
        let (cx, cy, r) = (34.0, 34.0, 9.0);
        layers.push((Box::new(move |x, y| (x - cx).hypot(y - cy) <= r), accent));
        let text = if count > 99 {
            "99".to_string()
        } else {
            count.to_string()
        };
        let n = text.len() as f32;
        let (gw, gh, px) = (3.0, 5.0, 1.6);
        let left = cx - (n * gw + (n - 1.0)) * px / 2.0;
        let top = cy - gh * px / 2.0;
        let digits: Vec<u8> = text.bytes().map(|b| b - b'0').collect();
        layers.push((
            Box::new(move |x, y| {
                let (gx, gy) = ((x - left) / px, (y - top) / px);
                if gx < 0.0 || gy < 0.0 || gy >= gh {
                    return false;
                }
                let cell = gx as usize / 4;
                let col = gx as usize % 4;
                col < 3
                    && digits
                        .get(cell)
                        .is_some_and(|&d| glyph(d, col, gy as usize))
            }),
            white,
        ));
    }
    // 4 × 4 supersampling per pixel, layers painted back to front.
    for py in 0..side {
        for px in 0..side {
            let mut out = [0.0f32; 4];
            for (inside, color) in &layers {
                let mut hits = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let x = (px as f32 + (sx as f32 + 0.5) / 4.0) / s;
                        let y = (py as f32 + (sy as f32 + 0.5) / 4.0) / s;
                        hits += inside(x, y) as u32;
                    }
                }
                let a = color.a * hits as f32 / 16.0;
                for (o, c) in out.iter_mut().zip([color.r, color.g, color.b]) {
                    *o = c * a + *o * (1.0 - a);
                }
                out[3] = a + out[3] * (1.0 - a);
            }
            let i = ((py * side + px) * 4) as usize;
            for k in 0..3 {
                let straight = if out[3] > 0.0 { out[k] / out[3] } else { 0.0 };
                rgba[i + k] = (straight.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            rgba[i + 3] = (out[3].clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    Icon {
        rgba,
        width: side,
        height: side,
        scale,
        hotspot: (8, 6),
    }
}

/// A 3 × 5 digit: is column `x`, row `y` of digit `d` lit?
fn glyph(d: u8, x: usize, y: usize) -> bool {
    const ROWS: [[u8; 5]; 10] = [
        [0b111, 0b101, 0b101, 0b101, 0b111],
        [0b010, 0b110, 0b010, 0b010, 0b111],
        [0b111, 0b001, 0b111, 0b100, 0b111],
        [0b111, 0b001, 0b011, 0b001, 0b111],
        [0b101, 0b101, 0b111, 0b001, 0b001],
        [0b111, 0b100, 0b111, 0b001, 0b111],
        [0b111, 0b100, 0b111, 0b101, 0b111],
        [0b111, 0b001, 0b010, 0b010, 0b010],
        [0b111, 0b101, 0b111, 0b101, 0b111],
        [0b111, 0b101, 0b111, 0b001, 0b111],
    ];
    ROWS[d as usize % 10][y.min(4)] >> (2 - x.min(2)) & 1 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_fires_once_per_press() {
        let mut t = Tracker::default();
        let start = Start::default();
        assert!(!t.moved(Point::new(50.0, 50.0), start), "no press, no drag");
        t.press(Point::new(10.0, 10.0));
        assert!(
            !t.moved(Point::new(13.0, 12.0), start),
            "within the 4 px slop"
        );
        assert!(t.moved(Point::new(15.0, 10.0), start));
        assert!(!t.moved(Point::new(30.0, 10.0), start), "once per press");
        t.release();
        assert!(!t.moved(Point::new(60.0, 10.0), start));
    }

    #[test]
    fn leave_window_hands_over_at_the_edge() {
        let size = Size::new(800.0, 600.0);
        assert!(!left_window(Point::new(0.0, 0.0), size));
        assert!(left_window(Point::new(-1.0, 300.0), size));
        assert!(left_window(Point::new(800.0, 300.0), size));
        let mut t = Tracker::default();
        t.press(Point::new(400.0, 300.0));
        let start = Start::LeaveWindow(size);
        assert!(
            !t.moved(Point::new(700.0, 590.0), start),
            "far, but still inside"
        );
        assert!(t.moved(Point::new(700.0, 601.0), start));
    }

    #[test]
    fn file_icon_is_drawn_at_scale() {
        let accent = Color::from_rgb8(0x3b, 0x82, 0xf6);
        let one = file_icon(1, accent, 2);
        assert_eq!((one.width, one.height, one.scale), (88, 88, 2));
        assert_eq!(one.rgba.len(), 88 * 88 * 4);
        let alpha = |i: &Icon, x: u32, y: u32| i.rgba[((y * i.width + x) * 4 + 3) as usize];
        assert_eq!(alpha(&one, 0, 0), 0, "transparent outside the document");
        assert!(alpha(&one, 20, 50) > 240, "paper inside");
        assert_eq!(alpha(&one, 68, 68), 0, "no badge for one file");
        let many = file_icon(3, accent, 2);
        assert!(alpha(&many, 68, 68) > 240, "badge for several");
        assert_ne!(
            many.rgba,
            file_icon(4, accent, 2).rgba,
            "the count is drawn"
        );
    }

    #[test]
    fn uri_list_round_trips_through_the_drop_parser() {
        let paths = vec![PathBuf::from("/tmp/a b/ü%#.png"), PathBuf::from("/x")];
        let list = winit::bato_drag::uri_list(&paths);
        assert_eq!(list, "file:///tmp/a%20b/%C3%BC%25%23.png\r\nfile:///x\r\n");
        assert_eq!(winit::bato_uri_list::parse(list.as_bytes(), None), paths);
        assert_eq!(winit::bato_drag::plain_text(&paths), "/tmp/a b/ü%#.png\n/x");
    }

    #[test]
    fn without_an_event_loop_the_drag_fails_at_once() {
        let (tx, rx) = std::sync::mpsc::channel();
        winit::bato_drag::start(winit::bato_drag::Request {
            window: 1,
            paths: vec![PathBuf::from("/x")],
            icon: None,
            allow_move: false,
            done: Box::new(move |o| tx.send(o).unwrap()),
        });
        assert!(matches!(rx.try_recv(), Ok(Outcome::Failed(_))));
        assert_eq!(own_drop(), None, "no drag came back");
    }

    #[test]
    fn digits() {
        assert!(glyph(1, 1, 0) && !glyph(1, 0, 0));
        assert!(glyph(8, 0, 2) && glyph(8, 2, 2) && glyph(8, 1, 2));
        assert!(!glyph(0, 1, 2));
    }
}
