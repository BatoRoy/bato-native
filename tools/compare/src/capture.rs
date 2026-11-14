//! Launching a client on the off-screen output and capturing its window.
//!
//! Safety rules (docs/POC-BRIEF.md "Testing"): `tools/offscreen.sh check` before
//! every launch; right after the window maps, its workspace must be `batopoc`
//! on output BATOPOC, else the process is killed at once; native windows are
//! captured only through `tools/shot.sh <pid>` (which refuses windows anywhere
//! else); kill by PID. Never `hyprctl dispatch`, never input.
//!
//! Electron captures itself (`BATO_CAPTURE`, app-client/electron/main.js):
//! Chromium's own screenshot of the page, which matches a compositor capture of
//! the window pixel for pixel except the 4 rounded window corners (masked in
//! every diff), and never contains the stray cursor or another window.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::image::{Rect, Rgb};

pub enum Client {
    /// The packaged Electron build (`dist/electron/linux-unpacked/app`).
    Electron { app: PathBuf },
    /// A native binary; it must honour `BATO_APP_ID` (or have a `batopoc-*` app_id).
    Native { bin: PathBuf, app_id: String, env: Vec<(String, String)>, args: Vec<String> },
}

impl Client {
    pub fn label(&self) -> &'static str {
        match self {
            Client::Electron { .. } => "electron",
            Client::Native { .. } => "native",
        }
    }
}

pub struct Shot {
    pub img: Rgb,
    /// Areas where the user's cursor may be painted (see `cursor_mask`).
    pub mask: Vec<Rect>,
    /// Two consecutive captures were identical (false for animated content).
    pub stable: bool,
    pub notes: Vec<String>,
}

pub struct Session {
    root: PathBuf,
    tmp: PathBuf,
    n: usize,
    _lock: File,
}

