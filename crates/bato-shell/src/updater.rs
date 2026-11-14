//! Self-update for a `desktop` release, through the bato CLI.
//!
//! The Electron updater (electron-updater, AppImage) cannot work for a
//! `desktop` release: the bucket's `desktop/` prefix is not anonymously
//! readable and there is no `latest-linux.yml`. What is anonymous is the
//! registry API, and `bato install` already knows how to put a release down
//! (receipt, launcher, icon, PATH links, the unlink-swap that keeps the
//! running binary intact). So:
//!
//! | Step | How |
//! |---|---|
//! | Check | `GET <registry>/api/apps/<slug>` → `latest`, compared with the running version. On launch and every 30 min. |
//! | Apply | `bato install <slug> --json --progress` with `BATO_REGISTRY_URL` set; its NDJSON progress becomes `downloading{percent}`. |
//! | Restart to update | Start the installed binary again ([`Updater::relaunch`]), then exit. |
//! | No bato CLI | Report `bato install <slug>` for the user to run ([`ApplyError::CliMissing`]); never a second installer in-app. |
//!
//! Phases are Electron's (`update:status`): `checking`, `available{version}`,
//! `none`, `downloading{percent}`, `downloaded{version}`, `error{message}`.
//! Only an installed build (running from `~/.local/share/bato/apps/<slug>/`)
//! updates; a dev build gets "Updates apply to packaged builds only."
//!
//! CLI contract (bato 1.4.9, `cli/src/index.ts` `install` action), verified by
//! running it against a local fake registry (`fixtures/bato-install-*`):
//! stdout is NDJSON, `{"event":"plan",name,version,type}`, then
//! `{"event":"progress",received,total?}` (`total` absent without a
//! Content-Length), then `{"event":"done","ok":true,name,version,type}`; any
//! failure is `{"ok":false,"error":"…"}` (no `event` key) and exit code 1.
//! Human output goes to stderr.

use std::cmp::Ordering;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::Value;

use crate::http::Http;
use crate::identity::{self, AppIdentity};

