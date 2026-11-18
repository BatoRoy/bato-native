//! The desktop's file actions (feature `desktop`): reveal in the file
//! manager, open with the default app, and detached spawns. Electron's
//! `shell.showItemInFolder`, `shell.openPath` and `child_process.spawn(…,
//! {detached: true})`, lifted from BatoFetch's `desktop.rs` and hardened.
//!
//! | Function | Does | Falls back to |
//! |---|---|---|
//! | [`reveal`] | `org.freedesktop.FileManager1.ShowItems([uri])` (Nautilus, Dolphin, Nemo, Thunar, BatoBrowse) | `xdg-open <parent dir>` |
//! | [`open`] | `xdg-open <path>`, detached; an early non-zero exit is an error | — |
//! | [`Detached`] | `setsid`, stdio on `/dev/null`, every inherited fd closed at exec, reaped on a thread (no zombie), the app's own `BATO_*` and activation variables removed | — |
//!
//! Windows: [`reveal`] runs `explorer /select,`, [`open`] is `ShellExecuteW`
//! ("open"), and [`Detached`] uses `DETACHED_PROCESS` in a new process group
//! with stdio on `NUL`.
//!
//! Everything here blocks (D-Bus, process spawns) and runs on a worker
//! (`iced_ext::blocking`).

use std::ffi::OsStr;
#[cfg(unix)]
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub const FILE_MANAGER1: &str = "org.freedesktop.FileManager1";
pub const FILE_MANAGER1_PATH: &str = "/org/freedesktop/FileManager1";

/// Variables a detached child must not inherit: activation tokens are
/// single-use (the child would try to use ours), and `BATO_*` steer this app
/// (`BATO_APP_ID` would give a Bato app started from here our window class).
pub const SCRUBBED_ENV: [&str; 8] = [
    "XDG_ACTIVATION_TOKEN",
    "DESKTOP_STARTUP_ID",
    "BATO_APP_ID",
    "BATO_VIEW",
    "BATO_SMOKE",
    "BATO_TIMING",
    "BATO_DRAW_DEBUG",
    "BATO_SESSION_SECRET",
];

// ── URIs ─────────────────────────────────────────────────────────────────────

/// `file://` URI for an absolute path, percent-encoded as RFC 8089 wants
/// (Electron wrote the raw path, which breaks on spaces and `#`).
#[cfg(unix)]
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~!$&'()*+,;=:@".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// Windows: `file:///C:/Users/me/a%20b.txt` for `C:\Users\me\a b.txt`, and
/// `file://server/share/x` for a UNC path.
#[cfg(windows)]
pub fn file_uri(path: &Path) -> String {
    let p = path.to_string_lossy().replace('\\', "/");
    let (prefix, rest) = match p.strip_prefix("//") {
        Some(unc) => ("file://", unc.to_string()),
        None => ("file:///", p),
    };
    let mut s = String::from(prefix);
    for &b in rest.as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~!$&'()*+,;=:@".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// The local path of a `file:` URI: `file:///p`, `file://localhost/p`,
/// `file://<this host>/p`, or `file:/p` (KDE writes those). `None` for other
/// schemes, other hosts, relative paths, or a bad escape.
#[cfg(unix)]
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let uri = uri.trim();
    let rest = uri
        .strip_prefix("file:")
        .or_else(|| uri.strip_prefix("FILE:"))?;
    let path = match rest.strip_prefix("//") {
        Some(r) => {
            let slash = r.find('/')?;
            let host = &r[..slash];
            if !(host.is_empty()
                || host.eq_ignore_ascii_case("localhost")
                || host.eq_ignore_ascii_case(&hostname()))
            {
                return None;
            }
            &r[slash..]
        }
        None => rest,
    };
    let path = path.split(['?', '#']).next()?;
    if !path.starts_with('/') {
        return None;
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    if out.contains(&0) {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(out)))
}

