//! The app frame: the Hybrid shell every Bato app shares. Sidebar (brand,
//! search box, the app's nav rows, About / App Options / Collapse with the
//! 200 ms collapse animation and rail tips), the page scroller with BatoScroll,
//! the App Options and About dialogs over a blurred backdrop, and the toast
//! stack. Parameterized by a [`Brand`] and the app's own pages.
//!
//! The frame does no I/O. It owns the [`Settings`] the app hands it and
//! reports what needs the platform as an [`Action`] from [`Frame::update`]:
//! persist the settings, test a server address, check for updates.
//!
//! ```ignore
//! use bato_kit::frame::{self, Brand, Frame, NavItem};
//!
//! const BRAND: Brand = Brand { name: "App", tagline: "…", version: "0.4.1", slug: "app",
//!                              accent: "#8b5cf6", icon: Ico::Box };
//! #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//! enum Page { Home, Settings }
//!
//! #[derive(Debug, Clone)]
//! enum Msg { Frame(frame::Msg<Page>), /* the app's own */ }
//!
//! let mut frame = Frame::new(BRAND, vec![
//!     NavItem::new(Page::Home, Ico::Home, "Home"),
//!     NavItem::new(Page::Settings, Ico::Settings, "Settings"),
//! ], settings, Page::Home);
//!
//! // update
//! Msg::Frame(m) => match frame.update(m) {
//!     Action::SaveSettings => save(frame.settings()),
//!     Action::TestServer(url) => return Task::perform(health(url), |ok| Msg::Frame(frame::Msg::ServerTested(ok))),
//!     Action::ServerSaved => { save(frame.settings()); return ping(); }
//!     Action::CheckUpdates => return check_update(&updater).map(Msg::Update),
//!     Action::RestartToUpdate => return restart_to_update(&updater, Msg::RestartFailed),
//!     Action::None | Action::Navigated(_) => {}
//! }
//! // the updater's phases
//! Msg::Update(phase) => frame.set_update_status(match phase {
//!     UpdatePhase::Downloaded { version } => UpdateStatus::Downloaded { version },
//!     /* … one arm per phase */
//! }),
//! }
//!
//! // view
//! let cx = frame.ctx();
//! let page = match frame.page() { Page::Home => home(cx), Page::Settings => settings(cx) };
//! frame.view(page, Msg::Frame)
//! ```
//!
//! Wire `frame.subscription().map(Msg::Frame)`, `.scale_factor(|a| a.frame.scale_factor())`,
//! `.style(|a, _| a.frame.style())` and `.default_font(bato_kit::fonts::sans())`
//! into `iced::application`.

mod dialogs;
mod sidebar;
mod update;

pub use sidebar::MOD_K;
pub use update::UpdateStatus;

use std::time::{Duration, Instant};

use bato_theme::settings::{Settings, backend_url};
use bato_theme::{Rgb, ThemeId, palette};
use iced::widget::{row, stack};
use iced::{Length, Subscription, window};

use crate::El;
use crate::anim::{Tween, ease_standard};
use crate::autoscroll::{AutoscrollBus, autoscroll_layer, new_bus};
use crate::badge::Tone;
use crate::blur::backdrop_blur;
use crate::ctx::Ctx;
use crate::icon::Ico;
use crate::modal::modal;
use crate::scale::set_device_scale;
use crate::scroller::scroller;
use crate::style::c;
use crate::toast::{Toasts, toast_stack};

/// Who the app is. Shown in the sidebar header, App Options and About.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Brand {
    /// Display name (`App`).
    pub name: &'static str,
    /// One line under the name (Home, About).
    pub tagline: &'static str,
    /// Version without the `v`.
    pub version: &'static str,
    /// Config directory name (`~/.config/<slug>`).
    pub slug: &'static str,
    /// The brand accent (`#rrggbb`), the default and first swatch.
    pub accent: &'static str,
    /// The AppMark glyph.
    pub icon: Ico,
}

impl Brand {
    /// The brand accent as a colour (violet if the hex is malformed).
    pub fn accent_rgb(&self) -> Rgb {
        Rgb::hex(self.accent).unwrap_or(Rgb::new(0x8b, 0x5c, 0xf6))
    }
}

