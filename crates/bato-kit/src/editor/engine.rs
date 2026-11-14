//! The document model: cosmic-text's `SyntaxEditor` (CodeMirror keys, our
//! own undo history with CodeMirror's grouping) or `ViEditor` (vim mode),
//! over one shared `Arc<Buffer>` that the widget draws through `fill_raw`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::advanced::graphics::text::cosmic_text as ct;
use iced::advanced::graphics::text::cosmic_text::Edit as _;

use bato_theme::Palette;

use super::syntax::{self, Class, Language, THEMES};
use super::text::{self, INDENT_UNIT};

/// CodeMirror's text: `monospace` (cosmic-text resolves it to Noto Sans
/// Mono on Linux, as fontconfig does for Chromium) at Chromium's default
/// 13 px, `.cm-scroller { line-height: 1.4 }`.
pub const FONT_SIZE: f32 = 13.0;
pub const LINE_HEIGHT: f32 = FONT_SIZE * 1.4;

/// `history({ newGroupDelay: 500 })`.
const GROUP_DELAY: Duration = Duration::from_millis(500);

pub(crate) enum Engine {
    Plain(Box<ct::SyntaxEditor<'static, 'static>>),
    Vi(Box<ct::ViEditor<'static, 'static>>),
}

macro_rules! with_ed {
    ($eng:expr, |$e:ident| $body:expr) => {
        match $eng {
            Engine::Plain($e) => {
                let $e = &mut **$e;
                $body
            }
            Engine::Vi($e) => {
                let $e = &mut **$e;
                $body
            }
        }
    };
}

macro_rules! with_ed_ref {
    ($eng:expr, |$e:ident| $body:expr) => {
        match $eng {
            Engine::Plain($e) => {
                let $e = &**$e;
                $body
            }
            Engine::Vi($e) => {
                let $e = &**$e;
                $body
            }
        }
    };
}

/// Cursor motions (CodeMirror's `cursor*` / `select*` commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    CharLeft,
    CharRight,
    GroupLeft,
    GroupRight,
    LineUp,
    LineDown,
    /// Home: to the indentation, then to the line start.
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
    PageUp,
    PageDown,
}

/// The editing commands of the default keymap the widget binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    Move(Move, bool),
    SelectAll,
    SelectLine,
    /// Typed text (`closeBrackets` applies to a single character).
    Type(String),
    Paste(String),
    Newline,
    BlankLine,
    Backspace,
    Delete,
    DeleteGroupBack,
    DeleteGroupForward,
    IndentMore,
    IndentLess,
    ToggleComment,
    MoveLine(bool),
    CopyLine(bool),
    DeleteLine,
    MatchingBracket(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Input,
    Delete,
    Other,
}

struct Entry {
    change: ct::Change,
    before: (ct::Cursor, ct::Selection),
    after: (ct::Cursor, ct::Selection),
    kind: Kind,
    at: Instant,
}

#[derive(Default)]
struct History {
    done: Vec<Entry>,
    undone: Vec<Entry>,
    /// The selection moved since the last change: the next one starts a
    /// new group.
    broken: bool,
}

/// What a key did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub changed: bool,
    pub moved: bool,
}

/// The editor's document and state. The app keeps it (like iced's
/// `text_editor::Content`); the widget edits it in place.
pub struct Doc {
    pub(crate) engine: Option<Engine>,
    pending: String,
    pub lang: Language,
    pub vim: bool,
    palette: Option<Palette>,
    theme_i: usize,
    history: History,
    /// Bumped on every text change.
    pub version: u64,
    /// The vertical motion's goal column (px).
    goal: Option<i32>,
    /// The last line-wise copy (`lineWiseCopyCut`): pasting exactly this
    /// with an empty selection inserts it above the line.
    linewise: Option<String>,
    /// The widest line (px), after [`Doc::refresh`].
    pub width: f32,
    /// The keys of the vim command being typed (`vim.status`).
    keys: String,
}

impl Doc {
    pub fn new(text: &str) -> Self {
        Self {
            engine: None,
            pending: text.to_string(),
            lang: Language::Text,
            vim: false,
            palette: None,
            theme_i: 0,
            history: History::default(),
            version: 0,
            goal: None,
            linewise: None,
            width: 0.0,
            keys: String::new(),
        }
    }

    pub fn text(&self) -> String {
        match &self.engine {
            None => self.pending.clone(),
            Some(eng) => with_ed_ref!(eng, |e| e.with_buffer(buffer_text)),
        }
    }

    pub fn line_count(&self) -> usize {
        match &self.engine {
            None => self.pending.split('\n').count(),
            Some(eng) => with_ed_ref!(eng, |e| e.with_buffer(|b| b.lines.len())),
        }
    }

