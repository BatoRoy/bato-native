//! `TimeInput` (`inputs/TimeInput.tsx`): a typeable H:M(:S) field.
//!
//! One control box (`controlClasses` + `sizeClasses`, `inline-flex gap-0.5`)
//! holding a `w-7` box per segment (each its own Tab stop, like React's
//! inputs), `:` separators (`muted`, `px-0.5`) and the up/down chevron
//! column (`ml-1 -my-1`). A focused segment is selected and turns
//! `accent@.15` / `accentBright` (150 ms); after a value change the caret
//! sits at its end and blinks.
//!
//! Keys on a segment: digits type (a first digit that can't be a tens digit
//! moves on at once; two digits move on), ↑/↓ step by the segment's unit and
//! repeat while held ([`HoldRepeat`]), ←/→ change segment, Backspace /
//! Delete zero it, other characters are refused. Stepping goes through total
//! seconds, so 59 m + 1 carries into the hour; at the ends it wraps or
//! clamps (refuses). The chevrons step the last focused segment (seconds,
//! or minutes, at first), repeat while held, and don't take focus.
//!
//! ```ignore
//! time_input(&cx, self.time, Msg::Time).label("Duration").show_seconds(true).wrap(true)
//! ```

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::operation::Focusable;
use iced::advanced::widget::{self, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell};
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{Column, column};
use iced::{
    Border, Color, Element, Event, Length, Point, Rectangle, Size, Vector, mouse, touch, window,
};

use super::frame::{Look, frame_style};
use crate::El;
use crate::anim::{HoldRepeat, ease_standard, fade, with_opacity};
use crate::ctx::Ctx;
use crate::field::{FieldSize, Metrics};
use crate::focus::FocusState;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::style::{c, lerp, rgba};
use crate::surface::paint;

// ── The value and its rules ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TimeValue {
    pub hours: u32,
    pub minutes: u32,
    pub seconds: u32,
}

impl TimeValue {
    pub fn new(hours: u32, minutes: u32, seconds: u32) -> Self {
        Self {
            hours,
            minutes,
            seconds,
        }
    }
    pub fn total(&self) -> i64 {
        self.hours as i64 * 3600 + self.minutes as i64 * 60 + self.seconds as i64
    }
    pub fn from_total(t: i64) -> Self {
        let t = t.max(0);
        Self::new((t / 3600) as u32, ((t / 60) % 60) as u32, (t % 60) as u32)
    }
    /// `HH:MM:SS`.
    pub fn hms(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hours, self.minutes, self.seconds)
    }
    fn get(&self, i: usize) -> u32 {
        [self.hours, self.minutes, self.seconds][i]
    }
}

/// The props that shape the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSpec {
    pub show_seconds: bool,
    /// Hours run `0..max_hours` (24 with `capHours`, 100 without).
    pub max_hours: u32,
    pub wrap: bool,
}

impl Default for TimeSpec {
    fn default() -> Self {
        Self {
            show_seconds: true,
            max_hours: 24,
            wrap: true,
        }
    }
}

impl TimeSpec {
    pub fn segments(&self) -> usize {
        if self.show_seconds { 3 } else { 2 }
    }
    pub fn unit(&self, i: usize) -> i64 {
        [3600, 60, 1][i]
    }
    pub fn seg_max(&self, i: usize) -> u32 {
        if i == 0 { self.max_hours - 1 } else { 59 }
    }
}

/// `applyDelta`: step through total seconds; wrap, or refuse past an end.
pub fn apply_delta(v: TimeValue, delta: i64, spec: TimeSpec) -> Option<TimeValue> {
    let max = spec.max_hours as i64 * 3600;
    let t = v.total() + delta;
    if spec.wrap {
        Some(TimeValue::from_total(t.rem_euclid(max)))
    } else if t < 0 || t >= max {
        None
    } else {
        Some(TimeValue::from_total(t))
    }
}

/// `setSegment`: one field, clamped to its range.
pub fn set_segment(v: TimeValue, i: usize, n: u32, spec: TimeSpec) -> TimeValue {
    let n = n.min(spec.seg_max(i));
    let mut out = v;
    match i {
        0 => out.hours = n,
        1 => out.minutes = n,
        _ => out.seconds = n,
    }
    out
}

