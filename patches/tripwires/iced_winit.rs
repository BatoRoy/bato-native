
/// bato-native tripwire: this crate is the patched copy from bato-native/patched/iced_winit
/// (Wayland clipboard reads through winit's data device, for Hyprland). bato-kit references it, so a build that silently falls back to
/// the unpatched crates.io release fails to compile instead of losing the fix.
#[doc(hidden)]
pub const BATO_PATCH: &str = "iced_winit";
