//! iced 0.14 glue (feature `iced`): window settings, `Task`s and
//! `Subscription`s over the toolkit-neutral API.
//!
//! ```ignore
//! fn subscription(&self) -> Subscription<Msg> {
//!     Subscription::batch([
//!         iced_ext::update_status(&self.updater, self.auto_update).map(Msg::Update),
//!         iced_ext::notification_clicks(&self.notifier).map(|_| Msg::FocusWindow),
//!     ])
//! }
//! // Msg::FocusWindow => iced::window::latest().and_then(iced::window::gain_focus)
//! ```

use std::path::PathBuf;

use futures::channel::mpsc;
use futures::{Stream, StreamExt};
use iced::{Size, Subscription, Task, window};

use crate::identity::{AppIdentity, WINDOW};
use crate::updater::{CHECK_INTERVAL, UpdatePhase, Updater};
use crate::worker::spawn_blocking;

/// `window::Settings` for the app: 1280×800, min 900×600, compositor
/// decorations, app_id = slug (or `BATO_APP_ID`), X11 icon from the PNG.
pub fn window_settings(app: &AppIdentity) -> window::Settings {
    window::Settings {
        size: Size::new(WINDOW.width, WINDOW.height),
        min_size: Some(Size::new(WINDOW.min_width, WINDOW.min_height)),
        decorations: WINDOW.decorations,
        icon: app.icon_png.and_then(icon_from_png),
        platform_specific: window::settings::PlatformSpecific {
            application_id: app.app_id(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Decode a PNG into a window icon (RGBA8). `None` if it is not a PNG iced
/// can take; the window simply has no X11 icon then.
pub fn icon_from_png(bytes: &[u8]) -> Option<window::Icon> {
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
    window::icon::from_rgba(rgba, info.width, info.height).ok()
}

/// Run blocking `f` on a worker thread; its result becomes a message.
pub fn blocking<T, F>(f: F) -> Task<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    Task::future(spawn_blocking(f))
}

/// A stream fed by a blocking producer on its own thread. The producer stops
/// when `send` returns false (the stream was dropped).
fn thread_stream<T: Send + 'static>(
    produce: impl FnOnce(&dyn Fn(T) -> bool) + Send + 'static,
) -> impl Stream<Item = T> {
    let (tx, rx) = mpsc::unbounded();
    let _ = std::thread::Builder::new()
        .name("bato-shell-stream".into())
        .spawn(move || {
            produce(&|v| tx.unbounded_send(v).is_ok());
        });
    rx
}

/// Update status, Electron's `update:status`: check on start and every 30
/// minutes; with `auto_apply`, install as soon as one is available (Electron's
/// `autoDownload`); quiet after `Downloaded`. Emits nothing for a dev build.
pub fn update_status(updater: &Updater, auto_apply: bool) -> Subscription<UpdatePhase> {
    Subscription::run_with((updater.clone(), auto_apply), |(u, auto)| {
        let (u, auto) = (u.clone(), *auto);
        thread_stream(move |send| u.run(auto, CHECK_INTERVAL, send))
    })
}

/// One check now (the About dialog's "Check for updates").
pub fn check_update(updater: &Updater) -> Task<UpdatePhase> {
    let u = updater.clone();
    Task::done(UpdatePhase::Checking).chain(blocking(move || u.check()))
}

/// Install the latest release with the bato CLI, streaming `Downloading`,
/// then `Downloaded` or `Error` (which names the command to run when there
/// is no CLI).
pub fn apply_update(updater: &Updater) -> Task<UpdatePhase> {
    let u = updater.clone();
    Task::run(
        thread_stream(move |send| {
            let _ = u.apply(|p| {
                send(p);
            });
        }),
        |p| p,
    )
}

/// "Restart to update": start the installed binary, then exit this process
/// (iced drops the app state, so a bundled server's `Supervisor` stops).
/// On failure the message carries the error and nothing exits.
pub fn restart_to_update<T: Send + 'static>(
    updater: &Updater,
    on_error: impl FnOnce(String) -> T + Send + 'static,
) -> Task<T> {
    match updater.relaunch() {
        Ok(()) => iced::exit(),
        Err(e) => Task::done(on_error(e.to_string())),
    }
}

/// Clicks on this app's notifications. Focus the window in response
/// (`window::gain_focus`); `Click::activation_token` is there for a future
/// winit that can activate with a given token.
#[cfg(feature = "notify")]
pub fn notification_clicks(
    notifier: &crate::notify::Notifier,
) -> Subscription<crate::notify::Click> {
    Subscription::run_with(notifier.clone(), |n| {
        let n = n.clone();
        futures::stream::once(async move { n.clicks().await })
            .flat_map(|s| s.unwrap_or_else(|| futures::stream::empty().boxed()))
    })
}

/// Show a notification; the message gets whether a server took it.
#[cfg(feature = "notify")]
pub fn notify(notifier: &crate::notify::Notifier, n: crate::notify::Notification) -> Task<bool> {
    let notifier = notifier.clone();
    Task::future(async move { notifier.notify(&n).await.is_some() })
}

/// The bundled server's events (health, exits, restarts), once per
/// supervisor.
#[cfg(any(test, feature = "supervisor"))]
pub fn supervisor_events(
    sup: &mut crate::supervisor::Supervisor,
) -> Task<crate::supervisor::Event> {
    match sup.take_events() {
        Some(rx) => Task::run(rx, |e| e),
        None => Task::none(),
    }
}

/// The tray's clicks, menu rows and status changes ([`crate::tray::Event`]).
/// Keyed on the tray instance: keep returning it while the tray lives.
#[cfg(any(test, feature = "tray"))]
pub fn tray_events(tray: &crate::tray::Tray) -> Subscription<crate::tray::Event> {
    Subscription::run_with(tray.clone(), |t| t.events())
}

/// The main window of an `iced::daemon` app, for closing to the tray.
///
/// Wayland cannot hide a window, so "hide" closes it and "show" opens a new
/// one; the app's state lives in the daemon, not the window, so the page,
/// data and settings come back as they were (widget-local state such as a
/// scroll offset or a caret does not). With `iced::daemon` closing the last
/// window does not exit: the app decides on `window::close_events()`.
///
/// ```ignore
/// // boot:   let mut main = MainWindow::new(iced_ext::window_settings(&APP));
/// //         (App { main, tray: Some(Tray::start(Spec::for_app(&APP))), .. }, main.show())
/// // subscription: window::close_events().map(Msg::Closed),
/// //               iced_ext::tray_events(tray).map(Msg::Tray)
/// Msg::Closed(id) if self.main.closed(id) => {
///     // Nobody sees a tray icon (no bar, Waybar without `tray`): quit as before.
///     if !self.tray.as_ref().is_some_and(Tray::is_shown) { return self.quit(); }
/// }
/// Msg::Tray(tray::Event::Activate) => return self.main.toggle(),
/// Msg::Tray(tray::Event::Menu(id)) if id == tray::SHOW => return self.main.show(),
/// Msg::Tray(tray::Event::Menu(id)) if id == tray::QUIT => return self.quit(),
/// // view(&self, id): the frame for `self.main.id()`, nothing for any other id.
/// ```
#[derive(Debug, Clone)]
pub struct MainWindow {
    settings: window::Settings,
    id: Option<window::Id>,
}

impl MainWindow {
    /// Not open yet: [`show`](Self::show) opens it (the daemon's boot task).
    pub fn new(settings: window::Settings) -> Self {
        Self { settings, id: None }
    }

    pub fn id(&self) -> Option<window::Id> {
        self.id
    }

    pub fn is_open(&self) -> bool {
        self.id.is_some()
    }

    /// Open it, or raise it when it is already open (the tray's "Show", a
    /// second launch).
    pub fn show<T: Send + 'static>(&mut self) -> Task<T> {
        match self.id {
            #[cfg(feature = "activation")]
            Some(id) => activate(id, None).discard(),
            #[cfg(not(feature = "activation"))]
            Some(id) => window::gain_focus(id),
            None => {
                let (id, open) = window::open(self.settings.clone());
                self.id = Some(id);
                open.discard()
            }
        }
    }

    /// Close it; the daemon keeps running.
    pub fn hide<T: Send + 'static>(&mut self) -> Task<T> {
        match self.id.take() {
            Some(id) => window::close(id),
            None => Task::none(),
        }
    }

    /// Electron's tray click: hide when open, show otherwise.
    pub fn toggle<T: Send + 'static>(&mut self) -> Task<T> {
        if self.is_open() {
            self.hide()
        } else {
            self.show()
        }
    }

    /// For each `window::close_events()` id: true when the user closed this
    /// window (the decision point: stay in the tray or quit). False for
    /// other windows and after [`hide`](Self::hide), which already forgot it.
    pub fn closed(&mut self, id: window::Id) -> bool {
        if self.id == Some(id) {
            self.id = None;
            true
        } else {
            false
        }
    }
}

