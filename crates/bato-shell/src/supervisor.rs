//! Sidecar supervisor for a bundled Go server (feature `supervisor`).
//!
//! A port of `backend.js` (BatoGit) and the QML flavor's `internal/backend`:
//! find `app-server`, spawn it as `app-server --port N` on a free loopback
//! port, poll `GET /api/health` every 100 ms for up to 8 s, stop it with
//! SIGTERM and then SIGKILL after 2 s. All of it off the UI thread: the URL is
//! known as soon as the child is spawned (the port is chosen first), so the
//! first frame never waits for the server; health arrives as an [`Event`].
//!
//! What the Electron supervisor lacked:
//! - **The child dies with the UI**, even on a crash or SIGKILL: it gets
//!   `PR_SET_PDEATHSIG(SIGTERM)` and runs in its own process group, which a
//!   normal stop signals as a whole.
//! - **Restart policy**: [`Restart::Never`] by default, like `backend.js` and
//!   `backend.go` (an exit is reported, the UI shows the backend offline);
//!   [`Restart::OnCrash`] restarts after a non-zero exit, a bounded number of
//!   times, on the same port so the URL stays valid.
//!
//! **Fixed port, attach or spawn** ([`Config::port`],
//! [`Supervisor::attach_or_start`]): a daemon-capable server on a claimed
//! port (BatoSound on 42108, which its Stream Deck plugin and `systemd
//! --user` unit also call) is used as is when something healthy already
//! answers there, and spawned on that port otherwise, as Electron's
//! `backend.js` did. In a UI, the probe ([`answers`]) goes on a worker and
//! the spawn ([`Supervisor::start_bundled`]) stays non-blocking:
//!
//! ```no_run
//! # use bato_shell::{AppIdentity, Settings, supervisor::*};
//! # const APP: AppIdentity = AppIdentity::new("BatoSound", "batosound", "1.0.0");
//! # let settings = Settings::default();
//! let cfg = Config { port: Some(42108), ..Config::default() };
//! match Supervisor::attach_or_start(&APP, &settings, cfg) {
//!     Attach::Attached { url } => { /* someone else's server: never stopped by us */ }
//!     Attach::Spawned(server) => { /* ours, on 42108; dropping it stops it */ }
//!     Attach::Offline { url } => { /* nothing answers and nothing was spawned */ }
//! }
//! ```
//!
//! **When to spawn** ([`wanted`]): only when nothing outranks the bundled
//! server (no `BATO_BACKEND_URL`, no saved `backendUrl`; the QML rule), and
//! only for an installed build or with `APP_SPAWN_SERVER=1` in dev, so the
//! `make dev-server` workflow is unchanged.
//!
//! **`BATO_AUTH_URL` (decision):** removed from the child's environment, with
//! `BATO_AUTH_PUBLIC_URL`. The suite exports it shell-wide, and the template
//! server reads it as "deployed": it then binds every interface and gates the
//! API behind bato-auth, so the UI (which has no token) gets 401 from its own
//! session server, and that server is reachable from the LAN. Without it the
//! server binds 127.0.0.1 and trusts loopback callers as admin, which is
//! exactly what a session-bound child is. An app whose bundled server really
//! must verify bato-auth identities sets [`Config::strip_env`] to keep it and
//! sends a Bearer token from the UI.
//!
//! **Session secret.** Loopback is not a boundary: any local process (and a
//! web page that finds the port) can call a server that trusts 127.0.0.1.
//! So each start makes a random secret, passes it to the server as
//! `BATO_SESSION_SECRET` ([`ENV_SESSION_SECRET`]) and registers it with
//! [`crate::http`], which sends it as `X-Bato-Session` with every request
//! to the bundled URL. A server that knows the convention refuses requests
//! without it; one that does not ignores both. [`Config::session_secret`]
//! turns it off.
//!
//! **Apps like BatoFetch and BatoBrowse** need more than `--port N`:
//!
//! ```no_run
//! # use bato_shell::{AppIdentity, Settings, backend::SavedUrl, supervisor::*};
//! # const APP: AppIdentity = AppIdentity::new("BatoBrowse", "batobrowse", "1.0.0");
//! # let settings = Settings::default();
//! let cfg = Config {
//!     // Staged as `app-server` (the template switch), seen as this in `ps`
//!     // and `/proc/<pid>/comm`.
//!     process_name: Some("batobrowse-server".into()),
//!     args: vec!["--data-dir".into(), "/home/me/.config/batobrowse".into()],
//!     env: vec![("APP_MODE".into(), "bundled".into())],
//!     // Bundled tools first on the server's PATH.
//!     path_prepend: vec!["/opt/batofetch/bin".into()],
//!     health_timeout: std::time::Duration::from_secs(15),
//!     restart: Restart::OnCrash { max: 3, backoff: std::time::Duration::from_millis(500) },
//!     // No Server address card: a saved backendUrl must not stop the spawn.
//!     saved_backend: SavedUrl::Ignore,
//!     ..Config::default()
//! };
//! let server = Supervisor::start_bundled(&APP, &settings, cfg);
//! ```

use std::io;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use futures::channel::mpsc;

use crate::identity::{self, AppIdentity};
use crate::settings::Settings;

