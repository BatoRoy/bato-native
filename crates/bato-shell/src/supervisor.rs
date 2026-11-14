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

use std::io;
use std::os::unix::fs::PermissionsExt;
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_name: SERVER_NAME.into(),
            args: Vec::new(),
            env: Vec::new(),
            strip_env: STRIPPED_ENV.iter().map(|s| s.to_string()).collect(),
            health_path: "/api/health".into(),
            health_timeout: Duration::from_secs(8),
            health_interval: Duration::from_millis(100),
            stop_grace: Duration::from_secs(2),
            restart: Restart::Never,
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
    crate::backend::may_spawn_bundled_for(settings)
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
    let dev = format!("{name}-linux-{arch}");
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
    if meta.permissions().mode() & 0o111 == 0 {
        return std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).is_ok();
    }
    true
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

/// A running bundled server. Dropping it stops the server.
pub struct Supervisor {
    url: String,
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
    /// [`wanted`] + [`find_binary`] + [`start`](Self::start). `None` when the
    /// app should not spawn, has no server, or the spawn failed (logged to
    /// stderr; the UI shows the backend offline, the suite rule).
    pub fn start_bundled(app: &AppIdentity, settings: &Settings, cfg: Config) -> Option<Self> {
        if !wanted(app, settings) {
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

    /// Spawn `bin --port N` on a free port and return at once; health is
    /// checked on the supervisor's thread.
    pub fn start(bin: &Path, cfg: Config) -> io::Result<Self> {
        let port = free_port()?;
        let url = format!("http://127.0.0.1:{port}");
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
                shared,
                events: Some(rx),
                monitor: Some(monitor),
                grace,
            }),
            Ok(Err(e)) => {
                let _ = monitor.join();
                Err(e)
            }
            Err(_) => Err(io::Error::other("supervisor thread ended before spawning")),
        }
    }

    /// `http://127.0.0.1:<port>`, valid from the start (restarts keep the port).
    pub fn url(&self) -> &str {
        &self.url
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
                signal_group(pid, libc::SIGTERM);
            }
            self.shared.cv.notify_all();
            let (mut s, _) = self
                .shared
                .cv
                .wait_timeout_while(s, self.grace, |s| s.pid.is_some())
                .unwrap();
            if let Some(pid) = s.pid {
                signal_group(pid, libc::SIGKILL);
                // Reaped by the monitor; wait for that so nothing outlives stop().
                s = self.shared.cv.wait_while(s, |s| s.pid.is_some()).unwrap();
            }
            drop(s);
        }
        let _ = monitor.join();
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

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
    cmd.arg("--port")
        .arg(port.to_string())
        .args(&cfg.args)
        .stdin(Stdio::null());
    for k in &cfg.strip_env {
        cmd.env_remove(k);
    }
    cmd.envs(cfg.env.iter().map(|(k, v)| (k, v)));
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
    cmd
}

/// Has `pid` exited? Leaves it unreaped (`WNOWAIT`), so the pid stays ours.
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
            Ok(s) => {
                use std::os::unix::process::ExitStatusExt;
                (s.code(), s.signal())
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bato-shell-sup-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        drop(f);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
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
        let mut sup = Supervisor::start(&bin, Config::default()).unwrap();
        let mut events = sup.take_events().unwrap();
        assert!(sup.url().starts_with("http://127.0.0.1:"));
        assert!(
            sup.wait_healthy(Duration::from_secs(8)),
            "fake server never became healthy"
        );
        assert_eq!(
            next_event(&mut events, Duration::from_secs(1)),
            Some(Event::Healthy {
                url: sup.url().into()
            })
        );
        assert!(crate::http::Http::new(sup.url()).health());

        let pid = sup.pid().unwrap();
        assert!(alive(pid));
        let t = Instant::now();
        sup.stop();
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "SIGTERM should be enough for the fake server"
        );
        assert!(!alive(pid));
        assert_eq!(
            next_event(&mut events, Duration::from_secs(1)),
            Some(Event::Exited {
                code: None,
                signal: Some(15)
            })
        );
        assert!(!crate::http::Http::new(sup.url()).health());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sigkill_after_the_grace_period() {
        let dir = tmp("stubborn");
        // Ignores SIGTERM, never serves health.
        let bin = script(
            &dir,
            "app-server",
            "#!/bin/sh\ntrap '' TERM\nwhile :; do sleep 0.05; done\n",
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
            next_event(&mut events, Duration::from_secs(1)),
            Some(Event::Unhealthy {
                url: sup.url().into()
            })
        );
        let pid = sup.pid().unwrap();
        drop(sup);
        assert!(!alive(pid));
        assert_eq!(
            next_event(&mut events, Duration::from_secs(1)),
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
        while let Some(e) = next_event(&mut events, Duration::from_secs(3)) {
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
        let end = Instant::now() + Duration::from_secs(3);
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
