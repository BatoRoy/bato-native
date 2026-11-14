//! **bato-kit**: the Hybrid widget kit for native Bato apps, on iced 0.14.
//!
//! Apps use iced only through [`iced`] (re-exported with the kit's feature
//! set: wgpu + tiny-skia, Wayland + X11, SVG, the advanced widget API).
//!
//! | Module | What |
//! |---|---|
//! | [`frame`] | The app frame: sidebar, page scroller, App Options, About, toasts, parameterized by a [`frame::Brand`] |
//! | [`ctx`] | [`Ctx`]: the theme plus the opaque colour a widget sits on |
//! | [`style`] | The palette bridge (tokens → iced colours, shadows) |
//! | [`surface`], [`label`], [`icon`] | The three building blocks: a CSS box, Chromium-placed text, lucide icons |
//! | [`button`], [`badge`], [`card`], [`kbd`], [`segmented`], [`checkbox`], [`switch`], [`input`], [`spinner`], [`empty`] | Components |
//! | [`focus`] | Keyboard focus: Tab order, `:focus-visible` rings, focus scopes (dialog trap), roving groups |
//! | [`scroller`], [`autoscroll`] | Chromium scrolling (wheel, touchpad, touch, keys, scrollbars, nesting, scroll-into-view, edge fade, virtual lists) and BatoScroll |
//! | [`field`] | The label/hint/error scaffold and the shared control box (`Field.tsx`, `controlClasses`) |
//! | [`modal`], [`blur`], [`toast`], [`rail_tip`], [`timer`] | The overlay stack (modals, drawers), toasts and timed things |
//! | [`floating`], [`overlay`], [`menu`], [`hotkeys`] | Anchored overlays (`floating.ts`), menus, keyboard shortcuts |
//! | [`layout`] | Grid with spans and tracks, AutoGrid, Masonry, AspectRatio, absolute placement, hairlines, page |
//! | [`anim`] | Easings, tweens, pulse, hold-repeat, group opacity |
//! | [`cursor`], [`datefmt`], [`kv`], [`fonts`], [`scale`] | Cursors, date formatting, UI-state store, bundled fonts, device pixels |
//! | [`inputs`], [`overlay`], [`feedback`], [`data`], [`date`], [`dnd`] | Phase 2/3 component batches |
//!
//! Sizes are CSS px at 100 %; the UI scale is iced's application scale
//! factor ([`frame::Frame::scale_factor`]).

pub use iced;
pub use {bato_theme as theme, scroll_physics};

pub mod anim;
pub mod autoscroll;
pub mod badge;
pub mod blur;
pub mod button;
pub mod card;
pub mod checkbox;
pub mod ctx;
pub mod cursor;
pub mod datefmt;
pub mod empty;
pub mod field;
pub mod floating;
pub mod focus;
pub mod fonts;
pub mod frame;
pub mod hotkeys;
pub mod icon;
pub mod input;
pub mod kbd;
pub mod kv;
pub mod label;
pub mod layout;
pub mod menu;
pub mod modal;
pub mod rail_tip;
pub mod scale;
pub mod scroller;
pub mod segmented;
pub mod spinner;
pub mod style;
pub mod surface;
pub mod switch;
pub mod timer;
pub mod toast;

// Phase 2/3 component batches (pre-declared so batches never share a file).
pub mod data;
pub mod date;
pub mod dnd;
#[cfg(feature = "editor")]
pub mod editor;
pub mod feedback;
pub mod inputs;
pub mod overlay;

pub use badge::Tone;
pub use ctx::Ctx;
pub use icon::Ico;

/// The element type every kit builder returns.
pub type El<'a, M> = iced::Element<'a, M, iced::Theme, iced::Renderer>;

/// Everything a page module usually needs.
pub mod prelude {
    pub use crate::anim::opacity;
    pub use crate::badge::{Tone, badge};
    pub use crate::button::{Variant, button};
    pub use crate::card::{card, card_box};
    pub use crate::checkbox::checkbox;
    pub use crate::ctx::Ctx;
    pub use crate::empty::empty_state;
    pub use crate::field::{FieldSize, field, text_control};
    pub use crate::focus::{Ring, autofocus, focus_scope, roving};
    pub use crate::icon::{Ico, icon};
    pub use crate::input::input;
    pub use crate::kbd::{app_mark, kbd};
    pub use crate::label::{Align, Span, label, rich};
    pub use crate::layout::{Grid, gap, grid, hline, page, vgap, vline};
    pub use crate::menu::{MenuEntry, action, menu_list, separator};
    pub use crate::overlay::{DismissReason, anchored, context_area};
    pub use crate::scroller::{Heights, scroll_area, virtual_list};
    pub use crate::segmented::{Segment, segmented, tabs};
    pub use crate::spinner::spinner;
    pub use crate::style::c;
    pub use crate::surface::{BoxStyle, Status, surface};
    pub use crate::switch::switch;
    pub use crate::{El, iced};
    pub use iced::widget::{Space, container};
    pub use iced::{Alignment, Length, Padding};
}

/// The patched-crate tripwires. Each patched crate (bato-native/patched)
/// exports `BATO_PATCH`; the crates.io releases do not, so a build that lost
/// the `[patch.crates-io]` block fails here instead of silently losing the
/// fix (wheel deltas, text hinting, gamma, backdrop blur).
#[doc(hidden)]
pub const BATO_PATCHES: [&str; 4] = [
    winit::BATO_PATCH,
    cosmic_text::BATO_PATCH,
    cryoglyph::BATO_PATCH,
    iced_wgpu::BATO_PATCH,
];

// Linked for its `bundled` feature only (FreeType built from source).
use freetype_sys as _;

#[cfg(test)]
mod tests {
    #[test]
    fn patched_crates_are_linked() {
        assert_eq!(
            super::BATO_PATCHES,
            ["winit", "cosmic-text", "cryoglyph", "iced_wgpu"]
        );
    }
}