/// The dialog parent for a window: its X11 id, or on Wayland an xdg-foreign
/// export of its toplevel ([`crate::dialogs::Parent::from_window`]). The
/// task yields nothing if the window closed meanwhile.
#[cfg(feature = "dialogs")]
pub fn dialog_parent(id: window::Id) -> Task<crate::dialogs::Parent> {
    window::run(id, |w| crate::dialogs::Parent::from_window(w))
}

/// [`dialog_parent`] of the most recently opened window (the app window),
/// `Parent::None` without one.
#[cfg(feature = "dialogs")]
pub fn latest_dialog_parent() -> Task<crate::dialogs::Parent> {
    window::latest().then(|id| match id {
        Some(id) => dialog_parent(id),
        None => Task::done(crate::dialogs::Parent::None),
    })
}

/// `parent`, or the app window's when it is `None`, then `open`.
#[cfg(feature = "dialogs")]
fn parented<O, T>(
    o: O,
    parent: impl Fn(&mut O) -> &mut crate::dialogs::Parent + Send + 'static,
    open: fn(O) -> Task<T>,
) -> Task<T>
where
    O: Clone + Send + 'static,
    T: Send + 'static,
{
    let mut o = o;
    if *parent(&mut o) != crate::dialogs::Parent::None {
        return open(o);
    }
    latest_dialog_parent().then(move |p| {
        let mut o = o.clone();
        *parent(&mut o) = p;
        open(o)
    })
}

