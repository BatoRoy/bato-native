//! Single instance: the second launch hands its command line to the first
//! and exits (Electron's `requestSingleInstanceLock` + `second-instance`).
//!
//! A unix socket under `$XDG_RUNTIME_DIR/<app_id>/` (mode 0700), guarded by
//! an `flock`ed lock file beside it:
//!
//! - **The first launch** takes the lock (released by the kernel even on a
//!   crash, so a stale socket never blocks a launch), binds `instance.sock`
//!   and gets a [`Primary`]: forwarded [`Request`]s arrive on its channel
//!   (`iced_ext::instance_requests`).
//! - **A later launch** finds the lock taken, connects, sends its argv, cwd
//!   and activation token, waits for the acknowledgement and gets
//!   [`Instance::Forwarded`]: exit at once. While the first one is still
//!   binding, it retries for up to [`CONNECT_RETRY`].
//!
//! Only the same user can connect (the directory is 0700, and the peer's
//! uid is checked). The app id is `BATO_APP_ID` or the slug, so a dev build
//! with its own id runs beside the installed one.
//!
//! **Raising the window:** the second launch's `XDG_ACTIVATION_TOKEN` (from
//! the launcher, or from BatoBrowse's FileManager1 helper) travels in the
//! request; hand it to `iced_ext::activate`. winit cannot activate an
//! existing window with a foreign token (its `focus_window` is a no-op on
//! Wayland), so bato-shell does the `xdg_activation_v1.activate` itself.
//!
//! ```no_run
//! use bato_shell::{AppIdentity, instance};
//! const APP: AppIdentity = AppIdentity::new("BatoBrowse", "batobrowse", "1.0.0");
//! let primary = match instance::acquire(&APP) {
//!     Ok(instance::Instance::Primary(p)) => Some(p),
//!     Ok(instance::Instance::Forwarded) => std::process::exit(0),
//!     // Something is wrong with the socket: run unshared rather than not at all.
//!     Err(e) => { eprintln!("single instance: {e}"); None }
//! };
//! // subscription(): iced_ext::instance_requests(primary).map(Msg::Forwarded)
//! // update(Msg::Forwarded(r)): open r.paths(), r.flag("pick"), then
//! //     iced_ext::activate(window_id, r.activation_token.clone())
//! ```

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc;

use crate::identity::AppIdentity;

const MAGIC: &[u8] = b"bato-instance 1\n";
const ACK: &[u8] = b"ok\n";
/// A request larger than this is refused.
const MAX_REQUEST: u64 = 1 << 20;
/// How long a later launch keeps trying to reach a first one that is still
/// starting.
pub const CONNECT_RETRY: Duration = Duration::from_secs(5);
/// How long either side waits for the other once connected.
pub const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// A forwarded launch.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Request {
    /// The arguments after the program name.
    pub argv: Vec<OsString>,
    /// The launch's working directory (relative paths are relative to it).
    pub cwd: PathBuf,
    /// `XDG_ACTIVATION_TOKEN` (or X11's `DESKTOP_STARTUP_ID`) of the launch.
    pub activation_token: Option<String>,
}

