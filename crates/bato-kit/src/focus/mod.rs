//! Focus (INF-2): keyboard focus for the kit's own widgets, Chromium's Tab
//! order, `:focus-visible` rings, focus scopes (the dialog trap with restore),
//! programmatic focus and roving arrow-key groups.
//!
//! iced 0.14 only focuses `text_input` / `text_editor`. The kit's controls
//! (every [`Surface`](crate::surface::Surface) with `on_press`, the switch, the
//! checkbox box, segments, swatches, sidebar rows) report themselves through
//! iced's own [`Focusable`] operation, so they share one focus with text
//! inputs and with iced's `operation::focus(id)` tasks.
//!
//! | Piece | What |
//! |---|---|
//! | [`focus_root`] | Wraps the window ([`crate::frame::Frame::view`] does it): Tab / Shift+Tab, input modality, focus requests, `BATO_SELFTEST=focus` |
//! | [`focus_scope`] | Confines Tab to its content while it is the topmost trap, focuses its first control on mount, gives focus back to the opener when it goes away (Modal) |
//! | [`autofocus`] | Focuses its first control on mount (Combobox search, palette input) |
//! | [`roving`] | Arrow keys move focus (and the selection) within a group |
//! | [`focus`], [`blur`] | Tasks: focus a widget by [`Id`], drop focus |
//! | [`Ring`] | The ring styles: `focusRing` (2 px accentBright, 2 px offset in the page bg), plain `ring-2`, offset on a card |
//! | [`focus_visible`], [`revision`], [`FocusedBounds`] | Modality, a counter of focus moves, the focused widget's bounds (scroll-into-view) |
//!
//! **`:focus-visible`** follows Chromium: a key press without Ctrl/Alt/Super
//! (and not a lone modifier) switches to keyboard modality, a pointer press
//! switches back. Rings show only on the focused widget while the modality is
//! keyboard: never after a click or a tap, but a key press after a click shows
//! the ring on the clicked control, and focus moved by a program (a dialog's
//! first control, a restore) shows it only if the last input was a key.
//!
//! **Tab order** is the widget tree order (= the DOM order the kit mirrors).
//! Disabled controls are skipped; `tab_stop(false)` widgets (roving group
//! members, `tabindex=-1`) can be focused by a click or a program but are
//! skipped by Tab. With nothing focused, Tab starts after the last pressed
//! [`Surface`](crate::surface::Surface) (Chromium's sequential focus
//! navigation starting point), else at the first control. Tab wraps at the
//! ends (Electron has no browser UI to tab into).
//!
//! ```ignore
//! button(&cx).label("Save").id("save").on_press(Msg::Save)          // focusable, focusRing
//! focus_scope(dialog_panel)                                         // trap + autofocus + restore
//! segmented(&cx, v, opts, Msg::Pick).roving()                       // arrows move the selection
//! return bato_kit::focus::focus("name");                            // Task from update()
//! ```

mod focus_out;
mod root;
mod roving;
mod scope;

pub use focus_out::{FocusOut, on_focus_out};
pub use root::{FocusRoot, focus_root};
pub use roving::{Axis, Roving, roving};
pub use scope::{FocusScope, autofocus, focus_scope};

use std::any::Any;
use std::cell::{Cell, RefCell};

use bato_theme::Rgb;
use iced::advanced::renderer::{Quad, Renderer as _};
use iced::advanced::widget::operation::{Focusable, Operation, Outcome};
use iced::advanced::widget::{self, Id};
use iced::keyboard::{self, Key, key::Named};
use iced::{Border, Color, Event, Rectangle, Task, mouse, touch};

use crate::anim::fade;
use crate::ctx::Ctx;
use crate::style::c;

// ── Global (per UI thread) state ────────────────────────────────────────

thread_local! {
    /// The input modality: true after a key press, false after a pointer press.
    static KEYBOARD: Cell<bool> = const { Cell::new(false) };
    /// Bumped on every keyboard / programmatic focus move.
    static REVISION: Cell<u64> = const { Cell::new(0) };
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(1) };
    /// The Tab starting point: the innermost widget the last press landed on.
    static ANCHOR: Cell<Option<u64>> = const { Cell::new(None) };
    /// A press is being dispatched and no widget has claimed the anchor yet.
    static PRESS_OPEN: Cell<bool> = const { Cell::new(false) };
    static QUEUE: RefCell<Vec<Request>> = const { RefCell::new(Vec::new()) };
}

