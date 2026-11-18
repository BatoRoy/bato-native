//! Copy and paste files through the system clipboard (feature
//! `clipboard`), so they paste as files in Nautilus, Dolphin, Thunar, Nemo,
//! browsers and chat apps, and files copied there paste here. iced's own
//! clipboard is text-only.
//!
//! A copy offers, in this order:
//!
//! | MIME type | Content | Read by |
//! |---|---|---|
//! | `text/uri-list` | `file:///a%20b\r\n…` | Dolphin, Thunar, GTK4/Qt apps, browsers, Electron |
//! | `x-special/gnome-copied-files` | `copy\nfile:///…` or `cut\n…` | Nautilus, Nemo, Caja (cut needs it) |
//! | `application/x-kde-cutselection` | `1` (cut only) | Dolphin (cut) |
//! | `text/plain;charset=utf-8`, `text/plain`, `UTF8_STRING`, `STRING`, `TEXT` | the paths, one per line | editors, terminals |
//!
//! A paste reads `x-special/gnome-copied-files` first (it says copy or
//! cut), else `text/uri-list` (cut when `application/x-kde-cutselection`
//! is `1`), else GNOME's older text form (`x-special/nautilus-clipboard`).
//!
//! **Where it works, and where it falls back** ([`Backend`]):
//!
//! | Session | Copy | Paste |
//! |---|---|---|
//! | Wayland with data-control (Hyprland, Sway, KDE, niri, river, Wayfire, COSMIC) | in-process, `ext-data-control-v1` else `zwlr-data-control-unstable-v1`: every type above | same protocol |
//! | Wayland without it (GNOME) | `wl-copy --type text/uri-list` (one type, so a cut pastes as a copy) | `wl-paste` |
//! | X11 | in-process CLIPBOARD owner (x11rb): every type above | `ConvertSelection` |
//! | neither, or the tools missing | [`ClipboardError::Unavailable`]: write the paths as text with `iced::clipboard` | same |
//!
//! The in-process owners serve from a thread until something else is
//! copied or the app exits; then the clipboard is empty unless a clipboard
//! manager took a copy (the usual Wayland/X11 rule; `wl-copy` forks and
//! survives instead). `BATO_FILE_CLIPBOARD=data-control|wl-clipboard|x11`
//! forces one backend (tests, a broken compositor).
//!
//! Everything here blocks (round-trips, a 2 s read limit) and runs on a
//! worker (`iced_ext::blocking`).
//!
//! ```no_run
//! use bato_shell::clipboard::{self, Op};
//! use std::path::PathBuf;
//! let backend = clipboard::copy_files(&[PathBuf::from("/home/me/a b.png")], Op::Cut)?;
//! if let Some(files) = clipboard::paste_files()? {
//!     println!("{:?} {:?}", files.op, files.paths);
//! }
//! # Ok::<(), clipboard::ClipboardError>(())
//! ```

#[cfg(unix)]
mod wayland;
#[cfg(unix)]
mod x11;

#[cfg(windows)]
#[path = "clipboard_win.rs"]
mod win;
#[cfg(windows)]
pub use win::{copy_files, paste_files, types};

#[cfg(unix)]
use std::io::Read;
use std::path::PathBuf;
#[cfg(unix)]
use std::process::{Command, Stdio};
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use crate::desktop::{file_uri, uri_to_path};

pub const URI_LIST: &str = "text/uri-list";
pub const GNOME_COPIED_FILES: &str = "x-special/gnome-copied-files";
pub const KDE_CUT_SELECTION: &str = "application/x-kde-cutselection";
/// The text types, as offered (the first is the canonical one).
pub const TEXT_TYPES: [&str; 5] = [
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
];
/// Forces a backend: `data-control`, `wl-clipboard` or `x11`.
pub const ENV_BACKEND: &str = "BATO_FILE_CLIPBOARD";
/// How long a paste waits for the owner to hand over one type.
pub const READ_TIMEOUT: Duration = Duration::from_secs(2);
/// A clipboard entry larger than this is refused.
pub const MAX_READ: usize = 16 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Op {
    #[default]
    Copy,
    /// The source asked for a move: delete the originals after pasting.
    Cut,
}

/// Files on the clipboard.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Files {
    pub op: Op,
    /// Local paths, in clipboard order.
    pub paths: Vec<PathBuf>,
    /// Items that are not local files (`sftp://…`, `smb://…` from gvfs, or
    /// a `file://` URI of another host), for a "cannot paste" message.
    pub other: Vec<String>,
}