/// Windows: `file:///C:/a%20b` → `C:\a b`; `file://server/share/x` →
/// `\\server\share\x`. `None` for other schemes, relative paths or a bad
/// escape.
#[cfg(windows)]
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let uri = uri.trim();
    let rest = uri
        .strip_prefix("file:")
        .or_else(|| uri.strip_prefix("FILE:"))?;
    let (unc, path) = match rest.strip_prefix("//") {
        Some(r) => {
            let slash = r.find('/')?;
            let host = &r[..slash];
            if host.is_empty()
                || host.eq_ignore_ascii_case("localhost")
                || host.eq_ignore_ascii_case(&hostname())
            {
                (None, &r[slash..])
            } else {
                (Some(host), &r[slash..])
            }
        }
        None => (None, rest),
    };
    let path = path.split(['?', '#']).next()?;
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    if out.contains(&0) {
        return None;
    }
    let decoded = String::from_utf8(out).ok()?.replace('/', "\\");
    match unc {
        Some(host) => Some(PathBuf::from(format!("\\\\{host}{decoded}"))),
        None => {
            // "\C:\x" → "C:\x"; a drive letter is required.
            let d = decoded.trim_start_matches('\\');
            let b = d.as_bytes();
            (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':').then(|| PathBuf::from(d))
        }
    }
}

#[cfg(windows)]
fn hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

#[cfg(unix)]
fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: gethostname writes at most len bytes into our buffer.
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

// ── Detached spawns ──────────────────────────────────────────────────────────

/// A process started to outlive the app: its own session (`setsid`, so no
/// terminal signal or process-group kill reaches it), stdio on `/dev/null`,
/// no file descriptor of ours past `exec` (Rust marks its own close-on-exec;
/// libraries' fds, a Wayland or D-Bus socket, a GPU node, may not be), no
/// parent-death signal, and reaped on a small thread so it never lingers as
/// a zombie.
///
/// ```no_run
/// use bato_shell::desktop::Detached;
/// let pid = Detached::new("mpv")
///     .args(["--force-window=immediate", "--", "/films/a.mkv"])
///     .spawn()?;
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug)]
pub struct Detached {
    cmd: Command,
    on_exit: Option<std::sync::mpsc::SyncSender<Option<i32>>>,
}

impl Detached {
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        let mut cmd = Command::new(program);
        for k in SCRUBBED_ENV {
            cmd.env_remove(k);
        }
        Self { cmd, on_exit: None }
    }

    pub fn arg(mut self, a: impl AsRef<OsStr>) -> Self {
        self.cmd.arg(a);
        self
    }

    pub fn args<I: IntoIterator<Item = S>, S: AsRef<OsStr>>(mut self, args: I) -> Self {
        self.cmd.args(args);
        self
    }

    pub fn current_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.cmd.current_dir(dir);
        self
    }

    pub fn env(mut self, k: impl AsRef<OsStr>, v: impl AsRef<OsStr>) -> Self {
        self.cmd.env(k, v);
        self
    }

    pub fn env_remove(mut self, k: impl AsRef<OsStr>) -> Self {
        self.cmd.env_remove(k);
        self
    }

    /// Hand the child an activation token (`XDG_ACTIVATION_TOKEN` and
    /// `DESKTOP_STARTUP_ID`), so the compositor lets its window take focus.
    pub fn activation_token(mut self, token: Option<&str>) -> Self {
        if let Some(t) = token.filter(|t| !t.is_empty()) {
            self.cmd.env("XDG_ACTIVATION_TOKEN", t);
            self.cmd.env("DESKTOP_STARTUP_ID", t);
        }
        self
    }

    /// Start it; the pid. `NotFound` when the program is not installed.
    pub fn spawn(mut self) -> io::Result<u32> {
        self.cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: only async-signal-safe calls (setsid, close_range, fcntl)
            // between fork and exec.
            unsafe {
                self.cmd.pre_exec(|| {
                    libc::setsid();
                    cloexec_from(3);
                    Ok(())
                });
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.cmd.creation_flags(crate::win::DETACHED_FLAGS);
        }
        let mut child = self.cmd.spawn()?;
        let pid = child.id();
        let on_exit = self.on_exit.take();
        std::thread::Builder::new()
            .name("bato-reap".into())
            .stack_size(64 << 10)
            .spawn(move || {
                let code = child.wait().ok().and_then(|s| s.code());
                if let Some(tx) = on_exit {
                    let _ = tx.try_send(code);
                }
            })?;
        Ok(pid)
    }

    /// [`spawn`](Self::spawn), then wait up to `within` for it to exit:
    /// `Ok(Some(code))` if it did, `Ok(None)` if it is still running (a
    /// launcher like `xdg-open` that stays for the app's lifetime).
    pub fn spawn_and_wait(mut self, within: Duration) -> io::Result<Option<i32>> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.on_exit = Some(tx);
        self.spawn()?;
        match rx.recv_timeout(within) {
            Ok(code) => Ok(Some(code.unwrap_or(-1))),
            Err(_) => Ok(None),
        }
    }
}