/// [`crate::dialogs::open_files`], modal to the app window unless
/// `o.parent` names another one.
#[cfg(feature = "dialogs")]
pub fn open_files(
    o: crate::dialogs::OpenFiles,
) -> Task<Result<Vec<PathBuf>, crate::dialogs::DialogError>> {
    parented(
        o,
        |o| &mut o.parent,
        |o| Task::future(crate::dialogs::open_files(o)),
    )
}

/// [`crate::dialogs::open_directory`], modal to the app window unless
/// `o.parent` names another one.
#[cfg(feature = "dialogs")]
pub fn open_directory(
    o: crate::dialogs::OpenDirectory,
) -> Task<Result<Option<PathBuf>, crate::dialogs::DialogError>> {
    parented(
        o,
        |o| &mut o.parent,
        |o| Task::future(crate::dialogs::open_directory(o)),
    )
}

/// [`crate::dialogs::save_file`], modal to the app window unless `o.parent`
/// names another one.
#[cfg(feature = "dialogs")]
pub fn save_file(
    o: crate::dialogs::SaveFile,
) -> Task<Result<Option<PathBuf>, crate::dialogs::DialogError>> {
    parented(
        o,
        |o| &mut o.parent,
        |o| Task::future(crate::dialogs::save_file(o)),
    )
}

/// What [`with_progress`] and its relatives yield: progress, then the
/// result once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progressed<P, R> {
    Progress(P),
    Done(R),
}

/// Run blocking `work` on a worker thread; each progress it reports becomes
/// a message, then its result.
///
/// ```ignore
/// Msg::Copy(job) => iced_ext::with_progress(move |report| {
///     http::relay(src, dst, &job.cancel, |p| report(p))
/// })
/// .map(move |m| Msg::Transfer(job.id, m)),
/// ```
pub fn with_progress<P, R, F>(work: F) -> Task<Progressed<P, R>>
where
    P: Send + 'static,
    R: Send + 'static,
    F: FnOnce(&mut dyn FnMut(P)) -> R + Send + 'static,
{
    Task::run(
        thread_stream(move |send| {
            let r = work(&mut |p| {
                send(Progressed::Progress(p));
            });
            send(Progressed::Done(r));
        }),
        |m| m,
    )
}

/// [`with_progress`] as a subscription keyed by `key`: it runs while the
/// app keeps returning it (a transfer list: one per job id), and starts
/// over if it is dropped and returned again. Pair it with an
/// [`http::Cancel`](crate::http::Cancel) in `key`'s job to stop the work.
pub fn progress_subscription<K, P, R>(
    key: K,
    work: fn(&K, &mut dyn FnMut(P)) -> R,
) -> Subscription<Progressed<P, R>>
where
    K: std::hash::Hash + Clone + Send + 'static,
    P: Send + 'static,
    R: Send + 'static,
{
    Subscription::run_with((key, work), |(k, w)| {
        let (k, w) = (k.clone(), *w);
        thread_stream(move |send| {
            let r = w(&k, &mut |p| {
                send(Progressed::Progress(p));
            });
            send(Progressed::Done(r));
        })
    })
}