pub const SERVER_NAME: &str = "app-server";
/// Explicit server binary (dev, tests).
pub const ENV_SERVER_BIN: &str = "APP_SERVER_BIN";
/// `1`: spawn in a dev build too.
pub const ENV_SPAWN_SERVER: &str = "APP_SPAWN_SERVER";
pub const STRIPPED_ENV: [&str; 2] = ["BATO_AUTH_URL", "BATO_AUTH_PUBLIC_URL"];
/// The per-session secret, in the server's environment.
pub const ENV_SESSION_SECRET: &str = "BATO_SESSION_SECRET";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Restart {
    Never,
    /// Restart after a non-zero exit or a signal, at most `max` times, waiting
    /// `backoff × attempt` first.
    OnCrash {
        max: u32,
        backoff: Duration,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Config {
    /// Binary name next to the client, and the stem of `dist/<name>-linux-<arch>`.
    pub server_name: String,
    /// `None`: a free loopback port each start. `Some(p)`: always `p` (a
    /// claimed suite port that other clients call); the spawn fails with
    /// `AddrInUse` when something already listens there, and
    /// [`Supervisor::attach_or_start`] uses a healthy server there instead.
    pub port: Option<u16>,
    /// How long [`answers`] waits for an already running server (Electron's
    /// `isHealthy(500)`).
    pub attach_timeout: Duration,
    /// Extra arguments after `--port N`.
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Variables removed from the inherited environment ([`STRIPPED_ENV`]).
    pub strip_env: Vec<String>,
    pub health_path: String,
    pub health_timeout: Duration,
    pub health_interval: Duration,
    pub stop_grace: Duration,
    pub restart: Restart,
    /// Run the binary under this name (`argv[0]` and `/proc/<pid>/comm`,
    /// through a symlink in `$XDG_RUNTIME_DIR`), e.g. `batobrowse-server`
    /// for a binary staged as `app-server`. `None`: its own name.
    pub process_name: Option<String>,
    /// Directories put before the inherited `PATH` (or before a `PATH` in
    /// [`env`](Self::env)): bundled tools the server shells out to.
    pub path_prepend: Vec<PathBuf>,
    /// Pass a random per-session secret and send it with every request to
    /// the server (see the module docs). On by default.
    pub session_secret: bool,
    /// Whether a saved `backendUrl` stops the spawn ([`wanted`]).
    pub saved_backend: crate::backend::SavedUrl,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_name: SERVER_NAME.into(),
            port: None,
            attach_timeout: Duration::from_millis(500),
            args: Vec::new(),
            env: Vec::new(),
            strip_env: STRIPPED_ENV.iter().map(|s| s.to_string()).collect(),
            health_path: "/api/health".into(),
            health_timeout: Duration::from_secs(8),
            health_interval: Duration::from_millis(100),
            stop_grace: Duration::from_secs(2),
            restart: Restart::Never,
            process_name: None,
            path_prepend: Vec::new(),
            session_secret: true,
            saved_backend: crate::backend::SavedUrl::Honor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event {
    /// `/api/health` answered 2xx.
    Healthy {
        url: String,
    },
    /// No healthy answer within `health_timeout`, or the child died first.
    Unhealthy {
        url: String,
    },
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    Restarting {
        attempt: u32,
    },
}

/// Should this launch spawn the bundled server?
pub fn wanted(app: &AppIdentity, settings: &Settings) -> bool {
    wanted_with(app, settings, crate::backend::SavedUrl::Honor)
}

/// [`wanted`], with a saved `backendUrl` honoured or ignored
/// ([`Config::saved_backend`]).
pub fn wanted_with(
    app: &AppIdentity,
    settings: &Settings,
    saved: crate::backend::SavedUrl,
) -> bool {
    crate::backend::may_spawn_bundled_with(settings, saved)
        && (app.is_installed_build()
            || std::env::var_os(ENV_SPAWN_SERVER).is_some_and(|v| v == "1"))
}

/// The server binary: `APP_SERVER_BIN`, else `<client dir>/<name>` (an
/// installed release: `bato install` unpacks both side by side), else
/// `dist/<name>-linux-<arch>` up to three levels above the client (a
/// checkout: `app-client-native/target/release/` → repo `dist/`). Not looked
/// up on PATH: a generic `app-server` there could be anyone's.
pub fn find_binary(cfg: &Config) -> Option<PathBuf> {
    let env = std::env::var_os(ENV_SERVER_BIN)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    find_binary_from(
        env.as_deref(),
        identity::current_exe().as_deref(),
        &cfg.server_name,
    )
}

fn find_binary_from(env: Option<&Path>, exe: Option<&Path>, name: &str) -> Option<PathBuf> {
    if let Some(p) = env {
        return executable(p).then(|| p.to_path_buf());
    }
    let dir = exe?.parent()?;
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        a => a,
    };
    #[cfg(unix)]
    let dev = format!("{name}-linux-{arch}");
    // Windows: `<name>.exe` beside the client, `dist/<name>-windows-amd64.exe`
    // in a checkout.
    #[cfg(windows)]
    let (name, dev) = (format!("{name}.exe"), format!("{name}-windows-{arch}.exe"));
    std::iter::once(dir.join(name))
        .chain(
            dir.ancestors()
                .skip(1)
                .take(3)
                .map(|d| d.join("dist").join(&dev)),
        )
        .find(|p| executable(p))
}

/// A regular file we can execute; repairs a lost exec bit (`bato install`
/// only chmods declared bins, and the bundled server is not one).
fn executable(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    if meta.permissions().mode() & 0o111 == 0 {
        return std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).is_ok();
    }
    true
}

/// `http://127.0.0.1:<port>` for a [`Config::port`].
pub fn fixed_url(cfg: &Config) -> Option<String> {
    cfg.port.map(|p| format!("http://127.0.0.1:{p}"))
}

/// Does a healthy server answer `cfg.health_path` at `url` within
/// [`Config::attach_timeout`]? Blocking: call it from a worker.
pub fn answers(url: &str, cfg: &Config) -> bool {
    crate::http::Http::new(url)
        .with_timeout(cfg.attach_timeout)
        .send(crate::http::Method::Get, &cfg.health_path, None)
        .is_ok_and(|r| r.ok())
}

/// What [`Supervisor::attach_or_start`] found.
#[derive(Debug)]
pub enum Attach {
    /// A healthy server already answered on the fixed port (the user's
    /// daemon, another client's server). Not ours: nothing stops it.
    Attached { url: String },
    /// Nothing answered: ours, spawned on the fixed port.
    Spawned(Supervisor),
    /// Nothing answered and nothing was spawned (not [`wanted`], no binary,
    /// or the port is taken by something unhealthy); the UI shows the
    /// backend offline and may retry `url`.
    Offline { url: String },
}

impl Attach {
    /// Where the server is (or would be).
    pub fn url(&self) -> &str {
        match self {
            Attach::Attached { url } | Attach::Offline { url } => url,
            Attach::Spawned(s) => s.url(),
        }
    }
}

/// Ask the kernel for a free loopback port (bind :0, read it back, close).
/// Racy in theory, like Electron's trick; the window is tiny.
pub fn free_port() -> io::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

#[derive(Default)]
struct State {
    /// The running child (also its process group id), until it is reaped.
    pid: Option<i32>,
    stopping: bool,
    healthy: Option<bool>,
    finished: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    cv: Condvar,
}

/// A random secret: 32 bytes from the system RNG (`BCryptGenRandom`), hex.
#[cfg(windows)]
pub fn new_secret() -> String {
    use windows_sys::Win32::Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
    };
    let mut buf = [0u8; 32];
    // SAFETY: fills our buffer of the given length.
    let r = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            buf.as_mut_ptr(),
            buf.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    assert_eq!(r, 0, "no source of randomness");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// A random secret: 32 bytes from the kernel, hex.
#[cfg(unix)]
pub fn new_secret() -> String {
    let mut buf = [0u8; 32];
    let mut filled = 0;
    while filled < buf.len() {
        // SAFETY: writes at most the remaining length into our buffer.
        let n =
            unsafe { libc::getrandom(buf[filled..].as_mut_ptr().cast(), buf.len() - filled, 0) };
        if n > 0 {
            filled += n as usize;
        } else if io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            // No getrandom (very old kernel): the device.
            use std::io::Read;
            std::fs::File::open("/dev/urandom")
                .and_then(|mut f| f.read_exact(&mut buf))
                .expect("no source of randomness");
            break;
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Windows: no alias (a symlink needs a privilege there); the server keeps
/// its file name, `process_name` is ignored.
#[cfg(windows)]
fn alias(bin: &Path, _name: &str, _port: u16) -> io::Result<(PathBuf, PathBuf)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("no process alias on Windows for {}", bin.display()),
    ))
}