/// Which mechanism served a copy or a paste.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    /// Wayland `ext-data-control-v1` / `zwlr-data-control-unstable-v1`, in-process.
    DataControl,
    /// The `wl-copy` / `wl-paste` tools (GNOME). A copy then offers only
    /// `text/uri-list`: a cut pastes as a copy.
    WlClipboard,
    /// An X11 CLIPBOARD owner, in-process.
    X11,
    /// Windows: `CF_HDROP` plus `Preferred DropEffect` (Explorer's format).
    #[cfg(windows)]
    Win32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardError {
    /// No backend in this session (no display, no data-control and no
    /// wl-clipboard tools). The message says what was tried.
    Unavailable(String),
    /// A backend was there but failed (or timed out).
    Failed(String),
}

impl std::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(m) => write!(f, "no file clipboard: {m}"),
            Self::Failed(m) => write!(f, "clipboard: {m}"),
        }
    }
}

impl std::error::Error for ClipboardError {}

// ── Formats ──────────────────────────────────────────────────────────────────

/// What a copy of `paths` offers: `(MIME type, bytes)`, in offer order.
pub fn offers(paths: &[PathBuf], op: Op) -> Vec<(String, Vec<u8>)> {
    let uris: Vec<String> = paths.iter().map(|p| file_uri(p)).collect();
    let mut uri_list = uris.join("\r\n");
    uri_list.push_str("\r\n");
    let verb = match op {
        Op::Copy => "copy",
        Op::Cut => "cut",
    };
    let gnome = std::iter::once(verb.to_string())
        .chain(uris.iter().cloned())
        .collect::<Vec<_>>()
        .join("\n");
    let text = paths
        .iter()
        .map(|p| p.to_string_lossy())
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = vec![
        (URI_LIST.to_string(), uri_list.into_bytes()),
        (GNOME_COPIED_FILES.to_string(), gnome.into_bytes()),
    ];
    if op == Op::Cut {
        out.push((KDE_CUT_SELECTION.to_string(), b"1".to_vec()));
    }
    out.extend(
        TEXT_TYPES
            .iter()
            .map(|t| (t.to_string(), text.clone().into_bytes())),
    );
    out
}

#[cfg(unix)]
fn has(types: &[String], t: &str) -> bool {
    types.iter().any(|x| x.eq_ignore_ascii_case(t))
}

/// Which types to fetch, given the offered ones.
#[cfg(unix)]
fn to_fetch(types: &[String]) -> Vec<&'static str> {
    if has(types, GNOME_COPIED_FILES) {
        return vec![GNOME_COPIED_FILES];
    }
    if has(types, URI_LIST) {
        return if has(types, KDE_CUT_SELECTION) {
            vec![URI_LIST, KDE_CUT_SELECTION]
        } else {
            vec![URI_LIST]
        };
    }
    TEXT_TYPES
        .iter()
        .find(|t| has(types, t))
        .map(|t| vec![*t])
        .unwrap_or_default()
}

/// Files from what a paste fetched (`(type, bytes)`); `None` when it is
/// not a file list (plain text, an image…).
pub fn parse(fetched: &[(String, Vec<u8>)]) -> Option<Files> {
    let get = |t: &str| {
        fetched
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(t))
            .map(|(_, v)| String::from_utf8_lossy(v).into_owned())
    };
    if let Some(g) = get(GNOME_COPIED_FILES)
        && let Some(f) = parse_gnome(&g)
    {
        return Some(f);
    }
    if let Some(list) = get(URI_LIST) {
        let cut = get(KDE_CUT_SELECTION).is_some_and(|v| v.trim_start().starts_with('1'));
        let f = from_uris(
            if cut { Op::Cut } else { Op::Copy },
            list.lines().filter(|l| !l.starts_with('#')),
        );
        if !f.paths.is_empty() || !f.other.is_empty() {
            return Some(f);
        }
    }
    // GNOME Files 3.x on Wayland: text/plain "x-special/nautilus-clipboard\ncut\nfile://…".
    let text = TEXT_TYPES.iter().find_map(|t| get(t))?;
    let rest = text.strip_prefix("x-special/nautilus-clipboard\n")?;
    parse_gnome(rest)
}

fn parse_gnome(s: &str) -> Option<Files> {
    let mut lines = s.lines();
    let op = match lines.next()?.trim() {
        "copy" => Op::Copy,
        "cut" => Op::Cut,
        _ => return None,
    };
    let f = from_uris(op, lines);
    (!f.paths.is_empty() || !f.other.is_empty()).then_some(f)
}