    /// Replace the whole text (an outside `value` change); the history
    /// starts over, like a new CodeMirror state.
    pub fn set_text(&mut self, fs: &mut ct::FontSystem, text: &str) {
        self.version += 1;
        self.history = History::default();
        let Some(eng) = &mut self.engine else {
            self.pending = text.to_string();
            return;
        };
        let attrs = default_attrs();
        with_ed!(eng, |e| {
            e.set_selection(ct::Selection::None);
            e.set_cursor(ct::Cursor::new(0, 0));
            e.with_buffer_mut(|b| {
                b.set_text(fs, text, &attrs, ct::Shaping::Advanced, None);
            });
        });
        // A new buffer text: re-highlight from the top.
        self.flip_theme();
    }

    /// Create the engine, or bring it in line with the props.
    pub fn configure(
        &mut self,
        fs: &mut ct::FontSystem,
        palette: &Palette,
        lang: Language,
        vim: bool,
    ) {
        if self.engine.is_none() {
            let mut buffer = ct::Buffer::new(fs, ct::Metrics::new(FONT_SIZE, LINE_HEIGHT));
            buffer.set_wrap(fs, ct::Wrap::None);
            buffer.set_size(fs, None, None);
            buffer.set_tab_width(fs, text::TAB_SIZE as u16);
            buffer.set_text(
                fs,
                &std::mem::take(&mut self.pending),
                &default_attrs(),
                ct::Shaping::Advanced,
                None,
            );
            let ed = ct::SyntaxEditor::new(Arc::new(buffer), syntax::system(), THEMES[0])
                .expect("bato theme");
            self.engine = Some(Engine::Plain(Box::new(ed)));
            self.theme_i = 0;
            self.lang = Language::Text;
            self.vim = false;
            self.apply_lang(lang);
        }
        if self.lang != lang {
            self.apply_lang(lang);
        }
        if self.vim != vim {
            self.switch_vim(vim);
        }
        if self.palette.as_ref() != Some(palette) {
            if self.palette.is_some() {
                self.flip_theme();
            }
            self.palette = Some(palette.clone());
        }
    }

    fn apply_lang(&mut self, lang: Language) {
        self.lang = lang;
        if let Some(eng) = &mut self.engine {
            match eng {
                Engine::Plain(e) => e.syntax_by_extension(lang.extension()),
                // `ViEditor` has no syntax setter: rebuild it around the
                // same buffer.
                Engine::Vi(_) => {
                    self.switch_vim(false);
                    if let Some(Engine::Plain(e)) = &mut self.engine {
                        e.syntax_by_extension(lang.extension());
                    }
                    self.switch_vim(true);
                }
            }
        }
    }

    /// Plain ↔ vi, keeping the buffer, cursor and selection. The vim
    /// history starts empty (and the plain one is kept for later).
    fn switch_vim(&mut self, vim: bool) {
        self.vim = vim;
        let Some(eng) = self.engine.take() else {
            return;
        };
        let (buffer, cursor, selection) = with_ed!(&mut { eng }, |e| {
            let buffer = match e.buffer_ref() {
                ct::BufferRef::Arc(a) => a.clone(),
                _ => unreachable!("the editor buffer is an Arc"),
            };
            (buffer, e.cursor(), e.selection())
        });
        // The old engine is gone (dropped above), so the Arc is unique
        // again and edits don't copy the buffer.
        let mut se = ct::SyntaxEditor::new(buffer, syntax::system(), THEMES[self.theme_i])
            .expect("bato theme");
        se.syntax_by_extension(self.lang.extension());
        se.set_cursor(cursor);
        se.set_selection(selection);
        self.engine = Some(if vim {
            let mut vi = ct::ViEditor::new(se);
            vi.set_passthrough(false);
            // Normal mode: the cursor sits on a character.
            vi.set_selection(ct::Selection::None);
            Engine::Vi(Box::new(vi))
        } else {
            Engine::Plain(Box::new(se))
        });
        // Re-highlight in the new editor.
        self.flip_theme();
    }

    fn flip_theme(&mut self) {
        self.theme_i ^= 1;
        let name = THEMES[self.theme_i];
        if let Some(eng) = &mut self.engine {
            match eng {
                Engine::Plain(e) => {
                    e.update_theme(name);
                }
                Engine::Vi(e) => {
                    e.update_theme(name);
                }
            }
        }
    }