pub const DEFAULT_REGISTRY: &str = "https://registry.bato.lan";
pub const ENV_REGISTRY: &str = "BATO_REGISTRY_URL";
/// Explicit bato CLI path (bato-store's convention).
pub const ENV_BATO_BIN: &str = "BATO_BIN";
pub const CHECK_INTERVAL: Duration = Duration::from_secs(30 * 60);
pub const NOT_PACKAGED: &str = "Updates apply to packaged builds only.";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UpdatePhase {
    Checking,
    Available { version: String },
    None,
    Downloading { percent: u8 },
    Downloaded { version: String },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// No bato CLI on this machine: show `command` to the user.
    CliMissing {
        command: String,
    },
    NotPackaged,
    Failed(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CliMissing { command } => {
                write!(f, "The bato CLI was not found. To update, run: {command}")
            }
            Self::NotPackaged => f.write_str(NOT_PACKAGED),
            Self::Failed(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for ApplyError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Updater {
    slug: String,
    current: String,
    registry: String,
    install_dir: PathBuf,
    packaged: bool,
}

impl Updater {
    /// Registry: `BATO_REGISTRY_URL`, else `default_registry` (the app's
    /// `brand.rs` constant, normally [`DEFAULT_REGISTRY`]). The CLI's own
    /// default is `http://localhost:8080`, which is why the URL is always
    /// passed to it explicitly.
    pub fn new(app: &AppIdentity, default_registry: &str) -> Self {
        let registry = std::env::var(ENV_REGISTRY)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| default_registry.to_string());
        Self {
            slug: app.slug.to_string(),
            current: app.version.to_string(),
            registry: crate::backend::normalize(&registry),
            install_dir: app.install_dir(),
            packaged: app.is_installed_build(),
        }
    }

    pub fn with_registry(mut self, url: &str) -> Self {
        self.registry = crate::backend::normalize(url);
        self
    }

    /// Treat this build as installed (or not), whatever its path. For tests
    /// and for apps that install themselves elsewhere.
    pub fn with_packaged(mut self, packaged: bool) -> Self {
        self.packaged = packaged;
        self
    }

    /// Where `bato install` puts this app (default
    /// `~/.local/share/bato/apps/<slug>`); [`relaunch`](Self::relaunch)
    /// starts `<dir>/<slug>` from here unless this process runs from it.
    pub fn with_install_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.install_dir = dir.into();
        self
    }

    pub fn install_dir(&self) -> &Path {
        &self.install_dir
    }

    pub fn registry(&self) -> &str {
        &self.registry
    }

    pub fn current_version(&self) -> &str {
        &self.current
    }

    /// Updates apply only to a build `bato install` put down.
    pub fn is_supported(&self) -> bool {
        self.packaged
    }

    /// What to tell a user who has no bato CLI.
    pub fn install_command(&self) -> String {
        format!("bato install {}", self.slug)
    }

    /// The registry's `latest` for this app. Blocking.
    pub fn latest(&self) -> Result<String, String> {
        let meta: Value = Http::new(&self.registry)
            .get(&format!("/api/apps/{}", self.slug))
            .map_err(|e| format!("Update check failed: {e}"))?;
        latest_from_meta(&meta)
            .ok_or_else(|| "Update check failed: the registry sent no version".to_string())
    }

    /// `Available{latest}` when the registry has a newer version, else
    /// `None`; `Error` when the registry cannot be read. Blocking.
    pub fn check(&self) -> UpdatePhase {
        match self.latest() {
            Ok(latest) if compare_versions(&latest, &self.current) == Ordering::Greater => {
                UpdatePhase::Available { version: latest }
            }
            Ok(_) => UpdatePhase::None,
            Err(message) => UpdatePhase::Error { message },
        }
    }

    /// Install the latest release with the bato CLI, reporting `Downloading`
    /// and then `Downloaded` or `Error` through `on_phase`. Blocking. Returns
    /// the installed version.
    pub fn apply(&self, on_phase: impl FnMut(UpdatePhase)) -> Result<String, ApplyError> {
        let mut on_phase = on_phase;
        if !self.packaged {
            on_phase(UpdatePhase::Error {
                message: NOT_PACKAGED.into(),
            });
            return Err(ApplyError::NotPackaged);
        }
        let Some(bato) = find_bato() else {
            let e = ApplyError::CliMissing {
                command: self.install_command(),
            };
            on_phase(UpdatePhase::Error {
                message: e.to_string(),
            });
            return Err(e);
        };
        self.apply_with(&bato, on_phase)
    }

    /// [`apply`](Self::apply) with an explicit CLI path, skipping the
    /// packaged-build check.
    pub fn apply_with(
        &self,
        bato: &Path,
        mut on_phase: impl FnMut(UpdatePhase),
    ) -> Result<String, ApplyError> {
        let fail = |on_phase: &mut dyn FnMut(UpdatePhase), msg: String| {
            on_phase(UpdatePhase::Error {
                message: msg.clone(),
            });
            Err(ApplyError::Failed(msg))
        };
        let child = Command::new(bato)
            .args(["install", &self.slug, "--json", "--progress"])
            .env(ENV_REGISTRY, &self.registry)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                return fail(
                    &mut on_phase,
                    format!("Could not run {}: {e}", bato.display()),
                );
            }
        };

        // Drain stderr concurrently so a chatty CLI cannot block on a full pipe.
        let mut stderr = child.stderr.take().expect("piped stderr");
        let stderr = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });

        let mut done: Option<String> = None;
        let mut failure: Option<String> = None;
        let mut last_percent: Option<u8> = None;
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        for line in stdout.lines() {
            let Ok(line) = line else { break };
            match parse_install_event(&line) {
                Some(InstallEvent::Plan { .. }) => {
                    last_percent = Some(0);
                    on_phase(UpdatePhase::Downloading { percent: 0 });
                }
                Some(InstallEvent::Progress { received, total }) => {
                    if let Some(p) = percent(received, total)
                        && last_percent != Some(p)
                    {
                        last_percent = Some(p);
                        on_phase(UpdatePhase::Downloading { percent: p });
                    }
                }
                Some(InstallEvent::Done { version }) => done = Some(version),
                Some(InstallEvent::Failed { error }) => failure = Some(error),
                None => {}
            }
        }
        let status = child.wait();
        let stderr = stderr.join().unwrap_or_default();

        if let Some(error) = failure {
            return fail(&mut on_phase, error);
        }
        match (status, done) {
            (Ok(s), Some(version)) if s.success() => {
                on_phase(UpdatePhase::Downloaded {
                    version: version.clone(),
                });
                Ok(version)
            }
            (Ok(s), _) => {
                let tail = stderr
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let code = s
                    .code()
                    .map_or_else(|| "a signal".to_string(), |c| format!("code {c}"));
                fail(
                    &mut on_phase,
                    if tail.is_empty() {
                        format!("bato install exited with {code}")
                    } else {
                        tail
                    },
                )
            }
            (Err(e), _) => fail(&mut on_phase, format!("bato install: {e}")),
        }
    }

    /// The binary to start after an update: the path this process was started
    /// from (it now holds the new release, since `bato install` swapped the
    /// directory entry), else `<install dir>/<slug>`.
    pub fn installed_binary(&self) -> PathBuf {
        identity::current_exe()
            .filter(|p| p.starts_with(&self.install_dir) && p.exists())
            .unwrap_or_else(|| self.install_dir.join(&self.slug))
    }

    /// "Restart to update": start the installed binary (same arguments, own
    /// process group, so it survives this one), then the caller exits. A
    /// bundled server is stopped by the old process's exit and started fresh
    /// by the new one.
    pub fn relaunch(&self) -> std::io::Result<()> {
        use std::os::unix::process::CommandExt;
        if !self.packaged {
            return Err(std::io::Error::other(NOT_PACKAGED));
        }
        Command::new(self.installed_binary())
            .args(std::env::args_os().skip(1))
            .stdin(Stdio::null())
            .process_group(0)
            .spawn()
            .map(drop)
    }

    /// The whole Electron loop on the calling thread: check now and every
    /// `interval`, apply when `auto_apply` (electron-updater's
    /// `autoDownload`), stop checking once an update is downloaded (it only
    /// needs a restart). Returns when `on_phase` returns `false`.
    pub fn run(
        &self,
        auto_apply: bool,
        interval: Duration,
        mut on_phase: impl FnMut(UpdatePhase) -> bool,
    ) {
        if !self.packaged {
            return;
        }
        loop {
            if !on_phase(UpdatePhase::Checking) {
                return;
            }
            let phase = self.check();
            let available = matches!(phase, UpdatePhase::Available { .. });
            if !on_phase(phase) {
                return;
            }
            if available && auto_apply {
                let mut alive = true;
                let applied = self.apply(|p| alive = alive && on_phase(p));
                if !alive {
                    return;
                }
                if applied.is_ok() {
                    return;
                }
            }
            std::thread::sleep(interval);
        }
    }
}

