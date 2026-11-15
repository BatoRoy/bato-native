
/// bato-native tripwire: this crate is the patched copy from bato-native/patched/winit
/// (value120 wheel deltas on Wayland (iced winit fork) + smol_str 0.2 + Wayland file drops
/// + NumLock-aware keypad keys + the clipboard selection through the file-drop data device
/// + OS drags of files from a window on Wayland, `bato_drag`). bato-kit references it, so a build that silently falls back to
/// the unpatched crates.io release fails to compile instead of losing the fix.
#[doc(hidden)]
pub const BATO_PATCH: &str = "winit";