/// `handleDigit`: the new value, the new typed count, and whether focus
/// moves to the next segment.
pub fn handle_digit(
    v: TimeValue,
    i: usize,
    d: u32,
    typed: u8,
    spec: TimeSpec,
) -> (TimeValue, u8, bool) {
    if typed == 0 {
        let next = set_segment(v, i, d, spec);
        if d * 10 > spec.seg_max(i) {
            (next, 0, true)
        } else {
            (next, 1, false)
        }
    } else {
        (set_segment(v, i, v.get(i) * 10 + d, spec), 0, true)
    }
}

// ── Builder ─────────────────────────────────────────────────────────────

pub struct TimeInput<'a, M> {
    cx: Ctx,
    value: TimeValue,
    on_change: Box<dyn Fn(TimeValue) -> M + 'a>,
    spec: TimeSpec,
    spinners: bool,
    disabled: bool,
    size: FieldSize,
    label: Option<String>,
    hint: Option<String>,
    error: Option<String>,
    required: bool,
}

pub fn time_input<'a, M: Clone + 'a>(
    cx: &Ctx,
    value: TimeValue,
    on_change: impl Fn(TimeValue) -> M + 'a,
) -> TimeInput<'a, M> {
    TimeInput {
        cx: cx.clone(),
        value,
        on_change: Box::new(on_change),
        spec: TimeSpec::default(),
        spinners: true,
        disabled: false,
        size: FieldSize::Md,
        label: None,
        hint: None,
        error: None,
        required: false,
    }
}

impl<'a, M: Clone + 'a> TimeInput<'a, M> {
    pub fn show_seconds(mut self, on: bool) -> Self {
        self.spec.show_seconds = on;
        self
    }
    /// Hours 0–23 (default) or, off, 0–99.
    pub fn cap_hours(mut self, on: bool) -> Self {
        self.spec.max_hours = if on { 24 } else { 100 };
        self
    }
    /// An explicit hour count (wins over `cap_hours`).
    pub fn max_hours(mut self, n: u32) -> Self {
        self.spec.max_hours = n.max(1);
        self
    }
    pub fn wrap(mut self, on: bool) -> Self {
        self.spec.wrap = on;
        self
    }
    pub fn spinners(mut self, on: bool) -> Self {
        self.spinners = on;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn size(mut self, s: FieldSize) -> Self {
        self.size = s;
        self
    }
    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = Some(l.into());
        self
    }
    pub fn hint(mut self, h: impl Into<String>) -> Self {
        self.hint = Some(h.into());
        self
    }
    pub fn error(mut self, e: Option<&str>) -> Self {
        self.error = e.map(str::to_string);
        self
    }
    pub fn required(mut self, on: bool) -> Self {
        self.required = on;
        self
    }
}

impl<'a, M: Clone + 'a> From<TimeInput<'a, M>> for El<'a, M> {
    fn from(t: TimeInput<'a, M>) -> Self {
        let cx = t.cx.clone();
        let p = cx.p();
        let m = t.size.metrics();
        let n = t.spec.segments();
        let mut kids: Vec<El<'a, M>> = Vec::new();
        for i in 0..n {
            kids.push(
                label(format!("{:02}", t.value.get(i)), m.text, m.line)
                    .tabular()
                    .into(),
            );
        }
        for _ in 1..n {
            kids.push(label(":", m.text, m.line).color(c(p.muted)).into());
        }
        let spin = t.spinners && !t.disabled;
        if spin {
            kids.push(icon(Ico::ChevronUp, 13.0).into());
            kids.push(icon(Ico::ChevronDown, 13.0).into());
        }
        let control = Element::new(TimeBox {
            cx: cx.clone(),
            kids,
            value: t.value,
            spec: t.spec,
            metrics: m,
            spin,
            disabled: t.disabled,
            error: t.error.is_some(),
            on_change: t.on_change,
        });
        // React's Field, sized to the inline-flex control (`w-auto`).
        let mut col: Column<'a, M, iced::Theme, iced::Renderer> = column![].spacing(6);
        if let Some(l) = t.label {
            let lab = label(l, 12.0, 16.0).medium().color(c(p.subtext));
            col = col.push(if t.required {
                El::from(iced::widget::row![
                    lab,
                    iced::widget::container(label("*", 12.0, 16.0).medium().color(c(p.red)))
                        .padding(iced::Padding::default().left(2))
                ])
            } else {
                lab.into()
            });
        }
        col = col.push(control);
        if let Some(e) = t.error {
            col = col.push(label(e, 12.0, 16.0).color(c(p.red)));
        } else if let Some(h) = t.hint {
            col = col.push(label(h, 12.0, 16.0).color(c(p.muted)));
        }
        col.into()
    }
}