    /// Highlight, resolve the palette colours, shape; update the width.
    pub fn refresh(&mut self, fs: &mut ct::FontSystem) {
        let Some(palette) = self.palette.clone() else {
            return;
        };
        let Some(eng) = &mut self.engine else {
            return;
        };
        let width = with_ed!(eng, |e| {
            e.shape_as_needed(fs, false);
            let again = e.with_buffer_mut(|b| syntax::resolve(b, &palette));
            if again {
                e.with_buffer_mut(|b| b.shape_until_scroll(fs, false));
            }
            e.with_buffer(|b| b.layout_runs().fold(0.0f32, |w, r| w.max(r.line_w)))
        });
        self.width = width;
    }

    pub fn cursor(&self) -> ct::Cursor {
        self.engine.as_ref().map_or(ct::Cursor::new(0, 0), |eng| {
            with_ed_ref!(eng, |e| e.cursor())
        })
    }

    /// The selection as ordered bounds (empty: `None`).
    pub fn selection_bounds(&self) -> Option<(ct::Cursor, ct::Cursor)> {
        let eng = self.engine.as_ref()?;
        let (a, b) = with_ed_ref!(eng, |e| e.selection_bounds())?;
        // Vim's characterwise visual mode includes the character under
        // the cursor.
        if let Engine::Vi(v) = eng
            && v.parser().mode == ct::ViMode::Visual
        {
            let line = v.with_buffer(|buf| buf.lines[b.line].text().to_string());
            let end = text::char_step(&line, b.index, true).unwrap_or(b.index);
            return Some((a, cur(b.line, end)));
        }
        (a != b).then_some((a, b))
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection_bounds()?;
        self.engine
            .as_ref()
            .and_then(|eng| with_ed_ref!(eng, |e| e.copy_selection()))
    }

    pub fn vi_mode(&self) -> Option<ct::ViMode> {
        match &self.engine {
            Some(Engine::Vi(v)) => Some(v.parser().mode.clone()),
            _ => None,
        }
    }

    /// The pending vim command keys (`vim.status`: count, operator…).
    pub fn vi_pending(&self) -> String {
        self.keys.clone()
    }

    pub fn buffer(&self) -> Option<&Arc<ct::Buffer>> {
        let eng = self.engine.as_ref()?;
        with_ed_ref!(eng, |e| match e.buffer_ref() {
            ct::BufferRef::Arc(a) => Some(a),
            _ => None,
        })
    }

    pub fn lines(&self) -> Vec<String> {
        match &self.engine {
            None => self.pending.split('\n').map(str::to_string).collect(),
            Some(eng) => with_ed_ref!(eng, |e| e.with_buffer(|b| b
                .lines
                .iter()
                .map(|l| l.text().to_string())
                .collect())),
        }
    }

    // ── Pointer ─────────────────────────────────────────────────────────

    /// A press, double or triple press, drag or Shift+press at buffer
    /// coordinates.
    pub fn pointer(&mut self, fs: &mut ct::FontSystem, action: Pointer, x: f32, y: f32) {
        let Some(eng) = &mut self.engine else { return };
        let (xi, yi) = (x.round() as i32, y.round() as i32);
        self.goal = None;
        self.history.broken = true;
        with_ed!(eng, |e| {
            match action {
                Pointer::Click => {
                    e.action(fs, ct::Action::Click { x: xi, y: yi });
                }
                Pointer::Double => e.action(fs, ct::Action::DoubleClick { x: xi, y: yi }),
                Pointer::Triple => e.action(fs, ct::Action::TripleClick { x: xi, y: yi }),
                Pointer::Drag => e.action(fs, ct::Action::Drag { x: xi, y: yi }),
                Pointer::Extend => {
                    let old = e.cursor();
                    if matches!(e.selection(), ct::Selection::None) {
                        e.set_selection(ct::Selection::Normal(old));
                    }
                    if let Some(hit) = e.with_buffer(|b| b.hit(x, y)) {
                        e.set_cursor(hit);
                    }
                }
            }
            // A click without a drag leaves an empty `Normal` selection.
            if let Some((a, b)) = e.selection_bounds()
                && a == b
                && matches!(e.selection(), ct::Selection::Normal(_))
            {
                e.set_selection(ct::Selection::None);
            }
        });
    }

    // ── Keys (plain mode) ───────────────────────────────────────────────