impl Session {
    /// One capture at a time across every `compare` process on this machine:
    /// windows on the off-screen workspace all float at the same spot.
    pub fn new(root: &Path) -> Result<Self, String> {
        let run = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let lock_path = run.join("bato-compare.lock");
        let lock = File::create(&lock_path).map_err(|e| format!("{}: {e}", lock_path.display()))?;
        if lock.try_lock().is_err() {
            eprintln!("compare: another compare run holds {}; waiting…", lock_path.display());
            lock.lock().map_err(|e| format!("lock: {e}"))?;
        }
        let tmp = std::env::temp_dir().join(format!("bato-compare-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        Ok(Session { root: root.to_path_buf(), tmp, n: 0, _lock: lock })
    }

    fn fresh_dir(&mut self, what: &str) -> PathBuf {
        self.n += 1;
        let d = self.tmp.join(format!("{what}-{}", self.n));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).ok();
        d
    }

    fn launch(&mut self, client: &Client, bato_view: &str, extra_env: &[(&str, String)]) -> Result<Proc, String> {
        let check = Command::new(self.root.join("tools/offscreen.sh"))
            .arg("check")
            .stdout(Stdio::null())
            .status()
            .map_err(|e| format!("offscreen.sh: {e}"))?;
        if !check.success() {
            return Err("the off-screen output is not up (tools/offscreen.sh check failed); not launching".into());
        }
        let dir = self.fresh_dir(client.label());
        let log = dir.join("output.log");
        let mut cmd = match client {
            Client::Electron { app } => {
                // Private profile: localStorage (theme, accent, scale, width) starts at defaults.
                let mut c = Command::new(app);
                c.arg(format!("--user-data-dir={}", dir.join("user-data").display()));
                c
            }
            Client::Native { bin, app_id, env, args } => {
                let mut c = Command::new(bin);
                c.args(args).env("BATO_APP_ID", app_id).env("XDG_CONFIG_HOME", dir.join("config"));
                c.envs(env.iter().map(|(k, v)| (k, v)));
                c
            }
        };
        cmd.env("BATO_VIEW", bato_view);
        cmd.envs(extra_env.iter().map(|(k, v)| (k, v)));
        let out = File::create(&log).map_err(|e| e.to_string())?;
        cmd.stdin(Stdio::null()).stdout(out.try_clone().map_err(|e| e.to_string())?).stderr(out);
        let child = cmd.spawn().map_err(|e| format!("spawn {:?}: {e}", cmd.get_program()))?;
        let mut p = Proc { pid: child.id(), child, log };

        // Find the window and check where it went, polling fast so a misrouted
        // window is killed within ~50 ms.
        let t0 = Instant::now();
        loop {
            if let Some(w) = window_of(p.pid)? {
                if w.workspace != "batopoc" || w.monitor != "BATOPOC" {
                    p.kill_now();
                    return Err(format!(
                        "{} window (class `{}`) opened on workspace `{}` / {} instead of batopoc; killed it. \
                         Is its app_id routed by the off-screen rule (^(batopoc-.*|app-client)$)?",
                        client.label(),
                        w.class,
                        w.workspace,
                        w.monitor
                    ));
                }
                return Ok(p);
            }
            if let Ok(Some(st)) = p.child.try_wait() {
                return Err(format!("{} exited ({st}) before mapping a window; log:\n{}", client.label(), p.log_tail()));
            }
            if t0.elapsed() > Duration::from_secs(20) {
                return Err(format!("{}: no window after 20 s; log:\n{}", client.label(), p.log_tail()));
            }
            sleep(Duration::from_millis(25));
        }
    }

    /// Launches a native client on `bato_view`, waits until the window is
    /// settled and captures it with tools/shot.sh.
    pub fn capture_native(&mut self, client: &Client, bato_view: &str) -> Result<Shot, String> {
        let mut p = self.launch(client, bato_view, &[])?;
        let res = self.settle_and_shoot(&mut p);
        p.terminate();
        res
    }

    fn settle_and_shoot(&mut self, p: &mut Proc) -> Result<Shot, String> {
        sleep(Duration::from_millis(1000));
        let mut notes = Vec::new();

        // Another window over ours (someone else's test window) would end up in the shot.
        let t0 = Instant::now();
        loop {
            let others = overlapping(p.pid)?;
            if others.is_empty() {
                break;
            }
            if t0.elapsed() > Duration::from_secs(15) {
                notes.push(format!("overlap:{}", others.join(",")));
                break;
            }
            sleep(Duration::from_millis(500));
        }

        // Settled = two consecutive captures 400 ms apart are identical (outside
        // the cursor mask); give up after 8 s (animated content: "unstable").
        let shot_path = self.tmp.join(format!("shot-{}.png", p.pid));
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut prev: Option<(Rgb, Vec<Rect>)> = None;
        loop {
            let w = window_of(p.pid)?.ok_or("window disappeared")?;
            let mut mask = cursor_mask(&w)?;
            let st = Command::new(self.root.join("tools/shot.sh"))
                .arg(p.pid.to_string())
                .arg(&shot_path)
                .stdout(Stdio::null())
                .status()
                .map_err(|e| format!("shot.sh: {e}"))?;
            if !st.success() {
                return Err("tools/shot.sh refused or failed".into());
            }
            for r in cursor_mask(&w)? {
                if !mask.contains(&r) {
                    mask.push(r);
                }
            }
            let img = Rgb::load(&shot_path)?;
            let stable = prev.as_ref().is_some_and(|(pi, pm)| {
                let both: Vec<Rect> = mask.iter().chain(pm.iter()).copied().collect();
                img.same_as(pi, &both)
            });
            if stable || Instant::now() > deadline {
                if !stable {
                    notes.push("unstable".into());
                }
                if !mask.is_empty() {
                    notes.push("cursor".into());
                }
                let _ = std::fs::remove_file(&shot_path);
                return Ok(Shot { img, mask, stable, notes });
            }
            prev = Some((img, mask));
            sleep(Duration::from_millis(400));
        }
    }

    /// Electron: launched with BATO_CAPTURE, it screenshots itself once settled
    /// and quits. `full` adds BATO_CAPTURE_FULL=1 (viewport grown to the whole
    /// card, or page).
    pub fn capture_electron(&mut self, app: &Path, bato_view: &str, full: bool) -> Result<Shot, String> {
        let file = self.tmp.join(format!("electron-{}.png", self.n + 1));
        let _ = std::fs::remove_file(&file);
        let client = Client::Electron { app: app.to_path_buf() };
        let mut env = vec![("BATO_CAPTURE", file.display().to_string())];
        if full {
            env.push(("BATO_CAPTURE_FULL", "1".into()));
        }
        let mut p = self.launch(&client, bato_view, &env)?;
        let t0 = Instant::now();
        let status = loop {
            if let Ok(Some(st)) = p.child.try_wait() {
                break st;
            }
            if t0.elapsed() > Duration::from_secs(40) {
                p.terminate();
                return Err("BATO_CAPTURE: the app did not quit within 40 s".into());
            }
            sleep(Duration::from_millis(100));
        };
        let log = std::fs::read_to_string(&p.log).unwrap_or_default();
        if !status.success() || !file.exists() || !log.contains("BATO_CAPTURE ") {
            return Err(format!(
                "BATO_CAPTURE failed ({status}); is the build older than the capture hook? log:\n{}",
                p.log_tail()
            ));
        }
        let stable = log.contains("BATO_CAPTURE stable");
        let img = Rgb::load(&file)?;
        let _ = std::fs::remove_file(&file);
        let notes = if stable { vec![] } else { vec!["unstable".to_string()] };
        Ok(Shot { img, mask: vec![], stable, notes })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

struct Proc {
    pid: u32,
    child: Child,
    log: PathBuf,
}

impl Proc {
    fn kill_now(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// SIGTERM (Electron then takes its helper processes down), SIGKILL after 3 s.
    fn terminate(&mut self) {
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            return;
        }
        let _ = Command::new("kill").arg("-TERM").arg(self.pid.to_string()).status();
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(3) {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            sleep(Duration::from_millis(50));
        }
        self.kill_now();
    }

    fn log_tail(&self) -> String {
        let s = std::fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = s.lines().collect();
        lines[lines.len().saturating_sub(15)..].join("\n")
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn hyprctl_json(what: &str) -> Result<Value, String> {
    let out = Command::new("hyprctl").args(["-j", what]).output().map_err(|e| format!("hyprctl: {e}"))?;
    serde_json::from_slice(&out.stdout).map_err(|e| format!("hyprctl -j {what}: {e}"))
}

struct Monitor {
    id: i64,
    name: String,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

fn monitors() -> Result<Vec<Monitor>, String> {
    let v = hyprctl_json("monitors")?;
    Ok(v.as_array()
        .ok_or("hyprctl monitors: not an array")?
        .iter()
        .map(|m| Monitor {
            id: m["id"].as_i64().unwrap_or(-1),
            name: m["name"].as_str().unwrap_or("").to_string(),
            x: m["x"].as_i64().unwrap_or(0),
            y: m["y"].as_i64().unwrap_or(0),
            w: m["width"].as_i64().unwrap_or(0),
            h: m["height"].as_i64().unwrap_or(0),
        })
        .collect())
}

struct Win {
    pid: i64,
    class: String,
    workspace: String,
    monitor: String,
    /// Position relative to its output, and size.
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

fn windows() -> Result<Vec<Win>, String> {
    let mons = monitors()?;
    let v = hyprctl_json("clients")?;
    Ok(v.as_array()
        .ok_or("hyprctl clients: not an array")?
        .iter()
        .map(|c| {
            let mon = mons.iter().find(|m| Some(m.id) == c["monitor"].as_i64());
            let at = |i| c["at"][i].as_i64().unwrap_or(0);
            let size = |i| c["size"][i].as_i64().unwrap_or(0);
            Win {
                pid: c["pid"].as_i64().unwrap_or(-1),
                class: c["class"].as_str().unwrap_or("").to_string(),
                workspace: c["workspace"]["name"].as_str().unwrap_or("").to_string(),
                monitor: mon.map(|m| m.name.clone()).unwrap_or_default(),
                x: at(0) - mon.map_or(0, |m| m.x),
                y: at(1) - mon.map_or(0, |m| m.y),
                w: size(0),
                h: size(1),
            }
        })
        .collect())
}

fn window_of(pid: u32) -> Result<Option<Win>, String> {
    Ok(windows()?.into_iter().find(|w| w.pid == pid as i64))
}

/// Other windows on the off-screen workspace that intersect ours.
fn overlapping(pid: u32) -> Result<Vec<String>, String> {
    let ws = windows()?;
    let Some(me) = ws.iter().find(|w| w.pid == pid as i64) else { return Ok(vec![]) };
    Ok(ws
        .iter()
        .filter(|o| o.pid != me.pid && o.workspace == "batopoc")
        .filter(|o| o.x < me.x + me.w && me.x < o.x + o.w && o.y < me.y + me.h && me.y < o.y + o.h)
        .map(|o| format!("{}({})", o.class, o.pid))
        .collect())
}

/// Hyprland paints the desktop pointer onto the headless output at the same
/// output-local position it has on its real monitor (seen on 0.56), so a
/// region capture can contain a cursor the app never saw. This returns the
/// window-relative box where it may be, for both readings of "same position"
/// (local to the pointer's monitor, and global).
fn cursor_mask(w: &Win) -> Result<Vec<Rect>, String> {
    let out = Command::new("hyprctl").arg("cursorpos").output().map_err(|e| format!("hyprctl: {e}"))?;
    let s = String::from_utf8_lossy(&out.stdout);
    let mut it = s.trim().split(',').map(|t| t.trim().parse::<f64>().unwrap_or(f64::NAN));
    let (gx, gy) = (it.next().unwrap_or(f64::NAN), it.next().unwrap_or(f64::NAN));
    if gx.is_nan() || gy.is_nan() {
        return Ok(vec![]);
    }
    let (gx, gy) = (gx as i64, gy as i64);
    let mut cands = vec![(gx, gy)];
    if let Some(m) = monitors()?.iter().find(|m| gx >= m.x && gx < m.x + m.w && gy >= m.y && gy < m.y + m.h) {
        if (gx - m.x, gy - m.y) != (gx, gy) {
            cands.push((gx - m.x, gy - m.y));
        }
    }
    let mut out = Vec::new();
    for (cx, cy) in cands {
        // Window-relative; generous for a 24–32 px theme cursor and its shadow.
        let (x0, y0, x1, y1) = (cx - w.x - 8, cy - w.y - 8, cx - w.x + 44, cy - w.y + 48);
        let (x0, y0, x1, y1) = (x0.max(0), y0.max(0), x1.min(w.w), y1.min(w.h));
        if x0 < x1 && y0 < y1 {
            out.push(Rect { x0: x0 as usize, y0: y0 as usize, x1: x1 as usize, y1: y1 as usize });
        }
    }
    Ok(out)
}
