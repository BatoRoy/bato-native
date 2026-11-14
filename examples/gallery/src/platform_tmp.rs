//! TEMPORARY platform glue, copied from the iced PoC. bato-shell (Phase 1.3)
//! replaces all of it: settings persistence, the backend calls, the desktop
//! notification, `BATO_VIEW` / `BATO_SMOKE`, window identity.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use bato_kit::frame::{Dialog, Start};
use bato_kit::iced;
use bato_kit::theme::settings::Settings;

// ── Window ──────────────────────────────────────────────────────────────

/// 1280×800, min 900×600, Wayland app_id `batopoc-gallery` (`BATO_APP_ID`
/// overrides it; `batopoc-*` is routed off-screen in automated tests).
pub fn window_settings() -> iced::window::Settings {
    iced::window::Settings {
        size: iced::Size::new(1280.0, 800.0),
        min_size: Some(iced::Size::new(900.0, 600.0)),
        decorations: false,
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: std::env::var("BATO_APP_ID")
                .unwrap_or_else(|_| "batopoc-gallery".into()),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// wgpu probes every backend by default; the GL probe loads Mesa (~45 MB
/// PSS) only to throw it away, and the backdrop blur needs Vulkan.
pub fn prefer_vulkan() {
    if std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: called first thing in main, single-threaded.
        unsafe { std::env::set_var("WGPU_BACKEND", "vulkan") };
    }
}

// ── Settings ────────────────────────────────────────────────────────────

pub fn load_settings(slug: &str) -> Settings {
    Settings::load(slug)
}

pub fn save_settings(slug: &str, s: &Settings) {
    if let Err(e) = s.save(slug) {
        eprintln!("settings: {e}");
    }
}

// ── BATO_VIEW ───────────────────────────────────────────────────────────

/// `BATO_VIEW`: `view=home|examples|scroll-lab`, `tab=<tab>`, `card=<card>`,
/// `modal=options|about`, `scroll=<px>`; debug extras `toast=1`,
/// `demo=sidebar`, `autoscroll=x,y,cursor_y`.
#[derive(Debug, Default, Clone)]
pub struct Boot {
    pub view: Option<String>,
    pub tab: Option<String>,
    pub card: Option<String>,
    pub modal: Option<String>,
    pub scroll: Option<f32>,
    pub smoke: bool,
    pub toast: bool,
    pub demo_sidebar: bool,
    pub autoscroll: Option<(f32, f32, f32)>,
}

impl Boot {
    pub fn from_env() -> Self {
        let mut b = Self::parse(&std::env::var("BATO_VIEW").unwrap_or_default());
        b.smoke = std::env::var_os("BATO_SMOKE").is_some_and(|v| v == "1");
        b
    }

    pub fn parse(s: &str) -> Self {
        let mut b = Boot::default();
        for kv in s.trim_start_matches('#').split('&') {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            match k {
                "view" => b.view = Some(v.to_string()),
                "tab" => b.tab = Some(v.to_string()),
                "card" => b.card = Some(v.to_string()),
                "modal" => b.modal = Some(v.to_string()),
                "scroll" => b.scroll = v.parse().ok(),
                "toast" => b.toast = v == "1",
                "demo" => b.demo_sidebar = v == "sidebar",
                "autoscroll" => {
                    let n: Vec<f32> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                    if n.len() == 3 {
                        b.autoscroll = Some((n[0], n[1], n[2]));
                    }
                }
                _ => {}
            }
        }
        b
    }

    /// The frame's start: dialog, scroll, anchor, animations.
    pub fn start(&self, anchor: Option<iced::widget::Id>) -> Start {
        Start {
            dialog: self.modal.as_deref().and_then(Dialog::parse),
            scroll: self.scroll,
            anchor,
            no_animations: self.smoke,
        }
    }
}

// ── BATO_SMOKE ──────────────────────────────────────────────────────────

/// Milliseconds since the kernel started this process (CLOCK_BOOTTIME).
pub fn process_age_ms() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    // Field 22 (starttime), counted after the `)` that closes comm.
    let after = stat.rsplit_once(')').map(|x| x.1).unwrap_or("");
    let start_ticks: f64 = after
        .split_whitespace()
        .nth(19)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    // SAFETY: plain libc queries.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) };
    let now_ms = ts.tv_sec as f64 * 1000.0 + ts.tv_nsec as f64 / 1e6;
    now_ms - start_ticks / hz * 1000.0
}

