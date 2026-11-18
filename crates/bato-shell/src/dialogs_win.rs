//! Windows file dialogs: the common item dialog (`IFileOpenDialog`,
//! `IFileSaveDialog`), each on its own single-threaded-apartment thread so
//! the app's event loop keeps running while it is open. Modal to the parent
//! window when there is one (the dialog disables its owner, as Electron's
//! does).

use std::path::PathBuf;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileDialog, IFileOpenDialog, IShellItem,
    SIGDN_FILESYSPATH,
};
use windows::core::{HSTRING, Interface, PCWSTR};

use super::{DialogError, FileFilter, OpenDirectory, OpenFiles, Parent, SaveFile, SaveTarget};

/// `HRESULT_FROM_WIN32(ERROR_CANCELLED)`: the user closed the dialog.
const CANCELLED: i32 = 0x8007_04C7_u32 as i32;

/// Run `f` on a fresh COM thread; its result as a future.
pub(super) async fn run<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, DialogError> + Send + 'static,
) -> Result<T, DialogError> {
    let (tx, rx) = futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name("bato-dialog".into())
        .spawn(move || {
            // SAFETY: once per thread, balanced by CoUninitialize below.
            let init =
                unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
            let r = if init.is_err() {
                Err(DialogError::Unavailable(format!("COM: {init:?}")))
            } else {
                let r = f();
                // SAFETY: the dialog objects were dropped inside `f`.
                unsafe { CoUninitialize() };
                r
            };
            let _ = tx.send(r);
        })
        .map_err(|e| DialogError::Failed(e.to_string()))?;
    rx.await
        .unwrap_or_else(|_| Err(DialogError::Failed("the dialog thread stopped".into())))
}

fn failed(e: windows::core::Error) -> DialogError {
    DialogError::Failed(e.message())
}

fn owner(p: &Parent) -> HWND {
    match p {
        Parent::Win32(h) => HWND(*h as *mut _),
        _ => HWND(std::ptr::null_mut()),
    }
}

/// `("Images", "*.png;*.jpg")` pairs; `*` → `*.*`.
pub(super) fn specs(filters: &[FileFilter]) -> Vec<(HSTRING, HSTRING)> {
    filters
        .iter()
        .filter_map(|f| {
            let globs: Vec<String> = f
                .globs()
                .into_iter()
                .map(|g| if g == "*" { "*.*".into() } else { g })
                .collect();
            (!globs.is_empty()).then(|| (HSTRING::from(&f.name), HSTRING::from(globs.join(";"))))
        })
        .collect()
}

/// Title and filters, then show it. `Ok(false)` when cancelled.
fn show(
    d: &IFileDialog,
    title: &str,
    filters: &[FileFilter],
    parent: &Parent,
) -> Result<bool, DialogError> {
    let specs = specs(filters);
    let raw: Vec<COMDLG_FILTERSPEC> = specs
        .iter()
        .map(|(n, s)| COMDLG_FILTERSPEC {
            pszName: PCWSTR(n.as_ptr()),
            pszSpec: PCWSTR(s.as_ptr()),
        })
        .collect();
    // SAFETY: COM calls on a live dialog; `specs` outlives `raw`'s use.
    unsafe {
        if !title.is_empty() {
            d.SetTitle(&HSTRING::from(title)).map_err(failed)?;
        }
        if !raw.is_empty() {
            d.SetFileTypes(&raw).map_err(failed)?;
        }
        match d.Show(owner(parent)) {
            Ok(()) => Ok(true),
            Err(e) if e.code().0 == CANCELLED => Ok(false),
            Err(e) => Err(failed(e)),
        }
    }
}

