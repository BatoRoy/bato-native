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
    FOS_ALLOWMULTISELECT, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST,
    FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog, IFileDialog, IFileOpenDialog, IFileSaveDialog,
    IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
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

pub(super) fn open_files(o: &OpenFiles) -> Result<Vec<PathBuf>, DialogError> {
    // SAFETY: COM on this thread's apartment.
    unsafe {
        let d: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(failed)?;
        let mut opts = d.GetOptions().map_err(failed)? | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST;
        if o.multiple {
            opts |= FOS_ALLOWMULTISELECT;
        }
        d.SetOptions(opts).map_err(failed)?;
        if !show(&d.cast().map_err(failed)?, &o.title, &o.filters, &o.parent)? {
            return Ok(Vec::new());
        }
        let items = d.GetResults().map_err(failed)?;
        let n = items.GetCount().map_err(failed)?;
        (0..n)
            .map(|i| item_path(&items.GetItemAt(i).map_err(failed)?))
            .collect()
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

pub(super) fn save_file(o: &SaveFile) -> Result<Option<PathBuf>, DialogError> {
    let target = SaveTarget::from_default_path(o.default_path.as_deref());
    // SAFETY: COM on this thread's apartment.
    unsafe {
        let d: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).map_err(failed)?;
        let opts = d.GetOptions().map_err(failed)? | FOS_OVERWRITEPROMPT | FOS_FORCEFILESYSTEM;
        d.SetOptions(opts).map_err(failed)?;
        let (folder, name) = match &target.file {
            Some(f) => (
                f.parent().map(|p| p.to_path_buf()),
                f.file_name().map(|n| n.to_string_lossy().into_owned()),
            ),
            None => (target.folder.clone(), target.name.clone()),
        };
        if let Some(dir) = folder {
            let item: Result<IShellItem, _> =
                SHCreateItemFromParsingName(&HSTRING::from(dir.as_os_str()), None);
            if let Ok(item) = item {
                let _ = d.SetFolder(&item);
            }
        }
        if let Some(name) = &name {
            d.SetFileName(&HSTRING::from(name.as_str()))
                .map_err(failed)?;
        }
        // The first filter's extension is appended when the name has none.
        if let Some(ext) = o
            .filters
            .first()
            .and_then(|f| f.globs().into_iter().next())
            .and_then(|g| g.strip_prefix("*.").map(str::to_string))
        {
            let _ = d.SetDefaultExtension(&HSTRING::from(ext));
        }
        if !show(&d.cast().map_err(failed)?, &o.title, &o.filters, &o.parent)? {
            return Ok(None);
        }
        item_path(&d.GetResult().map_err(failed)?).map(Some)
    }
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
    }
}
