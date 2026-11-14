//! `ViEditor`, vendored from cosmic-text 0.15.0 (`src/edit/vi.rs`, MIT OR
//! Apache-2.0, © System76) so the kit owns vim's register and operator
//! semantics; modit (the parser) is unchanged. The upstream editor:
//!
//! - kept registers private (no `"+` clipboard bridge);
//! - left an empty line behind `dd` / `Vd` and ignored `2dd` / `3yy`;
//! - treated `dj` / `yk` / `dG` as characterwise;
//! - made characterwise visual mode exclusive of the cursor (`vey` yanked
//!   one character short) and left visual mode after `p`;
//! - never yanked with `x`, nor replaced the unnamed register on visual `p`;
//! - kept a `"x` register and count pending after `p`.
//!
//! Every change from upstream is marked `bato:`. Drawing, file loading and
//! the theme getters are dropped (the widget draws the shared buffer).

// Upstream's nesting, kept so the file still diffs against cosmic-text.
#![allow(clippy::collapsible_if)]

use std::collections::BTreeMap;

use iced::advanced::graphics::text::cosmic_text::{
    Action, AttrsList, BufferRef, Change, Cursor, Edit, FontSystem, Motion, Selection, SyntaxEditor,
};
use modit::{Event, Key, Parser, TextObject, WordIter};

pub use modit::{ViMode, ViParser};

fn undo_2_action<'buffer, E: Edit<'buffer>>(
    editor: &mut E,
    action: cosmic_undo_2::Action<&Change>,
) {
    match action {
        cosmic_undo_2::Action::Do(change) => {
            editor.apply_change(change);
        }
        cosmic_undo_2::Action::Undo(change) => {
            //TODO: make this more efficient
            let mut reversed = change.clone();
            reversed.reverse();
            editor.apply_change(&reversed);
        }
    }
}

fn finish_change<'buffer, E: Edit<'buffer>>(
    editor: &mut E,
    commands: &mut cosmic_undo_2::Commands<Change>,
    changed: &mut bool,
    pivot: Option<usize>,
) -> Option<Change> {
    //TODO: join changes together
    match editor.finish_change() {
        Some(change) => {
            if !change.items.is_empty() {
                commands.push(change.clone());
                *changed = eval_changed(commands, pivot);
            }
            Some(change)
        }
        None => None,
    }
}

/// Evaluate if an [`ViEditor`] changed based on its last saved state.
fn eval_changed(commands: &cosmic_undo_2::Commands<Change>, pivot: Option<usize>) -> bool {
    // Editors are considered modified if the current change index is unequal to the last
    // saved index or if `pivot` is None.
    // The latter case handles a never saved editor with a current command index of None.
    // Check the unit tests for an example.
    match (commands.current_command_index(), pivot) {
        (Some(current), Some(pivot)) => current != pivot,
        // Edge case for an editor with neither a save point nor any changes.
        // This could be a new editor or an editor without a save point where undo() is called
        // until the editor is fresh.
        (None, None) => false,
        // Default to true because it's safer to assume a buffer has been modified so as to not
        // lose changes
        _ => true,
    }
}

fn search<'buffer, E: Edit<'buffer>>(editor: &mut E, value: &str, forwards: bool) -> bool {
    let mut cursor = editor.cursor();
    let start_line = cursor.line;
    if forwards {
        while cursor.line < editor.with_buffer(|buffer| buffer.lines.len()) {
            if let Some(index) = editor.with_buffer(|buffer| {
                buffer.lines[cursor.line]
                    .text()
                    .match_indices(value)
                    .filter_map(|(i, _)| {
                        if cursor.line != start_line || i > cursor.index {
                            Some(i)
                        } else {
                            None
                        }
                    })
                    .next()
            }) {
                cursor.index = index;
                editor.set_cursor(cursor);
                return true;
            }

            cursor.line += 1;
        }
    } else {
        cursor.line += 1;
        while cursor.line > 0 {
            cursor.line -= 1;

            if let Some(index) = editor.with_buffer(|buffer| {
                buffer.lines[cursor.line]
                    .text()
                    .rmatch_indices(value)
                    .filter_map(|(i, _)| {
                        if cursor.line != start_line || i < cursor.index {
                            Some(i)
                        } else {
                            None
                        }
                    })
                    .next()
            }) {
                cursor.index = index;
                editor.set_cursor(cursor);
                return true;
            }
        }
    }
    false
}

