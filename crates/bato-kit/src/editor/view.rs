//! The editor surface inside the kit scroller: line-number gutter (sticky
//! on horizontal scroll), active line, selection, selection matches,
//! matching brackets, text, caret / vim block cursor; keys, pointer and
//! focus. All geometry follows CodeMirror's base theme: `.cm-content`
//! `padding: 4px 0`, `.cm-line` `padding: 0 2px 0 6px`, line-number cells
//! `padding: 0 3px 0 5px; min-width: 20px` (border-box).

use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::advanced::graphics::text::cosmic_text as ct;
use iced::advanced::graphics::text::{Raw, Renderer as _, font_system};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{self, Id, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, clipboard};
use iced::keyboard::{self, Key, key::Named};
use iced::{Border, Color, Element, Event, Length, Point, Rectangle, Size, Vector, mouse, window};

use bato_theme::Palette;

use super::engine::{Cmd, LINE_HEIGHT, Move, Outcome, Pointer, VimKey};
use super::{Content, syntax::Language, text};
use crate::focus::FocusState;
use crate::scale::device_scale;
use crate::style::{c, rgba};

/// `.cm-content` vertical padding.
pub const PAD_Y: f32 = 4.0;
/// `.cm-line` horizontal padding.
pub const LINE_PAD_L: f32 = 6.0;
pub const LINE_PAD_R: f32 = 2.0;
/// Line-number cell padding and minimum width (border-box).
const NUM_PAD_L: f32 = 5.0;
const NUM_PAD_R: f32 = 3.0;
const NUM_MIN: f32 = 20.0;
/// `cursorBlinkRate`: 1200 ms, half on.
const BLINK: Duration = Duration::from_millis(1200);
/// Escape, then Tab within this moves focus (`tabFocusMode`).
const TAB_FOCUS: Duration = Duration::from_secs(2);
/// Chromium's multi-click interval and slop.
const MULTI_CLICK: Duration = Duration::from_millis(500);

pub(crate) struct DocView<'a, M> {
    pub content: &'a Content,
    pub palette: Palette,
    pub lang: Language,
    pub vim: bool,
    pub line_numbers: bool,
    pub read_only: bool,
    pub placeholder: Option<String>,
    pub on_change: Option<Box<dyn Fn(String) -> M + 'a>>,
    pub id: Option<Id>,
    /// The scroller's viewport height (page moves, the content's
    /// `min-height: 100%`).
    pub view_h: f32,
    /// Round the gutter's bottom-left corner (no status bar below).
    pub round_bottom: bool,
}

#[derive(Default)]
struct Numbers {
    count: usize,
    scale: f32,
    buffer: Option<Arc<ct::Buffer>>,
    /// The widest number (px).
    width: f32,
}

struct State {
    focus: FocusState,
    numbers: Numbers,
    placeholder: Option<(String, Arc<ct::Buffer>)>,
    /// Font ascent / descent (em) of the editor font.
    metrics: (f32, f32),
    dragging: bool,
    clicks: Option<(Instant, Point, u8)>,
    blink: Instant,
    esc: Option<Instant>,
    /// Content point = window point + this (the scroller's offset), for
    /// drags that leave the viewport.
    to_content: Vector,
    /// The drag's last window point.
    drag_at: Option<Point>,
    /// The text this editor last made the primary selection.
    primary: Option<String>,
    /// The one-line buffers drawn this frame (`fill_raw` holds weak
    /// references until the frame is prepared).
    frame: std::cell::RefCell<Vec<Arc<ct::Buffer>>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            focus: FocusState::default(),
            numbers: Numbers::default(),
            placeholder: None,
            metrics: (1069.0 / 1000.0, 293.0 / 1000.0),
            dragging: false,
            clicks: None,
            blink: Instant::now(),
            esc: None,
            to_content: Vector::ZERO,
            drag_at: None,
            primary: None,
            frame: std::cell::RefCell::new(Vec::new()),
        }
    }
}

/// Blink's baseline offset (device px, unrounded) in a line box:
/// `A + floor(half-leading)` with the font's rounded ascent and descent
/// (see `label::blink_baseline`).
fn baseline(metrics: (f32, f32), s: f32) -> f32 {
    let size = super::engine::FONT_SIZE;
    let a = (metrics.0 * size * s).round();
    let d = (metrics.1 * size * s).round();
    let lh = (LINE_HEIGHT * s * 64.0).round() / 64.0;
    let half = (((lh - (a + d)) / 2.0) * 64.0).floor() / 64.0;
    a + half.floor()
}

impl<M> DocView<'_, M> {
    fn gutter(&self, st: &State) -> f32 {
        if !self.line_numbers {
            return 0.0;
        }
        let w = (st.numbers.width + NUM_PAD_L + NUM_PAD_R).max(NUM_MIN);
        // Light themes give `.cm-gutters` a 1 px (transparent) right border.
        if self.palette.is_light() { w + 1.0 } else { w }
    }

    /// The text origin in content coordinates.
    fn origin(&self, b: Rectangle, st: &State) -> Point {
        Point::new(b.x + self.gutter(st) + LINE_PAD_L, b.y + PAD_Y)
    }

    fn line_top(&self, b: Rectangle, i: usize) -> f32 {
        b.y + PAD_Y + i as f32 * LINE_HEIGHT
    }
}

