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

/// The dialog parent for a window: its X11 id on X11. On Wayland a parent
/// needs an xdg-foreign export, which is not wired up yet, so dialogs open
/// unparented there (still in front, just not attached).
#[cfg(feature = "dialogs")]
pub fn dialog_parent(id: window::Id) -> Task<crate::dialogs::Parent> {
    use raw_window_handle::RawWindowHandle;
    window::run(id, |w| match w.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Xlib(h)) => {
            // c_ulong: u64 here, u32 on 32-bit targets.
            #[allow(clippy::unnecessary_cast)]
            crate::dialogs::Parent::X11(h.window as u64)
        }
        Ok(RawWindowHandle::Xcb(h)) => crate::dialogs::Parent::X11(u64::from(h.window.get())),
        _ => crate::dialogs::Parent::None,
    })
}

#[cfg(feature = "dialogs")]
pub fn open_files(
    o: crate::dialogs::OpenFiles,
) -> Task<Result<Vec<PathBuf>, crate::dialogs::DialogError>> {
    Task::future(crate::dialogs::open_files(o))
}

#[cfg(feature = "dialogs")]
pub fn open_directory(
    o: crate::dialogs::OpenDirectory,
) -> Task<Result<Option<PathBuf>, crate::dialogs::DialogError>> {
    Task::future(crate::dialogs::open_directory(o))
}

#[cfg(feature = "dialogs")]
pub fn save_file(
    o: crate::dialogs::SaveFile,
) -> Task<Result<Option<PathBuf>, crate::dialogs::DialogError>> {
    Task::future(crate::dialogs::save_file(o))
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