impl Request {
    /// This process's launch.
    pub fn current() -> Self {
        let token = ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()));
        Self {
            argv: std::env::args_os().skip(1).collect(),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            activation_token: token,
        }
    }

    /// The value of `--name=value` (the last one wins).
    pub fn flag(&self, name: &str) -> Option<&OsStr> {
        let prefix = format!("--{name}=");
        self.options()
            .filter_map(|a| a.as_bytes().strip_prefix(prefix.as_bytes()))
            .map(OsStr::from_bytes)
            .last()
    }

    /// Whether `--name` (or `--name=…`) is present.
    pub fn has_flag(&self, name: &str) -> bool {
        let bare = format!("--{name}");
        self.options().any(|a| {
            let b = a.as_bytes();
            b == bare.as_bytes()
                || (b.starts_with(bare.as_bytes()) && b.get(bare.len()) == Some(&b'='))
        })
    }

    /// Arguments that are not options (`-x`, `--x…`; everything after `--`
    /// counts), resolved against [`cwd`](Self::cwd). Not checked for
    /// existence.
    pub fn paths(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut options_done = false;
        for a in &self.argv {
            if !options_done && a == "--" {
                options_done = true;
                continue;
            }
            if !options_done && a.as_bytes().starts_with(b"-") && a.len() > 1 {
                continue;
            }
            let p = Path::new(a);
            out.push(if p.is_absolute() {
                p.to_path_buf()
            } else {
                self.cwd.join(p)
            });
        }
        out
    }

    fn options(&self) -> impl Iterator<Item = &OsString> {
        self.argv
            .iter()
            .take_while(|a| *a != "--")
            .filter(|a| a.as_bytes().starts_with(b"-"))
    }

    /// `MAGIC`, then NUL-terminated fields: token, cwd, argv….
    fn encode(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        let mut field = |b: &[u8]| {
            out.extend_from_slice(b);
            out.push(0);
        };
        field(self.activation_token.as_deref().unwrap_or("").as_bytes());
        field(self.cwd.as_os_str().as_bytes());
        for a in &self.argv {
            field(a.as_bytes());
        }
        out
    }

    fn decode(b: &[u8]) -> Option<Self> {
        let rest = b.strip_prefix(MAGIC)?;
        let body = rest.strip_suffix(b"\0")?;
        let mut fields = body.split(|&c| c == 0);
        let token = String::from_utf8(fields.next()?.to_vec()).ok()?;
        let cwd = PathBuf::from(OsString::from_vec(fields.next()?.to_vec()));
        let argv = fields.map(|f| OsString::from_vec(f.to_vec())).collect();
        Some(Self {
            argv,
            cwd,
            activation_token: (!token.is_empty()).then_some(token),
        })
    }
}

/// The outcome of [`acquire`].
#[derive(Debug)]
pub enum Instance {
    /// This is the first instance: keep it alive while the app runs.
    Primary(Primary),
    /// Another instance took the request: exit now.
    Forwarded,
}

/// The running instance's end. Clones share it; the socket is removed when
/// the last one drops.
#[derive(Debug, Clone)]
pub struct Primary {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    socket: PathBuf,
    stop: Arc<AtomicBool>,
    requests: Mutex<Option<mpsc::UnboundedReceiver<Request>>>,
    /// Holds the lock; closed (unlocked) after the socket is removed.
    _lock: std::fs::File,
}

impl PartialEq for Primary {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &o.inner)
    }
}

impl Eq for Primary {}

impl std::hash::Hash for Primary {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.inner.socket.hash(h);
    }
}

impl Primary {
    pub fn socket(&self) -> &Path {
        &self.inner.socket
    }

    /// The forwarded requests, in arrival order. Can be taken once; they
    /// queue up until then.
    pub fn take_requests(&self) -> Option<mpsc::UnboundedReceiver<Request>> {
        self.inner
            .requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = UnixStream::connect(&self.socket);
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// The directory for `app`'s socket: `$XDG_RUNTIME_DIR/<app_id>`, else
/// `/tmp/<app_id>-<uid>`.
pub fn dir_for(app: &AppIdentity) -> PathBuf {
    let id = app.app_id().replace('/', "_");
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(r) => PathBuf::from(r).join(id),
        // SAFETY: getuid cannot fail.
        None => std::env::temp_dir().join(format!("{id}-{}", unsafe { libc::getuid() })),
    }
}

/// Become the primary instance, or forward this launch ([`Request::current`])
/// to the one running.
pub fn acquire(app: &AppIdentity) -> io::Result<Instance> {
    acquire_at(&dir_for(app), Request::current())
}

/// [`acquire`] with an explicit directory and request.
pub fn acquire_at(dir: &Path, request: Request) -> io::Result<Instance> {
    private_dir(dir)?;
    let lock_path = dir.join("instance.lock");
    let socket = dir.join("instance.sock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(&lock_path)?;
    if try_lock(&lock)? {
        return listen(socket, lock).map(Instance::Primary);
    }
    forward(&socket, &request)?;
    Ok(Instance::Forwarded)
}

fn try_lock(f: &std::fs::File) -> io::Result<bool> {
    use std::os::fd::AsRawFd;
    loop {
        // SAFETY: flock on our own open file.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(true);
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EWOULDBLOCK) => return Ok(false),
            Some(libc::EINTR) => continue,
            _ => return Err(e),
        }
    }
}