/// Mark every fd from `first` on close-on-exec. Not closing them: the error
/// pipe std uses to report a failed `exec` must stay open until then.
#[cfg(unix)]
fn cloexec_from(first: libc::c_uint) {
    // SAFETY: plain syscalls on our own descriptor table.
    unsafe {
        const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
        if libc::syscall(
            libc::SYS_close_range,
            first,
            libc::c_uint::MAX,
            CLOSE_RANGE_CLOEXEC,
        ) == 0
        {
            return;
        }
        // Before Linux 5.11: one by one, up to the soft limit.
        let max = match libc::sysconf(libc::_SC_OPEN_MAX) {
            n if n > 0 => n.min(65536) as libc::c_int,
            _ => 1024,
        };
        for fd in first as libc::c_int..max {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

/// `spawn_detached(program, args)`: [`Detached`] without options.
pub fn spawn_detached<I, S>(program: &str, args: I) -> io::Result<u32>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Detached::new(program).args(args).spawn()
}

// ── Open ─────────────────────────────────────────────────────────────────────

/// How long [`open`] waits for `xdg-open` to fail before calling it a success.
pub const OPEN_GRACE: Duration = Duration::from_secs(3);

/// `shell.openPath`: open a file or folder with its default application
/// (`xdg-open`, detached). Like Electron's error string, a quick failure is
/// reported: `xdg-open` exits 3 or 4 when nothing can open the file, 2 when
/// it does not exist.
pub fn open(path: &Path) -> io::Result<()> {
    open_activated(path, None)
}

/// [`open`] with an activation token for the opened app's window.
#[cfg(unix)]
pub fn open_activated(path: &Path, token: Option<&str>) -> io::Result<()> {
    open_with("xdg-open", path, token)
}

/// Windows: `ShellExecuteW(…, "open", path)`; the token is a Wayland notion.
#[cfg(windows)]
pub fn open_activated(path: &Path, token: Option<&str>) -> io::Result<()> {
    let _ = token;
    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} does not exist", path.display()),
        ));
    }
    shell_open(path.as_os_str())
}

/// Open a URL or path with its handler (`ShellExecuteW` "open"), Electron's
/// `shell.openExternal` / `openPath` on Windows.
#[cfg(windows)]
pub fn shell_open(target: &OsStr) -> io::Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let verb = crate::win::wide("open");
    let file = crate::win::wide(target);
    // SAFETY: NUL-terminated wide strings that outlive the call.
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    // > 32 is success; 2/3 file or path not found, 31 no association.
    match r {
        r if r > 32 => Ok(()),
        2 | 3 => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} does not exist", target.to_string_lossy()),
        )),
        31 => Err(io::Error::other(format!(
            "no application can open {}",
            target.to_string_lossy()
        ))),
        r => Err(io::Error::other(format!("ShellExecute failed ({r})"))),
    }
}