    /// Run a command. `page` is the viewport height (PageUp/PageDown).
    pub fn run(&mut self, fs: &mut ct::FontSystem, cmd: &Cmd, page: f32) -> Outcome {
        let lang = self.lang;
        let Some(eng) = &mut self.engine else {
            return Outcome::default();
        };
        let kind = match cmd {
            Cmd::Move(..) | Cmd::SelectAll | Cmd::SelectLine | Cmd::MatchingBracket(_) => None,
            Cmd::Type(_) | Cmd::Newline => Some(Kind::Input),
            Cmd::Backspace | Cmd::Delete | Cmd::DeleteGroupBack | Cmd::DeleteGroupForward => {
                Some(Kind::Delete)
            }
            _ => Some(Kind::Other),
        };
        let Some(kind) = kind else {
            let moved = with_ed!(eng, |e| {
                let before = (e.cursor(), e.selection());
                navigate(e, fs, cmd, page, &mut self.goal);
                before != (e.cursor(), e.selection())
            });
            if moved {
                self.history.broken = true;
            }
            return Outcome {
                changed: false,
                moved,
            };
        };
        if !matches!(cmd, Cmd::Move(Move::LineUp | Move::LineDown, _)) {
            self.goal = None;
        }
        let linewise = self.linewise.clone();
        let (change, before, after) = with_ed!(eng, |e| {
            let before = (e.cursor(), e.selection());
            e.start_change();
            edit(e, fs, cmd, lang, linewise.as_deref());
            let change = e.finish_change();
            (change, before, (e.cursor(), e.selection()))
        });
        let Some(change) = change.filter(|c| !c.items.is_empty()) else {
            return Outcome {
                changed: false,
                moved: before != after,
            };
        };
        self.version += 1;
        if matches!(self.engine, Some(Engine::Plain(_))) {
            self.record(change, before, after, kind);
        }
        Outcome {
            changed: true,
            moved: true,
        }
    }

    fn record(
        &mut self,
        change: ct::Change,
        before: (ct::Cursor, ct::Selection),
        after: (ct::Cursor, ct::Selection),
        kind: Kind,
    ) {
        let now = Instant::now();
        let h = &mut self.history;
        h.undone.clear();
        if !h.broken
            && kind != Kind::Other
            && let Some(last) = h.done.last_mut()
            && last.kind == kind
            && now.duration_since(last.at) < GROUP_DELAY
        {
            last.change.items.extend(change.items);
            last.after = after;
            last.at = now;
            return;
        }
        h.broken = false;
        h.done.push(Entry {
            change,
            before,
            after,
            kind,
            at: now,
        });
    }

    /// Undo (`undo`); vim mode uses its own history.
    pub fn undo(&mut self) -> bool {
        match &mut self.engine {
            Some(Engine::Vi(v)) => {
                v.undo();
                self.version += 1;
                true
            }
            Some(Engine::Plain(e)) => {
                let Some(entry) = self.history.done.pop() else {
                    return false;
                };
                let mut rev = entry.change.clone();
                rev.reverse();
                e.apply_change(&rev);
                e.set_cursor(entry.before.0);
                e.set_selection(entry.before.1);
                self.history.undone.push(entry);
                self.history.broken = true;
                self.version += 1;
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match &mut self.engine {
            Some(Engine::Vi(v)) => {
                v.redo();
                self.version += 1;
                true
            }
            Some(Engine::Plain(e)) => {
                let Some(entry) = self.history.undone.pop() else {
                    return false;
                };
                e.apply_change(&entry.change);
                e.set_cursor(entry.after.0);
                e.set_selection(entry.after.1);
                self.history.done.push(entry);
                self.history.broken = true;
                self.version += 1;
                true
            }
            None => false,
        }
    }

    /// Copy (`Ctrl+C`): the selection, or the whole line when it is empty
    /// (`lineWiseCopyCut`). With `cut`, deletes it too.
    pub fn copy(&mut self, fs: &mut ct::FontSystem, cut: bool) -> Option<String> {
        if let Some(text) = self.selected_text() {
            self.linewise = None;
            if cut {
                self.run(fs, &Cmd::Delete, 0.0);
            }
            return Some(text);
        }
        let c = self.cursor();
        let line = self.lines().get(c.line)?.clone();
        let text = format!("{line}\n");
        self.linewise = Some(text.clone());
        if cut {
            self.run(fs, &Cmd::DeleteLine, 0.0);
        }
        Some(text)
    }

    // ── Vim ─────────────────────────────────────────────────────────────

    /// Feed one key to the vim engine. Returns whether the text changed.
    pub fn vi_key(&mut self, fs: &mut ct::FontSystem, action: ct::Action) -> Outcome {
        let Some(Engine::Vi(v)) = &mut self.engine else {
            return Outcome::default();
        };
        let before_text = v.with_buffer(buffer_text);
        let before = (v.cursor(), v.selection());
        v.action(fs, action);
        let parser = v.parser();
        let idle = parser.cmd == Default::default() && !matches!(parser.mode, ct::ViMode::Extra(_));
        match (&parser.mode, action) {
            (ct::ViMode::Normal | ct::ViMode::Visual | ct::ViMode::VisualLine, _) if idle => {
                self.keys.clear();
            }
            (ct::ViMode::Insert | ct::ViMode::Replace | ct::ViMode::Command { .. }, _) => {
                self.keys.clear();
            }
            (_, ct::Action::Insert(ch)) => self.keys.push(ch),
            _ => {}
        }
        let changed = v.with_buffer(buffer_text) != before_text;
        if changed {
            self.version += 1;
        }
        Outcome {
            changed,
            moved: before != (v.cursor(), v.selection()),
        }
    }

    /// Paste in vim insert mode (one undo step).
    pub fn vi_paste(&mut self, text: &str) -> bool {
        let Some(Engine::Vi(v)) = &mut self.engine else {
            return false;
        };
        v.start_change();
        v.insert_string(text, None);
        v.finish_change();
        self.version += 1;
        true
    }
}

/// A pointer action at buffer coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pointer {
    Click,
    Double,
    Triple,
    Drag,
    Extend,
}

pub fn default_attrs() -> ct::Attrs<'static> {
    ct::Attrs::new()
        .family(ct::Family::Monospace)
        .color(syntax::sentinel(Class::Default))
}

