//! Files dropped anywhere in the window (`useWindowFileDrop`): the state,
//! the window-event subscription and the full-window "Drop files" overlay.
//!
//! The platform delivers one `FileDropped` per file; [`WindowDrop`] collects
//! them and hands the whole drop over on the next frame, so the app adds a
//! multi-file drop at once.
//!
//! ```ignore
//! // state
//! drop: WindowDrop,
//! // subscription
//! self.drop.subscription().map(Msg::Drop)
//! // update
//! Msg::Drop(m) => {
//!     let files = self.drop.update(m);
//!     if !files.is_empty() { self.add(files) }
//! }
//! // view: the topmost layer of the window's stack (input passes through)
//! if let Some(o) = self.drop.overlay(cx, "Drop files to add them", "Images and videos", animate) {
//!     layers.push(o);
//! }
//! ```

use std::path::PathBuf;
use std::time::Instant;

use iced::widget::{column, container, stack};
use iced::{Alignment, Length, Subscription, window};

use crate::El;
use crate::blur::backdrop_blur;
use crate::ctx::Ctx;
use crate::icon::{Ico, icon};
use crate::label::{Align, label};
use crate::overlay::context_menu::dashed;
use crate::style::{c, rgba, shadows};
use crate::surface::{BoxStyle, surface};

/// What the window reports about files dragged over it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropMsg {
    /// Files are over the window.
    Hovered,
    /// They left without a drop.
    Left,
    /// One dropped file.
    Dropped(PathBuf),
    /// The frame after a drop: hand the files over.
    Flush,
}

/// The window's file-drop state.
#[derive(Debug, Clone, Default)]
pub struct WindowDrop {
    /// Since when files hover over the window (the overlay's blur fades in
    /// from there).
    hovering: Option<Instant>,
    pending: Vec<PathBuf>,
}

impl WindowDrop {
    pub fn new() -> Self {
        Self::default()
    }

    /// Files are being dragged over the window.
    pub fn is_hovering(&self) -> bool {
        self.hovering.is_some()
    }

    /// The window events, and a frame while a drop is pending.
    pub fn subscription(&self) -> Subscription<DropMsg> {
        let events = window::events().filter_map(|(_, e)| match e {
            window::Event::FileHovered(_) => Some(DropMsg::Hovered),
            window::Event::FilesHoveredLeft => Some(DropMsg::Left),
            window::Event::FileDropped(p) => Some(DropMsg::Dropped(p)),
            _ => None,
        });
        if self.pending.is_empty() {
            events
        } else {
            Subscription::batch([events, window::frames().map(|_| DropMsg::Flush)])
        }
    }

    /// Apply a message; returns the dropped files once they are all in
    /// (empty otherwise).
    pub fn update(&mut self, msg: DropMsg) -> Vec<PathBuf> {
        match msg {
            DropMsg::Hovered => {
                self.hovering.get_or_insert_with(Instant::now);
            }
            DropMsg::Left => self.hovering = None,
            DropMsg::Dropped(p) => {
                self.hovering = None;
                if !self.pending.contains(&p) {
                    self.pending.push(p);
                }
            }
            DropMsg::Flush => return std::mem::take(&mut self.pending),
        }
        Vec::new()
    }

    /// The overlay while files hover over the window, else `None`.
    pub fn overlay<'a, M: Clone + 'a>(
        &self,
        cx: &Ctx,
        title: &str,
        hint: &str,
        animate: bool,
    ) -> Option<El<'a, M>> {
        self.hovering
            .map(|since| drop_overlay(cx, since, title, hint, animate))
    }
}

/// `fixed inset-0 bg-app-bg/80 backdrop-blur-sm`, and in the middle a card
/// (`rounded-2xl border-2 border-dashed border-app-accentBright bg-app-card
/// shadow-app-lg px-14 py-12`): an `Upload` badge (56 px, accent tint), the
/// title (16 px medium) and the hint (12 px muted). Input passes through.
pub fn drop_overlay<'a, M: Clone + 'a>(
    cx: &Ctx,
    since: Instant,
    title: &str,
    hint: &str,
    animate: bool,
) -> El<'a, M> {
    let on = cx.on_card();
    let p = on.p();
    let mut content = column![
        surface(icon(Ico::Upload, 26.0).color(c(p.accent_bright)))
            .width(56)
            .height(56)
            .center()
            .fixed(BoxStyle::default().bg(c(on.tint(0.15))).radius(28.0)),
        label(title.to_string(), 16.0, 24.0)
            .medium()
            .align(Align::Center)
            .color(c(p.text)),
    ]
    .spacing(12)
    .align_x(Alignment::Center);
    if !hint.is_empty() {
        content = content.push(
            label(hint.to_string(), 12.0, 16.0)
                .align(Align::Center)
                .color(c(p.muted)),
        );
    }
    // The dashes over the card's fill: the fill outside, the border inside.
    let card =
        surface(dashed(container(content).padding([48.0, 56.0]), c(p.accent_bright)).radius(16.0))
            .fixed(BoxStyle {
                bg: Some(c(p.card)),
                radius: 16.0,
                shadows: shadows(&p.shadow_lg),
                ..BoxStyle::default()
            });
    let bg = rgba(cx.p().bg, 0.8);
    stack![
        backdrop_blur(since, animate),
        container(card)
            .center(Length::Fill)
            .style(move |_| container::Style {
                background: Some(bg.into()),
                ..Default::default()
            }),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_multi_file_drop_arrives_at_once() {
        let mut d = WindowDrop::new();
        assert!(d.update(DropMsg::Hovered).is_empty());
        assert!(d.is_hovering());
        let since = d.hovering;
        d.update(DropMsg::Hovered);
        assert_eq!(d.hovering, since, "the overlay keeps its start");
        d.update(DropMsg::Dropped("/a.png".into()));
        assert!(!d.is_hovering());
        d.update(DropMsg::Dropped("/b.png".into()));
        d.update(DropMsg::Dropped("/b.png".into()));
        assert_eq!(
            d.update(DropMsg::Flush),
            vec![PathBuf::from("/a.png"), PathBuf::from("/b.png")]
        );
        assert!(d.update(DropMsg::Flush).is_empty());
        d.update(DropMsg::Hovered);
        d.update(DropMsg::Left);
        assert!(!d.is_hovering());
    }
}