/// [`crate::http::relay`] on a worker: progress (≤ 10 per second), then
/// the outcome. `cancel` stops it.
pub fn relay(
    source: crate::http::Call,
    dest: crate::http::Call,
    cancel: crate::http::Cancel,
) -> Task<Progressed<crate::http::Progress, Result<crate::http::Relayed, crate::http::RelayError>>>
{
    with_progress(move |report| crate::http::relay(source, dest, &cancel, report))
}

/// One item of [`sse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseItem {
    Event(crate::http::SseEvent),
    /// The request failed, or the stream broke; nothing follows.
    Error(String),
    /// The server closed the stream.
    End,
}

/// A Server-Sent Events request, each event a message as it arrives.
/// Stop early with `Task::abortable` (the worker ends at the next event or
/// the stall timeout).
pub fn sse(call: crate::http::Call) -> Task<SseItem> {
    Task::run(
        thread_stream(move |send| {
            let stream = match call.stream().and_then(crate::http::Streamed::ok_or_status) {
                Ok(s) => s,
                Err(e) => {
                    send(SseItem::Error(e.to_string()));
                    return;
                }
            };
            for ev in stream.sse() {
                let alive = match ev {
                    Ok(e) => send(SseItem::Event(e)),
                    Err(e) => {
                        send(SseItem::Error(e.to_string()));
                        return;
                    }
                };
                if !alive {
                    return;
                }
            }
            send(SseItem::End);
        }),
        |m| m,
    )
}

/// Launches forwarded by later instances ([`crate::instance`]). The
/// requests can be taken once: keep returning this subscription for the
/// app's lifetime.
pub fn instance_requests(
    primary: &crate::instance::Primary,
) -> Subscription<crate::instance::Request> {
    Subscription::run_with(primary.clone(), |p| match p.take_requests() {
        Some(rx) => rx.boxed(),
        None => futures::stream::empty().boxed(),
    })
}

/// Raise window `id`: xdg-activation with `token` on Wayland (a token of
/// its own when `None`), `window::gain_focus` elsewhere. Yields whether the
/// xdg-activation request went out. See [`crate::activation`] for what
/// compositors do with it.
#[cfg(feature = "activation")]
pub fn activate(id: window::Id, token: Option<String>) -> Task<bool> {
    window::run(id, move |w| {
        crate::activation::activate(w, token.as_deref())
    })
    .then(move |sent| {
        if sent {
            Task::done(true)
        } else {
            window::gain_focus(id).chain(Task::done(false))
        }
    })
}

/// A fresh activation token for window `id`, for a process it is about to
/// start ([`crate::activation::request_token`]); `None` off Wayland.
#[cfg(feature = "activation")]
pub fn activation_token(id: window::Id) -> Task<Option<String>> {
    window::run(id, |w| crate::activation::request_token(w))
}

/// [`crate::clipboard::copy_files`] on a worker.
#[cfg(feature = "clipboard")]
pub fn copy_files(
    paths: Vec<PathBuf>,
    op: crate::clipboard::Op,
) -> Task<Result<crate::clipboard::Backend, crate::clipboard::ClipboardError>> {
    blocking(move || crate::clipboard::copy_files(&paths, op))
}

/// [`crate::clipboard::paste_files`] on a worker.
#[cfg(feature = "clipboard")]
pub fn paste_files()
-> Task<Result<Option<crate::clipboard::Files>, crate::clipboard::ClipboardError>> {
    blocking(crate::clipboard::paste_files)
}