fn buffer_text(b: &ct::Buffer) -> String {
    let mut out = String::new();
    for (i, l) in b.lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(l.text());
    }
    out
}

fn line_of<E: ct::Edit<'static>>(e: &E, i: usize) -> String {
    e.with_buffer(|b| {
        b.lines
            .get(i)
            .map(|l| l.text().to_string())
            .unwrap_or_default()
    })
}

fn line_count<E: ct::Edit<'static>>(e: &E) -> usize {
    e.with_buffer(|b| b.lines.len())
}

fn cur(line: usize, index: usize) -> ct::Cursor {
    ct::Cursor::new(line, index)
}

/// Where a motion lands from `c` (no selection logic).
fn target<E: ct::Edit<'static>>(
    e: &mut E,
    fs: &mut ct::FontSystem,
    c: ct::Cursor,
    m: Move,
    page: f32,
    goal: &mut Option<i32>,
) -> ct::Cursor {
    let text = line_of(e, c.line);
    let n = line_count(e);
    match m {
        Move::CharLeft => match text::char_step(&text, c.index, false) {
            Some(i) => cur(c.line, i),
            None if c.line > 0 => cur(c.line - 1, line_of(e, c.line - 1).len()),
            None => c,
        },
        Move::CharRight => match text::char_step(&text, c.index, true) {
            Some(i) => cur(c.line, i),
            None if c.line + 1 < n => cur(c.line + 1, 0),
            None => c,
        },
        Move::GroupLeft => match text::group_end(&text, c.index, false) {
            Some(i) => cur(c.line, i),
            None if c.line > 0 => {
                let prev = line_of(e, c.line - 1);
                let i = text::group_end(&prev, prev.len(), false).unwrap_or(prev.len());
                cur(c.line - 1, i)
            }
            None => c,
        },
        Move::GroupRight => match text::group_end(&text, c.index, true) {
            Some(i) => cur(c.line, i),
            None if c.line + 1 < n => {
                let next = line_of(e, c.line + 1);
                cur(c.line + 1, text::group_end(&next, 0, true).unwrap_or(0))
            }
            None => c,
        },
        Move::LineUp | Move::LineDown | Move::PageUp | Move::PageDown => {
            let up = matches!(m, Move::LineUp | Move::PageUp);
            if up && c.line == 0 {
                return cur(0, 0);
            }
            if !up && c.line + 1 >= n {
                return cur(c.line, text.len());
            }
            let motion = match m {
                Move::LineUp => ct::Motion::Up,
                Move::LineDown => ct::Motion::Down,
                Move::PageUp => ct::Motion::Vertical(-(page.max(LINE_HEIGHT) as i32)),
                _ => ct::Motion::Vertical(page.max(LINE_HEIGHT) as i32),
            };
            let g = *goal;
            match e.with_buffer_mut(|b| b.cursor_motion(fs, c, g, motion)) {
                Some((nc, ng)) => {
                    *goal = ng;
                    nc
                }
                None => c,
            }
        }
        Move::LineStart => {
            let ind = text::indent_len(&text);
            if ind > 0 && c.index != ind {
                cur(c.line, ind)
            } else {
                cur(c.line, 0)
            }
        }
        Move::LineEnd => cur(c.line, text.len()),
        Move::DocStart => cur(0, 0),
        Move::DocEnd => cur(n - 1, line_of(e, n - 1).len()),
    }
}