fn select_in<'buffer, E: Edit<'buffer>>(editor: &mut E, start_c: char, end_c: char, include: bool) {
    // Find the largest encompasing object, or if there is none, find the next one.
    let cursor = editor.cursor();
    let (start, end) = editor.with_buffer(|buffer| {
        // Search forwards for isolated end character, counting start and end characters found
        let mut end = cursor;
        let mut starts = 0;
        let mut ends = 0;
        'find_end: loop {
            let line = &buffer.lines[end.line];
            let text = line.text();
            for (i, c) in text[end.index..].char_indices() {
                if c == end_c {
                    ends += 1;
                } else if c == start_c {
                    starts += 1;
                }
                if ends > starts {
                    end.index += if include { i + c.len_utf8() } else { i };
                    break 'find_end;
                }
            }
            if end.line + 1 < buffer.lines.len() {
                end.line += 1;
                end.index = 0;
            } else {
                break 'find_end;
            }
        }

        // Search backwards to resolve starts and ends
        let mut start = cursor;
        'find_start: loop {
            let line = &buffer.lines[start.line];
            let text = line.text();
            for (i, c) in text[..start.index].char_indices().rev() {
                if c == start_c {
                    starts += 1;
                } else if c == end_c {
                    ends += 1;
                }
                if starts >= ends {
                    start.index = if include { i } else { i + c.len_utf8() };
                    break 'find_start;
                }
            }
            if start.line > 0 {
                start.line -= 1;
                start.index = buffer.lines[start.line].text().len();
            } else {
                break 'find_start;
            }
        }

        (start, end)
    });

    editor.set_selection(Selection::Normal(start));
    editor.set_cursor(end);
}

/// bato: a register's contents. `linewise` is vim's `V` register type
/// (the text has no trailing newline).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Register {
    pub text: String,
    pub linewise: bool,
}

/// bato: what the operator being parsed has seen (one `parse` call).
#[derive(Debug, Default, Clone, Copy)]
struct OpState {
    /// A `SelectStart` (operator + motion) happened; the motion was
    /// linewise (`j`, `k`, `G`, `gg`, Enter, `-`, `+`).
    started: Option<bool>,
    /// The cursor before the command (yanks put it back).
    origin: Option<Cursor>,
}

#[derive(Debug)]
pub struct ViEditor<'syntax_system, 'buffer> {
    editor: SyntaxEditor<'syntax_system, 'buffer>,
    parser: ViParser,
    registers: BTreeMap<char, Register>,
    search_opt: Option<(String, bool)>,
    commands: cosmic_undo_2::Commands<Change>,
    changed: bool,
    save_pivot: Option<usize>,
}

impl<'syntax_system, 'buffer> ViEditor<'syntax_system, 'buffer> {
    pub fn new(editor: SyntaxEditor<'syntax_system, 'buffer>) -> Self {
        Self {
            editor,
            parser: ViParser::new(),
            registers: BTreeMap::new(),
            search_opt: None,
            commands: cosmic_undo_2::Commands::new(),
            changed: false,
            save_pivot: None,
        }
    }

    /// Modifies the theme of the [`SyntaxEditor`], returning false if the theme is missing
    pub fn update_theme(&mut self, theme_name: &str) -> bool {
        self.editor.update_theme(theme_name)
    }

    /// Get current vi parser
    pub fn parser(&self) -> &ViParser {
        &self.parser
    }

    /// bato: forget a pending command (count, register, operator) and go
    /// back to normal mode, clearing the selection.
    pub fn reset_parser(&mut self) {
        self.parser.reset();
        self.editor.set_selection(Selection::None);
    }

    /// bato: a register's contents.
    pub fn register(&self, name: char) -> Option<&Register> {
        self.registers.get(&name)
    }

    /// bato: set a register (the host fills `+` / `*` from the clipboard).
    pub fn set_register(&mut self, name: char, register: Register) {
        self.registers.insert(name, register);
    }

    /// bato: the register named by a pending `"x` prefix.
    pub fn pending_register(&self) -> Option<char> {
        let cmd = self.parser.cmd.to_string();
        let mut chars = cmd.chars();
        (chars.next() == Some('"')).then(|| chars.next()).flatten()
    }

    /// Redo a change
    pub fn redo(&mut self) {
        for action in self.commands.redo() {
            undo_2_action(&mut self.editor, action);
        }
        self.changed = eval_changed(&self.commands, self.save_pivot);
    }

    /// Undo a change
    pub fn undo(&mut self) {
        for action in self.commands.undo() {
            undo_2_action(&mut self.editor, action);
        }
        self.changed = eval_changed(&self.commands, self.save_pivot);
    }
}

