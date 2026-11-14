//! `CodeEditor` (`inputs/CodeEditor.tsx` + `CodeEditorInner.tsx`,
//! `lib/cmTheme.ts`): CodeMirror 6 with `basicSetup`, the theme built from
//! the app tokens, `@replit/codemirror-vim`. Feature `editor`.
//!
//! Built on **cosmic-text's own editor stack**, the crate the kit already
//! patches (FreeType light hinting), so the editor's text is rasterized like
//! every `Label`: `SyntaxEditor` (syntect highlighting) for CodeMirror's
//! keys, `ViEditor` (modit) for vim mode. iced's `text_editor` was the
//! fallback; it has no gutter, no active line and no vim, and its text is
//! placed by cosmic-text's centring instead of Blink's baseline model.
//!
//! - Scrolling is the kit [`scroller`](crate::scroller) (wheel, touchpad,
//!   touch, BatoScroll autoscroll, keyboard scrolling of the page); the caret
//!   and mouse drags scroll it with [`reveal`](crate::scroller::reveal).
//! - Syntax sets load lazily, on the first editor layout ([`syntax`]).
//! - The app owns a [`Content`] (like iced's `text_editor::Content`); the
//!   widget edits it in place and reports the text through `on_change`.
//!
//! ```ignore
//! code_editor(&cx, &state.code)
//!     .language(Language::JavaScript)
//!     .line_numbers(true)
//!     .vim(false)
//!     .height(224.0)
//!     .on_change(Msg::Code)
//! ```

mod engine;
mod status;
pub mod syntax;
pub mod text;
mod view;

use std::cell::RefCell;

use iced::advanced::graphics::text::font_system;
use iced::widget::{column, container};
use iced::{Length, Padding};

pub use engine::{Cmd, LINE_HEIGHT, Move};
pub use syntax::Language;

use crate::El;
use crate::ctx::Ctx;
use crate::field::field;
use crate::style::c;
use crate::surface::{BoxStyle, surface};

/// The editor's document (text, cursor, selection, history, vim state).
/// Keep it in the app state; read it with [`Content::text`].
pub struct Content(RefCell<engine::Doc>);

impl Content {
    pub fn new(text: &str) -> Self {
        Self(RefCell::new(engine::Doc::new(text)))
    }
    pub fn text(&self) -> String {
        self.0.borrow().text()
    }
    pub fn line_count(&self) -> usize {
        self.0.borrow().line_count()
    }
    /// Replace the text (an outside `value` change); the undo history
    /// starts over.
    pub fn set_text(&self, text: &str) {
        let mut fs = font_system().write().expect("font system");
        self.0.borrow_mut().set_text(fs.raw(), text);
    }
    /// The vim mode label of the status bar (`--NORMAL--`, …), if vim is on.
    pub fn vim_status(&self) -> Option<String> {
        status::mode_label(&self.0.borrow())
    }
}

impl Default for Content {
    fn default() -> Self {
        Self::new("")
    }
}

impl std::fmt::Debug for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Content")
            .field("lines", &self.line_count())
            .finish()
    }
}

/// `height` default: `16rem`.
pub const DEFAULT_HEIGHT: f32 = 256.0;

pub struct CodeEditor<'a, M> {
    cx: Ctx,
    content: &'a Content,
    language: Language,
    line_numbers: bool,
    vim: bool,
    read_only: bool,
    height: f32,
    placeholder: Option<String>,
    label: Option<String>,
    hint: Option<String>,
    on_change: Option<Box<dyn Fn(String) -> M + 'a>>,
    id: Option<iced::widget::Id>,
}

pub fn code_editor<'a, M>(cx: &Ctx, content: &'a Content) -> CodeEditor<'a, M> {
    CodeEditor {
        cx: cx.clone(),
        content,
        language: Language::Text,
        line_numbers: true,
        vim: false,
        read_only: false,
        height: DEFAULT_HEIGHT,
        placeholder: None,
        label: None,
        hint: None,
        on_change: None,
        id: None,
    }
}

impl<'a, M> CodeEditor<'a, M> {
    pub fn language(mut self, l: Language) -> Self {
        self.language = l;
        self
    }
    /// The line-number gutter (default on).
    pub fn line_numbers(mut self, on: bool) -> Self {
        self.line_numbers = on;
        self
    }
    /// Vim keybindings: modal editing and the status bar.
    pub fn vim(mut self, on: bool) -> Self {
        self.vim = on;
        self
    }
    pub fn read_only(mut self, on: bool) -> Self {
        self.read_only = on;
        self
    }
    /// The editor's height in px (`height`; the status bar is inside it).
    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }
    pub fn placeholder(mut self, p: impl Into<String>) -> Self {
        self.placeholder = Some(p.into());
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
    /// Called with the whole text after each change (`onChange`).
    pub fn on_change(mut self, f: impl Fn(String) -> M + 'a) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }
    /// For `focus::focus(id)`.
    pub fn id(mut self, id: impl Into<iced::widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl<'a, M: Clone + 'a> From<CodeEditor<'a, M>> for El<'a, M> {
    fn from(e: CodeEditor<'a, M>) -> Self {
        let cx = e.cx.on(e.cx.p().surface);
        let p = cx.p().clone();
        let bar = if e.vim { status::HEIGHT } else { 0.0 };
        // The `rounded-lg border border-app-control` box, 1 px border.
        let inner_h = (e.height - bar).max(0.0);
        let doc = view::DocView {
            content: e.content,
            palette: p.clone(),
            lang: e.language,
            vim: e.vim,
            line_numbers: e.line_numbers,
            read_only: e.read_only,
            placeholder: e.placeholder,
            on_change: e.on_change,
            id: e.id,
            view_h: inner_h,
            round_bottom: !e.vim,
        };
        let scroll = crate::scroller::scroll_area(&cx, doc)
            .both()
            .width(Length::Fill)
            .height(inner_h)
            .backdrop(c(p.surface));
        let mut body = column![scroll].width(Length::Fill);
        if e.vim {
            body = body.push(status::StatusBar {
                content: e.content,
                palette: p.clone(),
                cache: Default::default(),
            });
        }
        let boxed = surface(container(body).width(Length::Fill))
            .fixed(
                BoxStyle::default()
                    .bg(c(p.surface))
                    .border(c(e.cx.p().control))
                    .radius(8.0),
            )
            .padding(Padding::new(1.0))
            .width(Length::Fill)
            .clip();
        let mut f = field(&e.cx, boxed);
        if let Some(l) = e.label {
            f = f.label(l);
        }
        if let Some(h) = e.hint {
            f = f.hint(h);
        }
        f.into()
    }
}

#[cfg(test)]
mod tests;