fn navigate<E: ct::Edit<'static>>(
    e: &mut E,
    fs: &mut ct::FontSystem,
    cmd: &Cmd,
    page: f32,
    goal: &mut Option<i32>,
) {
    let c = e.cursor();
    let sel = e.selection_bounds().filter(|(a, b)| a != b);
    match cmd {
        Cmd::Move(m, extend) => {
            let vertical = matches!(
                m,
                Move::LineUp | Move::LineDown | Move::PageUp | Move::PageDown
            );
            if !vertical {
                *goal = None;
            }
            if *extend {
                if matches!(e.selection(), ct::Selection::None) {
                    e.set_selection(ct::Selection::Normal(c));
                }
                let t = target(e, fs, c, *m, page, goal);
                e.set_cursor(t);
                return;
            }
            // Char, group, line and page moves collapse a selection to
            // its end in that direction; Home/End move from the head.
            let collapses = !matches!(
                m,
                Move::LineStart | Move::LineEnd | Move::DocStart | Move::DocEnd
            );
            let t = match sel {
                Some((a, b)) if collapses => {
                    let forward = matches!(
                        m,
                        Move::CharRight | Move::GroupRight | Move::LineDown | Move::PageDown
                    );
                    *goal = None;
                    if forward { b } else { a }
                }
                _ => target(e, fs, c, *m, page, goal),
            };
            e.set_selection(ct::Selection::None);
            e.set_cursor(t);
        }
        Cmd::SelectAll => {
            let n = line_count(e);
            let end = cur(n - 1, line_of(e, n - 1).len());
            e.set_selection(ct::Selection::Normal(cur(0, 0)));
            e.set_cursor(end);
        }
        Cmd::SelectLine => {
            let (a, b) = sel.unwrap_or((c, c));
            let n = line_count(e);
            let end = if b.line + 1 < n {
                cur(b.line + 1, 0)
            } else {
                cur(b.line, line_of(e, b.line).len())
            };
            e.set_selection(ct::Selection::Normal(cur(a.line, 0)));
            e.set_cursor(end);
        }
        Cmd::MatchingBracket(extend) => {
            let lines: Vec<String> =
                e.with_buffer(|b| b.lines.iter().map(|l| l.text().to_string()).collect());
            let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
            let at_or_before = [
                Some(c.index),
                text::char_step(&lines[c.line], c.index, false),
            ];
            for at in at_or_before.into_iter().flatten() {
                if let Some((l, i)) = text::matching_bracket(&refs, c.line, at) {
                    if *extend {
                        if matches!(e.selection(), ct::Selection::None) {
                            e.set_selection(ct::Selection::Normal(c));
                        }
                    } else {
                        e.set_selection(ct::Selection::None);
                    }
                    e.set_cursor(cur(l, i));
                    break;
                }
            }
        }
        _ => {}
    }
}

/// Replace `[a, b)` with `s`; returns the end of the inserted text.
fn replace<E: ct::Edit<'static>>(e: &mut E, a: ct::Cursor, b: ct::Cursor, s: &str) -> ct::Cursor {
    if a != b {
        e.delete_range(a, b);
    }
    if s.is_empty() {
        a
    } else {
        e.insert_at(a, s, None)
    }
}

/// Delete the selection; returns where the cursor goes.
fn take_selection<E: ct::Edit<'static>>(e: &mut E) -> ct::Cursor {
    let c = e.cursor();
    let at = match e.selection_bounds() {
        Some((a, b)) if a != b => {
            e.delete_range(a, b);
            a
        }
        _ => c,
    };
    e.set_selection(ct::Selection::None);
    at
}

fn set(e: &mut impl ct::Edit<'static>, c: ct::Cursor) {
    e.set_selection(ct::Selection::None);
    e.set_cursor(c);
}