fn build_numbers(fs: &mut ct::FontSystem, count: usize) -> (Arc<ct::Buffer>, f32) {
    let mut buf = ct::Buffer::new(fs, ct::Metrics::new(super::engine::FONT_SIZE, LINE_HEIGHT));
    buf.set_wrap(fs, ct::Wrap::None);
    buf.set_size(fs, None, None);
    let text: String = (1..=count.max(1))
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    buf.set_text(
        fs,
        &text,
        &ct::Attrs::new().family(ct::Family::Name(super::engine::mono_family(fs))),
        ct::Shaping::Advanced,
        None,
    );
    buf.shape_until_scroll(fs, false);
    let w = buf.layout_runs().fold(0.0f32, |w, r| w.max(r.line_w));
    (Arc::new(buf), w)
}

fn font_metrics(fs: &mut ct::FontSystem, buffer: &ct::Buffer) -> Option<(f32, f32)> {
    let g = buffer
        .layout_runs()
        .find_map(|r| r.glyphs.first().cloned())?;
    let font = fs.get_font(g.font_id, g.font_weight)?;
    let m = font.metrics();
    let upem = f32::from(m.units_per_em);
    Some((m.ascent / upem, -m.descent / upem))
}

/// Line `i` of `buffer` as a one-line buffer (the shaped line is cloned,
/// not shaped again), so its baseline can be placed like `Label` does.
fn one_line(buffer: &ct::Buffer, i: usize) -> Option<Arc<ct::Buffer>> {
    let line = buffer.lines.get(i)?.clone();
    let mut one = ct::Buffer::new_empty(buffer.metrics());
    one.lines.push(line);
    Some(Arc::new(one))
}

/// [`one_line`] without its syntax colours (the vim block cursor draws its
/// letter in `bg`), shaped again.
fn one_line_plain(buffer: &ct::Buffer, i: usize) -> Option<Arc<ct::Buffer>> {
    let mut line = buffer.lines.get(i)?.clone();
    let list = line.attrs_list();
    let strip = |a: ct::Attrs<'_>| {
        let mut a = a.clone();
        a.color_opt = None;
        ct::AttrsOwned::new(&a)
    };
    let mut plain = ct::AttrsList::new(&strip(list.defaults()).as_attrs());
    for (range, a) in list.spans_iter() {
        plain.add_span(range.clone(), &strip(a.as_attrs()).as_attrs());
    }
    line.set_attrs_list(plain);
    let mut one = ct::Buffer::new_empty(buffer.metrics());
    one.lines.push(line);
    let mut fs = font_system().write().expect("font system");
    one.shape_until_scroll(fs.raw(), false);
    Some(Arc::new(one))
}

/// The x of byte `index` in a run (logical px from the buffer's left).
fn x_at(run: &ct::LayoutRun<'_>, index: usize) -> f32 {
    for g in run.glyphs {
        if index <= g.start {
            return g.x;
        }
        if index < g.end {
            // Inside a cluster: interpolate.
            let t = (index - g.start) as f32 / (g.end - g.start) as f32;
            return g.x + g.w * t;
        }
    }
    run.line_w
}

/// The advance of the character at `index` (the vim block cursor; a space
/// width at the line end).
fn char_w(run: &ct::LayoutRun<'_>, index: usize) -> f32 {
    run.glyphs
        .iter()
        .find(|g| g.start == index)
        .map(|g| g.w)
        .or_else(|| run.glyphs.first().map(|g| g.w))
        .unwrap_or(super::engine::FONT_SIZE * 0.6)
}