/// `first-frame-ms=<n>`: printed at the second redraw (on Wayland the first
/// frame is then known to be presented).
pub fn smoke_report() {
    println!("first-frame-ms={:.0}", process_age_ms());
}

/// `BATO_TIMING=1`: startup milestones on stderr.
pub fn mark(what: &str) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("BATO_TIMING").is_some()) {
        eprintln!("[timing] {:>7.1} ms  {what}", process_age_ms());
    }
}

// ── Backend (spec §7.3) ─────────────────────────────────────────────────

/// HTTP/1.0 GET over `std::net` with a 5 s timeout (plain http only).
pub fn get(base: &str, path: &str) -> Result<(u16, String), String> {
    let rest = base
        .strip_prefix("http://")
        .ok_or_else(|| "only http:// is supported".to_string())?;
    let (hostport, prefix) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let addr_str = if hostport.contains(':') {
        hostport.to_string()
    } else {
        format!("{hostport}:80")
    };
    let timeout = Duration::from_secs(5);
    let addrs: Vec<_> = addr_str
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .collect();
    let mut last = "no address".to_string();
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(mut s) => {
                s.set_read_timeout(Some(timeout)).ok();
                s.set_write_timeout(Some(timeout)).ok();
                let req = format!(
                    "GET {prefix}{path} HTTP/1.0\r\nHost: {hostport}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
                );
                s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
                let mut buf = Vec::new();
                s.read_to_end(&mut buf).map_err(|e| e.to_string())?;
                let text = String::from_utf8_lossy(&buf);
                let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
                let status = head
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|c| c.parse().ok())
                    .ok_or_else(|| "bad response".to_string())?;
                return Ok((status, body.to_string()));
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

/// `GET /api/health` → ok?
pub fn health(base: &str) -> bool {
    matches!(get(base, "/api/health"), Ok((s, _)) if (200..300).contains(&s))
}

/// `GET /api/info` → version.
pub fn info_version(base: &str) -> Option<String> {
    let (s, body) = get(base, "/api/info")
        .ok()
        .filter(|(s, _)| (200..300).contains(s))?;
    let _ = s;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    v.get("version")?.as_str().map(str::to_string)
}

/// Run blocking work off the UI thread, for `Task::perform`.
pub fn spawn<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> impl std::future::Future<Output = T> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    async move { rx.await.expect("worker thread") }
}

/// freedesktop Notifications over D-Bus. False when no server answers.
pub fn notify(app: &str, title: &str, body: &str) -> bool {
    use std::collections::HashMap;
    let Ok(conn) = zbus::blocking::Connection::session() else {
        return false;
    };
    let hints: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
    conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(app, 0u32, "", title, body, Vec::<&str>::new(), hints, -1i32),
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bato_view_keys() {
        let b = Boot::parse("view=examples&tab=data&card=charts&modal=about&scroll=12");
        assert_eq!(b.view.as_deref(), Some("examples"));
        assert_eq!(b.tab.as_deref(), Some("data"));
        assert_eq!(b.card.as_deref(), Some("charts"));
        assert_eq!(b.scroll, Some(12.0));
        assert_eq!(b.start(None).dialog, Some(Dialog::About));
    }

    #[test]
    fn http_get_fails_cleanly() {
        assert!(get("http://127.0.0.1:1", "/api/health").is_err());
    }
}