fn edit<E: ct::Edit<'static>>(
    e: &mut E,
    _fs: &mut ct::FontSystem,
    cmd: &Cmd,
    lang: Language,
    linewise: Option<&str>,
) {
    let c = e.cursor();
    let sel = e.selection_bounds().filter(|(a, b)| a != b);
    match cmd {
        Cmd::Type(s) => {
            let mut chars = s.chars();
            if let (Some(ch), None) = (chars.next(), chars.next())
                && sel.is_none()
            {
                let line = line_of(e, c.line);
                let prev = line[..c.index].chars().next_back();
                let next = line[c.index..].chars().next();
                if text::steps_over(ch, next) {
                    set(e, cur(c.line, c.index + ch.len_utf8()));
                    return;
                }
                if let Some(close) = text::auto_close(ch, prev, next) {
                    let at = e.insert_at(c, &format!("{ch}{close}"), None);
                    set(e, cur(at.line, at.index - close.len_utf8()));
                    return;
                }
                // `indentOnInput`: a closer typed on a blank line dedents.
                if matches!(ch, '}' | ']' | ')') && line[..c.index].trim().is_empty() && c.index > 0
                {
                    let col = text::column(&line, c.index);
                    let want = col.saturating_sub(INDENT_UNIT);
                    let ind = text::indent_string(want);
                    let end = replace(e, cur(c.line, 0), c, &format!("{ind}{ch}"));
                    set(e, end);
                    return;
                }
            }
            let at = take_selection(e);
            let end = e.insert_at(at, s, None);
            set(e, end);
        }
        Cmd::Paste(s) => {
            if sel.is_none() && linewise == Some(s.as_str()) {
                let end = e.insert_at(cur(c.line, 0), s, None);
                let _ = end;
                set(e, cur(c.line + 1, c.index));
                return;
            }
            let at = take_selection(e);
            let end = e.insert_at(at, s, None);
            set(e, end);
        }
        Cmd::Newline => {
            let at = take_selection(e);
            let line = line_of(e, at.line);
            let (col, split) = text::newline_indent(lang, &line, at.index);
            // `insertNewlineAndIndent`: a line that is blank up to the
            // cursor is cleared, and the spaces after the cursor go.
            let left = if line[..at.index].trim().is_empty() {
                0
            } else {
                at.index
            };
            let right = at.index
                + (line[at.index..].len() - line[at.index..].trim_start_matches([' ', '\t']).len());
            let ind = text::indent_string(col);
            let s = if split {
                let base = text::column(&line, text::indent_len(&line));
                format!("\n{ind}\n{}", text::indent_string(base))
            } else {
                format!("\n{ind}")
            };
            replace(e, cur(at.line, left), cur(at.line, right), &s);
            set(e, cur(at.line + 1, ind.len()));
        }
        Cmd::BlankLine => {
            let line = line_of(e, c.line);
            let ind = text::indent_string(text::column(&line, text::indent_len(&line)));
            e.insert_at(cur(c.line, line.len()), &format!("\n{ind}"), None);
            set(e, cur(c.line + 1, ind.len()));
        }
        Cmd::Backspace => {
            if sel.is_some() {
                let at = take_selection(e);
                set(e, at);
                return;
            }
            let line = line_of(e, c.line);
            if c.index == 0 {
                if c.line > 0 {
                    let prev = line_of(e, c.line - 1);
                    e.delete_range(cur(c.line - 1, prev.len()), c);
                    set(e, cur(c.line - 1, prev.len()));
                }
                return;
            }
            let prev = line[..c.index].chars().next_back();
            let next = line[c.index..].chars().next();
            if text::is_pair(prev, next) {
                let a = cur(c.line, c.index - prev.unwrap().len_utf8());
                e.delete_range(a, cur(c.line, c.index + next.unwrap().len_utf8()));
                set(e, a);
                return;
            }
            let to = text::backspace_indent(&line, c.index)
                .or_else(|| text::char_step(&line, c.index, false))
                .unwrap_or(0);
            e.delete_range(cur(c.line, to), c);
            set(e, cur(c.line, to));
        }
        Cmd::Delete => {
            if sel.is_some() {
                let at = take_selection(e);
                set(e, at);
                return;
            }
            let line = line_of(e, c.line);
            match text::char_step(&line, c.index, true) {
                Some(i) => e.delete_range(c, cur(c.line, i)),
                None if c.line + 1 < line_count(e) => e.delete_range(c, cur(c.line + 1, 0)),
                None => {}
            }
            set(e, c);
        }
        Cmd::DeleteGroupBack | Cmd::DeleteGroupForward => {
            if sel.is_some() {
                let at = take_selection(e);
                set(e, at);
                return;
            }
            let forward = matches!(cmd, Cmd::DeleteGroupForward);
            let line = line_of(e, c.line);
            let to = match text::group_end(&line, c.index, forward) {
                Some(i) => cur(c.line, i),
                None if forward && c.line + 1 < line_count(e) => cur(c.line + 1, 0),
                None if !forward && c.line > 0 => cur(c.line - 1, line_of(e, c.line - 1).len()),
                None => c,
            };
            let (a, b) = if forward { (c, to) } else { (to, c) };
            if a != b {
                e.delete_range(a, b);
            }
            set(e, a);
        }
        Cmd::IndentMore | Cmd::IndentLess => {
            let (a, b) = sel.unwrap_or((c, c));
            // A selection ending at a line start leaves that line alone.
            let last = if b.line > a.line && b.index == 0 {
                b.line - 1
            } else {
                b.line
            };
            let more = matches!(cmd, Cmd::IndentMore);
            let mut shift = |line: usize| -> isize {
                let text = line_of(e, line);
                let ind = text::indent_len(&text);
                let col = text::column(&text, ind);
                let want = if more {
                    col + INDENT_UNIT
                } else {
                    col.saturating_sub(INDENT_UNIT)
                };
                if want == col || (more && text.trim().is_empty() && sel.is_some()) {
                    return 0;
                }
                let new = text::indent_string(want);
                replace(e, cur(line, 0), cur(line, ind), &new);
                new.len() as isize - ind as isize
            };
            let mut deltas = Vec::new();
            for line in a.line..=last {
                deltas.push((line, shift(line)));
            }
            let adj = |p: ct::Cursor| {
                let d = deltas
                    .iter()
                    .find(|(l, _)| *l == p.line)
                    .map_or(0, |(_, d)| *d);
                cur(p.line, (p.index as isize + d).max(0) as usize)
            };
            match e.selection() {
                ct::Selection::None => e.set_cursor(adj(c)),
                ct::Selection::Normal(anchor) => {
                    e.set_selection(ct::Selection::Normal(adj(anchor)));
                    e.set_cursor(adj(c));
                }
                _ => {
                    e.set_selection(ct::Selection::Normal(adj(a)));
                    e.set_cursor(adj(b));
                }
            }
        }
        Cmd::ToggleComment => {
            let Some((open, close)) = text::comment_tokens(lang) else {
                return;
            };
            let (a, b) = sel.unwrap_or((c, c));
            let last = if b.line > a.line && b.index == 0 {
                b.line - 1
            } else {
                b.line
            };
            let old: Vec<String> = (a.line..=last).map(|l| line_of(e, l)).collect();
            let refs: Vec<&str> = old.iter().map(String::as_str).collect();
            let new = text::toggle_comment(&refs, open, close);
            for (i, (o, n)) in old.iter().zip(&new).enumerate() {
                if o != n {
                    replace(e, cur(a.line + i, 0), cur(a.line + i, o.len()), n);
                }
            }
            let fix = |p: ct::Cursor| {
                let i = p.line - a.line;
                match (old.get(i), new.get(i)) {
                    (Some(o), Some(n)) => {
                        let d = n.len() as isize - o.len() as isize;
                        cur(
                            p.line,
                            ((p.index as isize + d).max(0) as usize).min(n.len()),
                        )
                    }
                    _ => p,
                }
            };
            if sel.is_some() {
                let anchor = match e.selection() {
                    ct::Selection::Normal(an) => an,
                    _ => a,
                };
                e.set_selection(ct::Selection::Normal(fix(anchor)));
            }
            e.set_cursor(fix(c));
        }
        Cmd::DeleteLine => {
            let (a, b) = sel.unwrap_or((c, c));
            let n = line_count(e);
            let (from, to) = if b.line + 1 < n {
                (cur(a.line, 0), cur(b.line + 1, 0))
            } else if a.line > 0 {
                (
                    cur(a.line - 1, line_of(e, a.line - 1).len()),
                    cur(b.line, line_of(e, b.line).len()),
                )
            } else {
                (cur(0, 0), cur(b.line, line_of(e, b.line).len()))
            };
            e.delete_range(from, to);
            let line = from.line.min(line_count(e) - 1) + usize::from(from.index > 0);
            let line = line.min(line_count(e) - 1);
            let len = line_of(e, line).len();
            set(e, cur(line, c.index.min(len)));
        }
        Cmd::MoveLine(up) | Cmd::CopyLine(up) => {
            let (a, b) = sel.unwrap_or((c, c));
            let last = if b.line > a.line && b.index == 0 {
                b.line - 1
            } else {
                b.line
            };
            let n = line_count(e);
            let block: Vec<String> = (a.line..=last).map(|l| line_of(e, l)).collect();
            let joined = block.join("\n");
            let copy = matches!(cmd, Cmd::CopyLine(_));
            let shift: isize = if copy {
                // The copy goes below (down) or above (up); the selection
                // follows the moved-to copy.
                let end = cur(last, line_of(e, last).len());
                e.insert_at(end, &format!("\n{joined}"), None);
                if *up { 0 } else { block.len() as isize }
            } else if *up {
                if a.line == 0 {
                    return;
                }
                let above = line_of(e, a.line - 1);
                replace(
                    e,
                    cur(a.line - 1, 0),
                    cur(last, line_of(e, last).len()),
                    &format!("{joined}\n{above}"),
                );
                -1
            } else {
                if last + 1 >= n {
                    return;
                }
                let below = line_of(e, last + 1);
                replace(
                    e,
                    cur(a.line, 0),
                    cur(last + 1, below.len()),
                    &format!("{below}\n{joined}"),
                );
                1
            };
            let mv = |p: ct::Cursor| cur((p.line as isize + shift) as usize, p.index);
            match e.selection() {
                ct::Selection::Normal(anchor) if sel.is_some() => {
                    e.set_selection(ct::Selection::Normal(mv(anchor)));
                }
                _ => e.set_selection(ct::Selection::None),
            }
            e.set_cursor(mv(c));
        }
        _ => {}
    }
}