/// bato: the count in a `ViCmd`'s display (`"a3Delete` → 3).
fn pending_count(cmd: &str) -> Option<usize> {
    let rest = match cmd.strip_prefix('"') {
        Some(r) => {
            let mut c = r.chars();
            c.next();
            c.as_str()
        }
        None => cmd,
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// bato: store a yank in `name` (and the unnamed register), like vim:
/// `_` is the black hole; any other register also becomes `""`.
fn store(registers: &mut BTreeMap<char, Register>, name: char, register: Register) {
    if name == '_' {
        return;
    }
    if name != '"' {
        registers.insert('"', register.clone());
    }
    registers.insert(name, register);
}

/// bato: the first non-blank of a line.
fn soft_home<'buffer, E: Edit<'buffer>>(editor: &E, line: usize) -> Cursor {
    let index = editor.with_buffer(|buffer| {
        let text = buffer.lines[line].text();
        text.len() - text.trim_start().len()
    });
    Cursor::new(line, index)
}

impl<'buffer> Edit<'buffer> for ViEditor<'_, 'buffer> {
    fn buffer_ref(&self) -> &BufferRef<'buffer> {
        self.editor.buffer_ref()
    }

    fn buffer_ref_mut(&mut self) -> &mut BufferRef<'buffer> {
        self.editor.buffer_ref_mut()
    }

    fn cursor(&self) -> Cursor {
        self.editor.cursor()
    }

    fn set_cursor(&mut self, cursor: Cursor) {
        self.editor.set_cursor(cursor);
    }

    fn selection(&self) -> Selection {
        self.editor.selection()
    }

    fn set_selection(&mut self, selection: Selection) {
        self.editor.set_selection(selection);
    }

    fn auto_indent(&self) -> bool {
        self.editor.auto_indent()
    }

    fn set_auto_indent(&mut self, auto_indent: bool) {
        self.editor.set_auto_indent(auto_indent);
    }

    fn tab_width(&self) -> u16 {
        self.editor.tab_width()
    }

    fn set_tab_width(&mut self, font_system: &mut FontSystem, tab_width: u16) {
        self.editor.set_tab_width(font_system, tab_width);
    }

    fn shape_as_needed(&mut self, font_system: &mut FontSystem, prune: bool) {
        self.editor.shape_as_needed(font_system, prune);
    }

    fn delete_range(&mut self, start: Cursor, end: Cursor) {
        self.editor.delete_range(start, end);
    }

    fn insert_at(&mut self, cursor: Cursor, data: &str, attrs_list: Option<AttrsList>) -> Cursor {
        self.editor.insert_at(cursor, data, attrs_list)
    }

    fn copy_selection(&self) -> Option<String> {
        self.editor.copy_selection()
    }

    fn delete_selection(&mut self) -> bool {
        self.editor.delete_selection()
    }

    fn apply_change(&mut self, change: &Change) -> bool {
        self.editor.apply_change(change)
    }

    fn start_change(&mut self) {
        self.editor.start_change();
    }

    fn finish_change(&mut self) -> Option<Change> {
        finish_change(
            &mut self.editor,
            &mut self.commands,
            &mut self.changed,
            self.save_pivot,
        )
    }

    fn action(&mut self, font_system: &mut FontSystem, action: Action) {
        let editor = &mut self.editor;

        // Ensure a change is always started
        editor.start_change();

        let key = match action {
            //TODO: this leaves lots of room for issues in translation, should we directly accept Key?
            Action::Backspace => Key::Backspace,
            Action::Delete => Key::Delete,
            Action::Motion(Motion::Down) => Key::Down,
            Action::Motion(Motion::End) => Key::End,
            Action::Enter => Key::Enter,
            Action::Escape => Key::Escape,
            Action::Motion(Motion::Home) => Key::Home,
            Action::Indent => Key::Tab,
            Action::Insert(c) => Key::Char(c),
            Action::Motion(Motion::Left) => Key::Left,
            Action::Motion(Motion::PageDown) => Key::PageDown,
            Action::Motion(Motion::PageUp) => Key::PageUp,
            Action::Motion(Motion::Right) => Key::Right,
            Action::Unindent => Key::Backtab,
            Action::Motion(Motion::Up) => Key::Up,
            _ => {
                editor.action(font_system, action);
                // Always finish change when passing through (TODO: group changes)
                finish_change(
                    editor,
                    &mut self.commands,
                    &mut self.changed,
                    self.save_pivot,
                );
                return;
            }
        };

        // bato: what this key means to vim, before modit sees it.
        let mode = self.parser.mode.clone();
        let visual = matches!(mode, ViMode::Visual | ViMode::VisualLine);
        // Visual `x` / `s` delete / change the selection.
        let key = match key {
            Key::Char('x') if visual => Key::Char('d'),
            Key::Char('s') if visual => Key::Char('c'),
            key => key,
        };
        // Characterwise visual mode includes the character under the cursor.
        if mode == ViMode::Visual && matches!(key, Key::Char('y' | 'd' | 'c' | 'p' | 'P')) {
            include_cursor(editor);
        }
        let pending = self.parser.cmd.to_string();
        let count = pending_count(&pending).unwrap_or(1);
        let named = pending
            .strip_prefix('"')
            .and_then(|r| r.chars().next())
            .unwrap_or('"');
        let change = pending.contains("Change")
            || (matches!(mode, ViMode::Normal) && matches!(key, Key::Char('c' | 'C' | 'S')))
            || (visual && matches!(key, Key::Char('c')));
        // `x` / `X` yank what they delete.
        let x_yank = mode == ViMode::Normal
            && pending.find("Delete").is_none()
            && matches!(key, Key::Char('x' | 'X'));
        let mut x_text = String::new();
        let mut op = OpState {
            started: None,
            origin: Some(editor.cursor()),
        };
        // The start of a pure yank (the cursor goes there), and whether a
        // put or delete ran.
        let mut yank: Option<(Cursor, bool)> = None;
        // `O`: the line it opened, and the indent it should get.
        let mut open_above: Option<(usize, String)> = None;
        let mut deleted = false;
        let mut put = false;

        let has_selection = !matches!(editor.selection(), Selection::None);
        let registers = &mut self.registers;

        self.parser.parse(key, has_selection, |event| {
            let action = match event {
                Event::AutoIndent => {
                    return;
                }
                Event::Backspace => Action::Backspace,
                Event::BackspaceInLine => {
                    let cursor = editor.cursor();
                    if cursor.index > 0 {
                        if x_yank {
                            let ch = editor.with_buffer(|buffer| {
                                buffer.lines[cursor.line].text()[..cursor.index]
                                    .chars()
                                    .next_back()
                            });
                            if let Some(ch) = ch {
                                x_text.insert(0, ch);
                            }
                        }
                        Action::Backspace
                    } else {
                        return;
                    }
                }
                Event::ChangeStart => {
                    editor.start_change();
                    return;
                }
                Event::ChangeFinish => {
                    finish_change(
                        editor,
                        &mut self.commands,
                        &mut self.changed,
                        self.save_pivot,
                    );
                    return;
                }
                Event::Delete => {
                    deleted = true;
                    if op.started == Some(true) {
                        to_lines(editor);
                    }
                    // bato: a linewise delete takes the lines with it (a
                    // linewise change keeps one empty line to type in).
                    if !change && matches!(editor.selection(), Selection::Line(_)) {
                        delete_lines(editor);
                        return;
                    }
                    Action::Delete
                }
                Event::DeleteInLine => {
                    let cursor = editor.cursor();
                    let line =
                        editor.with_buffer(|buffer| buffer.lines[cursor.line].text().to_string());
                    if cursor.index < line.len() {
                        if x_yank {
                            x_text.extend(line[cursor.index..].chars().next());
                        }
                        Action::Delete
                    } else {
                        return;
                    }
                }
                Event::Escape => Action::Escape,
                Event::Insert(c) => Action::Insert(c),
                // bato: the user's Neovim has `autoindent` + `smartindent`:
                // a new line takes the indent of the text before the cursor,
                // one level (2 spaces) more after an opening brace. At column
                // 0 (`O`, or Enter before the text) the split is plain and
                // `O` gets its indent after the command, on the new line.
                Event::NewLine => {
                    let c = editor.cursor();
                    let line = editor.with_buffer(|b| b.lines[c.line].text().to_string());
                    if c.index == 0 {
                        open_above = Some((c.line, leading_ws(&line).to_string()));
                        Action::Enter
                    } else {
                        let before = &line[..c.index];
                        let mut ind = leading_ws(before).to_string();
                        if before.trim_end().ends_with('{') {
                            ind.push_str("  ");
                        }
                        editor.insert_string(&format!("\n{ind}"), None);
                        let line_i = editor.cursor().line;
                        editor.with_buffer_mut(|b| {
                            b.line_layout(font_system, line_i);
                        });
                        return;
                    }
                }
                Event::Put { register, after } => {
                    put = true;
                    if let Some(reg) = registers.get(&register).cloned() {
                        editor.start_change();
                        if let Some(old) = put_register(editor, font_system, &reg, after) {
                            // bato: visual `p` puts the replaced text in "".
                            registers.insert('"', old);
                        }
                        finish_change(
                            editor,
                            &mut self.commands,
                            &mut self.changed,
                            self.save_pivot,
                        );
                    }
                    return;
                }
                Event::Redraw => {
                    editor.with_buffer_mut(|buffer| buffer.set_redraw(true));
                    return;
                }
                Event::SelectClear => {
                    editor.set_selection(Selection::None);
                    return;
                }
                Event::SelectStart => {
                    let cursor = editor.cursor();
                    editor.set_selection(Selection::Normal(cursor));
                    op.started = Some(false);
                    return;
                }
                Event::SelectLineStart => {
                    let cursor = editor.cursor();
                    editor.set_selection(Selection::Line(cursor));
                    // bato: `3dd` / `2yy` take that many lines.
                    if count > 1 {
                        let last = editor.with_buffer(|buffer| buffer.lines.len() - 1);
                        editor.set_cursor(Cursor::new((cursor.line + count - 1).min(last), 0));
                    }
                    return;
                }
                Event::SelectTextObject(text_object, include) => {
                    match text_object {
                        TextObject::AngleBrackets => select_in(editor, '<', '>', include),
                        TextObject::CurlyBrackets => select_in(editor, '{', '}', include),
                        TextObject::DoubleQuotes => select_in(editor, '"', '"', include),
                        TextObject::Parentheses => select_in(editor, '(', ')', include),
                        TextObject::Search { forwards } => {
                            if let Some((value, _)) = &self.search_opt {
                                if search(editor, value, forwards) {
                                    let mut cursor = editor.cursor();
                                    editor.set_selection(Selection::Normal(cursor));
                                    //TODO: traverse lines if necessary
                                    cursor.index += value.len();
                                    editor.set_cursor(cursor);
                                }
                            }
                        }
                        TextObject::SingleQuotes => select_in(editor, '\'', '\'', include),
                        TextObject::SquareBrackets => select_in(editor, '[', ']', include),
                        TextObject::Ticks => select_in(editor, '`', '`', include),
                        TextObject::Word(word) => {
                            let mut cursor = editor.cursor();
                            let mut selection = editor.selection();
                            editor.with_buffer(|buffer| {
                                let text = buffer.lines[cursor.line].text();
                                match WordIter::new(text, word)
                                    .find(|&(i, w)| i <= cursor.index && i + w.len() > cursor.index)
                                {
                                    Some((i, w)) => {
                                        cursor.index = i;
                                        selection = Selection::Normal(cursor);
                                        cursor.index += w.len();
                                    }
                                    None => {
                                        //TODO
                                    }
                                }
                            });
                            editor.set_selection(selection);
                            editor.set_cursor(cursor);
                        }
                        _ => {}
                    }
                    // bato: in visual mode the cursor sits on the object's
                    // last character (the selection is inclusive there).
                    if mode == ViMode::Visual
                        && let Some((start, end)) = editor.selection_bounds()
                        && start != end
                        && editor.cursor() == end
                    {
                        let back = editor.with_buffer(|buffer| {
                            buffer.lines[end.line].text()[..end.index]
                                .char_indices()
                                .next_back()
                                .map(|(i, _)| i)
                        });
                        if let Some(i) = back {
                            editor.set_cursor(Cursor::new(end.line, i));
                        }
                    }
                    return;
                }
                Event::SetSearch(value, forwards) => {
                    self.search_opt = Some((value, forwards));
                    return;
                }
                Event::ShiftLeft | Event::ShiftRight => {
                    if op.started == Some(true) {
                        to_lines(editor);
                    }
                    if matches!(event, Event::ShiftLeft) {
                        Action::Unindent
                    } else {
                        Action::Indent
                    }
                }
                Event::SwapCase => {
                    return;
                }
                Event::Undo => {
                    for action in self.commands.undo() {
                        undo_2_action(editor, action);
                    }
                    return;
                }
                Event::Yank { register } => {
                    if op.started == Some(true) {
                        to_lines(editor);
                    }
                    if let Some(text) = editor.copy_selection() {
                        let linewise = matches!(editor.selection(), Selection::Line(_));
                        if let Some((start, _)) = editor.selection_bounds() {
                            yank = Some((start, linewise));
                        }
                        store(registers, register, Register { text, linewise });
                    }
                    return;
                }
                Event::Motion(motion) => {
                    if op.started.is_some()
                        && matches!(
                            motion,
                            modit::Motion::Down
                                | modit::Motion::Up
                                | modit::Motion::GotoLine(_)
                                | modit::Motion::GotoEof
                                | modit::Motion::ScreenHigh
                                | modit::Motion::ScreenLow
                                | modit::Motion::ScreenMiddle
                        )
                    {
                        op.started = Some(true);
                    }
                    match motion {
                        modit::Motion::Around => {
                            //TODO: what to do for this psuedo-motion?
                            return;
                        }
                        modit::Motion::Down => Action::Motion(Motion::Down),
                        modit::Motion::End => Action::Motion(Motion::End),
                        modit::Motion::GotoLine(line) => {
                            Action::Motion(Motion::GotoLine(line.saturating_sub(1)))
                        }
                        modit::Motion::GotoEof => Action::Motion(Motion::GotoLine(
                            editor.with_buffer(|buffer| buffer.lines.len().saturating_sub(1)),
                        )),
                        modit::Motion::Home => Action::Motion(Motion::Home),
                        modit::Motion::Inside => {
                            //TODO: what to do for this psuedo-motion?
                            return;
                        }
                        modit::Motion::Left => Action::Motion(Motion::Left),
                        modit::Motion::LeftInLine => {
                            let cursor = editor.cursor();
                            if cursor.index > 0 {
                                Action::Motion(Motion::Left)
                            } else {
                                return;
                            }
                        }
                        modit::Motion::Line => {
                            //TODO: what to do for this psuedo-motion?
                            return;
                        }
                        modit::Motion::NextChar(find_c) => {
                            let mut cursor = editor.cursor();
                            editor.with_buffer(|buffer| {
                                let text = buffer.lines[cursor.line].text();
                                if cursor.index < text.len() {
                                    if let Some((i, _)) = text[cursor.index..]
                                        .char_indices()
                                        .find(|&(i, c)| i > 0 && c == find_c)
                                    {
                                        cursor.index += i;
                                    }
                                }
                            });
                            editor.set_cursor(cursor);
                            return;
                        }
                        modit::Motion::NextCharTill(find_c) => {
                            let mut cursor = editor.cursor();
                            editor.with_buffer(|buffer| {
                                let text = buffer.lines[cursor.line].text();
                                if cursor.index < text.len() {
                                    let mut last_i = 0;
                                    for (i, c) in text[cursor.index..].char_indices() {
                                        if last_i > 0 && c == find_c {
                                            cursor.index += last_i;
                                            break;
                                        } else {
                                            last_i = i;
                                        }
                                    }
                                }
                            });
                            editor.set_cursor(cursor);
                            return;
                        }
                        modit::Motion::NextSearch => match &self.search_opt {
                            Some((value, forwards)) => {
                                search(editor, value, *forwards);
                                return;
                            }
                            None => return,
                        },
                        modit::Motion::NextWordEnd(word) => {
                            let mut cursor = editor.cursor();
                            editor.with_buffer(|buffer| {
                                loop {
                                    let text = buffer.lines[cursor.line].text();
                                    if cursor.index < text.len() {
                                        cursor.index = WordIter::new(text, word)
                                            .map(|(i, w)| {
                                                i + w
                                                    .char_indices()
                                                    .last()
                                                    .map(|(i, _)| i)
                                                    .unwrap_or(0)
                                            })
                                            .find(|&i| i > cursor.index)
                                            .unwrap_or(text.len());
                                        if cursor.index == text.len() {
                                            // Try again, searching next line
                                            continue;
                                        }
                                    } else if cursor.line + 1 < buffer.lines.len() {
                                        // Go to next line and rerun loop
                                        cursor.line += 1;
                                        cursor.index = 0;
                                        continue;
                                    }
                                    break;
                                }
                            });
                            editor.set_cursor(cursor);
                            return;
                        }
                        modit::Motion::NextWordStart(word) => {
                            let mut cursor = editor.cursor();
                            editor.with_buffer(|buffer| {
                                loop {
                                    let text = buffer.lines[cursor.line].text();
                                    if cursor.index < text.len() {
                                        cursor.index = WordIter::new(text, word)
                                            .map(|(i, _)| i)
                                            .find(|&i| i > cursor.index)
                                            .unwrap_or(text.len());
                                        if cursor.index == text.len() {
                                            // Try again, searching next line
                                            continue;
                                        }
                                    } else if cursor.line + 1 < buffer.lines.len() {
                                        // Go to next line and rerun loop
                                        cursor.line += 1;
                                        cursor.index = 0;
                                        continue;
                                    }
                                    break;
                                }
                            });
                            editor.set_cursor(cursor);
                            return;
                        }
                        modit::Motion::PageDown => Action::Motion(Motion::PageDown),
                        modit::Motion::PageUp => Action::Motion(Motion::PageUp),
                        modit::Motion::PreviousChar(find_c) => {
                            let mut cursor = editor.cursor();
                            editor.with_buffer(|buffer| {
                                let text = buffer.lines[cursor.line].text();
                                if cursor.index > 0 {
                                    if let Some((i, _)) = text[..cursor.index]
                                        .char_indices()
                                        .rfind(|&(_, c)| c == find_c)
                                    {
                                        cursor.index = i;
                                    }
                                }
                            });
                            editor.set_cursor(cursor);
                            return;
                        }
                        modit::Motion::PreviousCharTill(find_c) => {
                            let mut cursor = editor.cursor();
                            editor.with_buffer(|buffer| {
                                let text = buffer.lines[cursor.line].text();
                                if cursor.index > 0 {
                                    if let Some(i) = text[..cursor.index]
                                        .char_indices()
                                        .filter_map(|(i, c)| {
                                            if c == find_c {
                                                let end = i + c.len_utf8();
                                                if end < cursor.index {
                                                    return Some(end);
                                                }
                                            }
                                            None
                                        })
                                        .next_back()
                                    {
                                        cursor.index = i;
                                    }
                                }
                            });
                            editor.set_cursor(cursor);
                            return;
                        }
                        modit::Motion::PreviousSearch => match &self.search_opt {
                            Some((value, forwards)) => {
                                search(editor, value, !*forwards);
                                return;
                            }
                            None => return,
                        },
                        modit::Motion::PreviousWordEnd(word) => {
                            let mut cursor = editor.cursor();
                            editor.with_buffer(|buffer| {
                                loop {
                                    let text = buffer.lines[cursor.line].text();
                                    if cursor.index > 0 {
                                        cursor.index = WordIter::new(text, word)
                                            .map(|(i, w)| {
                                                i + w
                                                    .char_indices()
                                                    .last()
                                                    .map(|(i, _)| i)
                                                    .unwrap_or(0)
                                            })
                                            .filter(|&i| i < cursor.index)
                                            .last()
                                            .unwrap_or(0);
                                        if cursor.index == 0 {
                                            // Try again, searching previous line
                                            continue;
                                        }
                                    } else if cursor.line > 0 {
                                        // Go to previous line and rerun loop
                                        cursor.line -= 1;
                                        cursor.index = buffer.lines[cursor.line].text().len();
                                        continue;
                                    }
                                    break;
                                }
                            });
                            editor.set_cursor(cursor);
                            return;
                        }
                        modit::Motion::PreviousWordStart(word) => {
                            let mut cursor = editor.cursor();
                            editor.with_buffer(|buffer| {
                                loop {
                                    let text = buffer.lines[cursor.line].text();
                                    if cursor.index > 0 {
                                        cursor.index = WordIter::new(text, word)
                                            .map(|(i, _)| i)
                                            .filter(|&i| i < cursor.index)
                                            .last()
                                            .unwrap_or(0);
                                        if cursor.index == 0 {
                                            // Try again, searching previous line
                                            continue;
                                        }
                                    } else if cursor.line > 0 {
                                        // Go to previous line and rerun loop
                                        cursor.line -= 1;
                                        cursor.index = buffer.lines[cursor.line].text().len();
                                        continue;
                                    }
                                    break;
                                }
                            });
                            editor.set_cursor(cursor);
                            return;
                        }
                        modit::Motion::Right => Action::Motion(Motion::Right),
                        modit::Motion::RightInLine => {
                            let cursor = editor.cursor();
                            if cursor.index
                                < editor
                                    .with_buffer(|buffer| buffer.lines[cursor.line].text().len())
                            {
                                Action::Motion(Motion::Right)
                            } else {
                                return;
                            }
                        }
                        modit::Motion::ScreenHigh => {
                            //TODO: is this efficient?
                            if let Some(line_i) = editor.with_buffer(|buffer| {
                                buffer.layout_runs().next().map(|first| first.line_i)
                            }) {
                                Action::Motion(Motion::GotoLine(line_i))
                            } else {
                                return;
                            }
                        }
                        modit::Motion::ScreenLow => {
                            //TODO: is this efficient?
                            if let Some(line_i) = editor.with_buffer(|buffer| {
                                buffer.layout_runs().last().map(|last| last.line_i)
                            }) {
                                Action::Motion(Motion::GotoLine(line_i))
                            } else {
                                return;
                            }
                        }
                        modit::Motion::ScreenMiddle => {
                            //TODO: is this efficient?
                            let action_opt = editor.with_buffer(|buffer| {
                                let mut layout_runs = buffer.layout_runs();

                                match (layout_runs.next(), layout_runs.last()) {
                                    (Some(first), Some(last)) => Some(Action::Motion(
                                        Motion::GotoLine((last.line_i + first.line_i) / 2),
                                    )),
                                    _ => None,
                                }
                            });
                            match action_opt {
                                Some(action) => action,
                                None => return,
                            }
                        }
                        modit::Motion::Selection => {
                            //TODO: what to do for this psuedo-motion?
                            return;
                        }
                        modit::Motion::SoftHome => Action::Motion(Motion::SoftHome),
                        modit::Motion::Up => Action::Motion(Motion::Up),
                    }
                }
            };
            editor.action(font_system, action);
        });

        // bato: after the command.
        if let Some((line_i, ind)) = open_above.filter(|(_, ind)| !ind.is_empty()) {
            let c = self.editor.cursor();
            let blank = self
                .editor
                .with_buffer(|b| b.lines[c.line].text().is_empty());
            if c.line == line_i && c.index == 0 && blank {
                self.editor.insert_string(&ind, None);
            }
        }
        if x_yank && !x_text.is_empty() {
            store(
                &mut self.registers,
                named,
                Register {
                    text: x_text,
                    linewise: false,
                },
            );
        }
        if let Some((start, linewise)) = yank.filter(|_| !deleted && !put) {
            // A yank leaves the cursor at the start of what it took (on the
            // line where a linewise one starts, in the same column).
            let origin = op.origin.unwrap_or(start);
            let cursor = if !linewise {
                start
            } else if origin.line == start.line {
                origin
            } else {
                let len = self
                    .editor
                    .with_buffer(|b| b.lines[start.line].text().len());
                Cursor::new(start.line, origin.index.min(len))
            };
            self.editor.set_cursor(cursor);
        }
        // bato: in normal mode the cursor never sits past a line's last
        // character (`$`, a delete at the end of the line).
        if self.parser.mode == ViMode::Normal {
            let c = self.editor.cursor();
            let back = self.editor.with_buffer(|buffer| {
                let text = buffer.lines[c.line].text();
                (c.index >= text.len())
                    .then(|| text.char_indices().next_back().map(|(i, _)| i))
                    .flatten()
            });
            if let Some(i) = back {
                self.editor.set_cursor(Cursor::new(c.line, i));
            }
        }
        if put {
            // modit keeps the `"x` register and count of a put pending, and
            // stays in visual mode after it.
            self.parser.cmd = Default::default();
            if visual {
                self.parser.mode = ViMode::Normal;
                self.editor.set_selection(Selection::None);
            }
        }
    }

    fn cursor_position(&self) -> Option<(i32, i32)> {
        self.editor.cursor_position()
    }
}