/// Keyboard modality: rings are visible on the focused widget.
pub fn focus_visible() -> bool {
    KEYBOARD.get()
}

/// Set the modality (the root does this; tests and self-tests may too).
/// Returns whether it changed.
pub fn set_keyboard_modality(on: bool) -> bool {
    KEYBOARD.replace(on) != on
}

/// A counter bumped whenever focus moves by keyboard or by a program (not by
/// a click). A scroller that sees it change scrolls the focused widget into
/// view ([`FocusedBounds`]); Chromium does that for `focus()` and Tab with
/// "center if needed", instantly.
pub fn revision() -> u64 {
    REVISION.get()
}

fn bump() {
    REVISION.set(REVISION.get().wrapping_add(1));
}

/// A fresh widget identity (the focus states of kit widgets carry one).
pub fn next_token() -> u64 {
    let t = NEXT_TOKEN.get();
    NEXT_TOKEN.set(t + 1);
    t
}

/// A widget that a press landed on: the first (innermost) caller during a
/// press becomes the Tab starting point.
pub fn note_press(token: u64) {
    if PRESS_OPEN.get() {
        PRESS_OPEN.set(false);
        ANCHOR.set(Some(token));
    }
}

/// Make `token` the Tab starting point (a focused widget that went away).
pub fn set_anchor(token: u64) {
    ANCHOR.set(Some(token));
}

fn begin_press() {
    ANCHOR.set(None);
    PRESS_OPEN.set(true);
}

fn end_press() {
    PRESS_OPEN.set(false);
}

/// For widgets wrapping an iced `text_input` (which keeps its own focus):
/// call after the input's `update` with whether it is focused now. A field
/// that became focused other than by a press scrolls into view.
pub fn reveal_field<M>(
    was: &mut bool,
    now: bool,
    event: &Event,
    bounds: Rectangle,
    shell: &mut iced::advanced::Shell<'_, M>,
) {
    if now && !*was && !is_pointer_press(event) {
        crate::scroller::focus_into_view(bounds);
        shell.request_redraw();
    }
    *was = now;
}

/// Who to focus.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A kit widget's [`FocusState::token`].
    Token(u64),
    /// A widget [`Id`].
    Id(Id),
}

#[derive(Debug, Clone)]
pub(crate) enum Request {
    /// Focus the first tab stop (or `initial`) inside scope `scope`,
    /// remembering what had focus.
    Autofocus {
        scope: u64,
        initial: Option<Id>,
        /// Then select this cursor range of `initial` (a text input).
        select: Option<(usize, usize)>,
    },
    /// Give focus back.
    Restore(Target),
    /// Focus a target (self-test, programmatic).
    Focus(Target),
}

pub(crate) fn push(r: Request) {
    // `try_with`: scope states are dropped during thread teardown too.
    let _ = QUEUE.try_with(|q| {
        let mut q = q.borrow_mut();
        if q.len() < 64 {
            q.push(r);
        }
    });
}

fn take_requests() -> Vec<Request> {
    QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// Focus `target` once the root sees the next event (any thread-local
/// caller; a widget, a debug hook). From `update()` prefer [`focus`].
pub fn request(target: Target) {
    push(Request::Focus(target));
}

// ── Rings ───────────────────────────────────────────────────────────────

/// A `:focus-visible` ring: `ring-<width> ring-<color>` plus an optional
/// `ring-offset-<offset> ring-offset-<offset_color>` (box shadows, outside
/// the box, no layout).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ring {
    pub width: f32,
    pub color: Color,
    pub offset: f32,
    pub offset_color: Color,
}

