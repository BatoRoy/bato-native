//! Environment hooks shared with the Electron client and the test tooling.
//!
//! | Variable | Meaning |
//! |---|---|
//! | `BATO_VIEW` | Open at a page/modal/scroll position, URL-query syntax: `view=examples&tab=inputs&card=selects`, `modal=options`, `scroll=1200`. Same IDs as Electron (`docs/CARDS.md`). Debug extras: `toast=1`, `demo=sidebar`, `autoscroll=x,y,cursor_y`. |
//! | `BATO_SMOKE=1` | Print `first-frame-ms=<n>` (from the kernel's process start time) once the first frame is presented, then exit 0. |
//! | `BATO_TIMING` | Startup milestones on stderr ([`mark`]). |

use std::sync::OnceLock;

pub const ENV_VIEW: &str = "BATO_VIEW";
pub const ENV_SMOKE: &str = "BATO_SMOKE";
pub const ENV_TIMING: &str = "BATO_TIMING";

/// `BATO_VIEW`, parsed like the React client's
/// `new URLSearchParams(location.hash.slice(1))`: `&`-separated, percent- and
/// `+`-decoded, first occurrence of a key wins, a leading `#` is ignored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BootView {
    pub view: Option<String>,
    pub modal: Option<String>,
    /// Content scroll offset in CSS px.
    pub scroll: Option<f32>,
    /// Showcase tab (`inputs`, `date`, `feedback`, …).
    pub tab: Option<String>,
    /// Showcase card ID; the card is scrolled to the top of the content area.
    pub card: Option<String>,
    /// BatoScroll engaged at (x, y) with the cursor at (x, cursor_y).
    pub autoscroll: Option<(f32, f32, f32)>,
    /// `toast=1`: sample toasts (screenshots).
    pub toast: bool,
    /// `demo=<what>`, e.g. `sidebar`.
    pub demo: Option<String>,
    /// Every other key, in order, for app-specific hooks.
    pub extra: Vec<(String, String)>,
}

impl BootView {
    pub fn from_env() -> Self {
        Self::parse(&std::env::var(ENV_VIEW).unwrap_or_default())
    }

    pub fn parse(s: &str) -> Self {
        let mut b = BootView::default();
        let mut seen: Vec<String> = Vec::new();
        for pair in s
            .trim()
            .trim_start_matches('#')
            .split('&')
            .filter(|p| !p.is_empty())
        {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let (k, v) = (decode(k), decode(v));
            if seen.contains(&k) {
                continue;
            }
            seen.push(k.clone());
            match k.as_str() {
                "view" => b.view = Some(v),
                "modal" => b.modal = Some(v),
                "scroll" => b.scroll = v.trim().parse().ok(),
                "tab" => b.tab = Some(v),
                "card" => b.card = Some(v),
                "toast" => b.toast = v == "1",
                "demo" => b.demo = Some(v),
                "autoscroll" => {
                    let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if let [x, y, cy] = n[..] {
                        b.autoscroll = Some((x, y, cy));
                    }
                }
                _ => b.extra.push((k, v)),
            }
        }
        b
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.extra
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// `application/x-www-form-urlencoded` decoding: `+` is a space, `%XX` a byte.
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = b
            .get(i + 1..i + 3)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
            .and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (b[i], hex) {
            (b'+', _) => out.push(b' '),
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 2;
            }
            (c, _) => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `BATO_SMOKE=1`.
pub fn smoke_enabled() -> bool {
    std::env::var_os(ENV_SMOKE).is_some_and(|v| v == "1")
}

/// Milliseconds since the kernel started this process (`/proc/self/stat`
/// field 22, CLOCK_BOOTTIME based), so the count includes exec and dynamic
/// linking, not just `main`.
pub fn process_age_ms() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    start_ms_since_boot(&stat).map_or(0.0, |start| boottime_ms() - start)
}

/// Parse `starttime` (field 22) from a `/proc/<pid>/stat` line, in ms. Fields
/// are counted after the `)` that closes `comm`, which may contain spaces.
fn start_ms_since_boot(stat: &str) -> Option<f64> {
    let after = stat.rsplit_once(')')?.1;
    let ticks: f64 = after.split_whitespace().nth(19)?.parse().ok()?;
    // SAFETY: sysconf has no preconditions.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    (hz > 0.0).then(|| ticks / hz * 1000.0)
}

fn boottime_ms() -> f64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid pointer to a timespec.
    unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) };
    ts.tv_sec as f64 * 1000.0 + ts.tv_nsec as f64 / 1e6
}