/// Create `dir` 0700, or check that an existing one is ours and private.
fn private_dir(dir: &Path) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .recursive(true)
                .create(dir)?;
            return Ok(());
        }
        Err(e) => return Err(e),
    }
    let meta = std::fs::symlink_metadata(dir)?;
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    if !meta.is_dir() || meta.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not a directory of ours", dir.display()),
        ));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn listen(socket: PathBuf, lock: std::fs::File) -> io::Result<Primary> {
    // We hold the lock: any socket file is a dead instance's.
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::unbounded();
    {
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("bato-instance".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(conn) = conn else { continue };
                    if let Some((req, mut conn)) = receive(conn) {
                        // Queued before the acknowledgement: once a launch
                        // returns Forwarded, its request is here.
                        let _ = tx.unbounded_send(req);
                        let _ = conn.write_all(ACK);
                    }
                }
            })?;
    }
    Ok(Primary {
        inner: Arc::new(Inner {
            socket,
            stop,
            requests: Mutex::new(Some(rx)),
            _lock: lock,
        }),
    })
}

fn peer_uid(s: &UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: SO_PEERCRED fills a ucred of the given size.
    let r = unsafe {
        libc::getsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    (r == 0).then_some(cred.uid)
}

/// Read one request (to EOF).
fn receive(mut conn: UnixStream) -> Option<(Request, UnixStream)> {
    // SAFETY: getuid cannot fail.
    if peer_uid(&conn) != Some(unsafe { libc::getuid() }) {
        return None;
    }
    conn.set_read_timeout(Some(IO_TIMEOUT)).ok()?;
    conn.set_write_timeout(Some(IO_TIMEOUT)).ok()?;
    let mut buf = Vec::new();
    (&mut conn)
        .take(MAX_REQUEST + 1)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.len() as u64 > MAX_REQUEST {
        return None;
    }
    Some((Request::decode(&buf)?, conn))
}

/// Send `request` to the running instance and wait for its acknowledgement.
fn forward(socket: &Path, request: &Request) -> io::Result<()> {
    let end = Instant::now() + CONNECT_RETRY;
    let mut conn = loop {
        match UnixStream::connect(socket) {
            Ok(c) => break c,
            // Not bound yet (the first instance is starting), or a race
            // with its exit.
            Err(e) if Instant::now() < end => {
                if !matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) {
                    return Err(e);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                return Err(io::Error::new(
                    e.kind(),
                    format!(
                        "the running instance does not answer on {}: {e}",
                        socket.display()
                    ),
                ));
            }
        }
    };
    conn.set_read_timeout(Some(IO_TIMEOUT))?;
    conn.set_write_timeout(Some(IO_TIMEOUT))?;
    conn.write_all(&request.encode())?;
    conn.shutdown(std::net::Shutdown::Write)?;
    let mut ack = Vec::new();
    (&mut conn).take(16).read_to_end(&mut ack)?;
    if ack != ACK {
        return Err(io::Error::other(
            "the running instance did not take the request",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bato-shell-inst-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn req(args: &[&str]) -> Request {
        Request {
            argv: args.iter().map(OsString::from).collect(),
            cwd: "/home/me".into(),
            activation_token: Some("tok-42".into()),
        }
    }

    #[test]
    fn requests_round_trip_byte_exact() {
        let mut r = req(&["--pick=/run/x.sock", "a b", ""]);
        r.argv.push(OsString::from_vec(b"\xff\xfe".to_vec()));
        assert_eq!(Request::decode(&r.encode()), Some(r.clone()));
        r.activation_token = None;
        assert_eq!(Request::decode(&r.encode()), Some(r));
        assert_eq!(Request::decode(b"garbage\0"), None);
    }

    #[test]
    fn flags_and_paths() {
        let r = req(&[
            "--pick=/run/user/1000/batobrowse/pick-1.sock",
            "--verbose",
            "docs",
            "/etc",
            "-",
            "--",
            "--not-a-flag",
        ]);
        assert_eq!(
            r.flag("pick"),
            Some(OsStr::new("/run/user/1000/batobrowse/pick-1.sock"))
        );
        assert!(r.has_flag("verbose") && r.has_flag("pick") && !r.has_flag("pic"));
        assert_eq!(r.flag("verbose"), None);
        assert_eq!(
            r.paths(),
            [
                PathBuf::from("/home/me/docs"),
                "/etc".into(),
                "/home/me/-".into(),
                "/home/me/--not-a-flag".into()
            ]
        );
        assert!(!r.has_flag("not-a-flag"), "after --");
    }

    #[test]
    fn second_launch_forwards_to_the_first() {
        let dir = tmp("fwd");
        let Instance::Primary(p) = acquire_at(&dir, req(&[])).unwrap() else {
            panic!("first launch must be primary");
        };
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        let mut rx = p.take_requests().unwrap();
        assert!(p.take_requests().is_none(), "taken once");
        for n in 0..3 {
            let r = req(&["--pick=/x.sock", &format!("file{n}")]);
            assert!(matches!(
                acquire_at(&dir, r.clone()).unwrap(),
                Instance::Forwarded
            ));
            let got = futures::executor::block_on(rx.next()).unwrap();
            assert_eq!(got, r);
        }
        drop(p);
        assert!(!dir.join("instance.sock").exists(), "socket removed");
        // The next launch is primary again.
        assert!(matches!(
            acquire_at(&dir, req(&[])).unwrap(),
            Instance::Primary(_)
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A crashed first instance leaves its socket file; the kernel released
    /// its lock, so the next launch takes over instead of hanging.
    #[test]
    fn stale_socket_does_not_block() {
        let dir = tmp("stale");
        private_dir(&dir).unwrap();
        // A socket nobody listens on.
        drop(UnixListener::bind(dir.join("instance.sock")).unwrap());
        assert!(dir.join("instance.sock").exists());
        assert!(matches!(
            acquire_at(&dir, req(&[])).unwrap(),
            Instance::Primary(_)
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Launches racing each other: exactly one becomes primary, every other
    /// one is delivered to it.
    #[test]
    fn racing_launches_elect_one_primary() {
        let dir = tmp("race");
        let handles: Vec<_> = (0..8)
            .map(|n| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    let r = req(&[&format!("f{n}")]);
                    match acquire_at(&dir, r).unwrap() {
                        Instance::Primary(p) => Some(p),
                        Instance::Forwarded => None,
                    }
                })
            })
            .collect();
        let primaries: Vec<Primary> = handles
            .into_iter()
            .filter_map(|h| h.join().unwrap())
            .collect();
        assert_eq!(primaries.len(), 1);
        let mut rx = primaries[0].take_requests().unwrap();
        let mut got = 0;
        while rx.try_recv().is_ok() {
            got += 1;
        }
        assert_eq!(got, 7);
        drop(primaries);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn foreign_directories_are_refused() {
        // /tmp itself is not ours (root's, sticky).
        let e = private_dir(Path::new("/")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    }
}