impl Ring {
    /// `focusRing` (Button, Switch, Checkbox, Radio, SegmentedControl):
    /// 2 px accentBright with a 2 px offset in the **page** background, even
    /// on cards and dialogs (React's `ring-offset-app-bg`, spec §9.4).
    pub fn focus(cx: &Ctx) -> Self {
        Self::offset_on(cx, cx.p().bg)
    }
    /// `ring-2 ring-app-accentBright`, no offset (sidebar rows, Tabs, toast
    /// dismiss).
    pub fn plain(cx: &Ctx) -> Self {
        Ring {
            width: 2.0,
            color: c(cx.p().accent_bright),
            offset: 0.0,
            offset_color: Color::TRANSPARENT,
        }
    }
    /// 2 px accentBright with a 2 px offset in `over` (the accent swatches:
    /// `ring-offset-app-card`).
    pub fn offset_on(cx: &Ctx, over: Rgb) -> Self {
        Ring {
            width: 2.0,
            color: c(cx.p().accent_bright),
            offset: 2.0,
            offset_color: c(over),
        }
    }
    /// As [`crate::surface::BoxStyle::rings`]: cumulative spreads, innermost
    /// first.
    pub fn layers(&self) -> Vec<(f32, Color)> {
        if self.offset > 0.0 {
            vec![
                (self.offset, self.offset_color),
                (self.offset + self.width, self.color),
            ]
        } else {
            vec![(self.width, self.color)]
        }
    }
}

/// Paint `ring` around `b` (a box with corner radius `radius`), for widgets
/// that draw themselves instead of through a `Surface`.
pub fn paint_ring(renderer: &mut iced::Renderer, b: Rectangle, radius: f32, ring: &Ring) {
    paint_layers(renderer, b, radius, &ring.layers());
}

/// Paint `0 0 0 Npx` shadow layers (cumulative, innermost first) as rings
/// **outside** the box only: CSS never paints a box shadow under the
/// element's own border box, which matters for transparent boxes (a ghost
/// button, an inactive segment or nav row).
pub fn paint_layers(
    renderer: &mut iced::Renderer,
    b: Rectangle,
    radius: f32,
    layers: &[(f32, Color)],
) {
    let mut prev = 0.0;
    for (w, col) in layers {
        renderer.fill_quad(
            Quad {
                bounds: b.expand(*w),
                border: Border {
                    radius: (radius + w).into(),
                    width: w - prev,
                    color: fade(*col),
                },
                ..Quad::default()
            },
            Color::TRANSPARENT,
        );
        prev = *w;
    }
}

// ── Per-widget state ────────────────────────────────────────────────────

/// The focus part of a kit widget's state: iced's [`Focusable`], an identity
/// for restore / anchors, and the armed Space key.
#[derive(Debug)]
pub struct FocusState {
    focused: bool,
    token: u64,
    space: bool,
    /// Focused by a key or a program: scroll into view on the next event.
    reveal: bool,
}

impl Default for FocusState {
    fn default() -> Self {
        Self {
            focused: false,
            token: next_token(),
            space: false,
            reveal: false,
        }
    }
}

impl Focusable for FocusState {
    fn is_focused(&self) -> bool {
        self.focused
    }
    /// Operations (Tab, scopes, `focus(id)`) land here; a click does not.
    fn focus(&mut self) {
        self.focused = true;
        self.reveal = true;
    }
    fn unfocus(&mut self) {
        self.focused = false;
        self.space = false;
    }
}

/// Emitted (through [`Operation::custom`]) right before a widget's
/// `focusable` call: who it is and whether Tab stops on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub token: u64,
    pub tab_stop: bool,
}

/// Emitted by every kit box: a landmark for the Tab starting point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor(pub u64);

/// No Ctrl / Alt / Super: Chromium ignores those for activation and modality.
pub fn plain(m: keyboard::Modifiers) -> bool {
    !(m.control() || m.alt() || m.logo())
}

fn is_modifier(key: &Key) -> bool {
    matches!(
        key,
        Key::Named(
            Named::Shift
                | Named::Control
                | Named::Alt
                | Named::AltGraph
                | Named::Super
                | Named::Meta
                | Named::Hyper
                | Named::CapsLock
                | Named::NumLock
                | Named::Fn
                | Named::FnLock
        )
    )
}

/// Whether a pointer press may move focus (Chromium focuses on left and
/// right mousedown and on a touch press; a middle press is BatoScroll's).
pub fn is_pointer_press(event: &Event) -> bool {
    matches!(
        event,
        Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Left | mouse::Button::Right
        )) | Event::Touch(touch::Event::FingerPressed { .. })
    )
}

impl FocusState {
    pub fn is_focused(&self) -> bool {
        self.focused
    }
    pub fn token(&self) -> u64 {
        self.token
    }
    /// The ring shows: focused, and the last input was a key.
    pub fn ring_visible(&self) -> bool {
        self.focused && focus_visible()
    }