// ── The control ─────────────────────────────────────────────────────────

/// `w-7`.
const SEG_W: f32 = 28.0;
const CARET_BLINK: Duration = Duration::from_millis(500);
const FADE: Duration = Duration::from_millis(150);

struct TimeBox<'a, M> {
    cx: Ctx,
    /// Segment texts, colons, then the two chevrons.
    kids: Vec<El<'a, M>>,
    value: TimeValue,
    spec: TimeSpec,
    metrics: Metrics,
    spin: bool,
    disabled: bool,
    error: bool,
    on_change: Box<dyn Fn(TimeValue) -> M + 'a>,
}

#[derive(Debug, Default, Clone, Copy)]
struct Fade {
    on: bool,
    from: f32,
    t: f32,
    started: Option<Instant>,
}

impl Fade {
    fn set(&mut self, on: bool, now: Instant) {
        if on != self.on {
            self.on = on;
            self.from = self.t;
            self.started = Some(now);
        }
    }
    fn advance(&mut self, now: Instant) -> bool {
        let Some(s) = self.started else { return false };
        let target = if self.on { 1.0 } else { 0.0 };
        let k = (now.saturating_duration_since(s).as_secs_f32() / FADE.as_secs_f32()).min(1.0);
        self.t = self.from + (target - self.from) * ease_standard(k);
        if k >= 1.0 {
            self.started = None;
            false
        } else {
            true
        }
    }
}

#[derive(Debug)]
struct TimeState {
    focus: [FocusState; 3],
    was: [bool; 3],
    typed: [u8; 3],
    fades: [Fade; 3],
    /// `focusedIdx`: the segment the spinners step (the last focused one).
    last: Option<usize>,
    selected: bool,
    caret_from: Instant,
    shown: Option<TimeValue>,
    /// Keyboard ↑/↓ held: the repeat and its delta.
    kb: Option<(HoldRepeat, i64)>,
    /// A chevron held: the repeat and its direction.
    spin: Option<(HoldRepeat, i64)>,
    spin_hover: Option<usize>,
    finger: Option<u64>,
}

impl Default for TimeState {
    fn default() -> Self {
        Self {
            focus: Default::default(),
            was: [false; 3],
            typed: [0; 3],
            fades: Default::default(),
            last: None,
            selected: false,
            caret_from: Instant::now(),
            shown: None,
            kb: None,
            spin: None,
            spin_hover: None,
            finger: None,
        }
    }
}

impl TimeState {
    fn focused(&self) -> Option<usize> {
        self.focus.iter().position(FocusState::is_focused)
    }
    /// `focusSeg(i)`: focus and select a segment (no `onFocus` when it
    /// already had focus).
    fn focus_seg(&mut self, i: usize, n: usize) {
        let j = i.min(n - 1);
        for (k, f) in self.focus.iter_mut().enumerate() {
            if k == j {
                if !f.is_focused() {
                    Focusable::focus(f);
                }
            } else if f.is_focused() {
                Focusable::unfocus(f);
            }
        }
        self.selected = true;
    }
}