/// bato: characterwise visual mode includes the character under the cursor
/// (or the anchor's, selecting backwards): push the later end one character
/// on (to the next line's start at a line end).
fn include_cursor<'buffer, E: Edit<'buffer>>(editor: &mut E) {
    let Selection::Normal(anchor) = editor.selection() else {
        return;
    };
    let cursor = editor.cursor();
    let bump = |editor: &E, c: Cursor| -> Cursor {
        editor.with_buffer(|buffer| {
            let text = buffer.lines[c.line].text();
            match text[c.index..].chars().next() {
                Some(ch) => Cursor::new(c.line, c.index + ch.len_utf8()),
                None if c.line + 1 < buffer.lines.len() => Cursor::new(c.line + 1, 0),
                None => c,
            }
        })
    };
    if (cursor.line, cursor.index) >= (anchor.line, anchor.index) {
        let c = bump(editor, cursor);
        editor.set_cursor(c);
    } else {
        let a = bump(editor, anchor);
        editor.set_selection(Selection::Normal(a));
    }
}

/// bato: make the selection linewise (`dj`, `yk`, `dG`…).
fn to_lines<'buffer, E: Edit<'buffer>>(editor: &mut E) {
    if let Selection::Normal(anchor) = editor.selection() {
        editor.set_selection(Selection::Line(anchor));
    }
}

