//! `layout/editor-tabs`: Editor tabs (drag to reorder, close, add). The
//! strip over an "Active tab:" line, in a `rounded-lg border
//! overflow-hidden` box.

use bato_kit::dnd::array_move;
use bato_kit::iced::widget::column;
use bato_kit::label::Span;
use bato_kit::layout::{EditorTab, editor_tabs, rounded_clip};
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// The box, for the self-test.
pub const BOX: &str = "layout/editor-tabs/box";

#[derive(Debug)]
pub struct State {
    pub tabs: Vec<EditorTab>,
    pub active: String,
    /// Ids for new tabs (React uses `Date.now()`).
    next: u32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            tabs: vec![
                EditorTab::new("a", "index.tsx").icon(Ico::FileCode),
                EditorTab::new("b", "styles.css")
                    .icon(Ico::FileCode)
                    .dirty(true),
                EditorTab::new("c", "README.md").icon(Ico::FileText),
            ],
            active: "a".into(),
            next: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Activate(String),
    Close(String),
    Reorder(usize, usize),
    Add,
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Activate(id) => state.active = id,
        Msg::Close(id) => state.tabs.retain(|t| t.id != id),
        Msg::Reorder(from, to) => array_move(&mut state.tabs, from, to),
        Msg::Add => {
            // React's counter is a render-local `let added = 0`, so every
            // new tab is `untitled-1`.
            state.next += 1;
            let id = format!("new{}", state.next);
            state
                .tabs
                .push(EditorTab::new(id.clone(), "untitled-1").icon(Ico::FileCode));
            state.active = id;
        }
    }
    None
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    let strip = editor_tabs(&on, &state.tabs, &state.active, Msg::Activate)
        .on_close(Msg::Close)
        .on_reorder(Msg::Reorder)
        .on_add(Msg::Add);
    let name = state
        .tabs
        .iter()
        .find(|t| t.id == state.active)
        .map_or("—".to_string(), |t| t.label.clone());
    let line = rich(
        vec![
            Span::new("Active tab: ").color(c(p.subtext)),
            Span::new(name).mono().size(12.0).color(c(p.text)),
        ],
        14.0,
        20.0,
    );
    // The 12 px mono span (line-height 1.6) on the 14 / 20 line makes
    // Chromium's line box 21 px: the extra pixel lands below the text.
    let panel = surface(line)
        .padding(Padding {
            top: 16.0,
            right: 16.0,
            bottom: 17.0,
            left: 16.0,
        })
        .width(Length::Fill)
        .fixed(BoxStyle::default().bg(c(p.bg)));
    let boxed = rounded_clip(8.0, c(on.over), column![strip, panel]).border(c(p.border));
    card(cx, "Editor tabs (drag to reorder, close, add)")
        .body(container(boxed).id(BOX))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_close_reorder() {
        let mut s = State::default();
        update(&mut s, Msg::Add);
        update(&mut s, Msg::Add);
        assert_eq!(s.tabs.len(), 5);
        assert_eq!(s.tabs[4].label, "untitled-1");
        assert_eq!(s.active, "new2");
        update(&mut s, Msg::Close("b".into()));
        update(&mut s, Msg::Reorder(0, 3));
        let ids: Vec<&str> = s.tabs.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["c", "new1", "new2", "a"]);
    }
}