impl<'a, M: Clone + 'a> TimeBox<'a, M> {
    fn n(&self) -> usize {
        self.spec.segments()
    }
    /// The segment boxes, colon slots and the chevron column, relative to
    /// the control's top-left, and the control size.
    fn geometry(
        &self,
        colon_w: f32,
    ) -> (Vec<Rectangle>, Vec<Rectangle>, Option<[Rectangle; 2]>, Size) {
        let m = self.metrics;
        let top = 1.0 + m.pad_y;
        let mut x = 1.0 + m.pad_x;
        let mut segs = Vec::new();
        let mut colons = Vec::new();
        for i in 0..self.n() {
            if i > 0 {
                x += 2.0;
                colons.push(Rectangle::new(
                    Point::new(x + 2.0, top),
                    Size::new(colon_w, m.line),
                ));
                x += 4.0 + colon_w;
            }
            segs.push(Rectangle::new(Point::new(x, top), Size::new(SEG_W, m.line)));
            x += SEG_W;
        }
        let spin = self.spin.then(|| {
            x += 2.0 + 4.0;
            // `-my-1` around a 26 px column, centred on the line.
            let y = top + (m.line - 18.0) / 2.0 - 4.0;
            let r = [
                Rectangle::new(Point::new(x, y), Size::new(13.0, 13.0)),
                Rectangle::new(Point::new(x, y + 13.0), Size::new(13.0, 13.0)),
            ];
            x += 13.0;
            r
        });
        let w = x + m.pad_x + 1.0;
        (segs, colons, spin, Size::new(w, m.height()))
    }
    fn rects(&self, layout: Layout<'_>) -> (Vec<Rectangle>, Option<[Rectangle; 2]>) {
        let b = layout.bounds();
        let colon_w = layout
            .children()
            .nth(self.n())
            .map_or(4.0, |l| l.bounds().width);
        let (segs, _, spin, _) = self.geometry(colon_w);
        let off = Vector::new(b.x, b.y);
        (
            segs.into_iter().map(|r| r + off).collect(),
            spin.map(|s| [s[0] + off, s[1] + off]),
        )
    }
    fn step(&self, st: &TimeState, delta: i64, times: u32) -> Option<TimeValue> {
        let mut v = self.value;
        let mut changed = false;
        for _ in 0..times {
            if let Some(next) = apply_delta(v, delta, self.spec) {
                v = next;
                changed = true;
            }
        }
        let _ = st;
        changed.then_some(v)
    }
    fn spin_unit(&self, st: &TimeState) -> i64 {
        let n = self.n();
        self.spec.unit(st.last.unwrap_or(n - 1).min(n - 1))
    }
}