/// Open an `http(s)://` URL in the default browser (`xdg-open` on Linux,
/// `ShellExecuteW` on Windows). Other schemes are refused.
pub fn open_url(url: &str) -> io::Result<()> {
    let lower = url.trim().to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not a web address: {url}"),
        ));
    }
    #[cfg(windows)]
    {
        shell_open(OsStr::new(url.trim()))
    }
    #[cfg(unix)]
    {
        match Detached::new("xdg-open")
            .arg(url.trim())
            .spawn_and_wait(OPEN_GRACE)
        {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "xdg-open is not installed",
            )),
            Err(e) => Err(e),
            Ok(None) | Ok(Some(0)) => Ok(()),
            Ok(Some(c)) => Err(io::Error::other(format!("xdg-open failed ({c})"))),
        }
    }
}

#[cfg(unix)]
fn open_with(opener: &str, path: &Path, token: Option<&str>) -> io::Result<()> {
    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} does not exist", path.display()),
        ));
    }
    let r = Detached::new(opener)
        .arg(path)
        .activation_token(token)
        .spawn_and_wait(OPEN_GRACE);
    match r {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{opener} is not installed"),
        )),
        Err(e) => Err(e),
        Ok(None) | Ok(Some(0)) => Ok(()),
        Ok(Some(2)) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} does not exist", path.display()),
        )),
        Ok(Some(3 | 4)) => Err(io::Error::other(format!(
            "no application can open {}",
            path.display()
        ))),
        Ok(Some(c)) => Err(io::Error::other(format!("{opener} failed ({c})"))),
    }
}

// ── Reveal ───────────────────────────────────────────────────────────────────

/// What [`reveal`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Revealed {
    /// The file manager shows the item selected.
    FileManager,
    /// No FileManager1 service: its folder was opened with `xdg-open`.
    Folder,
}

/// `shell.showItemInFolder`: ask the file manager to show and select the
/// item, else open its folder with `xdg-open` (Chromium's fallback too).
pub fn reveal(path: &Path) -> io::Result<Revealed> {
    reveal_activated(path, None)
}

/// [`reveal`] with an activation token (`startup_id`), so the file
/// manager's window may take focus.
#[cfg(unix)]
pub fn reveal_activated(path: &Path, token: Option<&str>) -> io::Result<Revealed> {
    let shown = futures::executor::block_on(async {
        let conn = zbus::Connection::session().await?;
        show_items_on(&conn, &[path], token.unwrap_or("")).await
    });
    if shown.is_ok() {
        return Ok(Revealed::FileManager);
    }
    let dir = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    open_with("xdg-open", dir, token).map(|()| Revealed::Folder)
}