/// A sidebar nav row for one of the app's pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavItem<P> {
    pub page: P,
    pub icon: Ico,
    pub label: &'static str,
}

impl<P> NavItem<P> {
    pub const fn new(page: P, icon: Ico, label: &'static str) -> Self {
        Self { page, icon, label }
    }
}

/// The frame's own dialogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    Options,
    About,
}

impl Dialog {
    /// `BATO_VIEW` `modal=options|about`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "options" => Some(Dialog::Options),
            "about" => Some(Dialog::About),
            _ => None,
        }
    }
}

/// State of the Server address "Test connection" button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestState {
    Idle,
    Testing,
    Ok,
    Fail,
}

#[derive(Debug, Clone)]
pub enum Msg<P> {
    Navigate(P),
    ToggleSidebar,
    /// A vsync frame while the sidebar animates.
    Tick(Instant),
    Open(Dialog),
    Close,
    /// Show a toast (pages can publish this directly).
    Toast(String, Tone),
    Dismiss(u64),
    Theme(ThemeId),
    Accent(String),
    ToggleCustom,
    CustomHex(String),
    Scale(u32),
    Wide(bool),
    TextSelect(bool),
    ServerUrl(String),
    TestServer,
    /// The answer to [`Action::TestServer`].
    ServerTested(bool),
    SaveServer,
    SavedReset,
    CheckUpdates,
    /// The updater's status (also [`Frame::set_update_status`]).
    UpdateStatus(UpdateStatus),
    /// "Restart to install vX" in About, "Restart" in the sidebar card.
    RestartToUpdate,
    Rescaled(f32),
}

/// What the app must do after [`Frame::update`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action<P> {
    None,
    /// The page changed (already applied).
    Navigated(P),
    /// Settings changed: persist [`Frame::settings`].
    SaveSettings,
    /// Probe `GET <url>/api/health` and answer with [`Msg::ServerTested`].
    TestServer(String),
    /// A new backend address was saved: persist the settings and re-check
    /// the backend.
    ServerSaved,
    /// "Check for updates" in About.
    CheckUpdates,
    /// "Restart to install vX": start the installed release and exit
    /// (bato-shell `iced_ext::restart_to_update`).
    RestartToUpdate,
}

/// Where the frame starts (from `BATO_VIEW` or the app).
#[derive(Debug, Clone, Default)]
pub struct Start {
    pub dialog: Option<Dialog>,
    /// Initial scroll offset, CSS px.
    pub scroll: Option<f32>,
    /// Scroll the container with this id to the top (24 px gap).
    pub anchor: Option<iced::widget::Id>,
    /// `false` skips the dialog animations (smoke runs, screenshots).
    pub no_animations: bool,
}

const EXPANDED: f32 = 224.0;
const COLLAPSED: f32 = 64.0;

pub struct Frame<P> {
    brand: Brand,
    nav: Vec<NavItem<P>>,
    settings: Settings,
    cx: Ctx,
    page: P,
    dialog: Option<Dialog>,
    dialog_opened: Instant,
    sidebar: Tween,
    toasts: Toasts,
    custom_open: bool,
    custom_hex: String,
    server_url: String,
    server_card: bool,
    test: TestState,
    saved_at: Option<Instant>,
    update_note: Option<String>,
    update: UpdateStatus,
    start: Start,
    window_scale: f32,
    autoscroll: AutoscrollBus,
}

