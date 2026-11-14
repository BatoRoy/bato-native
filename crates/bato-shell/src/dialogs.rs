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

use ashpd::WindowIdentifier;
use ashpd::desktop::ResponseError;
use ashpd::desktop::file_chooser::{FileFilter as PortalFilter, SelectedFiles};

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
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum Parent {
    #[default]
    None,
    /// An X11 window id.
    X11(u64),
    /// An xdg-foreign exported handle (Wayland), as `wayland:<handle>` without
    /// the prefix.
    WaylandExported(String),
}

impl Parent {
    fn identifier(&self) -> Option<WindowIdentifier> {
        match self {
            Parent::None => None,
            Parent::X11(xid) => Some(WindowIdentifier::from_xid(*xid as std::os::raw::c_ulong)),
            Parent::WaylandExported(h) => {
                Some(WindowIdentifier::from_xdg_foreign_exported(h.clone()))
            }
        }
    }
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
    fn electron_defaults() {
        assert!(OpenFiles::default().multiple);
        assert_eq!(OpenFiles::default().parent, Parent::None);
    }
}