/// A symlink named `name` to `bin` in a private directory, so the server
/// runs under that name. Returns (directory, link).
#[cfg(unix)]
fn alias(bin: &Path, name: &str, port: u16) -> io::Result<(PathBuf, PathBuf)> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("bad process name {name:?}"),
        ));
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("bato-shell-{}-{port}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(&dir)?;
    let link = dir.join(name);
    let target = std::fs::canonicalize(bin)?;
    std::os::unix::fs::symlink(target, &link)?;
    Ok((dir, link))
}

/// A running bundled server. Dropping it stops the server.
pub struct Supervisor {
    url: String,
    secret: Option<String>,
    alias_dir: Option<PathBuf>,
    shared: Arc<Shared>,
    events: Option<mpsc::UnboundedReceiver<Event>>,
    monitor: Option<JoinHandle<()>>,
    grace: Duration,
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor")
            .field("url", &self.url)
            .field("pid", &self.pid())
            .finish()
    }
}

impl Supervisor {
    /// [`wanted_with`] + [`find_binary`] + [`start`](Self::start). `None`
    /// when the app should not spawn, has no server, or the spawn failed
    /// (logged to stderr; the UI shows the backend offline, the suite rule).
    pub fn start_bundled(app: &AppIdentity, settings: &Settings, cfg: Config) -> Option<Self> {
        if !wanted_with(app, settings, cfg.saved_backend) {
            return None;
        }
        let Some(bin) = find_binary(&cfg) else {
            eprintln!(
                "[bato-shell] no bundled {} found; backend offline",
                cfg.server_name
            );
            return None;
        };
        Self::start(&bin, cfg)
            .inspect_err(|e| eprintln!("[bato-shell] could not start {}: {e}", bin.display()))
            .ok()
    }

    /// Fixed-port mode ([`Config::port`]): use a healthy server that already
    /// answers there, else [`start_bundled`](Self::start_bundled) on that
    /// port. Blocks for up to [`Config::attach_timeout`] (the probe); a UI
    /// calls [`answers`] on a worker and `start_bundled` itself instead.
    /// Without a fixed port there is nothing to attach to: `start_bundled`.
    pub fn attach_or_start(app: &AppIdentity, settings: &Settings, cfg: Config) -> Attach {
        let url = fixed_url(&cfg);
        if let Some(url) = &url
            && answers(url, &cfg)
        {
            return Attach::Attached { url: url.clone() };
        }
        match Self::start_bundled(app, settings, cfg) {
            Some(s) => Attach::Spawned(s),
            None => Attach::Offline {
                url: url.unwrap_or_default(),
            },
        }
    }