/// Prints `first-frame-ms=<n>` on stdout.
pub fn report_first_frame() {
    println!("first-frame-ms={:.0}", process_age_ms());
}

/// `BATO_SMOKE` bookkeeping. Feed it every frame event (iced:
/// `window::frames()`); it returns `true` once the first frame is known to be
/// on screen, after printing the measurement. The app then exits 0.
///
/// "On screen" is the second redraw: on Wayland the next redraw only arrives
/// after the compositor's frame callback for the first.
#[derive(Debug, Default, Clone)]
pub struct Smoke {
    frames: u32,
}

impl Smoke {
    pub fn on_frame(&mut self) -> bool {
        self.frames += 1;
        mark(if self.frames == 1 {
            "first redraw event"
        } else {
            "frame presented"
        });
        if self.frames == 2 {
            report_first_frame();
            return true;
        }
        false
    }
}

/// `BATO_TIMING`: `[timing]  123.4 ms  <what>` on stderr.
pub fn mark(what: &str) {
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os(ENV_TIMING).is_some()) {
        eprintln!("[timing] {:>7.1} ms  {what}", process_age_ms());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_modal_scroll() {
        let b = BootView::parse("view=scroll-lab&modal=options&scroll=1200");
        assert_eq!(b.view.as_deref(), Some("scroll-lab"));
        assert_eq!(b.modal.as_deref(), Some("options"));
        assert_eq!(b.scroll, Some(1200.0));
        assert_eq!(BootView::parse(""), BootView::default());
    }

    #[test]
    fn showcase_tab_and_card() {
        let b = BootView::parse("#view=examples&tab=inputs&card=sliders-color");
        assert_eq!(b.view.as_deref(), Some("examples"));
        assert_eq!(b.tab.as_deref(), Some("inputs"));
        assert_eq!(b.card.as_deref(), Some("sliders-color"));
    }

    #[test]
    fn autoscroll_needs_three_numbers() {
        assert_eq!(
            BootView::parse("autoscroll=640,400,700").autoscroll,
            Some((640.0, 400.0, 700.0))
        );
        assert_eq!(BootView::parse("autoscroll=640,400").autoscroll, None);
        assert_eq!(BootView::parse("autoscroll=a,b,c").autoscroll, None);
    }

    #[test]
    fn debug_extras_and_unknown_keys() {
        let b = BootView::parse("toast=1&demo=sidebar&lang=de&flag");
        assert!(b.toast);
        assert_eq!(b.demo.as_deref(), Some("sidebar"));
        assert_eq!(b.get("lang"), Some("de"));
        assert_eq!(b.get("flag"), Some(""));
        assert!(!BootView::parse("toast=0").toast);
    }

    #[test]
    fn decodes_like_url_search_params() {
        let b = BootView::parse("view=a%20b&card=x+y&tab=%E2%9C%93&modal=100%&scroll=bad");
        assert_eq!(b.view.as_deref(), Some("a b"));
        assert_eq!(b.card.as_deref(), Some("x y"));
        assert_eq!(b.tab.as_deref(), Some("✓"));
        assert_eq!(b.modal.as_deref(), Some("100%"));
        assert_eq!(b.scroll, None);
        // First occurrence wins, as URLSearchParams.get does.
        assert_eq!(BootView::parse("view=a&view=b").view.as_deref(), Some("a"));
    }

    #[test]
    fn proc_stat_start_time() {
        // comm with spaces and a ')' inside.
        let stat = "123 (my (odd) app) S 1 123 123 0 -1 4194560 0 0 0 0 0 0 0 0 20 0 1 0 500 0 0";
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
        assert_eq!(start_ms_since_boot(stat), Some(500.0 / hz * 1000.0));
        let age = process_age_ms();
        assert!(age > 0.0 && age < 3_600_000.0, "{age}");
    }

    #[test]
    fn smoke_reports_on_the_second_frame() {
        let mut s = Smoke::default();
        assert!(!s.on_frame());
        assert!(s.on_frame());
        assert!(!s.on_frame());
    }
}
