//! `inputs/code-editor`: Code editor. A language Select, the "Line
//! numbers" and "Vim mode" switches (`flex items-center gap-3 mb-3`), then
//! the CodeEditor, 14rem tall, on the Showcase's sample function.
//!
//! Also [`selftest`]: `BATO_SELFTEST=editor` (batch B8) with
//! `BATO_VIEW=view=examples&tab=inputs&card=code-editor`.

use std::time::Duration;

use bato_kit::editor::{Content, Language, code_editor};
use bato_kit::iced::keyboard::{Modifiers, key::Named};
use bato_kit::iced::widget::{column, row};
use bato_kit::iced::{Point, mouse};
use bato_kit::overlay::script::{Step, Target};
use bato_kit::overlay::select::{SelectOption, opt, select};
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// `useState("function greet(name) {…")`.
pub const SAMPLE: &str =
    "function greet(name) {\n  // say hello\n  return `Hello, ${name}!`\n}\n\ngreet('world')";

#[derive(Debug)]
pub struct State {
    pub code: Content,
    pub lang: Language,
    pub line_numbers: bool,
    pub vim: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            code: Content::new(SAMPLE),
            lang: Language::JavaScript,
            line_numbers: true,
            vim: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Lang(Language),
    LineNumbers(bool),
    Vim(bool),
    /// `onChange` (the text lives in `code`).
    Code(String),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Lang(l) => state.lang = l,
        Msg::LineNumbers(on) => state.line_numbers = on,
        Msg::Vim(on) => state.vim = on,
        Msg::Code(_) => {}
    }
    None
}

/// The Select's options, in the card's order.
fn languages() -> Vec<SelectOption<Language>> {
    vec![
        opt(Language::JavaScript, "JavaScript"),
        opt(Language::TypeScript, "TypeScript"),
        opt(Language::Json, "JSON"),
        opt(Language::Python, "Python"),
        opt(Language::Html, "HTML"),
        opt(Language::Css, "CSS"),
        opt(Language::Markdown, "Markdown"),
    ]
}

/// The editor's id (self-test).
pub const EDITOR: &str = "code-editor";

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let top = row![
        select(&on, state.lang, languages(), Msg::Lang),
        switch(&on, state.line_numbers, Msg::LineNumbers).label("Line numbers"),
        switch(&on, state.vim, Msg::Vim).label("Vim mode"),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    let editor = code_editor(&on, &state.code)
        .language(state.lang)
        .line_numbers(state.line_numbers)
        .vim(state.vim)
        .height(224.0)
        .id(EDITOR)
        .on_change(Msg::Code);
    card(cx, "Code editor")
        .body(column![top, vgap(12.0), editor].width(Length::Fill))
        .into()
}

// ── BATO_SELFTEST=editor (batch B8) ─────────────────────────────────────

/// A window point on character `col` of line `line`, with the card at the
/// top (Electron reference: the editor box spans x 347–1152, y 131–356;
/// line `i` starts at y = 136 + 18.2 i, text at x = 374, 7.8 px a char).
fn at_char(line: usize, col: usize) -> Target {
    Target::At(Point::new(
        374.0 + col as f32 * 7.8 + 1.0,
        136.0 + line as f32 * 18.2 + 9.0,
    ))
}

fn click(t: Target) -> Step<Msg> {
    Step::Click(t, mouse::Button::Left)
}

fn key(k: Named) -> Step<Msg> {
    Step::Key(k, Modifiers::default())
}

fn typed(s: &str) -> Vec<Step<Msg>> {
    s.chars()
        .map(|c| match c {
            '\n' => key(Named::Enter),
            c => Step::Char(c, Modifiers::default()),
        })
        .collect()
}

/// Rest; click after the last line and type a block with auto-closed
/// brackets and indentation; select a word with Shift+Ctrl+→ (its other
/// occurrence lights up); undo; gutter off; TypeScript; vim: normal (block
/// cursor), `dw`, insert, visual; then the user's Neovim keys: a pending
/// `<leader>` (`<20>` in the status bar, then `"+y`), `<leader>yy` to the
/// clipboard and
/// Ctrl+Shift+V putting it below the last line, visual `<leader>p` twice
/// with the same register, `<leader>yiw`, Ctrl+Shift+V in insert mode and
/// Ctrl+C back to normal mode. (Under `BATO_SELFTEST` the editor uses a
/// private clipboard and logs its writes on stderr.)
pub fn selftest() -> Vec<Step<Msg>> {
    let mut s = vec![Step::Shot("rest"), click(at_char(5, 20))];
    s.extend(typed("\nif (x) {\nlog('hi'"));
    s.push(Step::Wait(Duration::from_millis(1000)));
    s.push(Step::Shot("typed"));
    s.push(click(at_char(0, 9)));
    s.push(Step::Key(Named::ArrowRight, Modifiers::CTRL | Modifiers::SHIFT));
    s.push(Step::Wait(Duration::from_millis(1000)));
    s.push(Step::Shot("selection"));
    s.push(Step::Char('z', Modifiers::CTRL));
    s.push(Step::Message(Msg::LineNumbers(false)));
    s.push(Step::Message(Msg::Lang(Language::TypeScript)));
    s.push(Step::Wait(Duration::from_millis(300)));
    s.push(Step::Shot("undo-no-gutter-ts"));
    s.push(Step::Message(Msg::LineNumbers(true)));
    s.push(Step::Message(Msg::Lang(Language::JavaScript)));
    s.push(Step::Message(Msg::Vim(true)));
    s.push(click(at_char(2, 2)));
    s.push(Step::Wait(Duration::from_millis(1000)));
    s.push(Step::Shot("vim-normal"));
    s.extend(typed("dw"));
    s.extend(typed("iyield "));
    s.push(Step::Wait(Duration::from_millis(1000)));
    s.push(Step::Shot("vim-insert"));
    s.push(key(Named::Escape));
    s.extend(typed("ve"));
    s.push(Step::Wait(Duration::from_millis(1000)));
    s.push(Step::Shot("vim-visual"));
    let ctrl_shift_v = || Step::Char('v', Modifiers::CTRL | Modifiers::SHIFT);
    s.push(key(Named::Escape));
    s.extend(typed("gg0"));
    s.push(key(Named::Space));
    s.push(Step::Wait(Duration::from_millis(300)));
    s.push(Step::Shot("vim-leader-pending"));
    // The shot outlasts `timeoutlen`: Space has moved right (`l`) by now.
    s.extend(typed("0"));
    s.push(key(Named::Space));
    s.extend(typed("y"));
    s.push(Step::Wait(Duration::from_millis(300)));
    s.push(Step::Shot("vim-leader-y-pending"));
    s.extend(typed("yG"));
    s.push(ctrl_shift_v());
    s.push(Step::Wait(Duration::from_millis(1000)));
    s.push(Step::Shot("vim-leader-yy-ctrl-shift-v"));
    // `greet` over `name` and over `hello`: the register is kept.
    s.extend(typed("ggwyiwfnviw"));
    s.push(key(Named::Space));
    s.extend(typed("pjfhviw"));
    s.push(key(Named::Space));
    s.extend(typed("p"));
    s.push(Step::Wait(Duration::from_millis(1000)));
    s.push(Step::Shot("vim-visual-leader-p"));
    s.extend(typed("0fs"));
    s.push(key(Named::Space));
    s.extend(typed("yiwox "));
    s.push(ctrl_shift_v());
    s.push(Step::Char('c', Modifiers::CTRL));
    s.push(Step::Wait(Duration::from_millis(1000)));
    s.push(Step::Shot("vim-insert-paste-ctrl-c"));
    s
}