fn item_path(item: &IShellItem) -> Result<PathBuf, DialogError> {
    // SAFETY: the returned string is CoTaskMemAlloc'd; freed after copying.
    unsafe {
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(failed)?;
        let s = p
            .to_string()
            .map_err(|e| DialogError::Failed(e.to_string()));
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

pub(super) fn open_directory(o: &OpenDirectory) -> Result<Option<PathBuf>, DialogError> {
    // SAFETY: COM on this thread's apartment.
    unsafe {
        let d: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(failed)?;
        let opts = d.GetOptions().map_err(failed)? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM;
        d.SetOptions(opts).map_err(failed)?;
        if !show(&d.cast().map_err(failed)?, &o.title, &[], &o.parent)? {
            return Ok(None);
        }
        item_path(&d.GetResult().map_err(failed)?).map(Some)
    }
}

// Files: the classic `GetOpenFileNameW` / `GetSaveFileNameW` without hooks,
// which Windows shows as the same modern dialog, and which Wine implements
// fully (its common item dialog cannot always be created). Folders keep the
// common item dialog (`FOS_PICKFOLDERS`), as Electron does.

/// `Name\0*.a;*.b\0…\0\0`, or `None` without filters.
pub(super) fn filter_string(filters: &[FileFilter]) -> Option<Vec<u16>> {
    let specs = specs(filters);
    if specs.is_empty() {
        return None;
    }
    let mut out: Vec<u16> = Vec::new();
    for (n, s) in specs {
        out.extend(n.as_wide());
        out.push(0);
        out.extend(s.as_wide());
        out.push(0);
    }
    out.push(0);
    Some(out)
}

/// Split a multi-select answer: `dir\0a\0b\0\0`, or `path\0\0` for one.
pub(super) fn split_multi(buf: &[u16]) -> Vec<PathBuf> {
    let parts: Vec<String> = buf
        .split(|&c| c == 0)
        .take_while(|p| !p.is_empty())
        .map(String::from_utf16_lossy)
        .collect();
    match parts.as_slice() {
        [] => Vec::new(),
        [one] => vec![PathBuf::from(one)],
        [dir, files @ ..] => files.iter().map(|f| PathBuf::from(dir).join(f)).collect(),
    }
}

const BUF: usize = 64 * 1024;

fn classic(
    title: &str,
    filters: &[FileFilter],
    parent: &Parent,
    flags: u32,
    initial_name: Option<&str>,
    initial_dir: Option<&std::path::Path>,
    save: bool,
) -> Result<Option<Vec<u16>>, DialogError> {
    use windows_sys::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, GetOpenFileNameW, GetSaveFileNameW, OPENFILENAMEW,
    };
    let mut buf = vec![0u16; BUF];
    if let Some(n) = initial_name {
        let w: Vec<u16> = n.encode_utf16().take(BUF - 1).collect();
        buf[..w.len()].copy_from_slice(&w);
    }
    let filter = filter_string(filters);
    let title_w = crate::win::wide(title);
    let dir_w = initial_dir.map(|d| crate::win::wide(d.as_os_str()));
    let def_ext = filters
        .first()
        .and_then(|f| f.globs().into_iter().next())
        .and_then(|g| g.strip_prefix("*.").map(crate::win::wide));
    // SAFETY: a zeroed OPENFILENAMEW is valid; every pointer outlives the call.
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = match parent {
        Parent::Win32(h) => *h as _,
        _ => std::ptr::null_mut(),
    };
    ofn.lpstrFilter = filter.as_ref().map_or(std::ptr::null(), |f| f.as_ptr());
    ofn.nFilterIndex = 1;
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = BUF as u32;
    ofn.lpstrTitle = if title.is_empty() {
        std::ptr::null()
    } else {
        title_w.as_ptr()
    };
    ofn.lpstrInitialDir = dir_w.as_ref().map_or(std::ptr::null(), |d| d.as_ptr());
    ofn.lpstrDefExt = def_ext.as_ref().map_or(std::ptr::null(), |e| e.as_ptr());
    ofn.Flags = flags;
    // SAFETY: as above.
    let ok = unsafe {
        if save {
            GetSaveFileNameW(&mut ofn)
        } else {
            GetOpenFileNameW(&mut ofn)
        }
    };
    if ok != 0 {
        return Ok(Some(buf));
    }
    // SAFETY: no preconditions.
    match unsafe { CommDlgExtendedError() } {
        0 => Ok(None), // cancelled
        code => Err(DialogError::Failed(format!(
            "common dialog error {code:#x}"
        ))),
    }
}

pub(super) fn open_files(o: &OpenFiles) -> Result<Vec<PathBuf>, DialogError> {
    use windows_sys::Win32::UI::Controls::Dialogs::{
        OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR,
        OFN_PATHMUSTEXIST,
    };
    let mut flags =
        OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR;
    if o.multiple {
        flags |= OFN_ALLOWMULTISELECT;
    }
    Ok(
        classic(&o.title, &o.filters, &o.parent, flags, None, None, false)?
            .map(|b| split_multi(&b))
            .unwrap_or_default(),
    )
}

pub(super) fn save_file(o: &SaveFile) -> Result<Option<PathBuf>, DialogError> {
    use windows_sys::Win32::UI::Controls::Dialogs::{
        OFN_EXPLORER, OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST,
    };
    let target = SaveTarget::from_default_path(o.default_path.as_deref());
    let (folder, name) = match &target.file {
        Some(f) => (
            f.parent().map(|p| p.to_path_buf()),
            f.file_name().map(|n| n.to_string_lossy().into_owned()),
        ),
        None => (target.folder.clone(), target.name.clone()),
    };
    let flags =
        OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR;
    Ok(classic(
        &o.title,
        &o.filters,
        &o.parent,
        flags,
        name.as_deref(),
        folder.as_deref(),
        true,
    )?
    .and_then(|b| split_multi(&b).into_iter().next()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_specs() {
        let s = specs(&[
            FileFilter::new("Mods", ["jar", "zip"]),
            FileFilter::new("All", ["*"]),
            FileFilter::new("None", [""]),
        ]);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].1.to_string(), "*.jar;*.zip");
        assert_eq!(s[1].1.to_string(), "*.*");
        let f = filter_string(&[FileFilter::new("Mods", ["jar"])]).unwrap();
        assert_eq!(String::from_utf16_lossy(&f), "Mods\0*.jar\0\0");
        assert!(filter_string(&[]).is_none());
    }

    #[test]
    fn multi_select_answers() {
        let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        assert_eq!(
            split_multi(&w("C:\\a.jar\0\0")),
            [PathBuf::from("C:\\a.jar")]
        );
        assert_eq!(
            split_multi(&w("C:\\m\0a.jar\0b.jar\0\0")),
            [
                PathBuf::from("C:\\m").join("a.jar"),
                PathBuf::from("C:\\m").join("b.jar")
            ]
        );
        assert!(split_multi(&w("\0\0")).is_empty());
    }
}