impl<P: Copy + PartialEq + std::fmt::Debug + Send + 'static> Frame<P> {
    pub fn new(brand: Brand, nav: Vec<NavItem<P>>, settings: Settings, page: P) -> Self {
        let cx = Ctx::from_palette(palette(settings.theme, accent_rgb(&settings, &brand)));
        set_device_scale(settings.scale_factor());
        let w = if settings.sidebar_collapsed {
            COLLAPSED
        } else {
            EXPANDED
        };
        Self {
            server_url: backend_url(&settings),
            brand,
            nav,
            settings,
            cx,
            page,
            dialog: None,
            dialog_opened: Instant::now(),
            sidebar: Tween::settled(w),
            toasts: Toasts::default(),
            custom_open: false,
            custom_hex: String::new(),
            server_card: true,
            test: TestState::Idle,
            saved_at: None,
            update_note: None,
            update: UpdateStatus::Idle,
            start: Start::default(),
            window_scale: 1.0,
            autoscroll: new_bus(),
        }
    }

    /// Initial dialog, scroll position and animation mode.
    pub fn start(mut self, start: Start) -> Self {
        self.dialog = start.dialog;
        self.start = start;
        self.dialog_opened = Instant::now();
        self
    }

    /// Hide the Server address card in App Options (apps without a backend).
    pub fn without_server_card(mut self) -> Self {
        self.server_card = false;
        self
    }

    // ── Accessors ───────────────────────────────────────────────────────

    pub fn brand(&self) -> &Brand {
        &self.brand
    }
    /// The theme on the page background; `.on_card()` inside cards.
    pub fn ctx(&self) -> &Ctx {
        &self.cx
    }
    pub fn page(&self) -> P {
        self.page
    }
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    /// Content width "Full".
    pub fn wide(&self) -> bool {
        self.settings.wide
    }
    /// The backend address in effect (`BATO_BACKEND_URL`, saved, default).
    pub fn backend_url(&self) -> String {
        backend_url(&self.settings)
    }
    pub fn dialog(&self) -> Option<Dialog> {
        self.dialog
    }
    /// The BatoScroll bus (debug hooks can engage it).
    pub fn autoscroll(&self) -> &AutoscrollBus {
        &self.autoscroll
    }

    // ── Commands ────────────────────────────────────────────────────────

    pub fn toast(&mut self, text: impl Into<String>, tone: Tone) {
        self.toasts.push(text, tone);
    }

    /// The line under "Check for updates" in About, over the status's own
    /// (React's `unsupported`: "Updates apply to packaged builds only.").
    /// Cleared by the next check and when About opens.
    pub fn set_update_note(&mut self, note: Option<String>) {
        self.update_note = note;
    }

    /// The updater's status: About's button and note, the sidebar card.
    pub fn set_update_status(&mut self, status: UpdateStatus) {
        self.update = status;
    }

    pub fn update_status(&self) -> &UpdateStatus {
        &self.update
    }

    pub fn navigate(&mut self, page: P) {
        self.page = page;
    }

    // ── iced plumbing ───────────────────────────────────────────────────

    pub fn scale_factor(&self) -> f32 {
        self.settings.scale_factor()
    }

    /// The window background and default text colour.
    pub fn style(&self) -> iced::theme::Style {
        iced::theme::Style {
            background_color: c(self.cx.p().bg),
            text_color: c(self.cx.p().text),
        }
    }

    pub fn subscription(&self) -> Subscription<Msg<P>> {
        let mut subs = vec![window::events().filter_map(|(_, e)| match e {
            window::Event::Rescaled(f) => Some(Msg::Rescaled(f)),
            _ => None,
        })];
        if self.sidebar.is_running(Instant::now()) {
            subs.push(window::frames().map(Msg::Tick));
        }
        Subscription::batch(subs)
    }

    fn retheme(&mut self) {
        self.cx = Ctx::from_palette(palette(
            self.settings.theme,
            accent_rgb(&self.settings, &self.brand),
        ));
    }

    pub fn update(&mut self, msg: Msg<P>) -> Action<P> {
        match msg {
            Msg::Navigate(p) => {
                self.page = p;
                return Action::Navigated(p);
            }
            Msg::ToggleSidebar => {
                let collapsed = !self.settings.sidebar_collapsed;
                self.settings.sidebar_collapsed = collapsed;
                let now = Instant::now();
                let from = self.sidebar.value(now);
                self.sidebar = Tween::new(
                    from,
                    if collapsed { COLLAPSED } else { EXPANDED },
                    Duration::from_millis(200),
                    ease_standard,
                );
                return Action::SaveSettings;
            }
            Msg::Tick(_) => {}
            Msg::Open(d) => {
                self.dialog = Some(d);
                self.dialog_opened = Instant::now();
                self.update_note = None;
                if d == Dialog::About {
                    self.update = self.update.on_about_open();
                }
                if d == Dialog::Options {
                    self.server_url = backend_url(&self.settings);
                    self.test = TestState::Idle;
                    self.custom_open = false;
                }
            }
            Msg::Close => self.dialog = None,
            Msg::Toast(t, tone) => self.toast(t, tone),
            Msg::Dismiss(id) => self.toasts.dismiss(id),
            Msg::Theme(t) => {
                self.settings.theme = t;
                self.retheme();
                return Action::SaveSettings;
            }
            Msg::Accent(hex) => {
                if Rgb::hex(&hex).is_some() {
                    self.settings.accent = Some(hex.to_lowercase());
                    self.retheme();
                    return Action::SaveSettings;
                }
            }
            Msg::ToggleCustom => {
                self.custom_open = !self.custom_open;
                self.custom_hex = self.settings.accent_or(self.brand.accent).to_string();
            }
            Msg::CustomHex(h) => {
                self.custom_hex = h.clone();
                let h = h.trim();
                let h = if h.starts_with('#') {
                    h.to_string()
                } else {
                    format!("#{h}")
                };
                if h.len() == 7 && Rgb::hex(&h).is_some() {
                    return self.update(Msg::Accent(h));
                }
            }
            Msg::Scale(s) => {
                self.settings.scale = s;
                set_device_scale(self.window_scale * self.settings.scale_factor());
                return Action::SaveSettings;
            }
            Msg::Wide(w) => {
                self.settings.wide = w;
                return Action::SaveSettings;
            }
            Msg::TextSelect(v) => {
                self.settings.text_select = v;
                return Action::SaveSettings;
            }
            Msg::ServerUrl(u) => {
                self.server_url = u;
                self.test = TestState::Idle;
            }
            Msg::TestServer => {
                self.test = TestState::Testing;
                return Action::TestServer(normalize(&self.server_url));
            }
            Msg::ServerTested(ok) => self.test = if ok { TestState::Ok } else { TestState::Fail },
            Msg::SaveServer => {
                self.settings.backend_url = Some(normalize(&self.server_url));
                self.server_url = backend_url(&self.settings);
                self.test = TestState::Idle;
                self.saved_at = Some(Instant::now());
                return Action::ServerSaved;
            }
            Msg::SavedReset => self.saved_at = None,
            Msg::CheckUpdates => {
                self.update_note = None;
                return Action::CheckUpdates;
            }
            Msg::UpdateStatus(s) => self.update = s,
            Msg::RestartToUpdate => return Action::RestartToUpdate,
            Msg::Rescaled(f) => {
                self.window_scale = f;
                set_device_scale(f * self.settings.scale_factor());
            }
        }
        Action::None
    }

    /// The whole window: sidebar, `page` in the scroller, dialogs, toasts.
    /// `wrap` lifts frame messages into the app's message type.
    pub fn view<'a, M: Clone + 'a>(&'a self, page: El<'a, M>, wrap: fn(Msg<P>) -> M) -> El<'a, M> {
        let p = self.cx.p();
        let main = scroller(page, c(p.border), c(p.muted))
            .initial(self.start.scroll)
            .initial_anchor(self.start.anchor.clone())
            .autoscroll(self.autoscroll.clone());
        let shell: El<'a, M> = autoscroll_layer(
            row![self.sidebar_view(wrap), main].height(Length::Fill),
            self.autoscroll.clone(),
        )
        .into();
        let mut layers: Vec<El<'a, M>> = vec![shell];
        if let Some(d) = self.dialog {
            let (panel, w) = match d {
                Dialog::Options => (self.options_view(wrap), 512.0),
                Dialog::About => (self.about_view(wrap), 448.0),
            };
            let animate = !self.start.no_animations;
            let cover = c(bato_theme::blend(Rgb::BLACK, 0.5, p.bg));
            layers.push(
                modal(panel, w, wrap(Msg::Close), cover)
                    .animate(animate)
                    .backdrop(backdrop_blur(self.dialog_opened, animate))
                    .into(),
            );
        }
        if !self.toasts.is_empty() {
            layers.push(toast_stack(&self.cx, &self.toasts.items, move |id| {
                wrap(Msg::Dismiss(id))
            }));
        }
        stack(layers)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