impl<M: Clone> Widget<M, iced::Theme, iced::Renderer> for DocView<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }
    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let st = tree.state.downcast_mut::<State>();
        let mut fs = font_system().write().expect("font system");
        let fs = fs.raw();
        let mut doc = self.content.0.borrow_mut();
        doc.configure(fs, &self.palette, self.lang, self.vim);
        doc.refresh(fs);
        let count = doc.line_count();
        let s = device_scale();
        if st.numbers.buffer.is_none() || st.numbers.count != count || st.numbers.scale != s {
            let (buf, w) = build_numbers(fs, count);
            if let Some(m) = font_metrics(fs, &buf) {
                st.metrics = m;
            }
            st.numbers = Numbers {
                count,
                scale: s,
                buffer: Some(buf),
                width: w,
            };
        }
        match (&self.placeholder, &st.placeholder) {
            (Some(p), Some((q, _))) if p == q => {}
            (Some(p), _) => {
                let (mut buf, _) = (
                    ct::Buffer::new(fs, ct::Metrics::new(super::engine::FONT_SIZE, LINE_HEIGHT)),
                    (),
                );
                buf.set_size(fs, None, None);
                buf.set_text(
                    fs,
                    p,
                    &ct::Attrs::new().family(ct::Family::Name(super::engine::mono_family(fs))),
                    ct::Shaping::Advanced,
                    None,
                );
                buf.shape_until_scroll(fs, false);
                st.placeholder = Some((p.clone(), Arc::new(buf)));
            }
            (None, _) => st.placeholder = None,
        }
        let w = self.gutter(st) + LINE_PAD_L + doc.width + LINE_PAD_R;
        let h = count as f32 * LINE_HEIGHT + 2.0 * PAD_Y;
        let size = Size::new(w.max(limits.min().width), h.max(limits.min().height));
        layout::Node::new(size)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let st = tree.state.downcast_mut::<State>();
        let b = layout.bounds();
        // `contenteditable=false` (readOnly) is not a Tab stop, but a
        // click still focuses it for selection and copy.
        st.focus
            .report(operation, self.id.as_ref(), b, true, !self.read_only);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_mut::<State>();
        let b = layout.bounds();
        let mut clip = Clip::new(clipboard);
        let clipboard: &mut dyn Clipboard = &mut clip;
        // The visible part of the content box: clicks below the last line
        // still land in the editor (`min-height: 100%`).
        let area = Rectangle {
            x: viewport.x,
            y: viewport.y,
            width: viewport.width,
            height: viewport.height,
        };
        let was_focused = st.focus.is_focused();
        st.focus.reveal(b, shell);

        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                // vim's `timeoutlen` for a pending `<leader>`.
                let deadline = self.content.0.borrow().vim_deadline();
                if let Some(at) = deadline {
                    if *now >= at {
                        let outcome = {
                            let mut fs = font_system().write().expect("font system");
                            let mut doc = self.content.0.borrow_mut();
                            doc.vim_timeout(fs.raw(), clipboard)
                        };
                        self.after(outcome, st, b, shell);
                    } else {
                        shell.request_redraw_at(at);
                    }
                }
                if st.focus.is_focused() {
                    let phase =
                        now.saturating_duration_since(st.blink).as_millis() % BLINK.as_millis();
                    let half = BLINK.as_millis() / 2;
                    let wait = if phase < half {
                        half - phase
                    } else {
                        BLINK.as_millis() - phase
                    };
                    shell.request_redraw_at(*now + Duration::from_millis(wait as u64 + 1));
                }
                return;
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(iced::touch::Event::FingerPressed { .. }) => {
                let inside = cursor.position().is_some_and(|p| area.contains(p));
                if st.focus.pointer(event, inside, true) {
                    shell.request_redraw();
                }
                let Some(p) = cursor.position().filter(|_| inside) else {
                    return;
                };
                if let Some(win) = window_point(event) {
                    st.to_content = p - win;
                }
                let now = Instant::now();
                let count = match st.clicks {
                    Some((t, q, n))
                        if now.duration_since(t) < MULTI_CLICK
                            && (q.x - p.x).abs() < 4.0
                            && (q.y - p.y).abs() < 4.0 =>
                    {
                        n % 3 + 1
                    }
                    _ => 1,
                };
                st.clicks = Some((now, p, count));
                let o = self.origin(b, st);
                let shift = MODS.with(|m| m.get().shift());
                let action = match count {
                    _ if shift && count == 1 => Pointer::Extend,
                    2 => Pointer::Double,
                    3 => Pointer::Triple,
                    _ => Pointer::Click,
                };
                {
                    let mut fs = font_system().write().expect("font system");
                    let mut doc = self.content.0.borrow_mut();
                    doc.pointer(fs.raw(), action, p.x - o.x, p.y - o.y);
                }
                st.dragging = true;
                st.blink = now;
                shell.capture_event();
                shell.request_redraw();
                return;
            }
            // X11 / Wayland primary selection: middle-click pastes it at
            // the pointer.
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle)) => {
                let Some(p) = cursor.position().filter(|p| area.contains(*p)) else {
                    return;
                };
                if self.read_only {
                    return;
                }
                if st.focus.pointer(event, true, true) {
                    shell.request_redraw();
                }
                let Some(text) = clipboard.read(clipboard::Kind::Primary) else {
                    return;
                };
                let text = text.replace("\r\n", "\n");
                let o = self.origin(b, st);
                let outcome = {
                    let mut fs = font_system().write().expect("font system");
                    let mut doc = self.content.0.borrow_mut();
                    doc.pointer(fs.raw(), Pointer::Click, p.x - o.x, p.y - o.y);
                    doc.paste_primary(fs.raw(), &text, clipboard)
                };
                st.blink = Instant::now();
                shell.capture_event();
                self.after(outcome, st, b, shell);
                return;
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let Some(p) = cursor.position() {
                    st.to_content = p - *position;
                }
                if !st.dragging {
                    return;
                }
                st.drag_at = Some(*position);
                let p = *position + st.to_content;
                let o = self.origin(b, st);
                {
                    let mut fs = font_system().write().expect("font system");
                    let mut doc = self.content.0.borrow_mut();
                    doc.pointer(fs.raw(), Pointer::Drag, p.x - o.x, (p.y - o.y).max(-1.0));
                }
                // Dragging past an edge scrolls (nearest, instant).
                crate::scroller::reveal(
                    Rectangle::new(Point::new(p.x - 1.0, p.y - 1.0), Size::new(2.0, 2.0)),
                    crate::scroller::Reveal {
                        smooth: false,
                        ..Default::default()
                    },
                );
                st.blink = Instant::now();
                shell.request_redraw();
                return;
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(iced::touch::Event::FingerLifted { .. }) => {
                if std::mem::take(&mut st.dragging) {
                    st.drag_at = None;
                    shell.capture_event();
                    self.own_primary(st, clipboard);
                }
                return;
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
                MODS.with(|c| c.set(*m));
                return;
            }
            _ => {}
        }

        if !st.focus.is_focused() {
            if was_focused {
                shell.request_redraw();
            }
            return;
        }
        if !was_focused {
            st.blink = Instant::now();
            shell.request_redraw();
        }

        let Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modifiers,
            text,
            physical_key,
            ..
        }) = event
        else {
            return;
        };
        // Shortcuts by their Latin letter, whatever the layout.
        let latin = key.to_latin(*physical_key);
        MODS.with(|c| c.set(*modifiers));
        if matches!(key, Key::Named(Named::Escape)) {
            st.esc = Some(Instant::now());
        } else if !matches!(key, Key::Named(Named::Tab)) {
            st.esc = None;
        }

        let page = (self.view_h - 2.0 * PAD_Y).max(LINE_HEIGHT);
        let handled = if self.vim {
            let lines = (page / LINE_HEIGHT).floor().max(1.0) as usize;
            self.vim_key(key, latin, *modifiers, text.as_deref(), clipboard, lines)
        } else {
            let tab_focus = matches!(key, Key::Named(Named::Tab))
                && st.esc.is_some_and(|t| t.elapsed() < TAB_FOCUS);
            if tab_focus {
                None
            } else {
                self.plain_key(key, latin, *modifiers, text.as_deref(), clipboard, page)
            }
        };
        let Some(outcome) = handled else {
            return;
        };
        shell.capture_event();
        st.blink = Instant::now();
        if !self.vim && (outcome.moved || outcome.changed) {
            self.own_primary(st, clipboard);
        }
        self.after(outcome, st, b, shell);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let st = tree.state.downcast_ref::<State>();
        let Some(p) = cursor.position().filter(|p| viewport.contains(*p)) else {
            return if st.dragging {
                mouse::Interaction::Text
            } else {
                mouse::Interaction::None
            };
        };
        let gx = viewport.x.max(layout.bounds().x);
        if p.x < gx + self.gutter(st) && !st.dragging {
            mouse::Interaction::Idle
        } else {
            mouse::Interaction::Text
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let st = tree.state.downcast_ref::<State>();
        let b = layout.bounds();
        let p = &self.palette;
        let s = device_scale();
        let doc = self.content.0.borrow();
        let Some(buffer) = doc.buffer() else { return };
        let o = self.origin(b, st);
        let g = self.gutter(st);
        let gx = viewport.x.max(b.x);
        let n = doc.line_count();
        let first = ((viewport.y - b.y - PAD_Y) / LINE_HEIGHT).floor().max(0.0) as usize;
        let last = (((viewport.y + viewport.height - b.y - PAD_Y) / LINE_HEIGHT)
            .ceil()
            .max(0.0) as usize)
            .min(n);
        let right = (b.x + b.width).max(viewport.x + viewport.width);
        let snap = |v: f32| (v * s).round() / s;
        let band = |i: usize| {
            let top = self.line_top(b, i);
            Rectangle {
                x: snap(b.x + g),
                y: snap(top),
                width: snap(right) - snap(b.x + g),
                height: snap(top + LINE_HEIGHT) - snap(top),
            }
        };
        let quad = |r: &mut iced::Renderer, rect: Rectangle, color: Color| {
            r.fill_quad(
                Quad {
                    bounds: rect,
                    ..Quad::default()
                },
                color,
            );
        };
        let focused = st.focus.is_focused();
        let cursor_pos = doc.cursor();
        let sel = doc.selection_bounds();
        let vi = doc.vi_mode();
        let block = matches!(
            vi,
            Some(
                ct::ViMode::Normal
                    | ct::ViMode::Visual
                    | ct::ViMode::VisualLine
                    | ct::ViMode::Extra(_)
                    | ct::ViMode::Replace
            )
        );

        let runs: Vec<ct::LayoutRun<'_>> = buffer
            .layout_runs()
            .filter(|r| r.line_i + 1 > first && r.line_i < last)
            .collect();

        // 1. Selection (`accent@.25`, behind the active line): to the end
        //    of the content on lines the selection runs past.
        if let Some((a, z)) = sel {
            let left = o.x;
            let full = right - LINE_PAD_R;
            for run in &runs {
                let i = run.line_i;
                if i < a.line || i > z.line {
                    continue;
                }
                let x0 = if i == a.line {
                    o.x + x_at(run, a.index)
                } else {
                    left
                };
                let x1 = if i == z.line {
                    o.x + x_at(run, z.index)
                } else {
                    full
                };
                if x1 > x0 {
                    let r = band(i);
                    quad(
                        renderer,
                        Rectangle {
                            x: snap(x0),
                            width: snap(x1) - snap(x0),
                            ..r
                        },
                        rgba(p.accent, 0.25),
                    );
                }
            }
            // 2. `highlightSelectionMatches`: other occurrences of a
            //    one-line selection (`accent@.18`).
            if a.line == z.line && !self.vim {
                let lines = doc.lines();
                let needle = &lines[a.line][a.index..z.index];
                if !needle.trim().is_empty() && needle.len() <= 200 {
                    for run in &runs {
                        let hay = &lines[run.line_i];
                        for (at, _) in hay.match_indices(needle) {
                            if run.line_i == a.line && at == a.index {
                                continue;
                            }
                            let x0 = o.x + x_at(run, at);
                            let x1 = o.x + x_at(run, at + needle.len());
                            let r = band(run.line_i);
                            quad(
                                renderer,
                                Rectangle {
                                    x: snap(x0),
                                    width: snap(x1) - snap(x0),
                                    ..r
                                },
                                rgba(p.accent, 0.18),
                            );
                        }
                    }
                }
            }
        }

        // 3. The active line (`border@.35`).
        if (first..last).contains(&cursor_pos.line) {
            quad(renderer, band(cursor_pos.line), rgba(p.border, 0.35));
        }

        // 4. Matching brackets (focused only).
        if focused && !block {
            let lines = doc.lines();
            let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
            let at = cursor_pos.index;
            let line = &lines[cursor_pos.line];
            let found = [text::char_step(line, at, false), Some(at)]
                .into_iter()
                .flatten()
                .filter(|&i| i < line.len())
                .find_map(|i| {
                    text::matching_bracket(&refs, cursor_pos.line, i)
                        .map(|m| ((cursor_pos.line, i), m))
                });
            if let Some((here, there)) = found {
                for (l, i) in [here, there] {
                    if let Some(run) = runs.iter().find(|r| r.line_i == l) {
                        let x0 = o.x + x_at(run, i);
                        let w = char_w(run, i);
                        quad(
                            renderer,
                            Rectangle {
                                x: snap(x0),
                                width: snap(x0 + w) - snap(x0),
                                ..band(l)
                            },
                            Color::from_rgba8(0x32, 0x8c, 0x82, 0x52 as f32 / 255.0),
                        );
                    }
                }
            }
        }

        // 5. Text, one line at a time so each baseline lands where Blink
        //    puts it.
        let base = baseline(st.metrics, s);
        let text_clip = Rectangle {
            x: viewport.x.max(b.x + g),
            width: (viewport.x + viewport.width - viewport.x.max(b.x + g)).max(0.0),
            ..*viewport
        };
        let mut frame = st.frame.borrow_mut();
        frame.clear();
        // `run_y`: the baseline inside the one-line buffer.
        let draw_line = |r: &mut iced::Renderer,
                         frame: &mut Vec<Arc<ct::Buffer>>,
                         buf: Arc<ct::Buffer>,
                         run_y: f32,
                         top: f32,
                         x: f32,
                         color: Color,
                         clip: Rectangle| {
            let baseline_dev = (top * s + base).round();
            let t = baseline_dev - (run_y * s).round();
            r.fill_raw(Raw {
                buffer: Arc::downgrade(&buf),
                position: Point::new(x, (t + 0.25) / s),
                color,
                clip_bounds: clip,
            });
            frame.push(buf);
        };
        let empty = n == 1 && doc.lines().first().is_some_and(String::is_empty);
        if empty
            && let Some((_, ph)) = &st.placeholder
            && let Some(run) = ph.layout_runs().next()
        {
            let top = self.line_top(b, 0);
            let clip = Rectangle {
                y: band(0).y - 2.0,
                height: band(0).height + 4.0,
                ..text_clip
            };
            draw_line(
                renderer,
                &mut frame,
                ph.clone(),
                run.line_y,
                top,
                o.x,
                Color::from_rgb8(0x88, 0x88, 0x88),
                clip,
            );
        }
        let mut cursor_line: Option<f32> = None;
        for run in &runs {
            let Some(one) = one_line(buffer, run.line_i) else {
                continue;
            };
            let top = self.line_top(b, run.line_i);
            let r = band(run.line_i);
            let clip = Rectangle {
                y: r.y - 2.0,
                height: r.height + 4.0,
                ..text_clip
            };
            let y = run.line_y - run.line_top;
            if run.line_i == cursor_pos.line {
                cursor_line = Some(y);
            }
            draw_line(renderer, &mut frame, one, y, top, o.x, c(p.text), clip);
        }

        // 6. Caret / vim block cursor.
        let blink_on = Instant::now()
            .saturating_duration_since(st.blink)
            .as_millis()
            % BLINK.as_millis()
            < BLINK.as_millis() / 2;
        if let Some(run) = runs.iter().find(|r| r.line_i == cursor_pos.line) {
            let top = self.line_top(b, cursor_pos.line);
            let a = (st.metrics.0 * super::engine::FONT_SIZE * s).round() / s;
            let d = (st.metrics.1 * super::engine::FONT_SIZE * s).round() / s;
            let ctop = snap(top + base / s - a);
            let ch = snap(top + base / s + d) - ctop;
            let x = o.x + x_at(run, cursor_pos.index);
            if block {
                let w = char_w(run, cursor_pos.index);
                let rect = Rectangle {
                    x: snap(x),
                    y: ctop,
                    width: snap(x + w) - snap(x),
                    height: ch,
                };
                if focused {
                    if blink_on {
                        quad(renderer, rect, c(p.accent));
                        // The letter in `bg` on the block.
                        if let Some(y) = cursor_line
                            && let Some(one) = one_line_plain(buffer, cursor_pos.line)
                        {
                            let clip = rect.intersection(&text_clip).unwrap_or_default();
                            draw_line(renderer, &mut frame, one, y, top, o.x, c(p.bg), clip);
                        }
                    }
                } else {
                    renderer.fill_quad(
                        Quad {
                            bounds: rect,
                            border: Border {
                                color: c(p.accent),
                                width: 1.0,
                                radius: 0.0.into(),
                            },
                            ..Quad::default()
                        },
                        Color::TRANSPARENT,
                    );
                }
            } else if focused && blink_on && x >= text_clip.x - 1.0 {
                quad(
                    renderer,
                    Rectangle {
                        x: snap(x - 0.6),
                        y: ctop,
                        width: 1.0 / s,
                        height: ch,
                    },
                    c(p.accent),
                );
            }
        }

        // 7. The gutter: sticky at the left edge, over scrolled text.
        if self.line_numbers {
            let gutter = Rectangle {
                x: snap(gx),
                y: viewport.y,
                width: snap(gx + g) - snap(gx),
                height: viewport.height,
            };
            renderer.fill_quad(
                Quad {
                    bounds: gutter,
                    border: Border {
                        radius: iced::border::Radius {
                            top_left: 7.0,
                            top_right: 0.0,
                            bottom_right: 0.0,
                            bottom_left: if self.round_bottom { 7.0 } else { 0.0 },
                        },
                        ..Border::default()
                    },
                    ..Quad::default()
                },
                c(p.surface),
            );
            if (first..last).contains(&cursor_pos.line) {
                let r = band(cursor_pos.line);
                quad(
                    renderer,
                    Rectangle {
                        x: gutter.x,
                        width: gutter.width,
                        ..r
                    },
                    rgba(p.border, 0.35),
                );
            }
            if let Some(nums) = &st.numbers.buffer {
                let cell_w = (st.numbers.width + NUM_PAD_L + NUM_PAD_R).max(NUM_MIN);
                for run in nums
                    .layout_runs()
                    .filter(|r| r.line_i + 1 > first && r.line_i < last)
                {
                    let Some(one) = one_line(nums, run.line_i) else {
                        continue;
                    };
                    let top = self.line_top(b, run.line_i);
                    let x = gx + cell_w - NUM_PAD_R - run.line_w;
                    let r = band(run.line_i);
                    let clip = Rectangle {
                        x: gx,
                        width: g,
                        y: r.y - 2.0,
                        height: r.height + 4.0,
                    }
                    .intersection(viewport)
                    .unwrap_or_default();
                    let color = if run.line_i == cursor_pos.line {
                        p.subtext
                    } else {
                        p.muted
                    };
                    draw_line(
                        renderer,
                        &mut frame,
                        one,
                        run.line_y - run.line_top,
                        top,
                        x,
                        c(color),
                        clip,
                    );
                }
            }
        }
    }
}

