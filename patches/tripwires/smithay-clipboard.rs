
/// bato-native tripwire: this crate is the patched copy from bato-native/patched/smithay-clipboard
/// (no panic on events of a released wl_pointer). A dependent can reference it, so a build that silently
/// falls back to the unpatched crates.io release fails to compile instead of losing the fix.
#[doc(hidden)]
pub const BATO_PATCH: &str = "smithay-clipboard";