    /// `operate`: the anchor landmark, then (when focusable) [`Meta`] and
    /// iced's `focusable`.
    pub fn report(
        &mut self,
        operation: &mut dyn Operation,
        id: Option<&Id>,
        bounds: Rectangle,
        focusable: bool,
        tab_stop: bool,
    ) {
        operation.custom(None, bounds, &mut Anchor(self.token));
        if focusable {
            operation.custom(
                id,
                bounds,
                &mut Meta {
                    token: self.token,
                    tab_stop,
                },
            );
            operation.focusable(id, bounds, self);
        }
    }

    /// A pointer press: focus when it lands inside a focusable widget, blur
    /// when it lands anywhere else (Chromium moves focus to the body). Notes
    /// the Tab starting point. Returns whether the focus changed.
    pub fn pointer(&mut self, event: &Event, inside: bool, focusable: bool) -> bool {
        if !is_pointer_press(event) {
            return false;
        }
        if inside {
            note_press(self.token);
        }
        let want = inside && focusable;
        if want != self.focused {
            if want {
                // Chromium does not scroll a clicked control into view.
                self.focused = true;
            } else {
                self.unfocus();
            }
            true
        } else {
            false
        }
    }

    /// Call from `update` with the widget's layout bounds: after a keyboard
    /// or programmatic focus, scrolls every scroller around it so it shows
    /// (Chromium: instant, `nearest`).
    pub fn reveal<M>(&mut self, bounds: Rectangle, shell: &mut iced::advanced::Shell<'_, M>) {
        if std::mem::take(&mut self.reveal) && self.focused {
            crate::scroller::focus_into_view(bounds);
            shell.request_redraw();
        }
    }

    /// A focused widget that can no longer take focus (disabled) drops it;
    /// it stays the Tab starting point. Returns whether it changed.
    pub fn sync(&mut self, focusable: bool) -> bool {
        if self.focused && !focusable {
            self.unfocus();
            set_anchor(self.token);
            true
        } else {
            false
        }
    }

    /// Chromium's button keys: Enter clicks on press, Space on release (the
    /// press is consumed so the page doesn't scroll). Returns `true` when the
    /// widget should activate; captures the keys it uses.
    pub fn activation<M>(
        &mut self,
        event: &Event,
        shell: &mut iced::advanced::Shell<'_, M>,
    ) -> bool {
        if let Event::Keyboard(keyboard::Event::KeyReleased {
            key: Key::Named(Named::Space),
            ..
        }) = event
            && self.space
        {
            self.space = false;
            if self.focused && !shell.is_event_captured() {
                shell.capture_event();
                return true;
            }
            return false;
        }
        if !self.focused || shell.is_event_captured() {
            return false;
        }
        match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Named(Named::Enter),
                modifiers,
                ..
            }) if plain(*modifiers) => {
                shell.capture_event();
                true
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Named(Named::Space),
                modifiers,
                ..
            }) if plain(*modifiers) => {
                self.space = true;
                shell.capture_event();
                false
            }
            _ => false,
        }
    }
}

// ── Operations ──────────────────────────────────────────────────────────

/// One focusable widget, in tree (document) order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    /// Position among all operation callbacks (shared with anchors and scopes).
    pub order: usize,
    pub token: Option<u64>,
    pub id: Option<Id>,
    pub bounds: Rectangle,
    pub focused: bool,
    pub tab_stop: bool,
    /// Keys of the scopes it is inside, outermost first.
    pub scopes: Vec<u64>,
}

