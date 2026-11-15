//! A system-tray icon (feature `tray`): Electron's `Tray`, as a
//! StatusNotifierItem with a `com.canonical.dbusmenu` menu on the session
//! bus (through `ksni`). Waybar's `tray` module, KDE Plasma and GNOME's
//! AppIndicator extension show it.
//!
//! - **Never blocks, never fails.** [`Tray::start`] returns at once; the
//!   connection is made on the tray's own thread and its outcome is a
//!   [`Status`] ([`Tray::status`], and [`Event::Status`] on every change).
//! - **No tray host.** Without a `StatusNotifierWatcher` (a tiling WM with no
//!   bar, Waybar without its `tray` module) the status is
//!   [`Status::NoWatcher`]: nobody can see the icon, so an app that closes to
//!   the tray must quit on close instead. Ask [`Tray::is_shown`] at the moment
//!   the window closes rather than once at start: the tray keeps listening and
//!   registers as soon as a watcher appears (a bar started late, Waybar
//!   restarted), and goes back to `NoWatcher` when it leaves.
//! - **Events** ([`Event`]): left click (`Activate`), middle click
//!   (`SecondaryActivate`), a menu row by its id, status changes. In iced:
//!   `iced_ext::tray_events`.
//! - **Updates** ([`Tray::set_tooltip`], [`set_icon_png`](Tray::set_icon_png),
//!   [`set_menu`](Tray::set_menu)) are queued to the tray's thread in order,
//!   so a slow bus never stalls the UI. BatoNotify's tray (an unread badge in
//!   the icon, a status line, recent events, a checkbox) is all of these.
//!
//! ```no_run
//! use bato_shell::{AppIdentity, tray::{self, Spec, Tray}};
//! const APP: AppIdentity = AppIdentity::new("BatoSound", "batosound", "1.0.0");
//! // Electron's menu: "Show BatoSound", a separator, "Quit"; tooltip and
//! // icon from the identity.
//! let tray = Tray::start(Spec::for_app(&APP));
//! // On the main window's close: keep running only if someone sees the icon.
//! let keep_running = tray.is_shown();
//! // Event::Menu(id) if id == tray::QUIT => quit
//! # let _ = (keep_running, tray::SHOW);
//! ```
//!
//! **Tests never touch the desktop's bus**: the protocol test runs in a child
//! process whose `DBUS_SESSION_BUS_ADDRESS` is a private `dbus-daemon`
//! (no service activation) with a fake watcher on it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use ksni::TrayMethods;

use crate::identity::AppIdentity;

/// The id of [`Spec::for_app`]'s "Show <product>" row.
pub const SHOW: &str = "show";
/// The id of [`Spec::for_app`]'s "Quit" row.
pub const QUIT: &str = "quit";

/// Icon sizes handed to the host (it scales the closest one); only those no
/// larger than the source image.
const SIZES: [u32; 6] = [16, 22, 24, 32, 48, 64];

/// A row of the tray menu.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MenuItem {
    /// A clickable row; a click is [`Event::Menu`] with its `id`.
    Item {
        id: String,
        label: String,
        enabled: bool,
    },
    /// A row with a checkmark. The host does not toggle it: the click is
    /// [`Event::Menu`], and the app sends the new state with
    /// [`Tray::set_menu`].
    Check {
        id: String,
        label: String,
        checked: bool,
        enabled: bool,
    },
    Separator,
}

impl MenuItem {
    pub fn item(id: impl Into<String>, label: impl Into<String>) -> Self {
        MenuItem::Item {
            id: id.into(),
            label: label.into(),
            enabled: true,
        }
    }

    pub fn check(id: impl Into<String>, label: impl Into<String>, checked: bool) -> Self {
        MenuItem::Check {
            id: id.into(),
            label: label.into(),
            checked,
            enabled: true,
        }
    }

    /// Greyed out (a status line, "No recent events").
    pub fn disabled(mut self) -> Self {
        match &mut self {
            MenuItem::Item { enabled, .. } | MenuItem::Check { enabled, .. } => *enabled = false,
            MenuItem::Separator => {}
        }
        self
    }
}