    /// Spawn `bin --port N` on [`Config::port`] or a free port and return at
    /// once; health is checked on the supervisor's thread.
    pub fn start(bin: &Path, mut cfg: Config) -> io::Result<Self> {
        let port = match cfg.port {
            Some(p) => {
                // Taken (by an unhealthy or foreign listener): say so now
                // rather than spawn a server that cannot bind.
                std::net::TcpListener::bind(("127.0.0.1", p))
                    .map_err(|e| io::Error::new(e.kind(), format!("port {p} on 127.0.0.1: {e}")))?;
                p
            }
            None => free_port()?,
        };
        let url = format!("http://127.0.0.1:{port}");
        let secret = cfg.session_secret.then(new_secret);
        if let Some(s) = &secret {
            cfg.env.retain(|(k, _)| k != ENV_SESSION_SECRET);
            cfg.env.push((ENV_SESSION_SECRET.into(), s.clone()));
            // Before the spawn: the health check already carries it.
            crate::http::register_session(&url, s);
        }
        // Windows has no alias: run the binary under its own name.
        #[cfg(windows)]
        cfg.process_name.take();
        let (alias_dir, exec) = match cfg.process_name.clone() {
            Some(name) => match alias(bin, &name, port) {
                Ok((dir, link)) => (Some(dir), link),
                Err(e) => {
                    if secret.is_some() {
                        crate::http::unregister_session(&url);
                    }
                    return Err(e);
                }
            },
            None => (None, bin.to_path_buf()),
        };
        let cleanup = |url: &str, dir: &Option<PathBuf>| {
            if secret.is_some() {
                crate::http::unregister_session(url);
            }
            if let Some(d) = dir {
                let _ = std::fs::remove_dir_all(d);
            }
        };
        let bin = exec.as_path();
        let shared = Arc::new(Shared::default());
        let (tx, rx) = mpsc::unbounded();
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let grace = cfg.stop_grace;
        let monitor = {
            let (bin, url, shared) = (bin.to_path_buf(), url.clone(), shared.clone());
            // The child's parent-death signal fires when *this thread* exits,
            // so the thread lives exactly as long as the child does.
            std::thread::Builder::new()
                .name("bato-supervisor".into())
                .spawn(move || monitor(bin, port, url, cfg, shared, tx, started_tx))?
        };
        match started_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                url,
                secret,
                alias_dir,
                shared,
                events: Some(rx),
                monitor: Some(monitor),
                grace,
            }),
            Ok(Err(e)) => {
                let _ = monitor.join();
                cleanup(&url, &alias_dir);
                Err(e)
            }
            Err(_) => {
                cleanup(&url, &alias_dir);
                Err(io::Error::other("supervisor thread ended before spawning"))
            }
        }
    }

    /// `http://127.0.0.1:<port>`, valid from the start (restarts keep the port).
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The per-session secret the server got as `BATO_SESSION_SECRET`
    /// (`None` with [`Config::session_secret`] off). `Http` sends it by
    /// itself; this is for anything else that talks to the server.
    pub fn session_secret(&self) -> Option<&str> {
        self.secret.as_deref()
    }

    /// The running child's pid.
    pub fn pid(&self) -> Option<u32> {
        self.shared.state.lock().unwrap().pid.map(|p| p as u32)
    }

    /// `Some(true)` once healthy, `Some(false)` after the health timeout,
    /// `None` while still checking.
    pub fn health(&self) -> Option<bool> {
        self.shared.state.lock().unwrap().healthy
    }

    /// Block until the health check has an answer (or `timeout`).
    pub fn wait_healthy(&self, timeout: Duration) -> bool {
        let s = self.shared.state.lock().unwrap();
        let (s, _) = self
            .shared
            .cv
            .wait_timeout_while(s, timeout, |s| s.healthy.is_none() && !s.finished)
            .unwrap();
        s.healthy == Some(true)
    }

    /// The event stream (health, exits, restarts). Can be taken once.
    pub fn take_events(&mut self) -> Option<mpsc::UnboundedReceiver<Event>> {
        self.events.take()
    }

    /// SIGTERM to the server's process group, SIGKILL after the grace period.
    /// Blocks for at most that long. Idempotent.
    pub fn stop(&mut self) {
        let Some(monitor) = self.monitor.take() else {
            return;
        };
        {
            let mut s = self.shared.state.lock().unwrap();
            s.stopping = true;
            if let Some(pid) = s.pid {
                signal_group(pid, SIGTERM);
            }
            self.shared.cv.notify_all();
            let (mut s, _) = self
                .shared
                .cv
                .wait_timeout_while(s, self.grace, |s| s.pid.is_some())
                .unwrap();
            if let Some(pid) = s.pid {
                signal_group(pid, SIGKILL);
                // Reaped by the monitor; wait for that so nothing outlives stop().
                s = self.shared.cv.wait_while(s, |s| s.pid.is_some()).unwrap();
            }
            drop(s);
        }
        let _ = monitor.join();
        if self.secret.is_some() {
            crate::http::unregister_session(&self.url);
        }
        if let Some(d) = self.alias_dir.take() {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(unix)]
const SIGTERM: i32 = libc::SIGTERM;
#[cfg(unix)]
const SIGKILL: i32 = libc::SIGKILL;
// Windows has no signals: both end the process (`TerminateProcess`); the
// job object takes anything it started with it.
#[cfg(windows)]
const SIGTERM: i32 = 15;
#[cfg(windows)]
const SIGKILL: i32 = 9;

#[cfg(windows)]
fn signal_group(pid: i32, _sig: i32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    // SAFETY: a handle we open and close; the pid is still ours (see unix).
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid as u32);
        if !h.is_null() {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
}

/// Windows: the job every bundled server joins. `KILL_ON_JOB_CLOSE`: when
/// this process ends, crash included, the system closes the job and ends
/// the server (Linux: `PR_SET_PDEATHSIG`).
#[cfg(windows)]
fn job() -> Option<isize> {
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    static JOB: OnceLock<Option<isize>> = OnceLock::new();
    *JOB.get_or_init(|| {
        // SAFETY: an unnamed job we keep open for the process's lifetime.
        unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            Some(h as isize)
        }
    })
}

/// Windows: put a started server in [`job`].
#[cfg(windows)]
fn join_job(child: &std::process::Child) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    if let Some(j) = job() {
        // SAFETY: both handles are live; failure leaves the child outside.
        unsafe { AssignProcessToJobObject(j as _, child.as_raw_handle() as _) };
    }
}