/// The accent the palette is derived from: the saved one, else the brand's.
pub fn accent_rgb(s: &Settings, brand: &Brand) -> Rgb {
    Rgb::hex(s.accent_or(brand.accent)).unwrap_or_else(|| brand.accent_rgb())
}

fn normalize(u: &str) -> String {
    u.trim().trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRAND: Brand = Brand {
        name: "Test",
        tagline: "t",
        version: "1.0.0",
        slug: "bato-kit-test",
        accent: "#8b5cf6",
        icon: Ico::Box,
    };

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Page {
        A,
        B,
    }

    fn frame() -> Frame<Page> {
        Frame::new(
            BRAND,
            vec![
                NavItem::new(Page::A, Ico::Home, "A"),
                NavItem::new(Page::B, Ico::Rows3, "B"),
            ],
            Settings::default(),
            Page::A,
        )
    }

    #[test]
    fn actions_for_the_platform() {
        let mut f = frame();
        assert_eq!(f.update(Msg::Navigate(Page::B)), Action::Navigated(Page::B));
        assert_eq!(f.page(), Page::B);
        assert_eq!(f.update(Msg::Theme(ThemeId::Light)), Action::SaveSettings);
        assert_eq!(f.settings().theme, ThemeId::Light);
        assert!(f.ctx().p().is_light());
        assert_eq!(f.update(Msg::Accent("not a colour".into())), Action::None);
        assert_eq!(
            f.update(Msg::CustomHex("2563EB".into())),
            Action::SaveSettings
        );
        assert_eq!(f.settings().accent.as_deref(), Some("#2563eb"));
        f.update(Msg::ServerUrl(" http://h:1/ ".into()));
        assert_eq!(
            f.update(Msg::TestServer),
            Action::TestServer("http://h:1".into())
        );
        assert_eq!(f.update(Msg::SaveServer), Action::ServerSaved);
        assert_eq!(f.settings().backend_url.as_deref(), Some("http://h:1"));
        assert_eq!(f.update(Msg::CheckUpdates), Action::CheckUpdates);
    }

    #[test]
    fn update_status_and_restart() {
        let mut f = frame();
        assert_eq!(f.update_status(), &UpdateStatus::Idle);
        f.set_update_note(Some("Updates apply to packaged builds only.".into()));
        assert_eq!(f.update(Msg::CheckUpdates), Action::CheckUpdates);
        assert_eq!(f.update_note, None, "a new check clears `unsupported`");
        let done = UpdateStatus::Downloaded {
            version: "1.3.0".into(),
        };
        assert_eq!(f.update(Msg::UpdateStatus(done.clone())), Action::None);
        assert_eq!(f.update(Msg::RestartToUpdate), Action::RestartToUpdate);
        // Reopening About keeps a pending restart, forgets a finished check.
        f.update(Msg::Open(Dialog::About));
        assert_eq!(f.update_status(), &done);
        f.set_update_status(UpdateStatus::UpToDate);
        f.update(Msg::Open(Dialog::About));
        assert_eq!(f.update_status(), &UpdateStatus::Idle);
    }

    #[test]
    fn sidebar_collapse_animates_and_persists() {
        let mut f = frame();
        assert_eq!(f.update(Msg::ToggleSidebar), Action::SaveSettings);
        assert!(f.settings().sidebar_collapsed);
        assert!(f.sidebar.is_running(Instant::now()));
        assert_eq!(f.sidebar.to, COLLAPSED);
        assert_eq!(
            f.sidebar.value(Instant::now() + Duration::from_millis(250)),
            COLLAPSED
        );
    }

    #[test]
    fn toasts_queue_and_dismiss() {
        let mut f = frame();
        f.update(Msg::Toast("a".into(), Tone::Info));
        f.toast("b", Tone::Error);
        assert_eq!(f.toasts.items.len(), 2);
        let id = f.toasts.items[0].id;
        f.update(Msg::Dismiss(id));
        assert_eq!(f.toasts.items[0].text, "b");
    }
}