fn from_uris<'a>(op: Op, lines: impl Iterator<Item = &'a str>) -> Files {
    let mut f = Files {
        op,
        ..Files::default()
    };
    for l in lines.map(str::trim).filter(|l| !l.is_empty()) {
        match uri_to_path(l) {
            Some(p) => f.paths.push(p),
            None => f.other.push(l.to_string()),
        }
    }
    f
}

// ── Backends ─────────────────────────────────────────────────────────────────

#[cfg(unix)]
/// The backends to try, in order, for this session.
fn plan() -> Result<Vec<Backend>, ClipboardError> {
    let forced = std::env::var(ENV_BACKEND).ok().filter(|v| !v.is_empty());
    if let Some(f) = forced {
        return match f.as_str() {
            "data-control" => Ok(vec![Backend::DataControl]),
            "wl-clipboard" => Ok(vec![Backend::WlClipboard]),
            "x11" => Ok(vec![Backend::X11]),
            other => Err(ClipboardError::Unavailable(format!(
                "{ENV_BACKEND}={other} is not a backend (data-control, wl-clipboard, x11)"
            ))),
        };
    }
    let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    if set("WAYLAND_DISPLAY") {
        // Under Wayland, Xwayland's CLIPBOARD is bridged by the compositor;
        // the Wayland one is the real one.
        Ok(vec![Backend::DataControl, Backend::WlClipboard])
    } else if set("DISPLAY") {
        Ok(vec![Backend::X11])
    } else {
        Err(ClipboardError::Unavailable(
            "neither WAYLAND_DISPLAY nor DISPLAY is set".into(),
        ))
    }
}

#[cfg(unix)]
/// Put `paths` on the clipboard as files ([`offers`]); the backend that
/// took them.
pub fn copy_files(paths: &[PathBuf], op: Op) -> Result<Backend, ClipboardError> {
    if paths.is_empty() {
        return Err(ClipboardError::Failed("nothing to copy".into()));
    }
    let offers = offers(paths, op);
    let mut tried = Vec::new();
    for b in plan()? {
        let r = match b {
            Backend::DataControl => wayland::copy(offers.clone()),
            Backend::WlClipboard => wl_copy(paths),
            Backend::X11 => x11::copy(None, offers.clone()),
        };
        match r {
            Ok(()) => return Ok(b),
            Err(e) => tried.push(format!("{b:?}: {e}")),
        }
    }
    Err(unavailable(tried))
}

#[cfg(unix)]
/// Files on the clipboard, `None` when it holds something else (or
/// nothing).
pub fn paste_files() -> Result<Option<Files>, ClipboardError> {
    let mut tried = Vec::new();
    for b in plan()? {
        let r = match b {
            Backend::DataControl => wayland::paste(&to_fetch),
            Backend::WlClipboard => wl_paste(),
            Backend::X11 => x11::paste(None, &to_fetch),
        };
        match r {
            Ok(fetched) => return Ok(parse(&fetched)),
            Err(e) => tried.push(format!("{b:?}: {e}")),
        }
    }
    Err(unavailable(tried))
}

#[cfg(unix)]
/// The MIME types on the clipboard now (empty when it is empty).
pub fn types() -> Result<Vec<String>, ClipboardError> {
    let mut tried = Vec::new();
    for b in plan()? {
        let r = match b {
            Backend::DataControl => wayland::types(),
            Backend::WlClipboard => run_capture("wl-paste", &["--list-types"]).map(|o| {
                String::from_utf8_lossy(&o)
                    .lines()
                    .map(str::to_string)
                    .collect()
            }),
            Backend::X11 => x11::types(None),
        };
        match r {
            Ok(t) => return Ok(t),
            Err(e) => tried.push(format!("{b:?}: {e}")),
        }
    }
    Err(unavailable(tried))
}

#[cfg(unix)]
fn unavailable(tried: Vec<String>) -> ClipboardError {
    ClipboardError::Unavailable(tried.join("; "))
}

// ── wl-clipboard tools ───────────────────────────────────────────────────────

#[cfg(unix)]
fn wl_copy(paths: &[PathBuf]) -> Result<(), String> {
    use std::io::Write;
    let mut child = Command::new("wl-copy")
        .args(["--type", URI_LIST])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("wl-copy: {e}"))?;
    let list = offers(paths, Op::Copy).swap_remove(0).1;
    let wrote = child
        .stdin
        .take()
        .map(|mut s| s.write_all(&list))
        .unwrap_or(Ok(()));
    // wl-copy forks a server and its foreground process exits at once.
    let status = wait_timeout(&mut child, READ_TIMEOUT)?;
    wrote.map_err(|e| format!("wl-copy: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("wl-copy failed ({status})"))
    }
}