/// What the tray shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Spec {
    /// The SNI `Id`: stable across sessions (the slug).
    pub id: String,
    /// The SNI `Title` (the product name).
    pub title: String,
    /// The hover text (Electron's `setToolTip`).
    pub tooltip: String,
    /// The icon, a PNG of any size (scaled down to the host's sizes). `None`
    /// or an undecodable PNG: no icon (hosts show a placeholder).
    pub icon_png: Option<Vec<u8>>,
    pub menu: Vec<MenuItem>,
}

impl Spec {
    /// Electron's tray (`tray.js`): the app's icon, its product name as
    /// title and tooltip, and the menu "Show <product>" ([`SHOW`]), a
    /// separator, "Quit" ([`QUIT`]).
    pub fn for_app(app: &AppIdentity) -> Self {
        Self {
            id: app.slug.into(),
            title: app.product_name.into(),
            tooltip: app.product_name.into(),
            icon_png: app.icon_png.map(<[u8]>::to_vec),
            menu: vec![
                MenuItem::item(SHOW, format!("Show {}", app.product_name)),
                MenuItem::Separator,
                MenuItem::item(QUIT, "Quit"),
            ],
        }
    }
}

/// Where the tray stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    /// Connecting (the first few milliseconds).
    Starting,
    /// Registered with a `StatusNotifierWatcher`: the icon is visible.
    Shown,
    /// On the bus, but no watcher (no tray host runs). Registers by itself
    /// when one appears.
    NoWatcher,
    /// No session bus, or the watcher refused the item: no tray this run.
    /// [`Tray::error`] says why.
    Failed,
}

/// Something the user did to the tray, or a status change.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event {
    /// Primary activation: a left click.
    Activate,
    /// Secondary activation: usually a middle click.
    SecondaryActivate,
    /// A menu row, by its [`MenuItem`] id.
    Menu(String),
    Status(Status),
}

type Update = Box<dyn FnOnce(&mut Item) + Send>;

enum Cmd {
    Update(Update),
    Shutdown,
}

struct Shared {
    serial: u64,
    status: Mutex<(Status, Option<String>)>,
    changed: Condvar,
    subscribers: Mutex<Vec<mpsc::UnboundedSender<Event>>>,
}

impl Shared {
    fn emit(&self, e: Event) {
        self.subscribers
            .lock()
            .unwrap()
            .retain(|s| s.unbounded_send(e.clone()).is_ok());
    }

    /// Set the status (and emit it) unless it already is that. With `only_from`,
    /// only when the current status is that one.
    fn set_status(&self, to: Status, error: Option<String>, only_from: Option<Status>) {
        {
            let mut s = self.status.lock().unwrap();
            if s.0 == to || only_from.is_some_and(|f| f != s.0) {
                return;
            }
            *s = (to, error);
            self.changed.notify_all();
        }
        self.emit(Event::Status(to));
    }
}

/// A running tray icon. Clones share it; it goes away with [`shutdown`]
/// (Tray::shutdown) or when the last clone is dropped.
#[derive(Clone)]
pub struct Tray {
    shared: Arc<Shared>,
    cmds: mpsc::UnboundedSender<Cmd>,
}

impl std::fmt::Debug for Tray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tray")
            .field("serial", &self.shared.serial)
            .field("status", &self.status())
            .finish()
    }
}

/// By instance, so a `Subscription` keyed on a tray survives its updates.
impl PartialEq for Tray {
    fn eq(&self, other: &Self) -> bool {
        self.shared.serial == other.shared.serial
    }
}
impl Eq for Tray {}
impl std::hash::Hash for Tray {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.shared.serial.hash(state);
    }
}

static SERIAL: AtomicU64 = AtomicU64::new(1);

impl Tray {
    /// Put the icon in the tray. Returns at once: the bus connection and the
    /// registration happen on the tray's thread ([`status`](Self::status)).
    pub fn start(spec: Spec) -> Self {
        let shared = Arc::new(Shared {
            serial: SERIAL.fetch_add(1, Ordering::Relaxed),
            status: Mutex::new((Status::Starting, None)),
            changed: Condvar::new(),
            subscribers: Mutex::default(),
        });
        let (tx, rx) = mpsc::unbounded();
        let item = Item::new(spec, shared.clone());
        let thread_shared = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("bato-tray".into())
            .spawn(move || run(item, rx));
        if let Err(e) = spawned {
            thread_shared.set_status(Status::Failed, Some(e.to_string()), None);
        }
        Self { shared, cmds: tx }
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().unwrap().0
    }