/// `meta.latest` from `GET /api/apps/:name`.
pub fn latest_from_meta(meta: &Value) -> Option<String> {
    meta.get("latest")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The bato CLI, probed in bato-store's order: `BATO_BIN`, `bato` on PATH,
/// `/usr/local/bin/bato`, `~/.local/bin/bato`. A GUI started from a launcher
/// often lacks the login shell's PATH, hence the fixed fallbacks. Each
/// candidate must answer `bato -V`.
pub fn find_bato() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os(ENV_BATO_BIN).filter(|p| !p.is_empty()) {
        candidates.push(p.into());
    }
    candidates.push("bato".into());
    candidates.push("/usr/local/bin/bato".into());
    candidates.push(identity::home().join(".local/bin/bato"));
    candidates.into_iter().find(|c| {
        Command::new(c)
            .arg("-V")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// One line of `bato install --json --progress`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallEvent {
    Plan {
        name: String,
        version: String,
        kind: String,
    },
    Progress {
        received: u64,
        total: Option<u64>,
    },
    Done {
        version: String,
    },
    Failed {
        error: String,
    },
}

/// `None` for anything that is not an event line (the CLI keeps prose on
/// stderr, but a caller should not trust that blindly).
pub fn parse_install_event(line: &str) -> Option<InstallEvent> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    if v.get("ok") == Some(&Value::Bool(false)) {
        return Some(InstallEvent::Failed { error: s("error") });
    }
    match v.get("event")?.as_str()? {
        "plan" => Some(InstallEvent::Plan {
            name: s("name"),
            version: s("version"),
            kind: s("type"),
        }),
        "progress" => Some(InstallEvent::Progress {
            received: v.get("received")?.as_u64()?,
            total: v.get("total").and_then(Value::as_u64).filter(|t| *t > 0),
        }),
        "done" => Some(InstallEvent::Done {
            version: s("version"),
        }),
        _ => None,
    }
}

/// `Math.round(received / total * 100)`, clamped; `None` without a total.
pub fn percent(received: u64, total: Option<u64>) -> Option<u8> {
    let total = total?;
    Some(((received as f64 / total as f64 * 100.0).round()).clamp(0.0, 100.0) as u8)
}

/// Version order for `x.y.z` releases: numeric segments (missing = 0, a
/// leading `v` ignored), then a pre-release (`1.0.0-rc.1`) sorts before its
/// release, as in semver. The CLI's `cmpVersion` is the numeric part of this.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let (an, ap) = split_version(a);
    let (bn, bp) = split_version(b);
    for i in 0..an.len().max(bn.len()) {
        let o = an.get(i).unwrap_or(&0).cmp(bn.get(i).unwrap_or(&0));
        if o != Ordering::Equal {
            return o;
        }
    }
    match (ap, bp) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => compare_prerelease(x, y),
    }
}