#[cfg(unix)]
fn wl_paste() -> Result<Vec<(String, Vec<u8>)>, String> {
    let list = run_capture("wl-paste", &["--list-types"])?;
    let types: Vec<String> = String::from_utf8_lossy(&list)
        .lines()
        .map(str::to_string)
        .collect();
    let mut out = Vec::new();
    for t in to_fetch(&types) {
        out.push((
            t.to_string(),
            run_capture("wl-paste", &["--no-newline", "--type", t])?,
        ));
    }
    Ok(out)
}

#[cfg(unix)]
fn wait_timeout(
    child: &mut std::process::Child,
    within: Duration,
) -> Result<std::process::ExitStatus, String> {
    let end = Instant::now() + within;
    loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(s);
        }
        if Instant::now() >= end {
            let _ = child.kill();
            let _ = child.wait();
            return Err("timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
/// Run a tool, capture stdout (≤ [`MAX_READ`]) within [`READ_TIMEOUT`].
fn run_capture(program: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let r = (&mut stdout)
            .take(MAX_READ as u64 + 1)
            .read_to_end(&mut buf);
        r.map(|_| buf)
    });
    let status = wait_timeout(&mut child, READ_TIMEOUT)?;
    let out = reader
        .join()
        .map_err(|_| "reader panicked".to_string())?
        .map_err(|e| e.to_string())?;
    if out.len() > MAX_READ {
        return Err("clipboard content too large".into());
    }
    // wl-paste exits 1 with "No selection" / "No suitable type": empty.
    if !status.success() {
        return Ok(Vec::new());
    }
    Ok(out)
}