    /// Whether a tray host shows the icon right now: the test for closing
    /// to the tray instead of quitting.
    pub fn is_shown(&self) -> bool {
        self.status() == Status::Shown
    }

    /// Why the tray [`Failed`](Status::Failed) (or the last watcher error).
    pub fn error(&self) -> Option<String> {
        self.shared.status.lock().unwrap().1.clone()
    }

    /// Block until the tray is past [`Status::Starting`] or `timeout` passes.
    pub fn wait_started(&self, timeout: Duration) -> Status {
        let s = self.shared.status.lock().unwrap();
        let (s, _) = self
            .shared
            .changed
            .wait_timeout_while(s, timeout, |s| s.0 == Status::Starting)
            .unwrap();
        s.0
    }

    /// A new stream of this tray's [`Event`]s (each caller gets every event
    /// from now on).
    pub fn events(&self) -> mpsc::UnboundedReceiver<Event> {
        let (tx, rx) = mpsc::unbounded();
        self.shared.subscribers.lock().unwrap().push(tx);
        rx
    }

    pub fn set_tooltip(&self, tooltip: impl Into<String>) {
        let tooltip = tooltip.into();
        self.update(move |i| i.spec.tooltip = tooltip);
    }

    pub fn set_title(&self, title: impl Into<String>) {
        let title = title.into();
        self.update(move |i| i.spec.title = title);
    }

    /// A new icon (BatoNotify's badge). Returns false, and keeps the old
    /// icon, if `png` does not decode.
    pub fn set_icon_png(&self, png: &[u8]) -> bool {
        let Some(icons) = pixmaps(png) else {
            return false;
        };
        self.update(move |i| i.icons = icons);
        true
    }

    pub fn set_menu(&self, menu: Vec<MenuItem>) {
        self.update(move |i| i.spec.menu = menu);
    }

    fn update(&self, f: impl FnOnce(&mut Item) + Send + 'static) {
        let _ = self.cmds.unbounded_send(Cmd::Update(Box::new(f)));
    }

    /// Remove the icon from the tray (Quit). Further updates are ignored.
    pub fn shutdown(&self) {
        let _ = self.cmds.unbounded_send(Cmd::Shutdown);
    }
}

/// The tray's thread: connect and register, then apply updates in order
/// until shutdown or the last [`Tray`] is dropped.
fn run(item: Item, mut rx: mpsc::UnboundedReceiver<Cmd>) {
    let shared = item.shared.clone();
    // With "assume available", a missing watcher is not an error: ksni
    // reports it to `watcher_offline` and keeps watching the name, so the
    // item registers when a watcher (re)appears.
    let handle = match futures::executor::block_on(item.assume_sni_available(true).spawn()) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[bato-shell] tray: {e}");
            shared.set_status(Status::Failed, Some(e.to_string()), None);
            return;
        }
    };
    // Registered, unless watcher_offline already said otherwise.
    shared.set_status(Status::Shown, None, Some(Status::Starting));
    futures::executor::block_on(async {
        while let Some(cmd) = rx.next().await {
            match cmd {
                Cmd::Update(f) => {
                    if handle.update(f).await.is_none() {
                        break;
                    }
                }
                Cmd::Shutdown => break,
            }
        }
        handle.shutdown().await;
    });
    shared.set_status(Status::Failed, Some("shut down".into()), None);
}

/// The ksni side: what the host reads, and where its calls go.
struct Item {
    spec: Spec,
    icons: Vec<ksni::Icon>,
    shared: Arc<Shared>,
}