impl<'a, M: Clone + 'a> Widget<M, iced::Theme, iced::Renderer> for TimeBox<'a, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<TimeState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(TimeState::default())
    }
    fn children(&self) -> Vec<Tree> {
        self.kids.iter().map(Tree::new).collect()
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.kids);
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        let loose = layout::Limits::new(Size::ZERO, Size::INFINITE);
        let n = self.n();
        let mut nodes: Vec<layout::Node> = self
            .kids
            .iter_mut()
            .zip(tree.children.iter_mut())
            .map(|(k, t)| k.as_widget_mut().layout(t, renderer, &loose))
            .collect();
        let colon_w = nodes.get(n).map_or(4.0, |c| c.size().width);
        let (segs, colons, spin, size) = self.geometry(colon_w);
        for (i, r) in segs.iter().enumerate() {
            let w = nodes[i].size().width;
            // Chromium's centred text lands on the whole pixel to the left.
            let x = r.x + ((SEG_W - w) / 2.0).floor();
            nodes[i] = nodes[i].clone().move_to(Point::new(x, r.y));
        }
        for (j, r) in colons.iter().enumerate() {
            // Measured against Chromium: its `:` lands a pixel to the left.
            nodes[n + j] = nodes[n + j].clone().move_to(Point::new(r.x - 1.0, r.y));
        }
        if let Some(s) = spin {
            let k = 2 * n - 1;
            nodes[k] = nodes[k].clone().move_to(Point::new(s[0].x, s[0].y));
            nodes[k + 1] = nodes[k + 1].clone().move_to(Point::new(s[1].x, s[1].y));
        }
        layout::Node::with_children(size, nodes)
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let n = self.n();
        let (segs, spin) = self.rects(layout);
        let st = tree.state.downcast_mut::<TimeState>();
        let now = Instant::now();
        let enabled = !self.disabled;

        // A press on a chevron: step and hold, keeping focus where it is.
        let press_at = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => cursor.position(),
            Event::Touch(touch::Event::FingerPressed { position, .. }) => Some(*position),
            _ => None,
        }
        .filter(|p| viewport.contains(*p));
        let on_spin =
            press_at.and_then(|p| spin.and_then(|s| s.iter().position(|r| r.contains(p))));
        if let Some(k) = on_spin {
            let dir = if k == 0 { 1 } else { -1 };
            let delta = dir * self.spin_unit(st);
            if let Some(v) = self.step(st, delta, 1) {
                shell.publish((self.on_change)(v));
            }
            st.spin = Some((HoldRepeat::press(now), delta));
            if let Event::Touch(touch::Event::FingerPressed { id, .. }) = event {
                st.finger = Some(id.0);
            }
            shell.request_redraw_at(now + HoldRepeat::DELAY);
            shell.capture_event();
            return;
        }

        // Pointer focus: a press on a segment focuses it, anywhere else blurs.
        for (i, seg) in segs.iter().enumerate() {
            let inside = press_at.is_some_and(|p| seg.contains(p));
            if st.focus[i].sync(enabled) | st.focus[i].pointer(event, inside, enabled) {
                shell.request_redraw();
            }
            st.focus[i].reveal(layout.bounds(), shell);
        }
        if press_at.is_some_and(|p| segs.iter().any(|r| r.contains(p))) {
            shell.capture_event();
        }
        // Focus gained (`onFocus`) / lost (`onBlur`).
        for i in 0..n {
            let now_f = st.focus[i].is_focused();
            if now_f && !st.was[i] {
                st.typed[i] = 0;
                st.last = Some(i);
                st.selected = true;
                st.caret_from = now;
            }
            if !now_f && st.was[i] {
                st.kb = None;
            }
            st.was[i] = now_f;
            st.fades[i].set(now_f, now);
        }
        // A new value collapses the focused segment's selection.
        if st.shown != Some(self.value) {
            if let (Some(old), Some(f)) = (st.shown, st.focused())
                && old.get(f) != self.value.get(f)
            {
                st.selected = false;
                st.caret_from = now;
            }
            st.shown = Some(self.value);
            shell.request_redraw();
        }

        match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modifiers,
                text,
                repeat,
                ..
            }) if enabled && !shell.is_event_captured() => {
                let Some(i) = st.focused() else { return };
                let plain = crate::focus::plain(*modifiers);
                let digit = text
                    .as_ref()
                    .and_then(|t| t.chars().next())
                    .filter(|c| c.is_ascii_digit() && plain);
                if let Some(d) = digit {
                    let (v, typed, next) = handle_digit(
                        self.value,
                        i,
                        d.to_digit(10).unwrap(),
                        st.typed[i],
                        self.spec,
                    );
                    st.typed[i] = typed;
                    shell.publish((self.on_change)(v));
                    if next {
                        st.focus_seg(i + 1, n);
                    }
                    st.caret_from = now;
                    shell.capture_event();
                    shell.request_redraw();
                    return;
                }
                match key {
                    Key::Named(Named::ArrowUp | Named::ArrowDown) => {
                        if !repeat {
                            let up = *key == Key::Named(Named::ArrowUp);
                            let delta = if up { 1 } else { -1 } * self.spec.unit(i);
                            if let Some(v) = self.step(st, delta, 1) {
                                shell.publish((self.on_change)(v));
                            }
                            st.kb = Some((HoldRepeat::press(now), delta));
                            shell.request_redraw_at(now + HoldRepeat::DELAY);
                        }
                        shell.capture_event();
                    }
                    Key::Named(Named::ArrowLeft) => {
                        st.focus_seg(i.saturating_sub(1), n);
                        shell.capture_event();
                        shell.request_redraw();
                    }
                    Key::Named(Named::ArrowRight) => {
                        st.focus_seg(i + 1, n);
                        shell.capture_event();
                        shell.request_redraw();
                    }
                    Key::Named(Named::Backspace | Named::Delete) => {
                        shell.publish((self.on_change)(set_segment(self.value, i, 0, self.spec)));
                        st.typed[i] = 0;
                        shell.capture_event();
                    }
                    _ => {
                        let printable = text
                            .as_ref()
                            .is_some_and(|t| t.chars().any(|c| !c.is_control()));
                        if printable && !modifiers.control() && !modifiers.logo() {
                            shell.capture_event();
                        }
                    }
                }
            }
            Event::Keyboard(keyboard::Event::KeyReleased {
                key: Key::Named(Named::ArrowUp | Named::ArrowDown),
                ..
            }) => {
                st.kb = None;
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if st.finger.is_none() && st.spin.take().is_some() {
                    shell.capture_event();
                }
            }
            Event::Touch(
                touch::Event::FingerLifted { id, .. } | touch::Event::FingerLost { id, .. },
            ) if st.finger == Some(id.0) => {
                st.finger = None;
                st.spin = None;
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let hover = spin.and_then(|s| s.iter().position(|r| r.contains(*position)));
                // `onPointerLeave` stops a held chevron.
                if let Some((_, d)) = st.spin
                    && st.finger.is_none()
                    && hover != Some(if d > 0 { 0 } else { 1 })
                {
                    st.spin = None;
                }
                if hover != st.spin_hover {
                    st.spin_hover = hover;
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::CursorLeft) => {
                if st.finger.is_none() {
                    st.spin = None;
                }
                if st.spin_hover.take().is_some() {
                    shell.request_redraw();
                }
            }
            Event::Window(window::Event::RedrawRequested(at)) => {
                let mut more = false;
                for f in &mut st.fades {
                    more |= f.advance(*at);
                }
                if more {
                    shell.request_redraw();
                }
                for (h, delta) in [&mut st.kb, &mut st.spin].into_iter().flatten() {
                    {
                        let (k, next) = h.due(*at);
                        let delta = *delta;
                        if k > 0 {
                            let v = {
                                let mut v = self.value;
                                for _ in 0..k {
                                    if let Some(x) = apply_delta(v, delta, self.spec) {
                                        v = x;
                                    }
                                }
                                v
                            };
                            if v != self.value {
                                shell.publish((self.on_change)(v));
                            }
                        }
                        shell.request_redraw_at(next);
                    }
                }
                // The caret blinks while a segment is focused and unselected.
                if st.focused().is_some() && !st.selected && enabled {
                    let el = at.saturating_duration_since(st.caret_from).as_millis() as u64;
                    let half = CARET_BLINK.as_millis() as u64;
                    let next = st.caret_from + Duration::from_millis((el / half + 1) * half);
                    shell.request_redraw_at(next);
                }
            }
            _ => {}
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
        let Some(p) = cursor.position() else {
            return mouse::Interaction::None;
        };
        let (segs, spin) = self.rects(layout);
        if spin.is_some_and(|s| s.iter().any(|r| r.contains(p))) {
            mouse::Interaction::Pointer
        } else if segs.iter().any(|r| r.contains(p)) {
            if self.disabled {
                mouse::Interaction::Idle
            } else {
                mouse::Interaction::Text
            }
        } else {
            mouse::Interaction::None
        }
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let (segs, _) = self.rects(layout);
        let st = tree.state.downcast_mut::<TimeState>();
        for (i, r) in segs.iter().enumerate() {
            st.focus[i].report(operation, None, *r, !self.disabled, true);
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
        let st = tree.state.downcast_ref::<TimeState>();
        let raw = layout.bounds();
        let b = crate::scale::snap_rect(raw);
        let shift = Vector::new(b.x - raw.x, b.y - raw.y);
        let p = self.cx.p();
        let inner = self.cx.on(self.cx.recessed());
        let alpha = if self.disabled { 0.5 } else { 1.0 };
        let n = self.n();
        let (segs, _) = self.rects(layout);
        let kids: Vec<_> = layout.children().collect();
        let focused = st.focused();
        let caret_on = focused.is_some()
            && !st.selected
            && (Instant::now()
                .saturating_duration_since(st.caret_from)
                .as_millis()
                / CARET_BLINK.as_millis())
            .is_multiple_of(2);
        with_opacity(alpha, || {
            renderer.with_translation(shift, |renderer| {
                paint(
                    renderer,
                    raw,
                    &frame_style(&self.cx, Look::Plain, self.error, 0.0),
                );
                for i in 0..n {
                    let t = st.fades[i].t;
                    if t > 0.0 {
                        renderer.fill_quad(
                            Quad {
                                bounds: segs[i],
                                border: Border {
                                    radius: 4.0.into(),
                                    ..Border::default()
                                },
                                ..Quad::default()
                            },
                            fade(lerp(c(inner.over), c(inner.tint(0.15)), t)),
                        );
                    }
                    let text = lerp(c(p.text), c(p.accent_bright), t);
                    let tb = kids[i].bounds();
                    if focused == Some(i) && st.selected {
                        renderer.fill_quad(
                            Quad {
                                bounds: Rectangle {
                                    x: tb.x,
                                    y: tb.y + (tb.height - 17.0) / 2.0,
                                    width: tb.width,
                                    height: 17.0,
                                },
                                ..Quad::default()
                            },
                            fade(rgba(p.accent, 0.35)),
                        );
                    }
                    self.kids[i].as_widget().draw(
                        &tree.children[i],
                        renderer,
                        theme,
                        &renderer::Style { text_color: text },
                        kids[i],
                        cursor - shift,
                        viewport,
                    );
                    if focused == Some(i) && caret_on {
                        renderer.fill_quad(
                            Quad {
                                bounds: Rectangle {
                                    x: tb.x + tb.width,
                                    y: tb.y + (tb.height - 17.0) / 2.0,
                                    width: 1.0,
                                    height: 17.0,
                                },
                                ..Quad::default()
                            },
                            fade(c(p.accent_bright)),
                        );
                    }
                }
                #[allow(clippy::needless_range_loop)]
                for k in n..self.kids.len() {
                    let chevron = k + 1 >= 2 * n;
                    let color = if chevron {
                        let which = k - (2 * n - 1);
                        if st.spin_hover == Some(which) {
                            c(p.text)
                        } else {
                            c(p.muted)
                        }
                    } else {
                        style.text_color
                    };
                    self.kids[k].as_widget().draw(
                        &tree.children[k],
                        renderer,
                        theme,
                        &renderer::Style { text_color: color },
                        kids[k],
                        cursor - shift,
                        viewport,
                    );
                }
            });
        });
        let _ = Color::TRANSPARENT;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32, s: u32) -> TimeValue {
        TimeValue::new(h, m, s)
    }

    #[test]
    fn steps_carry_and_wrap() {
        let spec = TimeSpec::default();
        assert_eq!(
            apply_delta(t(9, 59, 0), 60, spec),
            Some(t(10, 0, 0)),
            "59m + 1 carries"
        );
        assert_eq!(
            apply_delta(t(10, 0, 0), -1, spec),
            Some(t(9, 59, 59)),
            "borrow"
        );
        assert_eq!(
            apply_delta(t(23, 59, 59), 1, spec),
            Some(t(0, 0, 0)),
            "wraps"
        );
        assert_eq!(apply_delta(t(0, 0, 0), -3600, spec), Some(t(23, 0, 0)));
        let clamp = TimeSpec {
            wrap: false,
            ..spec
        };
        assert_eq!(
            apply_delta(t(23, 59, 59), 1, clamp),
            None,
            "refuses past the end"
        );
        assert_eq!(apply_delta(t(0, 0, 0), -1, clamp), None);
        let long = TimeSpec {
            max_hours: 100,
            ..spec
        };
        assert_eq!(apply_delta(t(23, 0, 0), 3600, long), Some(t(24, 0, 0)));
        assert_eq!(apply_delta(t(99, 0, 0), 3600, long), Some(t(0, 0, 0)));
    }

    #[test]
    fn typing_digits() {
        let spec = TimeSpec::default();
        let v = t(9, 41, 0);
        // "1" then "5" into hours: 1, then 15 and move on.
        let (v1, ty, next) = handle_digit(v, 0, 1, 0, spec);
        assert_eq!((v1, ty, next), (t(1, 41, 0), 1, false));
        let (v2, ty, next) = handle_digit(v1, 0, 5, ty, spec);
        assert_eq!((v2, ty, next), (t(15, 41, 0), 0, true));
        // "3" can't be a tens digit of hours (30 > 23): moves on at once.
        assert_eq!(handle_digit(v, 0, 3, 0, spec), (t(3, 41, 0), 0, true));
        // Minutes: "7" (70 > 59) moves on; "2","9" → 29.
        assert_eq!(handle_digit(v, 1, 7, 0, spec), (t(9, 7, 0), 0, true));
        // A second digit past the range clamps (2, 9 → 29 is fine; 5,9 → 59).
        assert_eq!(
            handle_digit(t(2, 0, 0), 0, 9, 1, spec),
            (t(23, 0, 0), 0, true),
            "29 h clamps to 23"
        );
        let long = TimeSpec {
            max_hours: 100,
            ..spec
        };
        assert_eq!(
            handle_digit(v, 0, 9, 0, long),
            (t(9, 41, 0), 1, false),
            "99 h: 9 may be a tens digit"
        );
        assert_eq!(t(9, 41, 0).hms(), "09:41:00");
    }
}