#[cfg(unix)]
/// Read a pipe until EOF, at most [`MAX_READ`] bytes, within `deadline`.
pub(crate) fn read_pipe(fd: std::os::fd::OwnedFd, deadline: Instant) -> Result<Vec<u8>, String> {
    use std::os::fd::AsRawFd;
    let mut file = std::fs::File::from(fd);
    let mut out = Vec::new();
    let mut buf = [0u8; 16 << 10];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("the clipboard owner did not answer".into());
        }
        let mut pfd = libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        let r = unsafe { libc::poll(&mut pfd, 1, left.as_millis().min(i32::MAX as u128) as i32) };
        if r < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e.to_string());
        }
        if r == 0 {
            continue;
        }
        match file.read(&mut buf) {
            Ok(0) => return Ok(out),
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if out.len() > MAX_READ {
                    return Err("clipboard content too large".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[cfg(unix)]
/// A pipe, both ends close-on-exec.
pub(crate) fn pipe() -> Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd), String> {
    use std::os::fd::FromRawFd;
    let mut fds = [0; 2];
    // SAFETY: pipe2 fills two fds we then own.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: fresh fds from pipe2.
    Ok(unsafe {
        (
            std::os::fd::OwnedFd::from_raw_fd(fds[0]),
            std::os::fd::OwnedFd::from_raw_fd(fds[1]),
        )
    })
}

#[cfg(unix)]
/// Serve one data request: write `data` to `fd` on a short-lived thread, so
/// a reader that never reads cannot stall the owner.
pub(crate) fn serve_fd(fd: std::os::fd::OwnedFd, data: Vec<u8>) {
    let _ = std::thread::Builder::new()
        .name("bato-clip-send".into())
        .stack_size(64 << 10)
        .spawn(move || {
            use std::io::Write;
            let mut f = std::fs::File::from(fd);
            let _ = f.write_all(&data);
        });
}

#[cfg(unix)]
/// The data for `mime` among `offers` (text aliases share the text).
pub(crate) fn lookup<'a>(offers: &'a [(String, Vec<u8>)], mime: &str) -> Option<&'a [u8]> {
    offers
        .iter()
        .find(|(k, _)| k == mime)
        .map(|(_, v)| v.as_slice())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn copy_offers_every_format() {
        let o = offers(&[p("/d/a b.mp4"), p("/d/c#")], Op::Copy);
        let types: Vec<&str> = o.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(
            types,
            [
                URI_LIST,
                GNOME_COPIED_FILES,
                "text/plain;charset=utf-8",
                "text/plain",
                "UTF8_STRING",
                "STRING",
                "TEXT"
            ]
        );
        assert_eq!(o[0].1, b"file:///d/a%20b.mp4\r\nfile:///d/c%23\r\n");
        assert_eq!(o[1].1, b"copy\nfile:///d/a%20b.mp4\nfile:///d/c%23");
        assert_eq!(o[2].1, b"/d/a b.mp4\n/d/c#");
        let cut = offers(&[p("/x")], Op::Cut);
        assert_eq!(cut[1].1, b"cut\nfile:///x");
        assert_eq!(cut[2], (KDE_CUT_SELECTION.to_string(), b"1".to_vec()));
    }

    #[test]
    fn our_own_offers_parse_back() {
        for op in [Op::Copy, Op::Cut] {
            let paths = vec![p("/d/a b"), p("/caf\u{e9}")];
            let o = offers(&paths, op);
            let types: Vec<String> = o.iter().map(|(t, _)| t.clone()).collect();
            let fetched: Vec<_> = to_fetch(&types)
                .into_iter()
                .map(|t| (t.to_string(), lookup(&o, t).unwrap().to_vec()))
                .collect();
            assert_eq!(
                parse(&fetched),
                Some(Files {
                    op,
                    paths,
                    other: vec![]
                })
            );
        }
    }

    #[test]
    fn pastes_from_file_managers() {
        let f = |pairs: &[(&str, &str)]| -> Vec<(String, Vec<u8>)> {
            pairs
                .iter()
                .map(|(t, v)| (t.to_string(), v.as_bytes().to_vec()))
                .collect()
        };
        // Nautilus: gnome-copied-files (and uri-list).
        assert_eq!(
            parse(&f(&[(
                GNOME_COPIED_FILES,
                "cut\nfile:///home/me/a.txt\nsftp://nas/b\n"
            )])),
            Some(Files {
                op: Op::Cut,
                paths: vec![p("/home/me/a.txt")],
                other: vec!["sftp://nas/b".into()]
            })
        );
        // Dolphin cut: uri-list + kde flag; KDE's file:/ short form.
        assert_eq!(
            parse(&f(&[
                (URI_LIST, "file:/home/me/a.txt\n"),
                (KDE_CUT_SELECTION, "1")
            ])),
            Some(Files {
                op: Op::Cut,
                paths: vec![p("/home/me/a.txt")],
                other: vec![]
            })
        );
        // Thunar / browsers: plain uri-list with a comment.
        assert_eq!(
            parse(&f(&[(
                URI_LIST,
                "# comment\r\nfile:///a\r\nfile://localhost/b\r\n"
            )]))
            .map(|x| (x.op, x.paths)),
            Some((Op::Copy, vec![p("/a"), p("/b")]))
        );
        // GNOME Files 3.x on Wayland.
        assert_eq!(
            parse(&f(&[(
                "text/plain;charset=utf-8",
                "x-special/nautilus-clipboard\ncopy\nfile:///a\n"
            )]))
            .map(|x| x.paths),
            Some(vec![p("/a")])
        );
        // Plain text is not a file list.
        assert_eq!(parse(&f(&[("text/plain", "/etc/passwd")])), None);
        assert_eq!(parse(&f(&[(GNOME_COPIED_FILES, "move\nfile:///a")])), None);
        assert_eq!(parse(&[]), None);
    }

    #[test]
    fn fetch_plan() {
        let t = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            to_fetch(&t(&[URI_LIST, GNOME_COPIED_FILES, "text/plain"])),
            [GNOME_COPIED_FILES]
        );
        assert_eq!(
            to_fetch(&t(&[KDE_CUT_SELECTION, URI_LIST])),
            [URI_LIST, KDE_CUT_SELECTION]
        );
        assert_eq!(to_fetch(&t(&["UTF8_STRING"])), ["UTF8_STRING"]);
        assert!(to_fetch(&t(&["image/png"])).is_empty());
    }

    #[test]
    fn pipes_time_out_and_cap() {
        let (r, w) = pipe().unwrap();
        let t = Instant::now();
        let e = read_pipe(r, Instant::now() + Duration::from_millis(200)).unwrap_err();
        assert!(e.contains("did not answer"), "{e}");
        assert!(t.elapsed() >= Duration::from_millis(200));
        drop(w);
        let (r, w) = pipe().unwrap();
        serve_fd(w, b"hello".to_vec());
        assert_eq!(
            read_pipe(r, Instant::now() + READ_TIMEOUT).unwrap(),
            b"hello"
        );
    }
}