/// The clipboard the editor uses: the window's, or with `BATO_SELFTEST`
/// set a private in-process one, so a scripted run never reads or writes
/// the desktop's clipboard (its writes are logged on stderr instead).
struct Clip<'a> {
    real: &'a mut dyn Clipboard,
    sandbox: bool,
}

static SANDBOX: std::sync::Mutex<[Option<String>; 2]> = std::sync::Mutex::new([None, None]);

impl<'a> Clip<'a> {
    fn new(real: &'a mut dyn Clipboard) -> Self {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let sandbox = *ON.get_or_init(|| std::env::var_os("BATO_SELFTEST").is_some());
        Self { real, sandbox }
    }
}

impl Clipboard for Clip<'_> {
    fn read(&self, kind: clipboard::Kind) -> Option<String> {
        if self.sandbox {
            let slots = SANDBOX.lock().expect("sandbox clipboard");
            return slots[usize::from(kind == clipboard::Kind::Primary)].clone();
        }
        self.real.read(kind)
    }
    fn write(&mut self, kind: clipboard::Kind, contents: String) {
        if self.sandbox {
            eprintln!("SELFTEST clipboard {kind:?} {contents:?}");
            let mut slots = SANDBOX.lock().expect("sandbox clipboard");
            slots[usize::from(kind == clipboard::Kind::Primary)] = Some(contents);
            return;
        }
        self.real.write(kind, contents);
    }
}