impl Entry {
    fn target(&self) -> Option<Target> {
        self.token
            .map(Target::Token)
            .or_else(|| self.id.clone().map(Target::Id))
    }
    fn matches(&self, t: &Target) -> bool {
        match t {
            Target::Token(k) => self.token == Some(*k),
            Target::Id(id) => self.id.as_ref() == Some(id),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScopeInfo {
    pub key: u64,
    pub trap: bool,
}

/// The scope markers (see [`scope`]).
pub(crate) struct ScopeEnd;

/// Pass 1: list every focusable, the scopes and the anchor's position.
#[derive(Default)]
pub(crate) struct Collect {
    order: usize,
    meta: Option<Meta>,
    open: Vec<u64>,
    want_anchor: Option<u64>,
    pub entries: Vec<Entry>,
    pub scopes: Vec<ScopeInfo>,
    /// The anchor's position and the scopes around it.
    pub anchor: Option<(usize, Vec<u64>)>,
}

impl Collect {
    pub fn new(anchor: Option<u64>) -> Self {
        Self {
            want_anchor: anchor,
            ..Self::default()
        }
    }
}

impl Operation for Collect {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
    fn container(&mut self, _id: Option<&Id>, _bounds: Rectangle) {
        self.order += 1;
    }
    fn custom(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Any) {
        self.order += 1;
        if let Some(m) = state.downcast_ref::<Meta>() {
            self.meta = Some(*m);
        } else if let Some(a) = state.downcast_ref::<Anchor>() {
            if self.want_anchor == Some(a.0) {
                self.anchor = Some((self.order, self.open.clone()));
            }
        } else if let Some(s) = state.downcast_ref::<scope::ScopeState>() {
            self.scopes.push(ScopeInfo {
                key: s.key,
                trap: s.trap,
            });
            self.open.push(s.key);
        } else if state.is::<ScopeEnd>() {
            self.open.pop();
        }
    }
    fn focusable(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn Focusable) {
        self.order += 1;
        let meta = self.meta.take();
        self.entries.push(Entry {
            order: self.order,
            token: meta.map(|m| m.token),
            id: id.cloned(),
            bounds,
            focused: state.is_focused(),
            tab_stop: meta.is_none_or(|m| m.tab_stop),
            scopes: self.open.clone(),
        });
    }
}

/// Pass 2: focus the `target`-th focusable (or none), unfocus the rest, and
/// optionally hand a scope its opener.
pub(crate) struct Apply {
    index: usize,
    target: Option<usize>,
    opener: Option<(u64, Option<Target>)>,
}

impl Apply {
    pub fn new(target: Option<usize>) -> Self {
        Self {
            index: 0,
            target,
            opener: None,
        }
    }
    pub fn with_opener(mut self, scope: u64, opener: Option<Target>) -> Self {
        self.opener = Some((scope, opener));
        self
    }
}

impl Operation for Apply {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
    fn custom(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Any) {
        if let Some((key, opener)) = &self.opener
            && let Some(s) = state.downcast_mut::<scope::ScopeState>()
            && s.key == *key
        {
            s.opener = opener.clone();
        }
    }
    fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if Some(self.index) == self.target {
            state.focus();
        } else if state.is_focused() {
            state.unfocus();
        }
        self.index += 1;
    }
}

/// Where Tab (`back` = Shift+Tab) goes: an index into `entries`.
pub(crate) fn plan(
    entries: &[Entry],
    scopes: &[ScopeInfo],
    anchor: Option<&(usize, Vec<u64>)>,
    back: bool,
) -> Option<usize> {
    // The topmost trap is the last one in document order (the top layer).
    let trap = scopes.iter().rev().find(|s| s.trap).map(|s| s.key);
    let inside = |e: &Entry| trap.is_none_or(|k| e.scopes.contains(&k));
    let cands: Vec<usize> = (0..entries.len())
        .filter(|&i| entries[i].tab_stop && inside(&entries[i]))
        .collect();
    let (first, last) = (*cands.first()?, *cands.last()?);
    let current = entries.iter().rposition(|e| e.focused);
    let from = match current {
        // Focus escaped the trap: pull it back in at the near end.
        Some(c) if !inside(&entries[c]) => return Some(if back { last } else { first }),
        Some(c) => Some(entries[c].order),
        // An anchor outside the trap is no starting point.
        None => anchor
            .filter(|(_, sc)| trap.is_none_or(|k| sc.contains(&k)))
            .map(|(o, _)| *o),
    };
    let Some(from) = from else {
        return Some(if back { last } else { first });
    };
    if back {
        Some(
            cands
                .iter()
                .rev()
                .copied()
                .find(|&i| entries[i].order < from)
                .unwrap_or(last),
        )
    } else {
        Some(
            cands
                .iter()
                .copied()
                .find(|&i| entries[i].order > from)
                .unwrap_or(first),
        )
    }
}

/// The bounds of the focused widget, in the coordinates its `operate` sees
/// (layout coordinates: a scroller's content is not translated). Run it on a
/// subtree from a widget's `update`:
///
/// ```ignore
/// if focus::revision() != state.seen {
///     state.seen = focus::revision();
///     let mut op = focus::FocusedBounds::default();
///     self.content.as_widget_mut().operate(&mut tree.children[0], content_layout, renderer, &mut op);
///     if let Some(r) = op.0 { /* scroll r into view (center if needed) */ }
/// }
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct FocusedBounds(pub Option<Rectangle>);

impl Operation for FocusedBounds {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
    fn focusable(&mut self, _id: Option<&Id>, bounds: Rectangle, state: &mut dyn Focusable) {
        if state.is_focused() {
            self.0 = Some(bounds);
        }
    }
}

struct FocusId(Id);

impl Operation for FocusId {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&self.0) {
            state.focus();
            bump();
        } else if state.is_focused() {
            state.unfocus();
        }
    }
    fn finish(&self) -> Outcome<()> {
        Outcome::None
    }
}

