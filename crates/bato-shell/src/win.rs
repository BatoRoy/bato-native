//! Win32 helpers shared by the Windows halves of the modules.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
};

/// `CREATE_NO_WINDOW`: a console program (a Go server, a helper CLI) starts
/// without flashing a console window (Electron's `windowsHide`).
#[cfg_attr(not(any(test, feature = "supervisor")), allow(dead_code))]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`: the child outlives this
/// process and gets no console and no Ctrl+C of ours (`setsid`).
pub const DETACHED_FLAGS: u32 = 0x0000_0008 | 0x0000_0200;

/// A NUL-terminated UTF-16 copy of `s`, for `*W` functions.
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// UTF-16 up to the first NUL.
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Restore `hwnd` if it is minimised and bring it to the front.
pub fn raise(hwnd: HWND) -> bool {
    if hwnd.is_null() {
        return false;
    }
    // SAFETY: plain window calls; a stale handle only makes them fail.
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_round_trip() {
        let w = wide("C:\\Users\\Ünï");
        assert_eq!(*w.last().unwrap(), 0);
        assert_eq!(from_wide(&w), "C:\\Users\\Ünï");
    }
}
