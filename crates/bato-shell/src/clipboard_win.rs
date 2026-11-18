//! Windows file clipboard: `CF_HDROP` (a `DROPFILES` block of UTF-16 paths),
//! `Preferred DropEffect` (copy or move, as Explorer writes it) and the
//! paths as `CF_UNICODETEXT`.

use std::path::PathBuf;

use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardFormatNameW, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Ole::{CF_HDROP, CF_UNICODETEXT};
use windows_sys::Win32::UI::Shell::{DROPFILES, DragQueryFileW, HDROP};

use super::{Backend, ClipboardError, Files, Op};
use crate::win::{from_wide, wide};

const DROPEFFECT_COPY: u32 = 1;
const DROPEFFECT_MOVE: u32 = 2;

fn drop_effect_format() -> u32 {
    let name = wide("Preferred DropEffect");
    // SAFETY: a NUL-terminated name.
    unsafe { RegisterClipboardFormatW(name.as_ptr()) }
}

/// The clipboard, open for this scope (retried briefly: another app may
/// hold it for a moment).
struct Open;

impl Open {
    fn new() -> Result<Self, ClipboardError> {
        for _ in 0..20 {
            // SAFETY: no owner window; closed in Drop.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Ok(Open);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Err(ClipboardError::Failed("the clipboard is busy".into()))
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: we opened it.
        unsafe { CloseClipboard() };
    }
}

/// A movable global block holding `bytes`, for `SetClipboardData`.
fn global(bytes: &[u8]) -> Result<HGLOBAL, ClipboardError> {
    // SAFETY: allocate, copy within bounds, unlock.
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        if h.is_null() {
            return Err(ClipboardError::Failed("out of memory".into()));
        }
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            GlobalFree(h);
            return Err(ClipboardError::Failed("GlobalLock failed".into()));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        GlobalUnlock(h);
        Ok(h)
    }
}

fn set(format: u32, bytes: &[u8]) -> Result<(), ClipboardError> {
    let h = global(bytes)?;
    // SAFETY: on success the system owns `h`; otherwise we free it.
    unsafe {
        if SetClipboardData(format, h as HANDLE).is_null() {
            GlobalFree(h);
            return Err(ClipboardError::Failed("SetClipboardData failed".into()));
        }
    }
    Ok(())
}

/// The `DROPFILES` header followed by the NUL-separated, double-NUL-ended
/// UTF-16 paths.
pub(super) fn hdrop_bytes(paths: &[PathBuf]) -> Vec<u8> {
    let header = std::mem::size_of::<DROPFILES>();
    let mut out = vec![0u8; header];
    // pFiles (offset of the list), then pt and fNC, then fWide = 1.
    out[0..4].copy_from_slice(&(header as u32).to_le_bytes());
    out[header - 4..header].copy_from_slice(&1u32.to_le_bytes());
    for p in paths {
        for u in wide(p.as_os_str()) {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }
    out.extend_from_slice(&[0, 0]);
    out
}

/// Put `paths` on the clipboard as files.
pub fn copy_files(paths: &[PathBuf], op: Op) -> Result<Backend, ClipboardError> {
    if paths.is_empty() {
        return Err(ClipboardError::Failed("nothing to copy".into()));
    }
    let _open = Open::new()?;
    // SAFETY: the clipboard is open.
    unsafe { EmptyClipboard() };
    set(CF_HDROP as u32, &hdrop_bytes(paths))?;
    let effect = if op == Op::Cut {
        DROPEFFECT_MOVE
    } else {
        DROPEFFECT_COPY
    };
    set(drop_effect_format(), &effect.to_le_bytes())?;
    let text = paths
        .iter()
        .map(|p| p.to_string_lossy())
        .collect::<Vec<_>>()
        .join("\r\n");
    let text: Vec<u8> = wide(text).iter().flat_map(|u| u.to_le_bytes()).collect();
    set(CF_UNICODETEXT as u32, &text)?;
    Ok(Backend::Win32)
}

/// Files on the clipboard, `None` when it holds something else.
pub fn paste_files() -> Result<Option<Files>, ClipboardError> {
    let _open = Open::new()?;
    // SAFETY: the clipboard is open; handles are only read while it is.
    unsafe {
        if IsClipboardFormatAvailable(CF_HDROP as u32) == 0 {
            return Ok(None);
        }
        let h = GetClipboardData(CF_HDROP as u32) as HDROP;
        if h.is_null() {
            return Ok(None);
        }
        let n = DragQueryFileW(h, u32::MAX, std::ptr::null_mut(), 0);
        let mut paths = Vec::new();
        for i in 0..n {
            let len = DragQueryFileW(h, i, std::ptr::null_mut(), 0) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(h, i, buf.as_mut_ptr(), buf.len() as u32);
            paths.push(PathBuf::from(from_wide(&buf)));
        }
        let mut op = Op::Copy;
        let effect = GetClipboardData(drop_effect_format());
        if !effect.is_null() {
            let p = GlobalLock(effect as HGLOBAL) as *const u32;
            if !p.is_null() {
                if p.read_unaligned() & DROPEFFECT_MOVE != 0 {
                    op = Op::Cut;
                }
                GlobalUnlock(effect as HGLOBAL);
            }
        }
        Ok((!paths.is_empty()).then_some(Files {
            op,
            paths,
            other: Vec::new(),
        }))
    }
}

/// The formats on the clipboard now: `CF_HDROP` as `text/uri-list`,
/// `CF_UNICODETEXT` as `text/plain;charset=utf-8`, registered formats by
/// name, others as `#<id>`.
pub fn types() -> Result<Vec<String>, ClipboardError> {
    let _open = Open::new()?;
    let mut out = Vec::new();
    let mut f = 0;
    loop {
        // SAFETY: the clipboard is open.
        f = unsafe { EnumClipboardFormats(f) };
        if f == 0 {
            break;
        }
        out.push(match f {
            x if x == CF_HDROP as u32 => super::URI_LIST.to_string(),
            x if x == CF_UNICODETEXT as u32 => super::TEXT_TYPES[0].to_string(),
            x => {
                let mut buf = [0u16; 128];
                // SAFETY: writes at most the buffer's length.
                let n = unsafe { GetClipboardFormatNameW(x, buf.as_mut_ptr(), buf.len() as i32) };
                if n > 0 {
                    from_wide(&buf)
                } else {
                    format!("#{x}")
                }
            }
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdrop_layout() {
        let b = hdrop_bytes(&[PathBuf::from("C:\\a"), PathBuf::from("D:\\b")]);
        let header = std::mem::size_of::<DROPFILES>();
        assert_eq!(
            u32::from_le_bytes(b[0..4].try_into().unwrap()) as usize,
            header
        );
        assert_eq!(
            u32::from_le_bytes(b[header - 4..header].try_into().unwrap()),
            1
        );
        // "C:\a\0D:\b\0\0" in UTF-16.
        assert_eq!(b.len(), header + (5 + 5 + 1) * 2);
    }
}