impl Item {
    fn new(mut spec: Spec, shared: Arc<Shared>) -> Self {
        let icons = spec
            .icon_png
            .take()
            .and_then(|p| pixmaps(&p))
            .unwrap_or_default();
        Self {
            spec,
            icons,
            shared,
        }
    }
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        self.spec.id.clone()
    }

    fn title(&self) -> String {
        self.spec.title.clone()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.shared.emit(Event::Activate);
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.shared.emit(Event::SecondaryActivate);
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icons.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.spec.tooltip.clone(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        self.spec.menu.iter().map(menu_item).collect()
    }

    fn watcher_online(&self) {
        // ksni registers right after this; a failure comes back as offline.
        self.shared.set_status(Status::Shown, None, None);
    }

    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        let error = match reason {
            ksni::OfflineReason::Error(e) => Some(e.to_string()),
            _ => None,
        };
        self.shared.set_status(Status::NoWatcher, error, None);
        true // keep watching for the next watcher
    }
}

fn menu_item(m: &MenuItem) -> ksni::MenuItem<Item> {
    use ksni::menu::{CheckmarkItem, StandardItem};
    let click = |id: &str| {
        let id = id.to_string();
        Box::new(move |t: &mut Item| t.shared.emit(Event::Menu(id.clone())))
    };
    match m {
        MenuItem::Item { id, label, enabled } => StandardItem {
            label: escape_mnemonic(label),
            enabled: *enabled,
            activate: click(id),
            ..Default::default()
        }
        .into(),
        MenuItem::Check {
            id,
            label,
            checked,
            enabled,
        } => CheckmarkItem {
            label: escape_mnemonic(label),
            enabled: *enabled,
            checked: *checked,
            activate: click(id),
            ..Default::default()
        }
        .into(),
        MenuItem::Separator => ksni::MenuItem::Separator,
    }
}

/// dbusmenu reads `_x` as a mnemonic; a literal underscore is `__`.
fn escape_mnemonic(label: &str) -> String {
    label.replace('_', "__")
}

/// A PNG as the host's ARGB32 pixmaps: the sizes of [`SIZES`] that fit,
/// area-averaged (or the image itself when it is smaller than all of them).
fn pixmaps(png: &[u8]) -> Option<Vec<ksni::Icon>> {
    let (w, h, rgba) = decode_png(png)?;
    let mut out: Vec<ksni::Icon> = SIZES
        .iter()
        .filter(|&&s| s <= w.max(h))
        .map(|&s| {
            // Fit the longer side to `s`, keeping the aspect.
            let (tw, th) = if w >= h {
                (s, (h * s).div_ceil(w).max(1))
            } else {
                ((w * s).div_ceil(h).max(1), s)
            };
            argb(tw, th, &resample(w, h, &rgba, tw, th))
        })
        .collect();
    if out.is_empty() {
        out.push(argb(w, h, &rgba));
    }
    Some(out)
}

/// Any PNG as straight RGBA8.
fn decode_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let px = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => px
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[g, a]| [g, g, g, a])
            .collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None, // EXPAND turns palettes into RGB(A)
    };
    (info.width > 0 && info.height > 0).then_some((info.width, info.height, rgba))
}

/// Area-average resampling of straight RGBA (premultiplied while summing,
/// so transparent pixels do not darken the edges).
fn resample(w: u32, h: u32, src: &[u8], tw: u32, th: u32) -> Vec<u8> {
    let (sx, sy) = (w as f64 / tw as f64, h as f64 / th as f64);
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    for ty in 0..th {
        let (y0, y1) = (ty as f64 * sy, (ty + 1) as f64 * sy);
        for tx in 0..tw {
            let (x0, x1) = (tx as f64 * sx, (tx + 1) as f64 * sx);
            let mut acc = [0f64; 4];
            let mut area = 0f64;
            for y in y0.floor() as u32..(y1.ceil() as u32).min(h) {
                let cy = (y1.min(y as f64 + 1.0) - y0.max(y as f64)).max(0.0);
                for x in x0.floor() as u32..(x1.ceil() as u32).min(w) {
                    let cx = (x1.min(x as f64 + 1.0) - x0.max(x as f64)).max(0.0);
                    let k = cx * cy;
                    let i = ((y * w + x) * 4) as usize;
                    let a = src[i + 3] as f64 / 255.0;
                    acc[0] += src[i] as f64 * a * k;
                    acc[1] += src[i + 1] as f64 * a * k;
                    acc[2] += src[i + 2] as f64 * a * k;
                    acc[3] += a * k;
                    area += k;
                }
            }
            let a = if area > 0.0 { acc[3] / area } else { 0.0 };
            let un = |v: f64| {
                if acc[3] > 0.0 {
                    (v / acc[3]).round().clamp(0.0, 255.0) as u8
                } else {
                    0
                }
            };
            out.extend([
                un(acc[0]),
                un(acc[1]),
                un(acc[2]),
                (a * 255.0).round() as u8,
            ]);
        }
    }
    out
}