/// bato: delete the lines of a linewise selection with their newline; the
/// cursor goes to the first non-blank of the line that takes their place.
fn delete_lines<'buffer, E: Edit<'buffer>>(editor: &mut E) {
    let Some((start, end)) = editor.selection_bounds() else {
        return;
    };
    editor.set_selection(Selection::None);
    let (n, prev_len, end_len) = editor.with_buffer(|buffer| {
        (
            buffer.lines.len(),
            start
                .line
                .checked_sub(1)
                .map_or(0, |l| buffer.lines[l].text().len()),
            buffer.lines[end.line].text().len(),
        )
    });
    let line = if end.line + 1 < n {
        editor.delete_range(Cursor::new(start.line, 0), Cursor::new(end.line + 1, 0));
        start.line
    } else if start.line > 0 {
        editor.delete_range(
            Cursor::new(start.line - 1, prev_len),
            Cursor::new(end.line, end_len),
        );
        start.line - 1
    } else {
        editor.delete_range(Cursor::new(0, 0), Cursor::new(end.line, end_len));
        0
    };
    let c = soft_home(editor, line);
    editor.set_cursor(c);
}

/// bato: `p` / `P` (vim's `do_put`): linewise text goes on lines of its
/// own below / above and the cursor to its first non-blank; characterwise
/// text goes after / at the cursor and the cursor to its last character.
/// With a selection (visual mode) the text replaces it, and the replaced
/// text is returned (for the unnamed register).
fn put_register<'buffer, E: Edit<'buffer>>(
    editor: &mut E,
    font_system: &mut FontSystem,
    reg: &Register,
    after: bool,
) -> Option<Register> {
    let _ = font_system;
    let mut replaced = None;
    let mut after = after;
    if !matches!(editor.selection(), Selection::None) {
        let linewise = matches!(editor.selection(), Selection::Line(_));
        if let Some(text) = editor.copy_selection() {
            replaced = Some(Register { text, linewise });
        }
        if linewise {
            let (start, _) = editor.selection_bounds()?;
            let n = editor.with_buffer(|b| b.lines.len());
            delete_lines(editor);
            // The lines that followed moved up: put above them, or below
            // the new last line when the selection reached the end.
            let now = editor.with_buffer(|b| b.lines.len());
            if start.line < now && now < n {
                editor.set_cursor(Cursor::new(start.line, 0));
                after = false;
            } else {
                after = true;
            }
            if now == 1 && editor.with_buffer(|b| b.lines[0].text().is_empty()) && reg.linewise {
                // Everything went: the text replaces the empty buffer.
                editor.insert_at(Cursor::new(0, 0), &reg.text, None);
                let c = soft_home(editor, 0);
                editor.set_cursor(c);
                return replaced;
            }
        } else {
            editor.delete_selection();
            after = false;
        }
    }
    let cursor = editor.cursor();
    if reg.linewise {
        let n = editor.with_buffer(|b| b.lines.len());
        let line = if after { cursor.line + 1 } else { cursor.line };
        if line >= n {
            let len = editor.with_buffer(|b| b.lines[n - 1].text().len());
            editor.insert_at(Cursor::new(n - 1, len), &format!("\n{}", reg.text), None);
        } else {
            editor.insert_at(Cursor::new(line, 0), &format!("{}\n", reg.text), None);
        }
        let c = soft_home(editor, line);
        editor.set_cursor(c);
    } else {
        let mut at = cursor;
        if after {
            editor.with_buffer(|buffer| {
                if let Some(c) = buffer.lines[at.line].text()[at.index..].chars().next() {
                    at.index += c.len_utf8();
                }
            });
        }
        let end = editor.insert_at(at, &reg.text, None);
        let c = if reg.text.contains('\n') {
            at
        } else {
            editor.with_buffer(|buffer| {
                let text = buffer.lines[end.line].text();
                let i = text[..end.index]
                    .char_indices()
                    .next_back()
                    .map_or(end.index, |(i, _)| i);
                Cursor::new(end.line, i.max(at.index))
            })
        };
        editor.set_selection(Selection::None);
        editor.set_cursor(c);
    }
    replaced
}

/// The spaces and tabs a line starts with.
fn leading_ws(s: &str) -> &str {
    &s[..s.len() - s.trim_start_matches([' ', '\t']).len()]
}
