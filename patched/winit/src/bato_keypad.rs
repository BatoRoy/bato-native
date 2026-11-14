//! bato-native: NumLock-aware `key_without_modifiers` for keypad keys on X11
//! and Wayland (both go through `platform_impl/linux/common/xkb`).
//!
//! Upstream reads `key_without_modifiers` from shift level 1 of the key with
//! every modifier ignored. That is right for Shift, Ctrl, Alt and AltGr, but
//! NumLock is a lock that changes which key a keypad key *is*: level 1 of
//! `<KP4>` is `KP_Left`, so with NumLock on keypad 4 still reported
//! `Named(ArrowLeft)` while `logical_key` and the text were "4". iced fills
//! `keyboard::Event::KeyPressed::key` from `key_without_modifiers`, so every
//! text field moved the caret instead of typing the digit.
//!
//! For a key whose level-1 keysym is a keypad keysym, the key now reports
//! the keysym the xkb state yields for it (the level NumLock, and the Shift
//! that inverts it, select: `KP_4` / `KP_Left`, `KP_Decimal` / `KP_Delete`,
//! `KP_Separator` or `comma` on layouts whose keypad decimal is a comma), as
//! Chromium's `KeyboardEvent.key` does, provided that keysym is itself a
//! keypad keysym or a plain character. Every other key, and a keypad key
//! whose current keysym is something else (Ctrl+Alt+KP_Subtract's
//! `XF86Prev_VMode`), keeps level 1.

/// `true` for the `KP_*` keysyms (`KP_Space` 0xff80 to `KP_Equal` 0xffbd).
pub fn is_keypad(keysym: u32) -> bool {
    (0xff80..=0xffbd).contains(&keysym)
}

/// A keysym that is a printable character: Latin-1 or a Unicode keysym.
fn is_character(keysym: u32) -> bool {
    matches!(keysym, 0x20..=0x7e | 0xa0..=0xff) || (0x0100_00a0..=0x0110_ffff).contains(&keysym)
}

/// The keysym `key_without_modifiers` reports for a key whose level-1
/// keysym is `base` and whose keysym under the current xkb state (NumLock
/// included) is `current`.
pub fn key_without_modifiers_keysym(base: u32, current: u32) -> u32 {
    if is_keypad(base) && (is_keypad(current) || is_character(current)) {
        current
    } else {
        base
    }
}

/// What one key press turned into on the real xkb path (`process_key_event`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub key_without_modifiers: crate::keyboard::Key,
    pub logical_key: crate::keyboard::Key,
    pub text: Option<String>,
    pub location: crate::keyboard::KeyLocation,
}

/// Press the key with Linux evdev code `evdev` on the xkb `layout` ("us",
/// "de", rules `evdev`, model `pc105`, no options, environment ignored) with
/// NumLock locked or not and Shift held or not, through the same
/// `process_key_event` X11 and Wayland use. `None` when libxkbcommon or the
/// keymap is not available. For tests.
#[cfg(any(x11_platform, wayland_platform))]
pub fn probe(layout: &str, evdev: u32, num_lock: bool, shift: bool) -> Option<Probe> {
    crate::platform_impl::common::xkb::bato_probe(layout, evdev, num_lock, shift)
}