/// Windows: has `pid` exited? (`WaitForSingleObject` on a fresh handle.)
#[cfg(windows)]
fn exited(pid: i32, block: bool) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    // SAFETY: a handle we open and close.
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid as u32);
        if h.is_null() {
            return true;
        }
        let r = WaitForSingleObject(h, if block { INFINITE } else { 0 });
        CloseHandle(h);
        r != WAIT_TIMEOUT
    }
}

#[cfg(unix)]
fn signal_group(pid: i32, sig: i32) {
    // SAFETY: plain syscall. The pid is still ours: the monitor clears it
    // under the same lock before reaping, so it cannot have been reused.
    unsafe {
        if libc::kill(-pid, sig) != 0 {
            libc::kill(pid, sig);
        }
    }
}

pub(crate) fn command(bin: &Path, port: u16, cfg: &Config) -> Command {
    let mut cmd = Command::new(bin);
    #[cfg(unix)]
    if let Some(name) = &cfg.process_name {
        cmd.arg0(name);
    }
    cmd.arg("--port")
        .arg(port.to_string())
        .args(&cfg.args)
        .stdin(Stdio::null());
    for k in &cfg.strip_env {
        cmd.env_remove(k);
    }
    cmd.envs(cfg.env.iter().map(|(k, v)| (k, v)));
    if !cfg.path_prepend.is_empty() {
        let base = cfg
            .env
            .iter()
            .rev()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| std::ffi::OsString::from(v))
            .or_else(|| std::env::var_os("PATH"));
        let mut parts = cfg.path_prepend.clone();
        if let Some(b) = base {
            parts.extend(std::env::split_paths(&b));
        }
        if let Ok(path) = std::env::join_paths(parts) {
            cmd.env("PATH", path);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window; its own group, so our Ctrl+C is not its.
        cmd.creation_flags(crate::win::CREATE_NO_WINDOW | 0x0000_0200);
    }
    #[cfg(unix)]
    {
        cmd.process_group(0);
        let parent = std::process::id() as libc::pid_t;
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong) != 0 {
                    return Err(io::Error::last_os_error());
                }
                // The parent died before prctl took effect: nobody will signal us.
                if libc::getppid() != parent {
                    libc::_exit(0);
                }
                Ok(())
            });
        }
    }
    cmd
}

/// Has `pid` exited? Leaves it unreaped (`WNOWAIT`), so the pid stays ours.
#[cfg(unix)]
fn exited(pid: i32, block: bool) -> bool {
    // SAFETY: zeroed siginfo_t is a valid out-parameter for waitid.
    unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        let flags = libc::WEXITED | libc::WNOWAIT | if block { 0 } else { libc::WNOHANG };
        loop {
            let r = libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, flags);
            if r == 0 {
                return info.si_pid() != 0;
            }
            if io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                return true; // ECHILD: already gone
            }
        }
    }
}

fn monitor(
    bin: PathBuf,
    port: u16,
    url: String,
    cfg: Config,
    shared: Arc<Shared>,
    tx: mpsc::UnboundedSender<Event>,
    started: std::sync::mpsc::SyncSender<io::Result<()>>,
) {
    let emit = |e: Event| {
        let _ = tx.unbounded_send(e);
    };
    let health = crate::http::Http::new(&url).with_timeout(Duration::from_secs(1));
    let mut attempt = 0u32;
    let mut started = Some(started);
    loop {
        let mut child = {
            let mut s = shared.state.lock().unwrap();
            if s.stopping {
                break;
            }
            match command(&bin, port, &cfg).spawn() {
                Ok(c) => {
                    #[cfg(windows)]
                    join_job(&c);
                    s.pid = Some(c.id() as i32);
                    c
                }
                Err(e) => {
                    drop(s);
                    match started.take() {
                        Some(tx) => {
                            let _ = tx.send(Err(e));
                        }
                        None => emit(Event::Exited {
                            code: None,
                            signal: None,
                        }),
                    }
                    break;
                }
            }
        };
        let pid = child.id() as i32;
        if let Some(tx) = started.take() {
            let _ = tx.send(Ok(()));
        }

        // Health: poll until 2xx, the deadline, the child's death or stop().
        let deadline = Instant::now() + cfg.health_timeout;
        let healthy = loop {
            if shared.state.lock().unwrap().stopping || exited(pid, false) {
                break false;
            }
            let t = Instant::now();
            if health
                .send(crate::http::Method::Get, &cfg.health_path, None)
                .is_ok_and(|r| r.ok())
            {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(cfg.health_interval.saturating_sub(t.elapsed()));
        };
        let stopping = {
            let mut s = shared.state.lock().unwrap();
            s.healthy = Some(healthy);
            shared.cv.notify_all();
            s.stopping
        };
        if !stopping {
            emit(if healthy {
                Event::Healthy { url: url.clone() }
            } else {
                Event::Unhealthy { url: url.clone() }
            });
        }

        // Wait for the exit, forget the pid under the lock, then reap.
        exited(pid, true);
        shared.state.lock().unwrap().pid = None;
        let status = child.wait();
        shared.cv.notify_all();
        let (code, signal) = match &status {
            #[cfg(unix)]
            Ok(s) => {
                use std::os::unix::process::ExitStatusExt;
                (s.code(), s.signal())
            }
            #[cfg(windows)]
            Ok(s) => (s.code(), None),
            Err(_) => (None, None),
        };
        emit(Event::Exited { code, signal });

        let mut s = shared.state.lock().unwrap();
        let crashed = code != Some(0);
        match cfg.restart {
            Restart::OnCrash { max, backoff } if !s.stopping && crashed && attempt < max => {
                attempt += 1;
                s.healthy = None;
                emit(Event::Restarting { attempt });
                let wait = backoff * attempt;
                let (s2, _) = shared
                    .cv
                    .wait_timeout_while(s, wait, |s| !s.stopping)
                    .unwrap();
                if s2.stopping {
                    s = s2;
                    s.finished = true;
                    shared.cv.notify_all();
                    return;
                }
            }
            _ => {
                s.finished = true;
                shared.cv.notify_all();
                return;
            }
        }
    }
    let mut s = shared.state.lock().unwrap();
    s.finished = true;
    shared.cv.notify_all();
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bato-shell-sup-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        crate::test_support::write_script(&p, body);
        p
    }

    /// The tiny fake server: answers `/api/health`, records its argv.
    const FAKE_SERVER: &str = r#"#!/usr/bin/env python3
import http.server, sys
port = int(sys.argv[sys.argv.index("--port") + 1])
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        ok = self.path == "/api/health"
        b = b'{"status":"ok"}' if ok else b'{"error":"no"}'
        self.send_response(200 if ok else 404)
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)
    def log_message(self, *a):
        pass
