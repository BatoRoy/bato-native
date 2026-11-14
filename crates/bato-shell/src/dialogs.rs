//! File dialogs through xdg-desktop-portal (`org.freedesktop.portal.FileChooser`).
//!
//! The portal hands the dialog to whatever FileChooser backend the desktop
//! configured (here BatoBrowse, with KDE as the fallback, per
//! `~/.config/xdg-desktop-portal/hyprland-portals.conf`; GNOME/KDE use their
//! own). No GTK is linked, at build or at run time. `xdg-desktop-portal-hyprland`
//! itself has no FileChooser, so on a machine without a backend every call
//! returns [`DialogError::Unavailable`].
//!
//! Semantics follow the Electron handlers: cancelling is not an error
//! (`open_files` → empty, the others → `None`), `filters` are
//! `{ name, extensions }`, and `open_files` allows several files unless told
//! otherwise.
//!
//! ```no_run
//! use bato_shell::dialogs::{self, FileFilter, OpenFiles};
//! # async fn demo() -> Result<(), dialogs::DialogError> {
//! let picked = dialogs::open_files(OpenFiles {
//!     filters: vec![FileFilter::new("Images", ["png", "jpg"])],
//!     ..OpenFiles::default()
//! })
//! .await?;
//! # Ok(()) }
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ashpd::WindowIdentifier;
use ashpd::desktop::ResponseError;
use ashpd::desktop::file_chooser::{FileFilter as PortalFilter, SelectedFiles};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

/// Electron's `{ name, extensions }`; `"*"` matches everything.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

impl FileFilter {
    pub fn new<S: Into<String>>(
        name: impl Into<String>,
        extensions: impl IntoIterator<Item = S>,
    ) -> Self {
        Self {
            name: name.into(),
            extensions: extensions.into_iter().map(Into::into).collect(),
        }
    }

    /// Glob patterns for the portal: `png` → `*.png`, `*` → `*`.
    pub fn globs(&self) -> Vec<String> {
        self.extensions
            .iter()
            .map(|e| e.trim().trim_start_matches("*.").trim_start_matches('.'))
            .filter(|e| !e.is_empty())
            .map(|e| {
                if e == "*" {
                    "*".to_string()
                } else {
                    format!("*.{e}")
                }
            })
            .collect()
    }

    fn to_portal(&self) -> PortalFilter {
        self.globs()
            .iter()
            .fold(PortalFilter::new(&self.name), |f, g| f.glob(g))
    }
}

/// The window a dialog belongs to, so the backend can make it modal.
///
/// Get one with [`Parent::from_window`] (or `iced_ext::dialog_parent`, which
/// the `iced_ext` dialog wrappers call for you when `parent` is `None`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum Parent {
    #[default]
    None,
    /// An X11 window id.
    X11(u64),
    /// A Wayland toplevel exported through xdg-foreign.
    Wayland(Exported),
}

/// An xdg-foreign handle for a Wayland toplevel (`wayland:<handle>` without
/// the prefix). One made by [`Parent::from_window`] owns the export: the
/// compositor revokes the handle when the last clone drops, i.e. when the
/// dialog that holds it has finished.
#[derive(Clone)]
pub struct Exported {
    handle: String,
    /// The export (ashpd unexports on drop); `None` for a handle the app
    /// exported and keeps alive itself.
    export: Option<Arc<WindowIdentifier>>,
}

impl Exported {
    /// A handle the app exported itself and keeps alive for the dialog.
    pub fn from_handle(handle: impl Into<String>) -> Self {
        Self {
            handle: handle.into(),
            export: None,
        }
    }

    pub fn handle(&self) -> &str {
        &self.handle
    }

    /// Whether this value owns the export (made by [`Parent::from_window`]).
    pub fn owns_export(&self) -> bool {
        self.export.is_some()
    }
}

impl std::fmt::Debug for Exported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Exported").field(&self.handle).finish()
    }
}

impl PartialEq for Exported {
    fn eq(&self, o: &Self) -> bool {
        self.handle == o.handle
    }
}

impl Eq for Exported {}

impl std::hash::Hash for Exported {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.handle.hash(h);
    }
}

