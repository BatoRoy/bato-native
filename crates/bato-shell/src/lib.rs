//! Platform services for native Bato apps: what the Electron shell
//! (`main.js`, `preload.js`, `updater.js`, `backend.js`) does for the web
//! flavor, without the UI.
//!
//! | Module | Electron original |
//! |---|---|
//! | [`identity`] | `identity.js`, the `BrowserWindow` options |
//! | [`settings`] | `settings:get` / `settings:save` (the same `settings.json`) |
//! | [`http`] | `net:request` (now a real client: TLS, Bearer, JSON, per-call timeouts, headers, byte and streamed bodies, SSE, a GET→PUT relay with progress and cancel, the bundled server's session secret) |
//! | [`backend`] | `bridge.ts` backend URL resolution |
//! | `supervisor` (feature `supervisor`) | `backend.js` |
//! | `dialogs` (feature `dialogs`) | `dialog:openFiles` / `openDirectory` / `saveFile` |
//! | `notify` (feature `notify`) | `notify` + click-to-focus |
//! | [`updater`] | `updater.js` (through `bato install`, not electron-updater) |
//! | [`hooks`] | `BATO_VIEW`, `BATO_SMOKE`, `BATO_TIMING` |
//! | [`instance`] | `requestSingleInstanceLock` + `second-instance` (argv, cwd, activation token) |
//! | `desktop` (feature `desktop`) | `shell.showItemInFolder`, `shell.openPath`, detached spawns |
//! | `clipboard` (feature `clipboard`) | copy/cut/paste files (`text/uri-list`, `x-special/gnome-copied-files`) |
//! | `activation` (feature `activation`) | raise a window with an xdg-activation token |
//! | `tray` (feature `tray`) | `Tray` (a StatusNotifierItem), close-to-tray |
//! | `iced_ext` (feature `iced`) | `Task`/`Subscription` wrappers for all of the above |
//!
//! The core API is toolkit-neutral: blocking functions meant for worker
//! threads ([`worker::spawn_blocking`] turns one into a future), async
//! functions that run on any executor (zbus and ashpd use `async-io`, so they
//! need no tokio), and plain threads with `futures` channels.
//!
//! How an app wires it up (iced; see each module for the details):
//!
//! ```ignore
//! use bato_shell::{AppIdentity, SettingsExt, backend, hooks, http::Http, iced_ext, notify, updater};
//!
//! pub const APP: AppIdentity = AppIdentity::new("Bato Notes", "batonotes", env!("APP_VERSION"))
//!     .with_icon(include_bytes!("../assets/appicon.png"));
//!
//! fn main() -> iced::Result {
//!     hooks::mark("main");
//!     let view = hooks::BootView::from_env();               // BATO_VIEW
//!     let settings = APP.load_settings();                    // ~/.config/batonotes/settings.json
//!     // Optional (feature "supervisor"): a bundled Go server.
//!     // let server = supervisor::Supervisor::start_bundled(&APP, &settings, Default::default());
//!     let api = Http::new(&backend::resolve_for(&settings, None).url);
//!     let updater = updater::Updater::new(&APP, updater::DEFAULT_REGISTRY);
//!     let notifier = notify::Notifier::new(&APP);
//!     iced::application(/* boot with view, settings, api, updater, notifier */)
//!         .title(APP.product_name)
//!         .window(iced_ext::window_settings(&APP))
//!         .run()
//! }
//! // update():  Msg::Frame(_) if self.smoke.on_frame() => iced::exit(),
//! //            Msg::Load => iced_ext::blocking(move || api.get::<Vec<Note>>("/api/notes")).map(Msg::Loaded),
//! //            Msg::Pick => iced_ext::open_files(Default::default()).map(Msg::Picked),
//! //            Msg::Notify => iced_ext::notify(&self.notifier, notify::Notification::new("Saved", "")).map(Msg::Shown),
//! //            Msg::Clicked(_) => iced::window::latest().and_then(iced::window::gain_focus),
//! //            Msg::RestartToUpdate => iced_ext::restart_to_update(&self.updater, Msg::Error),
//! // subscription(): iced_ext::update_status(&self.updater, true).map(Msg::Update),
//! //                 iced_ext::notification_clicks(&self.notifier).map(Msg::Clicked),
//! //                 iced_ext::smoke_frames().map(Msg::Frame)
//! ```

pub mod backend;
pub mod gpu;
pub mod hooks;
pub mod http;
pub mod identity;
pub mod instance;
pub mod settings;
pub mod updater;
pub mod worker;

#[cfg(feature = "activation")]
pub mod activation;
#[cfg(feature = "clipboard")]
pub mod clipboard;
#[cfg(feature = "desktop")]
pub mod desktop;
#[cfg(feature = "dialogs")]
pub mod dialogs;
#[cfg(feature = "notify")]
pub mod notify;
// Built for `cargo test` too, so the workspace gates (which use default
// features) lint and test it although apps opt in.
#[cfg(any(test, feature = "supervisor"))]
pub mod supervisor;
// Same: built and tested by `cargo test`, opted into by apps.
#[cfg(any(test, feature = "tray"))]
pub mod tray;

#[cfg(feature = "iced")]
pub mod iced_ext;

#[cfg(test)]
mod test_support;

pub use identity::{AppIdentity, Brand};
pub use settings::{Settings, SettingsExt};