http.server.HTTPServer(("127.0.0.1", port), H).serve_forever()
"#;

    fn has_python() -> bool {
        Command::new("python3")
            .arg("-V")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn alive(pid: u32) -> bool {
        // A zombie still answers kill(0); count it as dead.
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| {
                s.rsplit_once(')')
                    .map(|(_, r)| !r.trim_start().starts_with('Z'))
            })
            .unwrap_or(false)
    }

    /// How long to wait for an event that must come: generous, it only
    /// bounds a failing run (a short bound flakes under a parallel build).
    const EXPECTED: Duration = Duration::from_secs(20);

    fn next_event(rx: &mut mpsc::UnboundedReceiver<Event>, within: Duration) -> Option<Event> {
        let end = Instant::now() + within;
        while Instant::now() < end {
            match rx.try_recv() {
                Ok(e) => return Some(e),
                Err(e) if e.is_closed() => return None,
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        None
    }

    #[test]
    fn spawns_reports_healthy_and_stops() {
        if !has_python() {
            eprintln!("skipped: no python3 for the fake server");
            return;
        }
        let dir = tmp("healthy");
        let bin = script(&dir, "app-server", FAKE_SERVER);
        // Python's start-up under a parallel build can take seconds.
        let cfg = Config {
            health_timeout: Duration::from_secs(30),
            ..Config::default()
        };
        let mut sup = Supervisor::start(&bin, cfg).unwrap();
        let mut events = sup.take_events().unwrap();
        assert!(sup.url().starts_with("http://127.0.0.1:"));
        assert!(
            sup.wait_healthy(Duration::from_secs(30)),
            "fake server never became healthy"
        );
        assert_eq!(
            next_event(&mut events, EXPECTED),
            Some(Event::Healthy {
                url: sup.url().into()
            })
        );
        assert!(crate::http::Http::new(sup.url()).health());

        let pid = sup.pid().unwrap();
        assert!(alive(pid));
        // SIGTERM is enough for the fake server: the exit below is signal
        // 15, not the SIGKILL after the grace period (no wall-clock bound,
        // which a loaded machine can break).
        sup.stop();
        assert!(!alive(pid));
        assert_eq!(
            next_event(&mut events, EXPECTED),
            Some(Event::Exited {
                code: None,
                signal: Some(15)
            })
        );
        assert!(!crate::http::Http::new(sup.url()).health());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fixed_port_spawns_there_then_attaches() {
        if !has_python() {
            eprintln!("skipped: no python3 for the fake server");
            return;
        }
        let dir = tmp("fixed");
        let bin = script(&dir, "app-server", FAKE_SERVER);
        let port = free_port().unwrap();
        let cfg = Config {
            port: Some(port),
            health_timeout: Duration::from_secs(30),
            attach_timeout: Duration::from_secs(5),
            session_secret: false,
            ..Config::default()
        };
        let mut sup = Supervisor::start(&bin, cfg.clone()).unwrap();
        assert_eq!(sup.url(), format!("http://127.0.0.1:{port}"));
        assert_eq!(fixed_url(&cfg).as_deref(), Some(sup.url()));
        assert!(sup.wait_healthy(Duration::from_secs(30)));
        assert!(answers(sup.url(), &cfg));
        // Another client attaches instead of spawning.
        const APP: AppIdentity = AppIdentity::new("Fixed", "bato-shell-fixed", "1.0.0");
        let a = Supervisor::attach_or_start(&APP, &Settings::default(), cfg.clone());
        assert!(
            matches!(&a, Attach::Attached { url } if url == sup.url()),
            "{a:?}"
        );
        // The port is taken: a second spawn there says so instead of
        // starting a server that cannot bind.
        let e = Supervisor::start(&bin, cfg.clone()).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AddrInUse, "{e}");
        sup.stop();
        assert!(!answers(&fixed_url(&cfg).unwrap(), &cfg));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn attach_or_start_with_nothing_to_attach_or_spawn_is_offline() {
        // A test binary is no installed build, and APP_SPAWN_SERVER is unset
        // in the gates: nothing spawns.
        if std::env::var_os(ENV_SPAWN_SERVER).is_some() {
            return;
        }
        const APP: AppIdentity = AppIdentity::new("Fixed", "bato-shell-fixed", "1.0.0");
        let port = free_port().unwrap();
        let cfg = Config {
            port: Some(port),
            attach_timeout: Duration::from_millis(300),
            ..Config::default()
        };
        let a = Supervisor::attach_or_start(&APP, &Settings::default(), cfg);
        assert!(matches!(a, Attach::Offline { .. }), "{a:?}");
        assert_eq!(a.url(), format!("http://127.0.0.1:{port}"));
        assert_eq!(Config::default().port, None);
        assert_eq!(fixed_url(&Config::default()), None);
    }

    #[test]
    fn sigkill_after_the_grace_period() {
        let dir = tmp("stubborn");
        // Ignores SIGTERM, never serves health. It says when the trap is set:
        // under load the shell may not have run that line yet when the
        // health timeout ends, and a SIGTERM then still kills it.
        let ready = dir.join("ready");
        let bin = script(
            &dir,
            "app-server",
            &format!(
                "#!/bin/sh\ntrap '' TERM\n: > '{}'\nwhile :; do sleep 0.05; done\n",
                ready.display()
            ),
        );
        let cfg = Config {
            health_timeout: Duration::from_millis(200),
            stop_grace: Duration::from_millis(300),
            ..Config::default()
        };
        let mut sup = Supervisor::start(&bin, cfg).unwrap();
        let mut events = sup.take_events().unwrap();
        assert!(!sup.wait_healthy(Duration::from_secs(3)));
        assert_eq!(
            next_event(&mut events, EXPECTED),
            Some(Event::Unhealthy {
                url: sup.url().into()
            })
        );
        let pid = sup.pid().unwrap();
        let end = Instant::now() + Duration::from_secs(30);
        while !ready.exists() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(ready.exists(), "the stubborn server never started");
        drop(sup);
        assert!(!alive(pid));
        assert_eq!(
            next_event(&mut events, EXPECTED),
            Some(Event::Exited {
                code: None,
                signal: Some(9)
            })
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restarts_on_crash_up_to_the_limit() {
        let dir = tmp("crash");
        let bin = script(&dir, "app-server", "#!/bin/sh\nexit 3\n");
        let cfg = Config {
            health_timeout: Duration::from_millis(500),
            restart: Restart::OnCrash {
                max: 2,
                backoff: Duration::from_millis(10),
            },
            ..Config::default()
        };
        let mut sup = Supervisor::start(&bin, cfg).unwrap();
        let mut events = sup.take_events().unwrap();
        let mut seen = Vec::new();
        while let Some(e) = next_event(&mut events, EXPECTED) {
            seen.push(e);
            if seen.len() == 8 {
                break;
            }
        }
        let url = sup.url().to_string();
        let crash = Event::Exited {
            code: Some(3),
            signal: None,
        };
        let down = Event::Unhealthy { url };
        assert_eq!(
            seen,
            vec![
                down.clone(),
                crash.clone(),
                Event::Restarting { attempt: 1 },
                down.clone(),
                crash.clone(),
                Event::Restarting { attempt: 2 },
                down,
                crash,
            ]
        );
        assert_eq!(
            next_event(&mut events, Duration::from_millis(300)),
            None,
            "no third restart"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A fake server that knows the session-secret convention: every route
    /// but `/api/health` wants `X-Bato-Session` = `$BATO_SESSION_SECRET`.
    const SECRET_SERVER: &str = r#"#!/usr/bin/env python3
import http.server, os, sys
port = int(sys.argv[sys.argv.index("--port") + 1])
secret = os.environ.get("BATO_SESSION_SECRET", "")
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        ok = bool(secret) and self.headers.get("X-Bato-Session") == secret
        if self.path == "/api/health":
            ok = True
        b = b'{"ok":true}' if ok else b'{"error":"session secret required"}'
        self.send_response(200 if ok else 401)
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)
    def log_message(self, *a):
        pass
http.server.HTTPServer(("127.0.0.1", port), H).serve_forever()
"#;

    #[test]
    fn session_secret_reaches_the_server_and_only_our_requests() {
        if !has_python() {
            eprintln!("skipped: no python3 for the fake server");
            return;
        }
        let dir = tmp("secret");
        let bin = script(&dir, "app-server", SECRET_SERVER);
        let cfg = Config {
            health_timeout: Duration::from_secs(30),
            ..Config::default()
        };
        let sup = Supervisor::start(&bin, cfg).unwrap();
        assert!(sup.wait_healthy(Duration::from_secs(30)));
        let secret = sup.session_secret().unwrap().to_string();
        assert_eq!(secret.len(), 64);
        // Any Http to the bundled URL carries it…
        let v: serde_json::Value = crate::http::Http::new(sup.url()).get("/api/files").unwrap();
        assert_eq!(v["ok"], true);
        // …a caller without it is refused.
        let raw = {
            use std::io::{Read, Write};
            let mut s =
                std::net::TcpStream::connect(sup.url().trim_start_matches("http://")).unwrap();
            write!(s, "GET /api/files HTTP/1.0\r\nHost: x\r\n\r\n").unwrap();
            let mut out = String::new();
            let _ = s.read_to_string(&mut out);
            out
        };
        assert!(raw.starts_with("HTTP/1.0 401"), "{raw}");
        let url = sup.url().to_string();
        drop(sup);
        assert_eq!(crate::http::session_for(&url), None, "unregistered on stop");
        // Off: no secret, and the server refuses (it was never told one).
        let cfg = Config {
            health_timeout: Duration::from_secs(30),
            session_secret: false,
            ..Config::default()
        };
        let sup = Supervisor::start(&bin, cfg).unwrap();
        assert!(sup.wait_healthy(Duration::from_secs(30)));
        assert_eq!(sup.session_secret(), None);
        assert!(
            crate::http::Http::new(sup.url())
                .get::<serde_json::Value>("/api/files")
                .is_err()
        );
        drop(sup);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn runs_under_its_process_name() {
        let dir = tmp("alias");
        let bin = script(
            &dir,
            "app-server",
            "#!/bin/sh\nwhile :; do sleep 0.1; done\n",
        );
        let cfg = Config {
            health_timeout: Duration::from_millis(100),
            process_name: Some("batobrowse-server".into()),
            ..Config::default()
        };
        let sup = Supervisor::start(&bin, cfg).unwrap();
        let pid = sup.pid().unwrap();
        // The kernel names the process after the path it executed (the
        // link), truncated to 15 bytes.
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        let alias_dir = sup.alias_dir.clone().unwrap();
        assert!(alias_dir.join("batobrowse-server").exists());
        drop(sup);
        assert!(
            !alias_dir.exists(),
            "the alias directory is removed on stop"
        );
        assert_eq!(comm.trim(), "batobrowse-serv");
        assert!(
            Supervisor::start(
                &bin,
                Config {
                    process_name: Some("../x".into()),
                    ..Config::default()
                }
            )
            .is_err()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn path_prepend_and_arg0() {
        let cfg = Config {
            process_name: Some("batobrowse-server".into()),
            path_prepend: vec!["/opt/tools".into(), "/opt/more".into()],
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            ..Config::default()
        };
        let cmd = command(Path::new("/x/app-server"), 1, &cfg);
        let path = cmd
            .get_envs()
            .filter(|(k, _)| *k == "PATH")
            .filter_map(|(_, v)| v)
            .last()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(path, "/opt/tools:/opt/more:/usr/bin:/bin");
        assert_eq!(cmd.get_program(), "/x/app-server");
        // Without an explicit PATH, the inherited one follows the prepends.
        let cfg = Config {
            path_prepend: vec!["/opt/tools".into()],
            ..Config::default()
        };
        let cmd = command(Path::new("/x/app-server"), 1, &cfg);
        let path = cmd
            .get_envs()
            .find(|(k, _)| *k == "PATH")
            .and_then(|(_, v)| v)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(path.starts_with("/opt/tools"), "{path}");
        let inherited = std::env::var("PATH").unwrap_or_default();
        assert!(path.ends_with(&inherited), "{path}");
    }

    #[test]
    fn secrets_are_random_hex() {
        let (a, b) = (new_secret(), new_secret());
        assert_ne!(a, b);
        assert!(a.len() == 64 && a.bytes().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn missing_binary_is_an_error_not_a_panic() {
        assert!(
            Supervisor::start(Path::new("/nonexistent/app-server"), Config::default()).is_err()
        );
    }

    #[test]
    fn command_strips_auth_and_passes_port() {
        let cfg = Config {
            env: vec![("APP_DATA_DIR".into(), "/x".into())],
            args: vec!["--quiet".into()],
            ..Config::default()
        };
        let cmd = command(Path::new("/bin/app-server"), 4321, &cfg);
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["--port", "4321", "--quiet"]);
        let envs: Vec<_> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(envs.contains(&("BATO_AUTH_URL".into(), None)));
        assert!(envs.contains(&("BATO_AUTH_PUBLIC_URL".into(), None)));
        assert!(envs.contains(&("APP_DATA_DIR".into(), Some("/x".into()))));
    }

    #[test]
    fn binary_lookup_order() {
        let root = tmp("lookup");
        let exe_dir = root.join("app-client-native/target/release");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(root.join("dist")).unwrap();
        let exe = exe_dir.join("myapp");
        let arch = if cfg!(target_arch = "x86_64") {
            "amd64"
        } else {
            "arm64"
        };
        let dev = root.join(format!("dist/app-server-linux-{arch}"));
        script(
            &root,
            &format!("dist/app-server-linux-{arch}"),
            "#!/bin/sh\n",
        );

        // Checkout layout: repo dist/.
        assert_eq!(
            find_binary_from(None, Some(&exe), "app-server"),
            Some(dev.clone())
        );
        // Installed layout wins: side by side, exec bit repaired.
        let side = exe_dir.join("app-server");
        std::fs::write(&side, "#!/bin/sh\n").unwrap();
        assert_eq!(
            find_binary_from(None, Some(&exe), "app-server"),
            Some(side.clone())
        );
        assert_ne!(
            std::fs::metadata(&side).unwrap().permissions().mode() & 0o111,
            0
        );
        // An explicit path is final, even when it is wrong.
        assert_eq!(
            find_binary_from(Some(&dev), Some(&exe), "app-server"),
            Some(dev)
        );
        assert_eq!(
            find_binary_from(Some(&root.join("nope")), Some(&exe), "app-server"),
            None
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The child must die when the UI process does, even by SIGKILL (no
    /// destructor runs). The UI is played by this test binary, re-executed to
    /// run `pdeathsig_helper` only.
    #[test]
    fn child_dies_with_the_ui_process() {
        let dir = tmp("pdeath");
        let bin = script(&dir, "app-server", "#!/bin/sh\nexec sleep 30\n");
        let mut helper = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "supervisor::tests::pdeathsig_helper",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("BATO_SHELL_TEST_SERVER", &bin)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut out = std::io::BufReader::new(helper.stdout.take().unwrap());
        let mut line = String::new();
        let pid: u32 = loop {
            line.clear();
            use std::io::BufRead;
            assert!(
                out.read_line(&mut line).unwrap() > 0,
                "helper ended without reporting a pid"
            );
            // libtest prints `test <name> ... ` without a newline first.
            if let Some((_, p)) = line.trim().split_once("CHILD_PID=") {
                break p.parse().unwrap();
            }
        };
        assert!(alive(pid));
        helper.kill().unwrap(); // SIGKILL
        helper.wait().unwrap();
        let end = Instant::now() + Duration::from_secs(20);
        while alive(pid) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive(pid), "server outlived the UI process");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "helper for child_dies_with_the_ui_process"]
    fn pdeathsig_helper() {
        let Some(bin) = std::env::var_os("BATO_SHELL_TEST_SERVER") else {
            return;
        };
        let cfg = Config {
            health_timeout: Duration::from_millis(100),
            ..Config::default()
        };
        let sup = Supervisor::start(Path::new(&bin), cfg).unwrap();
        println!("CHILD_PID={}", sup.pid().unwrap());
        std::thread::sleep(Duration::from_secs(30));
    }
}