impl Parent {
    /// The parent for a live window: its id on X11; on Wayland an
    /// xdg-foreign export of its toplevel (`zxdg_exporter_v2`, else v1),
    /// which blocks for two compositor roundtrips on a private event queue.
    /// `None` on other platforms, or when the compositor has no exporter
    /// (the dialog then opens unparented, as before).
    ///
    /// Call it where the window is borrowed (iced: `window::run`), not with
    /// handles saved for later: the export needs the live `wl_surface`.
    pub fn from_window<W>(window: &W) -> Self
    where
        W: HasWindowHandle + HasDisplayHandle + ?Sized,
    {
        let Ok(w) = window.window_handle() else {
            return Parent::None;
        };
        let display = window.display_handle().ok().map(|d| d.as_raw());
        // SAFETY: both handles are borrowed from `window` for this whole
        // call, and `from_raw` finishes the export before it returns.
        unsafe { Self::from_raw(w.as_raw(), display) }
    }

    /// [`from_window`](Self::from_window) on raw handles.
    ///
    /// # Safety
    ///
    /// For a Wayland handle, `window.surface` and `display.display` must be
    /// a live `wl_surface` and `wl_display` for the duration of the call, and
    /// the display must outlive the returned value (dropping it unexports the
    /// surface on that display). X11 handles are plain ids.
    pub unsafe fn from_raw(window: RawWindowHandle, display: Option<RawDisplayHandle>) -> Self {
        match (window, display) {
            (RawWindowHandle::Xlib(h), _) => {
                // c_ulong: u64 here, u32 on 32-bit targets.
                #[allow(clippy::unnecessary_cast)]
                Parent::X11(h.window as u64)
            }
            (RawWindowHandle::Xcb(h), _) => Parent::X11(u64::from(h.window.get())),
            (RawWindowHandle::Wayland(_), Some(RawDisplayHandle::Wayland(_))) => {
                let export = futures::executor::block_on(WindowIdentifier::from_raw_handle(
                    &window,
                    display.as_ref(),
                ));
                export.map_or(Parent::None, Parent::from_export)
            }
            _ => Parent::None,
        }
    }

    /// Wrap an export ashpd made; `None` unless it is a non-empty
    /// `wayland:` handle (a compositor that never sent the handle event).
    fn from_export(export: WindowIdentifier) -> Self {
        match wayland_handle(&export.to_string()) {
            Some(handle) => Parent::Wayland(Exported {
                handle: handle.to_string(),
                export: Some(Arc::new(export)),
            }),
            None => Parent::None,
        }
    }

    /// The portal's `parent_window` string: `x11:<hex>`, `wayland:<handle>`,
    /// or `""` for none.
    pub fn portal_string(&self) -> String {
        self.identifier().map(|i| i.to_string()).unwrap_or_default()
    }

    fn identifier(&self) -> Option<WindowIdentifier> {
        match self {
            Parent::None => None,
            Parent::X11(xid) => Some(WindowIdentifier::from_xid(*xid as std::os::raw::c_ulong)),
            Parent::Wayland(e) => Some(WindowIdentifier::from_xdg_foreign_exported(
                e.handle.clone(),
            )),
        }
    }
}