fn split_version(v: &str) -> (Vec<u64>, Option<&str>) {
    let v = v.trim();
    let v = v.strip_prefix(['v', 'V']).unwrap_or(v);
    let v = v.split('+').next().unwrap_or(v);
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let nums = core
        .split('.')
        .map(|s| {
            s.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect();
    (nums, pre)
}

fn compare_prerelease(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.split('.'), b.split('.'));
    loop {
        match (x.next(), y.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) => {
                let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => p.cmp(q),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    // Captured from bato 1.4.9 against a local fake registry (no real
    // registry involved): `install <slug> --json --progress`, its `--json`
    // single document, an unknown app and an unknown version.
    const PROGRESS: &str = include_str!("../fixtures/bato-install-progress.ndjson");
    const PROGRESS_FAST: &str = include_str!("../fixtures/bato-install-progress-fast.ndjson");
    const PROGRESS_NO_LENGTH: &str =
        include_str!("../fixtures/bato-install-progress-no-length.ndjson");
    const SINGLE_JSON: &str = include_str!("../fixtures/bato-install-json.json");
    const NOT_FOUND: &str = include_str!("../fixtures/bato-install-not-found.ndjson");
    const BAD_VERSION: &str = include_str!("../fixtures/bato-install-bad-version.ndjson");
    // GET https://registry.bato.lan/api/apps/batoai (a real desktop release).
    const REGISTRY_META: &str = include_str!("../fixtures/registry-app-desktop.json");

    const APP: AppIdentity = AppIdentity::new("Fixture", "batoshellfixture", "1.2.0");

    #[test]
    fn versions() {
        use Ordering::*;
        assert_eq!(compare_versions("1.2.3", "1.2.3"), Equal);
        assert_eq!(compare_versions("1.2.10", "1.2.9"), Greater);
        assert_eq!(compare_versions("1.10.0", "1.9.9"), Greater);
        assert_eq!(compare_versions("2.0", "1.99.99"), Greater);
        assert_eq!(compare_versions("1.2", "1.2.0"), Equal);
        assert_eq!(compare_versions("v1.2.3", "1.2.3"), Equal);
        assert_eq!(compare_versions("0.4.4", "0.5.0"), Less);
        assert_eq!(compare_versions("1.0.0-rc.1", "1.0.0"), Less);
        assert_eq!(compare_versions("1.0.0-rc.2", "1.0.0-rc.10"), Less);
        assert_eq!(compare_versions("1.0.0-alpha", "1.0.0-alpha.1"), Less);
        assert_eq!(compare_versions("1.0.0-beta", "1.0.0-alpha"), Greater);
        assert_eq!(compare_versions("1.0.0+build.5", "1.0.0"), Equal);
    }

    #[test]
    fn registry_meta_latest() {
        let meta: Value = serde_json::from_str(REGISTRY_META).unwrap();
        assert_eq!(latest_from_meta(&meta).as_deref(), Some("0.4.4"));
        assert_eq!(
            latest_from_meta(&serde_json::json!({"error": "no such program"})),
            None
        );
    }

    fn events(s: &str) -> Vec<InstallEvent> {
        s.lines().filter_map(parse_install_event).collect()
    }

    #[test]
    fn progress_stream() {
        let ev = events(PROGRESS);
        assert_eq!(ev.len(), PROGRESS.lines().count(), "every line is an event");
        assert_eq!(
            ev[0],
            InstallEvent::Plan {
                name: "batoshellfixture".into(),
                version: "1.2.3".into(),
                kind: "desktop".into()
            }
        );
        assert_eq!(
            ev[1],
            InstallEvent::Progress {
                received: 65536,
                total: Some(1049047)
            }
        );
        assert_eq!(
            *ev.last().unwrap(),
            InstallEvent::Done {
                version: "1.2.3".into()
            }
        );
        let pct: Vec<u8> = ev
            .iter()
            .filter_map(|e| match e {
                InstallEvent::Progress { received, total } => percent(*received, *total),
                _ => None,
            })
            .collect();
        assert_eq!(pct.first(), Some(&6));
        assert_eq!(pct.last(), Some(&100));
        assert!(pct.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(
            *events(PROGRESS_FAST).last().unwrap(),
            InstallEvent::Done {
                version: "1.2.3".into()
            }
        );
    }

    #[test]
    fn progress_without_content_length() {
        let ev = events(PROGRESS_NO_LENGTH);
        assert_eq!(
            ev[1],
            InstallEvent::Progress {
                received: 262144,
                total: None
            }
        );
        assert_eq!(percent(262144, None), None);
    }

    #[test]
    fn failures_and_single_document() {
        assert_eq!(
            events(NOT_FOUND),
            vec![InstallEvent::Failed {
                error: "404 Not Found".into()
            }]
        );
        assert_eq!(
            events(BAD_VERSION)[1],
            InstallEvent::Failed {
                error: "no such version: 9.9.9".into()
            }
        );
        // `--json` without `--progress`: one document, no `event` key.
        assert_eq!(events(SINGLE_JSON), vec![]);
        assert_eq!(parse_install_event("✓ installed x"), None);
        assert_eq!(parse_install_event(""), None);
    }

    /// A stand-in `bato` that replays a fixture and records how it was called.
    fn fake_cli(dir: &Path, fixture: &str, code: i32) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("out.ndjson"), fixture).unwrap();
        let script = dir.join("bato");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"$@\" > '{d}/args'\necho \"$BATO_REGISTRY_URL\" > '{d}/registry'\necho 'some prose' >&2\ncat '{d}/out.ndjson'\nexit {code}\n",
                d = dir.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("bato-shell-updater-{name}-{}", std::process::id()))
    }

    #[test]
    fn apply_maps_cli_output_to_phases() {
        let dir = tmp("ok");
        let cli = fake_cli(&dir, PROGRESS, 0);
        let up = Updater::new(&APP, DEFAULT_REGISTRY)
            .with_registry("https://registry.example/")
            .with_packaged(true);
        assert_eq!(up.registry(), "https://registry.example");
        let mut phases = Vec::new();
        let v = up.apply_with(&cli, |p| phases.push(p)).unwrap();
        assert_eq!(v, "1.2.3");
        assert_eq!(
            std::fs::read_to_string(dir.join("args")).unwrap().trim(),
            "install batoshellfixture --json --progress"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("registry"))
                .unwrap()
                .trim(),
            up.registry()
        );
        assert_eq!(
            phases.first(),
            Some(&UpdatePhase::Downloading { percent: 0 })
        );
        assert_eq!(
            phases.last(),
            Some(&UpdatePhase::Downloaded {
                version: "1.2.3".into()
            })
        );
        assert!(phases.contains(&UpdatePhase::Downloading { percent: 100 }));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn apply_reports_cli_errors() {
        let dir = tmp("fail");
        let cli = fake_cli(&dir, BAD_VERSION, 1);
        let up = Updater::new(&APP, DEFAULT_REGISTRY);
        let mut phases = Vec::new();
        let e = up.apply_with(&cli, |p| phases.push(p)).unwrap_err();
        assert_eq!(e, ApplyError::Failed("no such version: 9.9.9".into()));
        assert_eq!(
            phases.last(),
            Some(&UpdatePhase::Error {
                message: "no such version: 9.9.9".into()
            })
        );

        // No JSON at all: the last stderr line explains it.
        let cli = fake_cli(&dir, "", 3);
        assert_eq!(
            up.apply_with(&cli, |_| {}).unwrap_err(),
            ApplyError::Failed("some prose".into())
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dev_builds_do_not_update() {
        let up = Updater::new(&APP, DEFAULT_REGISTRY);
        assert!(!up.is_supported());
        let mut phases = Vec::new();
        assert_eq!(up.apply(|p| phases.push(p)), Err(ApplyError::NotPackaged));
        assert_eq!(
            phases,
            vec![UpdatePhase::Error {
                message: NOT_PACKAGED.into()
            }]
        );
        assert!(up.relaunch().is_err());
        assert_eq!(up.install_command(), "bato install batoshellfixture");
    }

    /// Wait up to 5 s for a file a child process writes.
    fn wait_for(path: &Path) -> String {
        for _ in 0..100 {
            if let Ok(s) = std::fs::read_to_string(path)
                && s.ends_with('\n')
            {
                return s;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("{} never appeared", path.display());
    }

    #[test]
    fn relaunch_starts_the_installed_binary_in_its_own_group() {
        let dir = tmp("relaunch");
        let install = dir.join("apps").join(APP.slug);
        std::fs::create_dir_all(&install).unwrap();
        let marker = dir.join("started");
        let bin = install.join(APP.slug);
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\necho \"$$ $(ps -o pgid= -p $$ | tr -d ' ')\" > '{m}.tmp'\nmv '{m}.tmp' '{m}'\n",
                m = marker.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

        let up = Updater::new(&APP, DEFAULT_REGISTRY).with_install_dir(&install);
        assert!(up.relaunch().is_err(), "a dev build never relaunches");
        let up = up.with_packaged(true);
        // The test binary is not inside the install dir: `<dir>/<slug>`.
        assert_eq!(up.installed_binary(), bin);
        up.relaunch().unwrap();
        let line = wait_for(&marker);
        let (pid, pgid) = line.trim().split_once(' ').unwrap();
        assert_eq!(pid, pgid, "the new process leads its own group: {line}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// GET only, against the real registry: an old version of a real
    /// desktop app sees its newer release.
    /// `cargo test -p bato-shell -- --ignored live_registry_check`.
    #[test]
    #[ignore = "needs https://registry.bato.lan"]
    fn live_registry_check() {
        let app = AppIdentity::new("BatoAI", "batoai", "0.0.1");
        let up = Updater::new(&app, DEFAULT_REGISTRY).with_registry(DEFAULT_REGISTRY);
        assert!(
            matches!(up.check(), UpdatePhase::Available { .. }),
            "{:?}",
            up.check()
        );
    }

    #[test]
    fn check_against_a_fake_registry() {
        use std::io::Write;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for (i, s) in l.incoming().take(3).enumerate() {
                let mut s = s.unwrap();
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let (status, body) = match i {
                    0 => (200, r#"{"name":"batoshellfixture","latest":"1.3.0"}"#),
                    1 => (200, r#"{"name":"batoshellfixture","latest":"1.2.0"}"#),
                    _ => (404, r#"{"error":"no such program"}"#),
                };
                let _ = write!(
                    s,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        // with_registry: BATO_REGISTRY_URL from the developer's shell must not win.
        let up = Updater::new(&APP, DEFAULT_REGISTRY).with_registry(&base);
        assert_eq!(
            up.check(),
            UpdatePhase::Available {
                version: "1.3.0".into()
            }
        );
        assert_eq!(up.check(), UpdatePhase::None);
        assert_eq!(
            up.check(),
            UpdatePhase::Error {
                message: "Update check failed: no such program".into()
            }
        );
    }
}