struct BlurAll;

impl Operation for BlurAll {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
    fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        state.unfocus();
    }
}

/// Focus the widget with this [`Id`] (any focusable, tab stop or not), like
/// `element.focus()`: the ring shows only if the last input was a key, and
/// scrollers bring it into view ([`revision`]).
pub fn focus<M: Send + 'static>(id: impl Into<Id>) -> Task<M> {
    widget::operate(FocusId(id.into())).discard()
}

/// React's rename preselection (`selectStem`): the stem before the last
/// `.` when that dot is not the first character (`notes.txt` → `notes`,
/// `archive.tar.gz` → `archive.tar`), else the whole name (`.bashrc`,
/// `Makefile`). A byte range of `name`.
pub fn stem_range(name: &str) -> std::ops::Range<usize> {
    match name.rfind('.') {
        Some(dot) if dot > 0 => 0..dot,
        _ => 0..name.len(),
    }
}

/// A byte range of `value` as iced `text_input` cursor positions (it
/// counts graphemes). Ends inside a character round down.
pub fn grapheme_range(value: &str, bytes: std::ops::Range<usize>) -> (usize, usize) {
    use unicode_segmentation::UnicodeSegmentation;
    let at = |b: usize| {
        let mut b = b.min(value.len());
        while !value.is_char_boundary(b) {
            b -= 1;
        }
        value[..b].graphemes(true).count()
    };
    let (a, b) = (at(bytes.start), at(bytes.end));
    (a.min(b), a.max(b))
}

/// Focus the text input `id` (whose text is `value`) and select the byte
/// range `bytes` of it: `select_range(ID, &name, stem_range(&name))`. For a
/// field that is already on screen; a dialog that opens with a selection
/// uses [`crate::modal::Modal::initial_select`] instead, which runs after
/// the dialog's own autofocus.
pub fn select_range<M: Send + 'static>(
    id: impl Into<Id>,
    value: &str,
    bytes: std::ops::Range<usize>,
) -> Task<M> {
    let (start, end) = grapheme_range(value, bytes);
    widget::operate(SelectRange {
        id: id.into(),
        start,
        end,
        focus: true,
    })
    .discard()
}

/// Focus (optionally) and select a cursor range of one text input.
pub(crate) struct SelectRange {
    pub id: Id,
    pub start: usize,
    pub end: usize,
    pub focus: bool,
}

impl Operation for SelectRange {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if !self.focus {
            return;
        }
        if id == Some(&self.id) {
            state.focus();
            bump();
        } else if state.is_focused() {
            state.unfocus();
        }
    }
    fn text_input(
        &mut self,
        id: Option<&Id>,
        _bounds: Rectangle,
        state: &mut dyn widget::operation::TextInput,
    ) {
        // iced's text_input reports `text_input` before `focusable`, and
        // focusing moves the cursor to the end: the focusing pass selects in
        // a second pass ([`Operation::finish`]).
        if !self.focus && id == Some(&self.id) {
            state.select_range(self.start, self.end);
        }
    }
    fn finish(&self) -> Outcome<()> {
        if self.focus {
            Outcome::Chain(Box::new(SelectRange {
                id: self.id.clone(),
                start: self.start,
                end: self.end,
                focus: false,
            }))
        } else {
            Outcome::None
        }
    }
}

/// Drop focus (`element.blur()`).
pub fn blur<M: Send + 'static>() -> Task<M> {
    widget::operate(BlurAll).discard()
}

#[cfg(test)]
mod tests;