thread_local! {
    static MODS: std::cell::Cell<keyboard::Modifiers> = const { std::cell::Cell::new(keyboard::Modifiers::empty()) };
}

fn window_point(event: &Event) -> Option<Point> {
    match event {
        Event::Touch(iced::touch::Event::FingerPressed { position, .. }) => Some(*position),
        _ => None,
    }
}

impl<M> DocView<'_, M> {
    /// Redraw, report a text change, keep the caret in view (or centre its
    /// line, vim's `zz`).
    fn after(&self, outcome: Outcome, st: &mut State, b: Rectangle, shell: &mut Shell<'_, M>) {
        shell.request_redraw();
        if let Some(at) = self.content.0.borrow().vim_deadline() {
            shell.request_redraw_at(at);
        }
        if outcome.changed {
            shell.invalidate_layout();
            if let Some(f) = &self.on_change {
                shell.publish(f(self.content.text()));
            }
        }
        if !(outcome.changed || outcome.moved || outcome.center) {
            return;
        }
        let Some(r) = self.caret_rect(b, st) else {
            return;
        };
        if outcome.center {
            // A view-tall band centred on the line: the editor scrolls it
            // into place; the page around only if the editor is off-screen.
            crate::scroller::reveal(
                Rectangle {
                    y: r.y + LINE_HEIGHT / 2.0 - self.view_h / 2.0,
                    height: self.view_h,
                    ..r
                },
                crate::scroller::Reveal {
                    smooth: false,
                    ..Default::default()
                },
            );
        } else {
            // `scrollIntoView` of the caret (CodeMirror's 5 px margins).
            crate::scroller::reveal(
                r,
                crate::scroller::Reveal {
                    smooth: false,
                    margin: 5.0,
                    ..Default::default()
                },
            );
        }
    }

    /// A selection made in plain mode becomes the primary selection (X11 /
    /// Wayland), so a middle-click elsewhere pastes it.
    fn own_primary(&self, st: &mut State, clipboard: &mut dyn Clipboard) {
        if self.vim {
            return;
        }
        let Some(t) = self.content.0.borrow().selected_text() else {
            return;
        };
        if st.primary.as_ref() != Some(&t) {
            clipboard.write(clipboard::Kind::Primary, t.clone());
            st.primary = Some(t);
        }
    }

    /// The caret's rectangle in content coordinates.
    fn caret_rect(&self, b: Rectangle, st: &State) -> Option<Rectangle> {
        let doc = self.content.0.borrow();
        let buffer = doc.buffer()?;
        let c = doc.cursor();
        let run = buffer.layout_runs().find(|r| r.line_i == c.line)?;
        let o = self.origin(b, st);
        let x = o.x + x_at(&run, c.index);
        // The gutter covers the left edge: keep the caret right of it.
        Some(Rectangle {
            x: x - self.gutter(st) - LINE_PAD_L,
            y: self.line_top(b, c.line),
            width: self.gutter(st) + LINE_PAD_L + 2.0,
            height: LINE_HEIGHT,
        })
    }

    pub(super) fn plain_key(
        &self,
        key: &Key,
        latin: Option<char>,
        m: keyboard::Modifiers,
        text: Option<&str>,
        clipboard: &mut dyn Clipboard,
        page: f32,
    ) -> Option<Outcome> {
        let ctrl = m.control();
        let shift = m.shift();
        let alt = m.alt();
        let ro = self.read_only;
        let mut fs = font_system().write().expect("font system");
        let fs = fs.raw();
        let mut doc = self.content.0.borrow_mut();
        let mv = |mv: Move| Some(Cmd::Move(mv, shift));
        let cmd: Option<Cmd> = match key {
            Key::Named(named) => match named {
                Named::ArrowLeft if alt => None,
                Named::ArrowLeft => mv(if ctrl {
                    Move::GroupLeft
                } else {
                    Move::CharLeft
                }),
                Named::ArrowRight if alt => None,
                Named::ArrowRight => mv(if ctrl {
                    Move::GroupRight
                } else {
                    Move::CharRight
                }),
                Named::ArrowUp if alt && !ro => Some(if shift {
                    Cmd::CopyLine(true)
                } else {
                    Cmd::MoveLine(true)
                }),
                Named::ArrowDown if alt && !ro => Some(if shift {
                    Cmd::CopyLine(false)
                } else {
                    Cmd::MoveLine(false)
                }),
                Named::ArrowUp => mv(Move::LineUp),
                Named::ArrowDown => mv(Move::LineDown),
                Named::Home => mv(if ctrl {
                    Move::DocStart
                } else {
                    Move::LineStart
                }),
                Named::End => mv(if ctrl { Move::DocEnd } else { Move::LineEnd }),
                Named::PageUp => mv(Move::PageUp),
                Named::PageDown => mv(Move::PageDown),
                _ if ro => None,
                Named::Backspace => Some(if ctrl {
                    Cmd::DeleteGroupBack
                } else {
                    Cmd::Backspace
                }),
                Named::Delete if shift => {
                    if let Some(t) = doc.copy(fs, true) {
                        clipboard.write(clipboard::Kind::Standard, t);
                    }
                    return Some(Outcome {
                        changed: true,
                        moved: true,
                        center: false,
                    });
                }
                Named::Delete => Some(if ctrl {
                    Cmd::DeleteGroupForward
                } else {
                    Cmd::Delete
                }),
                Named::Enter if ctrl => Some(Cmd::BlankLine),
                Named::Enter => Some(Cmd::Newline),
                Named::Tab => Some(if shift {
                    Cmd::IndentLess
                } else {
                    Cmd::IndentMore
                }),
                Named::Insert if shift => clipboard
                    .read(clipboard::Kind::Standard)
                    .map(|t| Cmd::Paste(t.replace("\r\n", "\n"))),
                Named::Insert if ctrl => {
                    if let Some(t) = doc.copy(fs, false) {
                        clipboard.write(clipboard::Kind::Standard, t);
                    }
                    return Some(Outcome::default());
                }
                Named::Space if !ctrl && !alt => Some(Cmd::Type(" ".into())),
                _ => None,
            },
            Key::Character(ch) if ctrl && !alt => {
                let ch = latin
                    .map(|c| c.to_ascii_lowercase().to_string())
                    .unwrap_or_else(|| ch.to_lowercase());
                match ch.as_str() {
                    "a" => Some(Cmd::SelectAll),
                    "c" => {
                        if let Some(t) = doc.copy(fs, false) {
                            clipboard.write(clipboard::Kind::Standard, t);
                        }
                        return Some(Outcome::default());
                    }
                    "l" => Some(Cmd::SelectLine),
                    _ if ro => None,
                    "z" if shift => return Some(redo(&mut doc)),
                    "z" => return Some(undo(&mut doc)),
                    "y" => return Some(redo(&mut doc)),
                    "x" => {
                        let t = doc.copy(fs, true)?;
                        clipboard.write(clipboard::Kind::Standard, t);
                        return Some(Outcome {
                            changed: true,
                            moved: true,
                            center: false,
                        });
                    }
                    "v" => clipboard
                        .read(clipboard::Kind::Standard)
                        .map(|t| Cmd::Paste(t.replace("\r\n", "\n"))),
                    "/" => Some(Cmd::ToggleComment),
                    "]" => Some(Cmd::IndentMore),
                    "[" => Some(Cmd::IndentLess),
                    "k" if shift => Some(Cmd::DeleteLine),
                    "\\" | "|" if shift => Some(Cmd::MatchingBracket(false)),
                    _ => None,
                }
            }
            _ if ro || ctrl || m.logo() => None,
            _ => text
                .filter(|t| !t.is_empty() && t.chars().all(|c| !c.is_control()))
                .map(|t| Cmd::Type(t.to_string())),
        };
        let cmd = cmd?;
        Some(doc.run(fs, &cmd, page))
    }

    pub(super) fn vim_key(
        &self,
        key: &Key,
        latin: Option<char>,
        m: keyboard::Modifiers,
        text: Option<&str>,
        clipboard: &mut dyn Clipboard,
        view_lines: usize,
    ) -> Option<Outcome> {
        let vk = if m.control() {
            let ch = match key {
                Key::Named(Named::Space) => ' ',
                _ => latin.or_else(|| match key {
                    Key::Character(c) => c.chars().next(),
                    _ => None,
                })?,
            };
            VimKey::Ctrl(ch.to_ascii_lowercase(), m.shift())
        } else {
            match key {
                Key::Named(Named::Space) => VimKey::Char(' '),
                Key::Named(Named::Insert) if m.shift() => VimKey::ShiftInsert,
                Key::Named(named) => VimKey::Action(match named {
                    Named::Escape => ct::Action::Escape,
                    Named::Enter => ct::Action::Enter,
                    Named::Backspace => ct::Action::Backspace,
                    Named::Delete => ct::Action::Delete,
                    Named::ArrowLeft => ct::Action::Motion(ct::Motion::Left),
                    Named::ArrowRight => ct::Action::Motion(ct::Motion::Right),
                    Named::ArrowUp => ct::Action::Motion(ct::Motion::Up),
                    Named::ArrowDown => ct::Action::Motion(ct::Motion::Down),
                    Named::Home => ct::Action::Motion(ct::Motion::Home),
                    Named::End => ct::Action::Motion(ct::Motion::End),
                    Named::PageUp => ct::Action::Motion(ct::Motion::PageUp),
                    Named::PageDown => ct::Action::Motion(ct::Motion::PageDown),
                    Named::Tab if m.shift() => ct::Action::Unindent,
                    Named::Tab => ct::Action::Indent,
                    _ => return None,
                }),
                _ if m.alt() || m.logo() => return None,
                _ => {
                    let t = text.filter(|t| !t.is_empty() && t.chars().all(|c| !c.is_control()))?;
                    let keys: Vec<VimKey> = t.chars().map(VimKey::Char).collect();
                    return self.vim_keys(&keys, clipboard, view_lines);
                }
            }
        };
        self.vim_keys(&[vk], clipboard, view_lines)
    }

    fn vim_keys(
        &self,
        keys: &[VimKey],
        clipboard: &mut dyn Clipboard,
        view_lines: usize,
    ) -> Option<Outcome> {
        let mut fs = font_system().write().expect("font system");
        let fs = fs.raw();
        let mut doc = self.content.0.borrow_mut();
        let mut out: Option<Outcome> = None;
        for &k in keys {
            if let Some(o) = doc.vim_input(fs, k, clipboard, view_lines, self.read_only) {
                *out.get_or_insert_default() |= o;
            }
        }
        out
    }
}

fn undo(doc: &mut super::engine::Doc) -> Outcome {
    let changed = doc.undo();
    Outcome {
        changed,
        moved: changed,
        center: false,
    }
}

fn redo(doc: &mut super::engine::Doc) -> Outcome {
    let changed = doc.redo();
    Outcome {
        changed,
        moved: changed,
        center: false,
    }
}

impl<'a, M: Clone + 'a> From<DocView<'a, M>> for Element<'a, M, iced::Theme, iced::Renderer> {
    fn from(v: DocView<'a, M>) -> Self {
        Element::new(v)
    }
}
