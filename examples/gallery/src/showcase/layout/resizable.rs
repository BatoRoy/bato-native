//! `layout/resizable`: Resizable panels (nested). A horizontal PanelGroup
//! (Sidebar 28 %, min 15) whose second panel is a vertical one (Editor,
//! Terminal 32 %, min 15). Sizes persist in the gallery's UI-state store
//! (`demo-panels-h` / `demo-panels-v`, React's `storageKey`s).

use bato_kit::iced::widget::{column, container};
use bato_kit::kv::UiState;
use bato_kit::layout::{Direction, initial_sizes, panel_group, rounded_clip};
use bato_kit::prelude::*;

use crate::showcase::Effect;

/// The gallery's config dir (`main.rs` `APP.slug`).
pub const SLUG: &str = "batopoc-gallery";
pub const KEY_H: &str = "demo-panels-h";
pub const KEY_V: &str = "demo-panels-v";
/// The panel box, for the self-test.
pub const BOX: &str = "layout/resizable/box";

const DEFAULT_H: [Option<f32>; 2] = [Some(28.0), None];
const DEFAULT_V: [Option<f32>; 2] = [None, Some(32.0)];

#[derive(Debug)]
pub struct State {
    pub h: Vec<f32>,
    pub v: Vec<f32>,
    kv: UiState,
}

impl State {
    pub fn with_store(kv: UiState) -> Self {
        let h = initial_sizes(&DEFAULT_H, kv.get::<Vec<f32>>(KEY_H).as_deref());
        let v = initial_sizes(&DEFAULT_V, kv.get::<Vec<f32>>(KEY_V).as_deref());
        Self { h, v, kv }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::with_store(UiState::load(SLUG))
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    H(Vec<f32>),
    V(Vec<f32>),
    SaveH(Vec<f32>),
    SaveV(Vec<f32>),
}

pub fn update(state: &mut State, msg: Msg) -> Option<Effect> {
    let saved = match msg {
        Msg::H(s) => {
            state.h = s;
            Ok(())
        }
        Msg::V(s) => {
            state.v = s;
            Ok(())
        }
        Msg::SaveH(s) => {
            state.h = s;
            state.kv.set(KEY_H, &state.h)
        }
        Msg::SaveV(s) => {
            state.v = s;
            state.kv.set(KEY_V, &state.v)
        }
    };
    if let Err(e) = saved {
        eprintln!("ui-state: {e}");
    }
    None
}

pub fn view<'a>(state: &'a State, cx: &Ctx) -> El<'a, Msg> {
    let on = cx.on_card();
    let p = on.p();
    let pane = |text: &str, bg: Option<bato_kit::theme::Rgb>| -> El<'a, Msg> {
        let s = surface(label(text.to_string(), 14.0, 20.0).color(c(p.subtext)))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(12);
        match bg {
            Some(bg) => s.fixed(BoxStyle::default().bg(c(bg))).into(),
            None => s.into(),
        }
    };
    let inner = panel_group(&on, Direction::Vertical, &state.v)
        .panel(pane("Editor", None), 5.0)
        .panel(pane("Terminal", Some(p.surface)), 15.0)
        .on_resize(Msg::V)
        .on_commit(Msg::SaveV);
    let outer = panel_group(&on, Direction::Horizontal, &state.h)
        .panel(pane("Sidebar", Some(p.surface)), 15.0)
        .panel(inner, 5.0)
        .on_resize(Msg::H)
        .on_commit(Msg::SaveH);
    let boxed = rounded_clip(8.0, c(on.over), outer)
        .border(c(p.border))
        .height(256);
    card(cx, "Resizable panels (nested)")
        .body(column![
            container(boxed).id(BOX),
            vgap(8.0),
            label("Drag the dividers — sizes persist across reloads.", 12.0, 16.0)
                .color(c(p.muted)),
        ])
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_persist_on_commit_only() {
        let dir = std::env::temp_dir().join(format!("gallery-panels-{}", std::process::id()));
        let file = dir.join("ui-state.json");
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = State::with_store(UiState::at(&file));
        assert_eq!(s.h, vec![28.0, 72.0]);
        assert_eq!(s.v, vec![68.0, 32.0]);
        update(&mut s, Msg::H(vec![40.0, 60.0]));
        assert!(!file.exists(), "moves are not written");
        update(&mut s, Msg::SaveH(vec![41.0, 59.0]));
        let again = State::with_store(UiState::at(&file));
        assert_eq!(again.h, vec![41.0, 59.0]);
        assert_eq!(again.v, vec![68.0, 32.0]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
