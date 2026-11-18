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
//! | [`inputs`], [`overlay`], [`feedback`], [`data`], [`date`], [`dnd`], [`graph`] | Phase 2/3 component batches |
//! | [`thumb`], [`reveal`] | Images (thumbnail cache, off-thread decode, picture/vector), hover/focus-within reveal |
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
pub mod graph;
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
pub mod reveal;
pub mod scale;
pub mod scroller;
pub mod segmented;
pub mod spinner;
pub mod style;
pub mod surface;
pub mod switch;
pub mod thumb;
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
/// fix (wheel deltas, text hinting, gamma, backdrop blur, Wayland paste,
/// pointer hot-unplug, the clipboard thread surviving it).
#[doc(hidden)]
#[cfg(unix)]
pub const BATO_PATCHES: [&str; 6] = [
    winit::BATO_PATCH,
    cosmic_text::BATO_PATCH,
    cryoglyph::BATO_PATCH,
    iced_wgpu::BATO_PATCH,
    iced_winit::BATO_PATCH,
    smithay_clipboard::BATO_PATCH,
];

/// Windows: smithay-clipboard (Wayland) is not built, so five tripwires.
#[doc(hidden)]
#[cfg(not(unix))]
pub const BATO_PATCHES: [&str; 5] = [
    winit::BATO_PATCH,
    cosmic_text::BATO_PATCH,
    cryoglyph::BATO_PATCH,
    iced_wgpu::BATO_PATCH,
    iced_winit::BATO_PATCH,
];

// Linked for its `bundled` feature only (FreeType built from source).
use freetype_sys as _;

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(unix)]
    fn patched_crates_are_linked() {
        assert_eq!(
            super::BATO_PATCHES,
            [
                "winit",
                "cosmic-text",
                "cryoglyph",
                "iced_wgpu",
                "iced_winit",
                "smithay-clipboard"
            ]
        );
    }

    /// The winit patch: NumLock picks what a keypad key is, so iced's
    /// `KeyPressed::key` (winit's `key_without_modifiers`) is a digit with
    /// NumLock on and a navigation key with it off, like the logical key.
    #[test]
    #[cfg(unix)]
    fn keypad_follows_num_lock() {
        use winit::bato_keypad::{key_without_modifiers_keysym as kwm, probe};
        use winit::keyboard::{Key, KeyLocation, NamedKey};
        let (kp_left, kp_4, kp_delete, kp_subtract) = (0xff96, 0xffb4, 0xff9f, 0xffad);
        assert_eq!(kwm(kp_left, kp_4), kp_4, "NumLock on");
        assert_eq!(kwm(kp_left, kp_left), kp_left, "NumLock off");
        assert_eq!(kwm(kp_delete, 0x2c), 0x2c, "a comma keypad decimal");
        assert_eq!(kwm(kp_subtract, 0x1008fe23), kp_subtract, "Ctrl+Alt+KP-");
        assert_eq!(kwm(0x34, 0x24), 0x34, "Shift+4 on the main row is still 4");

        // The real xkb path (libxkbcommon + xkeyboard-config); skipped
        // where they are missing. Evdev codes: KEY_4 5, KEY_KP4 75,
        // KEY_KP6 77, KEY_KP0 82, KEY_KPDOT 83, KEY_KPENTER 96,
        // KEY_KPPLUS 78, KEY_KPMINUS 74.
        if probe("us", 5, false, false).is_none() {
            eprintln!("skipped the xkb probe: no libxkbcommon or keymap");
            return;
        }
        let ch = |s: &str| Key::Character(s.into());
        let named = Key::Named;
        let k = |code, num, shift| {
            let p = probe("us", code, num, shift).unwrap();
            (p.key_without_modifiers, p.logical_key, p.text)
        };
        let t = |s: &str| Some(s.to_string());
        assert_eq!(k(75, true, false), (ch("4"), ch("4"), t("4")));
        assert_eq!(k(77, true, false), (ch("6"), ch("6"), t("6")));
        assert_eq!(k(82, true, false), (ch("0"), ch("0"), t("0")));
        assert_eq!(k(83, true, false), (ch("."), ch("."), t(".")));
        let left = named(NamedKey::ArrowLeft);
        assert_eq!(k(75, false, false), (left.clone(), left, None));
        // Shift with NumLock on yields whatever the keymap's keypad type
        // says (xkeyboard-config: the digit, or the arrow with the "Shift
        // releases NumLock" option); the key always agrees with it.
        let (key, logical, _) = k(75, true, true);
        assert_eq!(key, logical);
        let del = named(NamedKey::Delete);
        assert_eq!(k(83, false, false).0, del);
        let enter = named(NamedKey::Enter);
        assert_eq!(k(96, true, false).0, enter);
        assert_eq!(k(96, false, false).0, enter);
        for num in [true, false] {
            assert_eq!(k(78, num, false), (ch("+"), ch("+"), t("+")));
            assert_eq!(k(74, num, false), (ch("-"), ch("-"), t("-")));
        }
        // The main row is untouched: NumLock does nothing, Shift is dropped.
        assert_eq!(k(5, true, false), (ch("4"), ch("4"), t("4")));
        assert_eq!(k(5, false, true), (ch("4"), ch("$"), t("$")));
        assert_eq!(
            probe("us", 75, true, false).unwrap().location,
            KeyLocation::Numpad
        );
        // A comma keypad decimal (German) types its comma.
        if let Some(p) = probe("de", 83, true, false) {
            assert_eq!(p.key_without_modifiers, ch(","));
        }
    }
}