/// Straight RGBA8 → SNI's ARGB32 in network byte order.
fn argb(w: u32, h: u32, rgba: &[u8]) -> ksni::Icon {
    let mut data = rgba.to_vec();
    for p in data.as_chunks_mut::<4>().0 {
        p.rotate_right(1);
    }
    ksni::Icon {
        width: w as i32,
        height: h as i32,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: AppIdentity = AppIdentity::new("Bato_Sound", "batosound", "1.2.0");

    /// An `w`×`h` RGBA PNG filled by `px`.
    fn png(w: u32, h: u32, px: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                data.extend(px(x, y));
            }
        }
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(&data).unwrap();
        drop(wr);
        out
    }

    #[test]
    fn for_app_is_electrons_tray() {
        let s = Spec::for_app(&APP.with_icon(b"png"));
        assert_eq!(
            (s.id.as_str(), s.title.as_str()),
            ("batosound", "Bato_Sound")
        );
        assert_eq!(s.tooltip, "Bato_Sound");
        assert_eq!(s.icon_png.as_deref(), Some(&b"png"[..]));
        assert_eq!(
            s.menu,
            [
                MenuItem::item(SHOW, "Show Bato_Sound"),
                MenuItem::Separator,
                MenuItem::item(QUIT, "Quit"),
            ]
        );
    }

    #[test]
    fn menu_items_build_and_disable() {
        assert_eq!(
            MenuItem::check("dnd", "Do not disturb", true).disabled(),
            MenuItem::Check {
                id: "dnd".into(),
                label: "Do not disturb".into(),
                checked: true,
                enabled: false
            }
        );
        assert_eq!(MenuItem::Separator.disabled(), MenuItem::Separator);
        assert_eq!(escape_mnemonic("a_b__c"), "a__b____c");
    }

    #[test]
    fn menu_rows_emit_their_ids() {
        let shared = Arc::new(Shared {
            serial: 0,
            status: Mutex::new((Status::Starting, None)),
            changed: Condvar::new(),
            subscribers: Mutex::default(),
        });
        let mut item = Item::new(Spec::for_app(&APP), shared.clone());
        let (tx, mut rx) = mpsc::unbounded();
        shared.subscribers.lock().unwrap().push(tx);
        let menu = ksni::Tray::menu(&item);
        assert_eq!(menu.len(), 3);
        match &menu[0] {
            ksni::MenuItem::Standard(s) => {
                assert_eq!(s.label, "Show Bato__Sound");
                (s.activate)(&mut item);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(menu[1], ksni::MenuItem::Separator));
        ksni::Tray::activate(&mut item, 0, 0);
        ksni::Tray::watcher_offline(&item, ksni::OfflineReason::No);
        ksni::Tray::watcher_offline(&item, ksni::OfflineReason::No); // no repeat
        let got: Vec<Event> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(
            got,
            [
                Event::Menu(SHOW.into()),
                Event::Activate,
                Event::Status(Status::NoWatcher)
            ]
        );
        assert_eq!(ksni::Tray::tool_tip(&item).title, "Bato_Sound");
    }

    #[test]
    fn pixmaps_are_argb_at_the_sizes_that_fit() {
        // Opaque red left, transparent green right (the edge at x = 30).
        let p = png(64, 64, |x, _| {
            if x < 30 {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 0]
            }
        });
        let icons = pixmaps(&p).unwrap();
        let sizes: Vec<i32> = icons.iter().map(|i| i.width).collect();
        assert_eq!(sizes, [16, 22, 24, 32, 48, 64]);
        let i16 = &icons[0];
        assert_eq!((i16.height, i16.data.len()), (16, 16 * 16 * 4));
        // ARGB: alpha first. Left pixel opaque red, right one transparent.
        assert_eq!(&i16.data[..4], &[255, 255, 0, 0]);
        assert_eq!(i16.data[15 * 4], 0);
        // At 22 px, pixel 10 straddles the edge: partial alpha, but pure red
        // (premultiplied averaging: the transparent green never bleeds in).
        let mid = &icons[1].data[10 * 4..11 * 4];
        assert!(mid[0] > 0 && mid[0] < 255, "{mid:?}");
        assert_eq!(&mid[1..], &[255, 0, 0]);
    }

    #[test]
    fn small_and_wide_icons_and_garbage() {
        let small = pixmaps(&png(8, 8, |_, _| [1, 2, 3, 4])).unwrap();
        assert_eq!(small.len(), 1);
        assert_eq!(
            (small[0].width, &small[0].data[..4]),
            (8, &[4, 1, 2, 3][..])
        );
        let wide = pixmaps(&png(40, 20, |_, _| [9, 9, 9, 255])).unwrap();
        assert_eq!(
            wide.iter().map(|i| (i.width, i.height)).collect::<Vec<_>>(),
            [(16, 8), (22, 11), (24, 12), (32, 16)]
        );
        assert!(pixmaps(b"not a png").is_none());
    }

    #[test]
    fn set_icon_rejects_garbage_without_a_bus() {
        // A tray whose thread never connects: updates are only queued.
        let (tx, _rx) = mpsc::unbounded();
        let t = Tray {
            shared: Arc::new(Shared {
                serial: 7,
                status: Mutex::new((Status::NoWatcher, None)),
                changed: Condvar::new(),
                subscribers: Mutex::default(),
            }),
            cmds: tx,
        };
        assert!(!t.set_icon_png(b"nope"));
        assert!(t.set_icon_png(&png(16, 16, |_, _| [0, 0, 0, 255])));
        assert!(!t.is_shown());
        assert_eq!(t.wait_started(Duration::ZERO), Status::NoWatcher);
        assert_eq!(t.clone(), t);
    }

    // ---- The protocol, on a private bus ------------------------------------

    /// Set by [`sni_protocol_on_a_private_bus`] for its child: the private
    /// bus's address. The child test runs only when the session bus is it.
    const ENV_TEST_BUS: &str = "BATO_TRAY_TEST_BUS";

    /// Starts a private `dbus-daemon` (no service activation, nothing else on
    /// it) and runs [`sni_protocol_child`] in a child process whose
    /// `DBUS_SESSION_BUS_ADDRESS` is that bus. This process's environment is
    /// never changed, and nothing here connects to any other bus.
    #[test]
    fn sni_protocol_on_a_private_bus() {
        let dir = std::env::temp_dir().join(format!("bato-shell-tray-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("bus");
        let conf = dir.join("bus.conf");
        std::fs::write(
            &conf,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>"#,
                sock.display()
            ),
        )
        .unwrap();
        let daemon = std::process::Command::new("dbus-daemon")
            .arg(format!("--config-file={}", conf.display()))
            .arg("--nofork")
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .stdout(std::process::Stdio::null())
            .spawn();
        let Ok(mut daemon) = daemon else {
            eprintln!("no dbus-daemon: SNI protocol test skipped");
            return;
        };
        for _ in 0..200 {
            if sock.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let addr = format!("unix:path={}", sock.display());
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tray::tests::sni_protocol_child", "--nocapture"])
            .env("DBUS_SESSION_BUS_ADDRESS", &addr)
            .env(ENV_TEST_BUS, &addr)
            // No fallback to $XDG_RUNTIME_DIR/bus, ever.
            .env("XDG_RUNTIME_DIR", &dir)
            .output()
            .unwrap();
        let _ = daemon.kill();
        let _ = daemon.wait();
        // And with no bus at all: Failed, not a hang or a panic.
        let dead = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tray::tests::no_bus_child", "--nocapture"])
            .env("DBUS_SESSION_BUS_ADDRESS", &addr)
            .env(ENV_TEST_BUS, &addr)
            .env("XDG_RUNTIME_DIR", &dir)
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let dead_text = String::from_utf8_lossy(&dead.stdout).to_string()
            + &String::from_utf8_lossy(&dead.stderr);
        assert!(dead.status.success(), "{dead_text}");
        assert!(dead_text.contains("no bus: ok"), "{dead_text}");
        let text = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{text}");
        assert!(text.contains("1 passed"), "child did not run: {text}");
        assert!(text.contains("sni protocol: ok"), "{text}");
    }

    /// Runs only as the second child of [`sni_protocol_on_a_private_bus`],
    /// after its bus is gone.
    #[test]
    fn no_bus_child() {
        let bus = std::env::var(ENV_TEST_BUS).unwrap_or_default();
        let session = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
        if bus.is_empty() || bus != session || session.contains("/run/user/") {
            return;
        }
        let tray = Tray::start(Spec::for_app(&APP));
        assert_eq!(tray.wait_started(Duration::from_secs(10)), Status::Failed);
        assert!(!tray.is_shown());
        assert!(tray.error().is_some());
        tray.set_tooltip("ignored"); // no thread left: a no-op
        println!("no bus: ok");
    }

    /// SNI's `a(iiay)` and `(sa(iiay)ss)`; dbusmenu's `(ia{sv}av)`.
    type Pixmaps = Vec<(i32, i32, Vec<u8>)>;
    type ToolTipValue = (String, Pixmaps, String, String);
    type MenuNode = (
        i32,
        std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
        Vec<zbus::zvariant::OwnedValue>,
    );

    /// A StatusNotifierWatcher that records registrations.
    struct Watcher {
        items: Arc<Mutex<Vec<String>>>,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl Watcher {
        fn register_status_notifier_item(&self, service: String) {
            self.items.lock().unwrap().push(service);
        }
        fn register_status_notifier_host(&self, _service: String) {}
        #[zbus(property)]
        fn registered_status_notifier_items(&self) -> Vec<String> {
            self.items.lock().unwrap().clone()
        }
        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn protocol_version(&self) -> i32 {
            0
        }
    }

    async fn watcher(items: Arc<Mutex<Vec<String>>>) -> zbus::Connection {
        zbus::connection::Builder::session()
            .unwrap()
            .name("org.kde.StatusNotifierWatcher")
            .unwrap()
            .serve_at("/StatusNotifierWatcher", Watcher { items })
            .unwrap()
            .build()
            .await
            .unwrap()
    }

    fn next_event(rx: &mut mpsc::UnboundedReceiver<Event>, want: &Event) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            match rx.try_recv() {
                Ok(e) if &e == want => return,
                Ok(_) => {}
                Err(mpsc::TryRecvError::Closed) => panic!("event stream ended"),
                Err(mpsc::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        panic!("no {want:?}");
    }

    fn wait_for(what: &str, f: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !f() {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The SNI and dbusmenu sides as a host (Waybar) sees them. Runs only as
    /// the child of [`sni_protocol_on_a_private_bus`].
    #[test]
    fn sni_protocol_child() {
        let bus = std::env::var(ENV_TEST_BUS).unwrap_or_default();
        let session = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
        if bus.is_empty() || bus != session || session.contains("/run/user/") {
            return; // not on the private bus: never touch a real one
        }
        use zbus::zvariant::Value;
        zbus::block_on(async {
            let host = zbus::Connection::session().await.unwrap();
            let items: Arc<Mutex<Vec<String>>> = Arc::default();

            // 1. No watcher yet: NoWatcher, not an error, and still running.
            let icon = png(32, 32, |_, _| [10, 20, 30, 255]);
            let tray = Tray::start(Spec::for_app(
                &APP.with_icon(Box::leak(icon.into_boxed_slice())),
            ));
            let mut ev = tray.events();
            assert_eq!(
                tray.wait_started(Duration::from_secs(10)),
                Status::NoWatcher
            );
            assert!(!tray.is_shown());

            // 2. A watcher appears (Waybar starts): the item registers.
            let w = watcher(items.clone()).await;
            next_event(&mut ev, &Event::Status(Status::Shown));
            wait_for("registration", || !items.lock().unwrap().is_empty());
            let name = items.lock().unwrap()[0].clone();
            assert!(name.starts_with("org.kde.StatusNotifierItem-"), "{name}");

            // 3. What the host reads.
            // Uncached, like a host: SNI announces changes with its own
            // signals (NewToolTip, NewIcon), not PropertiesChanged.
            let sni: zbus::Proxy = zbus::proxy::Builder::new(&host)
                .destination(name.as_str())
                .unwrap()
                .path("/StatusNotifierItem")
                .unwrap()
                .interface("org.kde.StatusNotifierItem")
                .unwrap()
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()
                .await
                .unwrap();
            let id: String = sni.get_property("Id").await.unwrap();
            let title: String = sni.get_property("Title").await.unwrap();
            assert_eq!((id.as_str(), title.as_str()), ("batosound", "Bato_Sound"));
            let tip: ToolTipValue = sni.get_property("ToolTip").await.unwrap();
            assert_eq!(tip.2, "Bato_Sound");
            let pix: Pixmaps = sni.get_property("IconPixmap").await.unwrap();
            assert_eq!(
                pix.iter().map(|p| p.0).collect::<Vec<_>>(),
                [16, 22, 24, 32]
            );
            assert_eq!(&pix[0].2[..4], &[255, 10, 20, 30]);
            let menu: zbus::zvariant::OwnedObjectPath = sni.get_property("Menu").await.unwrap();

            // 4. Clicks.
            let _: () = sni.call("Activate", &(0i32, 0i32)).await.unwrap();
            next_event(&mut ev, &Event::Activate);
            let _: () = sni.call("SecondaryActivate", &(0i32, 0i32)).await.unwrap();
            next_event(&mut ev, &Event::SecondaryActivate);

            // 5. The menu: labels (mnemonics escaped), a click by id.
            let dm = zbus::Proxy::new(
                &host,
                name.as_str(),
                menu.as_str(),
                "com.canonical.dbusmenu",
            )
            .await
            .unwrap();
            let layout = || async {
                let (_rev, (_id, _props, children)): (u32, MenuNode) = dm
                    .call("GetLayout", &(0i32, -1i32, Vec::<String>::new()))
                    .await
                    .unwrap();
                children
                    .into_iter()
                    .map(|c| {
                        let (id, props, _): MenuNode = c.try_into().unwrap();
                        let label = props
                            .get("label")
                            .map(|v| String::try_from(v.clone()).unwrap())
                            .unwrap_or_default();
                        let sep = props
                            .get("type")
                            .is_some_and(|v| String::try_from(v.clone()).unwrap() == "separator");
                        (id, if sep { "---".to_string() } else { label })
                    })
                    .collect::<Vec<_>>()
            };
            let rows = layout().await;
            assert_eq!(
                rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(),
                ["Show Bato__Sound", "---", "Quit"]
            );
            let quit = rows[2].0;
            let _: () = dm
                .call("Event", &(quit, "clicked", Value::from(0i32), 0u32))
                .await
                .unwrap();
            next_event(&mut ev, &Event::Menu(QUIT.into()));

            // 6. Updates reach the host.
            tray.set_tooltip("BatoSound: 2 rules");
            tray.set_menu(vec![MenuItem::item("x", "Only row").disabled()]);
            wait_for("tooltip update", || {
                zbus::block_on(async {
                    let t: ToolTipValue = sni.get_property("ToolTip").await.unwrap();
                    t.2 == "BatoSound: 2 rules"
                })
            });
            let rows = layout().await;
            assert_eq!(
                rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(),
                ["Only row"]
            );

            // 7. The watcher goes away (Waybar restarts) and comes back.
            items.lock().unwrap().clear();
            w.close().await.unwrap();
            next_event(&mut ev, &Event::Status(Status::NoWatcher));
            assert!(!tray.is_shown());
            let _w = watcher(items.clone()).await;
            next_event(&mut ev, &Event::Status(Status::Shown));
            wait_for("re-registration", || {
                items.lock().unwrap().as_slice() == [name.clone()]
            });

            // 8. Shutdown takes the item off the bus.
            tray.shutdown();
            let dbus = zbus::fdo::DBusProxy::new(&host).await.unwrap();
            let bus_name = zbus::names::BusName::try_from(name.as_str()).unwrap();
            wait_for("item gone", || {
                !zbus::block_on(dbus.name_has_owner(bus_name.clone())).unwrap()
            });
            println!("sni protocol: ok");
        });
    }
}