/// Frame events while `BATO_SMOKE` is on, for [`crate::hooks::Smoke`].
pub fn smoke_frames() -> Subscription<std::time::Instant> {
    if crate::hooks::smoke_enabled() {
        window::frames()
    } else {
        Subscription::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2×1 RGB PNG (red, green).
    const PNG_2X1_RGB: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x7b,
        0x40, 0xe8, 0xdd, 0x00, 0x00, 0x00, 0x0f, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xcf, 0xc0, 0xc0, 0xf0, 0x9f, 0x01, 0x00, 0x07, 0xff, 0x01, 0xff, 0x01, 0x7f, 0x89, 0xa7,
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    /// Every output of a task, run to the end.
    fn outputs<T: Send + 'static>(task: Task<T>) -> Vec<T> {
        use iced_runtime::Action;
        let Some(stream) = iced_runtime::task::into_stream(task) else {
            return Vec::new();
        };
        futures::executor::block_on(
            stream
                .filter_map(|a| async move {
                    match a {
                        Action::Output(t) => Some(t),
                        _ => None,
                    }
                })
                .collect(),
        )
    }

    /// The first action of a task: which window action it is.
    fn window_action<T: Send + 'static>(task: Task<T>) -> &'static str {
        use iced_runtime::{Action, window::Action as W};
        let Some(stream) = iced_runtime::task::into_stream(task) else {
            return "none";
        };
        let first = futures::executor::block_on(stream.take(1).collect::<Vec<_>>());
        match first.first() {
            Some(Action::Window(W::Open(..))) => "open",
            Some(Action::Window(W::Close(_))) => "close",
            Some(Action::Window(W::Run(..))) => "run",
            Some(Action::Window(W::GainFocus(_))) => "focus",
            Some(_) => "other",
            None => "none",
        }
    }

    #[test]
    fn main_window_shows_hides_and_toggles() {
        let mut main = MainWindow::new(window::Settings::default());
        assert!(!main.is_open());
        assert_eq!(window_action(main.hide::<()>()), "none");
        assert_eq!(window_action(main.show::<()>()), "open");
        let id = main.id().unwrap();
        // Open: raising it (xdg-activation runs on the window) opens nothing.
        assert_ne!(window_action(main.show::<()>()), "open");
        assert_eq!(main.id(), Some(id));
        // Another window's close is not ours.
        assert!(!main.closed(window::Id::unique()));
        assert_eq!(window_action(main.toggle::<()>()), "close");
        assert!(!main.is_open());
        // A hide already forgot it: its close event is no decision.
        assert!(!main.closed(id));
        assert_eq!(window_action(main.toggle::<()>()), "open");
        let id2 = main.id().unwrap();
        assert_ne!(id2, id);
        // The user closed it.
        assert!(main.closed(id2));
        assert!(!main.is_open());
    }

    #[test]
    fn progress_then_result() {
        let got = outputs(with_progress(|report| {
            for i in 0..3 {
                report(i);
            }
            "done"
        }));
        assert_eq!(
            got,
            [
                Progressed::Progress(0),
                Progressed::Progress(1),
                Progressed::Progress(2),
                Progressed::Done("done")
            ]
        );
    }

    #[test]
    fn relay_task_reports_and_finishes() {
        use crate::http::test_server::{Reply, serve};
        use crate::http::{Cancel, Http, Method, Progress};
        let data = vec![7u8; 300_000];
        let (src, _) = serve(vec![Reply::new(200, &data)]);
        let (dst, rx) = serve(vec![Reply::new(200, r#"{"ok":true}"#)]);
        let got = outputs(relay(
            Http::new(&src).call(Method::Get, "/a"),
            Http::new(&dst).call(Method::Put, "/b"),
            Cancel::new(),
        ));
        assert_eq!(
            got.first(),
            Some(&Progressed::Progress(Progress {
                done: 0,
                total: Some(300_000)
            }))
        );
        match got.last() {
            Some(Progressed::Done(Ok(r))) => assert_eq!(r.bytes, 300_000),
            other => panic!("{other:?}"),
        }
        assert_eq!(rx.recv().unwrap().body.len(), 300_000);
    }

    #[test]
    fn sse_task_streams_events_then_end() {
        use crate::http::test_server::{Reply, serve};
        use crate::http::{Http, Method};
        let (base, _) = serve(vec![Reply::new(
            200,
            "data: {\"path\":\"/a\"}\n\nevent: done\ndata: {\"count\":1}\n\n",
        )]);
        let got = outputs(sse(Http::new(&base).call(Method::Get, "/api/fs/search")));
        assert_eq!(got.len(), 3, "{got:?}");
        assert!(matches!(&got[0], SseItem::Event(e) if e.data == r#"{"path":"/a"}"#));
        assert!(matches!(&got[1], SseItem::Event(e) if e.event == "done"));
        assert_eq!(got[2], SseItem::End);
        let (base, _) = serve(vec![Reply::new(500, r#"{"error":"boom"}"#)]);
        let got = outputs(sse(Http::new(&base).call(Method::Get, "/x")));
        assert_eq!(got, [SseItem::Error("boom".into())]);
    }

    #[test]
    fn window_settings_follow_the_identity() {
        let app = AppIdentity::new("Bato Notes", "batonotes", "1.0.0").with_icon(PNG_2X1_RGB);
        let s = window_settings(&app);
        assert_eq!(s.size, Size::new(1280.0, 800.0));
        assert_eq!(s.min_size, Some(Size::new(900.0, 600.0)));
        assert!(s.decorations);
        assert_eq!(s.platform_specific.application_id, app.app_id());
        assert!(s.icon.is_some(), "icon decodes");
        assert!(icon_from_png(b"not a png").is_none());
    }
}
