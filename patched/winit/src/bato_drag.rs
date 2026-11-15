//! bato-native: start an OS drag of files from a window, like Electron's
//! `webContents.startDrag`, and tell this process's own drag apart when it
//! comes back as a file drop.
//!
//! [`start`] queues a [`Request`] and wakes the event loop, which starts the
//! drag on its next turn (Wayland: a `wl_data_source` on the window's data
//! device; X11: an XDND source). The drag offers `text/uri-list`, the paths as
//! `text/plain`, and a private marker type that names this process and the
//! drag, so the file-drop code can report [`incoming_drag`] when the drag is
//! over one of this process's windows.
//!
//! The drag must start while a pointer button is held on the window (the
//! compositor or X server ties it to that press); otherwise it fails at once
//! with [`Outcome::Failed`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// The drag image: straight-alpha RGBA8, `width × height` pixels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Icon {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Pixels per logical pixel (the buffer scale on Wayland).
    pub scale: u32,
    /// Where the pointer is on the icon, in logical pixels from its top-left.
    pub hotspot: (i32, i32),
}

/// What the drop target did with the files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Copy,
    Move,
}

/// How a drag ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A target took the files.
    Dropped(Action),
    /// Dropped nowhere, refused, or cancelled (Esc).
    Cancelled,
    /// The drag did not start (no pointer button held, no backend, ...).
    Failed(String),
}

/// Called once, on the event loop thread, when the drag ends.
pub type Done = Box<dyn FnOnce(Outcome) + Send>;

/// A drag to start.
pub struct Request {
    /// The window the drag starts from: winit's `WindowId` as `u64` (iced's
    /// `window::raw_id`).
    pub window: u64,
    pub paths: Vec<PathBuf>,
    pub icon: Option<Icon>,
    /// Offer "move" besides "copy" (a file manager may then delete the
    /// originals).
    pub allow_move: bool,
    pub done: Done,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("window", &self.window)
            .field("paths", &self.paths)
            .field("icon", &self.icon.as_ref().map(|i| (i.width, i.height, i.scale)))
            .field("allow_move", &self.allow_move)
            .finish_non_exhaustive()
    }
}

type Waker = Box<dyn Fn() + Send + Sync>;

static QUEUE: Mutex<Vec<(u64, Request)>> = Mutex::new(Vec::new());
static WAKER: Mutex<Option<Waker>> = Mutex::new(None);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// The own drag the latest file hover or drop came from (0: another app's).
static INCOMING: AtomicU64 = AtomicU64::new(0);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Queue a drag; returns its id (what [`incoming_drag`] reports). Any thread.
/// Without a running event loop that supports it, `done` gets
/// [`Outcome::Failed`] right away.
pub fn start(request: Request) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let waker = lock(&WAKER);
    match waker.as_ref() {
        Some(wake) => {
            lock(&QUEUE).push((id, request));
            wake();
        },
        None => {
            drop(waker);
            (request.done)(Outcome::Failed("OS drags are not supported here".into()));
        },
    }
    id
}

/// The id of this process's own drag (from [`start`]) that the latest
/// `HoveredFile` / `DroppedFile` came from, or `None` when another app's drag
/// brought them. Valid from the hover until the next drag enters a window.
pub fn incoming_drag() -> Option<u64> {
    match INCOMING.load(Ordering::Relaxed) {
        0 => None,
        id => Some(id),
    }
}

/// The event loop can start drags: `wake` makes it call [`take`] soon.
#[allow(dead_code)]
pub(crate) fn set_waker(wake: Waker) {
    *lock(&WAKER) = Some(wake);
}

/// The queued drags.
#[allow(dead_code)]
pub(crate) fn take() -> Vec<(u64, Request)> {
    std::mem::take(&mut *lock(&QUEUE))
}

/// A drag entered one of our windows; `offered` are its types.
#[allow(dead_code)]
pub(crate) fn set_incoming<'a>(offered: impl IntoIterator<Item = &'a str>) {
    let id = offered.into_iter().find_map(parse_marker).unwrap_or(0);
    INCOMING.store(id, Ordering::Relaxed);
}

const MARKER: &str = "application/x-bato-drag;pid=";

/// The private type that marks drag `id` of this process.
#[allow(dead_code)]
pub(crate) fn marker(id: u64) -> String {
    format!("{MARKER}{};id={id}", std::process::id())
}

/// The drag id of a [`marker`] from this process.
fn parse_marker(mime: &str) -> Option<u64> {
    let (pid, id) = mime.strip_prefix(MARKER)?.split_once(";id=")?;
    (pid.parse::<u32>().ok()? == std::process::id()).then_some(())?;
    id.parse().ok().filter(|&id| id != 0)
}

/// `text/uri-list` (RFC 2483) for local paths: `file://` URIs, CRLF after
/// each, every byte outside RFC 3986 unreserved and `/` percent-encoded.
pub fn uri_list(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for path in paths {
        out.push_str("file://");
        for &b in path_bytes(path).iter() {
            if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out.push_str("\r\n");
    }
    out
}

/// The paths as text, one per line (`text/plain`).
pub fn plain_text(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n")
}

#[cfg(unix)]
fn path_bytes(path: &std::path::Path) -> std::borrow::Cow<'_, [u8]> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().into()
}

#[cfg(not(unix))]
fn path_bytes(path: &std::path::Path) -> std::borrow::Cow<'_, [u8]> {
    path.to_string_lossy().into_owned().into_bytes().into()
}

/// The icon as premultiplied ARGB8888 in native byte order (Wayland `wl_shm`
/// and X11 32-bit visuals).
#[allow(dead_code)]
pub(crate) fn premultiplied_argb(rgba: &[u8], out: &mut [u8]) {
    for (src, dst) in rgba.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        let a = src[3] as u32;
        let pm = |c: u8| ((c as u32 * a + 127) / 255) as u8;
        let px = u32::from_be_bytes([src[3], pm(src[0]), pm(src[1]), pm(src[2])]);
        dst.copy_from_slice(&px.to_ne_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_list_round_trips_through_the_drop_parser() {
        let paths = vec![PathBuf::from("/tmp/a b/ü%#.png"), PathBuf::from("/x")];
        let list = uri_list(&paths);
        assert_eq!(list, "file:///tmp/a%20b/%C3%BC%25%23.png\r\nfile:///x\r\n");
        assert_eq!(crate::bato_uri_list::parse(list.as_bytes(), None), paths);
    }

    #[test]
    fn marker_names_this_process_only() {
        assert_eq!(parse_marker(&marker(7)), Some(7));
        assert_eq!(parse_marker("application/x-bato-drag;pid=1;id=7"), None);
        assert_eq!(parse_marker("text/uri-list"), None);
        set_incoming(["text/uri-list", marker(9).as_str()]);
        assert_eq!(incoming_drag(), Some(9));
        set_incoming(["text/uri-list"]);
        assert_eq!(incoming_drag(), None);
    }

    #[test]
    fn premultiplies() {
        let mut out = [0u8; 8];
        premultiplied_argb(&[255, 0, 0, 128, 10, 20, 30, 255], &mut out);
        assert_eq!(u32::from_ne_bytes(out[..4].try_into().unwrap()), 0x8080_0000);
        assert_eq!(u32::from_ne_bytes(out[4..].try_into().unwrap()), 0xff0a_141e);
    }
}