/// `wayland:<handle>` → `<handle>`, if non-empty.
fn wayland_handle(identifier: &str) -> Option<&str> {
    identifier
        .strip_prefix("wayland:")
        .filter(|h| !h.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OpenFiles {
    pub title: String,
    pub filters: Vec<FileFilter>,
    /// Electron's `multiSelections`, default true.
    pub multiple: bool,
    pub parent: Parent,
}

impl Default for OpenFiles {
    fn default() -> Self {
        Self {
            title: "Open".into(),
            filters: Vec::new(),
            multiple: true,
            parent: Parent::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OpenDirectory {
    pub title: String,
    pub parent: Parent,
}

impl Default for OpenDirectory {
    fn default() -> Self {
        Self {
            title: "Open Folder".into(),
            parent: Parent::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SaveFile {
    pub title: String,
    /// Electron's `defaultPath`: a file name, a folder, or a full path.
    pub default_path: Option<PathBuf>,
    pub filters: Vec<FileFilter>,
    pub parent: Parent,
}

impl Default for SaveFile {
    fn default() -> Self {
        Self {
            title: "Save".into(),
            default_path: None,
            filters: Vec::new(),
            parent: Parent::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogError {
    /// No portal, or no FileChooser backend.
    Unavailable(String),
    Failed(String),
}

impl std::fmt::Display for DialogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(e) => {
                write!(f, "no file chooser available (xdg-desktop-portal): {e}")
            }
            Self::Failed(e) => write!(f, "file dialog failed: {e}"),
        }
    }
}

impl std::error::Error for DialogError {}

/// Selected files; empty when cancelled.
pub async fn open_files(o: OpenFiles) -> Result<Vec<PathBuf>, DialogError> {
    let req = SelectedFiles::open_file()
        .identifier(o.parent.identifier())
        .title(o.title.as_str())
        .modal(true)
        .multiple(o.multiple)
        .filters(o.filters.iter().map(FileFilter::to_portal));
    finish(req.send().await)
}

/// The chosen folder; `None` when cancelled.
pub async fn open_directory(o: OpenDirectory) -> Result<Option<PathBuf>, DialogError> {
    let req = SelectedFiles::open_file()
        .identifier(o.parent.identifier())
        .title(o.title.as_str())
        .modal(true)
        .multiple(false)
        .directory(true);
    Ok(finish(req.send().await)?.into_iter().next())
}

/// The path to save to; `None` when cancelled.
pub async fn save_file(o: SaveFile) -> Result<Option<PathBuf>, DialogError> {
    let target = SaveTarget::from_default_path(o.default_path.as_deref());
    let mut req = SelectedFiles::save_file()
        .identifier(o.parent.identifier())
        .title(o.title.as_str())
        .modal(true)
        .current_name(target.name.as_deref())
        .filters(o.filters.iter().map(FileFilter::to_portal));
    if let Some(dir) = &target.folder {
        req = req
            .current_folder::<&Path>(Some(dir.as_path()))
            .map_err(failed)?;
    }
    if let Some(file) = &target.file {
        req = req
            .current_file::<&Path>(Some(file.as_path()))
            .map_err(failed)?;
    }
    Ok(finish(req.send().await)?.into_iter().next())
}

type Sent = Result<ashpd::desktop::Request<SelectedFiles>, ashpd::Error>;

fn finish(sent: Sent) -> Result<Vec<PathBuf>, DialogError> {
    let files = match sent.and_then(|r| r.response()) {
        Ok(f) => f,
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => return Ok(Vec::new()),
        Err(e @ (ashpd::Error::Zbus(_) | ashpd::Error::Portal(_) | ashpd::Error::NoResponse)) => {
            return Err(DialogError::Unavailable(e.to_string()));
        }
        Err(e) => return Err(failed(e)),
    };
    Ok(files
        .uris()
        .iter()
        .filter_map(|u| file_uri_to_path(u.as_str()))
        .collect())
}

fn failed(e: ashpd::Error) -> DialogError {
    DialogError::Failed(e.to_string())
}

/// How Electron's `defaultPath` maps onto the portal's save options: an
/// existing file is `current_file` (the backend preselects it), an existing
/// folder is `current_folder`, anything else is a suggested name, in its
/// parent folder when that exists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveTarget {
    pub folder: Option<PathBuf>,
    pub name: Option<String>,
    pub file: Option<PathBuf>,
}

impl SaveTarget {
    pub fn from_default_path(p: Option<&Path>) -> Self {
        let Some(p) = p.filter(|p| !p.as_os_str().is_empty()) else {
            return Self::default();
        };
        if p.is_absolute() && p.is_file() {
            return Self {
                file: Some(p.to_path_buf()),
                ..Self::default()
            };
        }
        if p.is_absolute() && p.is_dir() {
            return Self {
                folder: Some(p.to_path_buf()),
                ..Self::default()
            };
        }
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
        let folder = p
            .parent()
            .filter(|d| d.is_absolute() && d.is_dir())
            .map(Path::to_path_buf);
        Self {
            folder,
            name,
            file: None,
        }
    }
}

/// `file:///home/me/a%20b.txt` → `/home/me/a b.txt`. Non-`file` URIs (a
/// backend may return `sftp:` or `trash:`) are dropped.
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file://host/path`: only the local host is meaningful.
    let path = if rest.starts_with('/') {
        rest
    } else {
        rest.strip_prefix("localhost")?
    };
    let b = path.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = b
            .get(i + 1..i + 3)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
            .and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (b[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_become_globs() {
        let f = FileFilter::new("Images", ["png", ".jpg", "*.gif", " webp ", ""]);
        assert_eq!(f.globs(), ["*.png", "*.jpg", "*.gif", "*.webp"]);
        assert_eq!(FileFilter::new("All Files", ["*"]).globs(), ["*"]);
        let p = f.to_portal();
        assert_eq!(p.label(), "Images");
        assert_eq!(p.pattern_filters(), ["*.png", "*.jpg", "*.gif", "*.webp"]);
    }

    #[test]
    fn uris_to_paths() {
        assert_eq!(
            file_uri_to_path("file:///home/me/a%20b.txt"),
            Some("/home/me/a b.txt".into())
        );
        assert_eq!(
            file_uri_to_path("file://localhost/tmp/x"),
            Some("/tmp/x".into())
        );
        assert_eq!(
            file_uri_to_path("file:///t/%E2%9C%93%zz"),
            Some("/t/✓%zz".into())
        );
        assert_eq!(file_uri_to_path("sftp://host/x"), None);
        assert_eq!(file_uri_to_path("file://otherhost/x"), None);
    }

    #[test]
    fn electron_default_path() {
        let dir = std::env::temp_dir().join(format!("bato-shell-dialogs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let existing = dir.join("notes.md");
        std::fs::write(&existing, "x").unwrap();

        assert_eq!(SaveTarget::from_default_path(None), SaveTarget::default());
        assert_eq!(
            SaveTarget::from_default_path(Some(Path::new("report.pdf"))),
            SaveTarget {
                name: Some("report.pdf".into()),
                ..SaveTarget::default()
            }
        );
        assert_eq!(
            SaveTarget::from_default_path(Some(&dir)),
            SaveTarget {
                folder: Some(dir.clone()),
                ..SaveTarget::default()
            }
        );
        assert_eq!(
            SaveTarget::from_default_path(Some(&existing)),
            SaveTarget {
                file: Some(existing.clone()),
                ..SaveTarget::default()
            }
        );
        assert_eq!(
            SaveTarget::from_default_path(Some(&dir.join("new.md"))),
            SaveTarget {
                folder: Some(dir.clone()),
                name: Some("new.md".into()),
                file: None
            }
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn x11_handles_become_parents() {
        use raw_window_handle::{XcbWindowHandle, XlibWindowHandle};
        let xlib = RawWindowHandle::Xlib(XlibWindowHandle::new(0x4a0_0007));
        // SAFETY: X11 handles are plain ids.
        let p = unsafe { Parent::from_raw(xlib, None) };
        assert_eq!(p, Parent::X11(0x4a0_0007));
        assert_eq!(p.portal_string(), "x11:4a00007");
        let xcb =
            RawWindowHandle::Xcb(XcbWindowHandle::new(std::num::NonZeroU32::new(42).unwrap()));
        assert_eq!(unsafe { Parent::from_raw(xcb, None) }, Parent::X11(42));
    }

    #[test]
    fn wayland_needs_its_display() {
        use raw_window_handle::{WaylandWindowHandle, XlibDisplayHandle};
        // Never dereferenced: without a Wayland display handle there is no
        // export, so no compositor is needed here.
        let surface = std::ptr::NonNull::<std::ffi::c_void>::dangling();
        let wl = RawWindowHandle::Wayland(WaylandWindowHandle::new(surface));
        assert_eq!(unsafe { Parent::from_raw(wl, None) }, Parent::None);
        let x = RawDisplayHandle::Xlib(XlibDisplayHandle::new(None, 0));
        assert_eq!(unsafe { Parent::from_raw(wl, Some(x)) }, Parent::None);
        let web = RawWindowHandle::Web(raw_window_handle::WebWindowHandle::new(1));
        assert_eq!(unsafe { Parent::from_raw(web, None) }, Parent::None);
        assert_eq!(Parent::None.portal_string(), "");
    }

    #[test]
    fn exported_handles() {
        // What ashpd hands back after a successful export.
        let id = WindowIdentifier::from_xdg_foreign_exported("a1b2-c3".into());
        let p = Parent::from_export(id);
        let Parent::Wayland(e) = &p else {
            panic!("{p:?}")
        };
        assert_eq!(e.handle(), "a1b2-c3");
        assert!(e.owns_export());
        assert_eq!(p.portal_string(), "wayland:a1b2-c3");
        // Equality and hashing go by handle, so an owned export equals a
        // handle the app passes itself.
        assert_eq!(p, Parent::Wayland(Exported::from_handle("a1b2-c3")));
        assert!(!Exported::from_handle("x").owns_export());
        // Clones share the export; it ends with the last one.
        let q = p.clone();
        drop(p);
        assert_eq!(q.portal_string(), "wayland:a1b2-c3");
        // A compositor that never sent the handle: no parent.
        let empty = WindowIdentifier::from_xdg_foreign_exported(String::new());
        assert_eq!(Parent::from_export(empty), Parent::None);
        assert_eq!(wayland_handle("x11:12"), None);
    }

    #[test]
    fn electron_defaults() {
        assert!(OpenFiles::default().multiple);
        assert_eq!(OpenFiles::default().parent, Parent::None);
    }
}