/// Windows: `explorer /select,"<path>"` (Electron's `showItemInFolder`);
/// a folder that does not exist is an error.
#[cfg(windows)]
pub fn reveal_activated(path: &Path, token: Option<&str>) -> io::Result<Revealed> {
    use std::os::windows::process::CommandExt;
    let _ = token;
    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} does not exist", path.display()),
        ));
    }
    let mut cmd = Command::new("explorer.exe");
    cmd.raw_arg(format!("/select,\"{}\"", path.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(crate::win::DETACHED_FLAGS);
    // explorer exits 1 even when it worked: only a failed start is an error.
    let mut child = cmd.spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(Revealed::FileManager)
}

/// `ShowItems(as uris, s startup_id)` on `conn`, for any bus.
#[cfg(unix)]
pub async fn show_items_on(
    conn: &zbus::Connection,
    paths: &[&Path],
    startup_id: &str,
) -> zbus::Result<()> {
    let uris: Vec<String> = paths.iter().map(|p| file_uri(p)).collect();
    conn.call_method(
        Some(FILE_MANAGER1),
        FILE_MANAGER1_PATH,
        Some(FILE_MANAGER1),
        "ShowItems",
        &(uris, startup_id),
    )
    .await?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn file_uris_round_trip() {
        let p = Path::new("/home/me/My Clip #1 100%.mp4");
        let u = file_uri(p);
        assert_eq!(u, "file:///home/me/My%20Clip%20%231%20100%25.mp4");
        assert_eq!(uri_to_path(&u).as_deref(), Some(p));
        assert_eq!(
            file_uri(Path::new("/tmp/caf\u{e9}")),
            "file:///tmp/caf%C3%A9"
        );
        assert_eq!(
            uri_to_path("file:///tmp/caf%C3%A9").as_deref(),
            Some(Path::new("/tmp/caf\u{e9}"))
        );
        // Non-UTF-8 bytes survive.
        use std::os::unix::ffi::OsStrExt;
        let raw = Path::new(OsStr::from_bytes(b"/tmp/\xff\xfe"));
        assert_eq!(uri_to_path(&file_uri(raw)).as_deref(), Some(raw));
    }

    #[test]
    fn uri_forms() {
        let p = Some(PathBuf::from("/a/b"));
        assert_eq!(uri_to_path("file:///a/b"), p);
        assert_eq!(uri_to_path("file://localhost/a/b"), p);
        assert_eq!(uri_to_path("file:/a/b"), p, "KDE's short form");
        assert_eq!(uri_to_path(&format!("file://{}/a/b", hostname())), p);
        assert_eq!(uri_to_path("  file:///a/b\r"), p);
        assert_eq!(uri_to_path("file://otherhost/a/b"), None);
        assert_eq!(uri_to_path("sftp://h/a/b"), None);
        assert_eq!(uri_to_path("file:a/b"), None);
        assert_eq!(uri_to_path("file:///a/%zz"), None);
        assert_eq!(uri_to_path("file:///a/%00"), None);
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bato-shell-desk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The child is in its own session, has no stdio and no fd of ours, and
    /// does not inherit the app's activation or BATO_* variables.
    #[test]
    fn detached_children_are_clean() {
        let dir = tmp("detached");
        let out = dir.join("out");
        let script = dir.join("probe");
        crate::test_support::write_script(
            &script,
            &format!(
                "#!/bin/sh\n{{ ps -o sid= -p $$; ls /proc/$$/fd | sort -n | tr '\\n' ' '; echo; \
                 echo \"tok=${{XDG_ACTIVATION_TOKEN-unset}} app=${{BATO_APP_ID-unset}} keep=${{KEEP-unset}}\"; }} > '{}.tmp'\n\
                 mv '{0}.tmp' '{0}'\n",
                out.display()
            ),
        );
        // An inheritable fd of ours (no CLOEXEC), as a library might leave.
        let null = std::fs::File::open("/dev/null").unwrap();
        use std::os::fd::AsRawFd;
        // SAFETY: F_DUPFD makes a new fd (>= 200) without CLOEXEC; closed below.
        let leak_fd = unsafe { libc::fcntl(null.as_raw_fd(), libc::F_DUPFD, 200) };
        assert!(leak_fd >= 200);
        let pid = Detached::new(&script)
            .env("XDG_ACTIVATION_TOKEN", "should-not-pass")
            .env("KEEP", "yes")
            .activation_token(None)
            .spawn()
            .unwrap();
        let end = std::time::Instant::now() + Duration::from_secs(20);
        while !out.exists() && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        let text = std::fs::read_to_string(&out).unwrap();
        let mut lines = text.lines();
        let sid: u32 = lines.next().unwrap().trim().parse().unwrap();
        assert_eq!(sid, pid, "leads its own session");
        let fds: Vec<i32> = lines
            .next()
            .unwrap()
            .split_whitespace()
            .map(|f| f.parse().unwrap())
            .collect();
        // 0-2 on /dev/null, plus whatever `ls` itself opens for the listing.
        assert!(!fds.contains(&leak_fd), "leaked fd {leak_fd}: {fds:?}");
        assert!(fds.starts_with(&[0, 1, 2]), "{fds:?}");
        // env: an explicit .env() wins over the scrub, but activation_token(None) adds none.
        assert_eq!(
            lines.next().unwrap(),
            "tok=should-not-pass app=unset keep=yes"
        );
        // SAFETY: our own fd from F_DUPFD above.
        unsafe { libc::close(leak_fd) };
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scrubbed_and_token_env() {
        let d = Detached::new("x").activation_token(Some("T"));
        let envs: Vec<_> = d
            .cmd
            .get_envs()
            .map(|(k, v)| (k.to_owned(), v.map(OsStr::to_owned)))
            .collect();
        assert!(envs.contains(&("BATO_APP_ID".into(), None)));
        assert!(envs.contains(&("BATO_SESSION_SECRET".into(), None)));
        assert!(envs.contains(&("XDG_ACTIVATION_TOKEN".into(), Some("T".into()))));
        assert!(envs.contains(&("DESKTOP_STARTUP_ID".into(), Some("T".into()))));
    }

    #[test]
    fn missing_program_is_not_found() {
        let e = spawn_detached("/nonexistent/prog", ["a"]).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
    }

    /// `open` reports xdg-open's exit codes; a fake opener stands in.
    #[test]
    fn open_reports_quick_failures() {
        let dir = tmp("open");
        let file = dir.join("a b.txt");
        std::fs::write(&file, "x").unwrap();
        let log = dir.join("log");
        let ok = dir.join("ok-open");
        crate::test_support::write_script(
            &ok,
            &format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", log.display()),
        );
        open_with(ok.to_str().unwrap(), &file, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            file.to_str().unwrap()
        );
        let none = dir.join("none-open");
        crate::test_support::write_script(&none, "#!/bin/sh\nexit 3\n");
        let e = open_with(none.to_str().unwrap(), &file, None).unwrap_err();
        assert!(e.to_string().starts_with("no application can open"), "{e}");
        // Still running after the grace period counts as opened.
        let slow = dir.join("slow-open");
        crate::test_support::write_script(&slow, "#!/bin/sh\nexec sleep 30\n");
        let t = std::time::Instant::now();
        open_with(slow.to_str().unwrap(), &file, None).unwrap();
        assert!(t.elapsed() >= OPEN_GRACE);
        assert_eq!(
            open(&dir.join("missing")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Reveal against a real (private) session bus with a stand-in
    /// FileManager1, as Nautilus or Dolphin would answer it. Skipped where
    /// there is no dbus-daemon.
    #[test]
    fn reveal_calls_file_manager1_show_items() {
        use std::io::BufRead;
        use std::sync::{Arc, Mutex};
        let Ok(mut daemon) = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            eprintln!("skipped: no dbus-daemon");
            return;
        };
        let mut addr = String::new();
        io::BufReader::new(daemon.stdout.take().unwrap())
            .read_line(&mut addr)
            .unwrap();
        let addr = addr.trim().to_string();

        type Seen = Arc<Mutex<Vec<(Vec<String>, String)>>>;
        struct Fm(Seen);
        #[zbus::interface(name = "org.freedesktop.FileManager1")]
        impl Fm {
            fn show_items(&self, uris: Vec<String>, startup_id: String) {
                self.0.lock().unwrap().push((uris, startup_id));
            }
        }

        let seen: Seen = Default::default();
        let result = futures::executor::block_on(async {
            let _fm = zbus::connection::Builder::address(addr.as_str())?
                .name(FILE_MANAGER1)?
                .serve_at(FILE_MANAGER1_PATH, Fm(seen.clone()))?
                .build()
                .await?;
            let client = zbus::connection::Builder::address(addr.as_str())?
                .build()
                .await?;
            show_items_on(
                &client,
                &[Path::new("/d/My clip.mp4"), Path::new("/d/b")],
                "tok-1",
            )
            .await
        });
        let _ = daemon.kill();
        let _ = daemon.wait();
        result.unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            [(
                vec![
                    "file:///d/My%20clip.mp4".to_string(),
                    "file:///d/b".to_string()
                ],
                "tok-1".to_string()
            )]
        );
    }
}
