//! The gallery: the Showcase and the kit's visual test bench, built exactly
//! as an app is: bato-kit's frame for the UI, bato-shell for the platform
//! (identity, settings, HTTP, notifications, `BATO_VIEW` / `BATO_SMOKE`).

mod home;

use std::time::{Duration, Instant};

use bato_kit::frame::{self, Action, Brand, Dialog, Frame, NavItem, Start, UpdateStatus};
use bato_kit::iced::{self, Subscription, Task};
use bato_kit::{El, Ico, Tone, fonts};
use bato_shell::hooks::{self, BootView, Smoke};
use bato_shell::http::Http;
use bato_shell::notify::{Notification, Notifier};
use bato_shell::updater::{self, UpdatePhase, Updater};
use bato_shell::{AppIdentity, iced_ext};
use gallery::interaction::gaps::{self, SelfTest};
use gallery::showcase::{self, Effect, Tab};

/// The platform identity. The slug is the default Wayland app_id, so the
/// gallery is `batopoc-gallery` (routed off-screen in automated tests);
/// `BATO_APP_ID` overrides it.
/// The icon is the template's (256 px from `BatoApps/bato/icons`), for X11.
const APP: AppIdentity = AppIdentity::new("App", "batopoc-gallery", "0.4.1")
    .with_icon(include_bytes!("../../../assets/appicon.png"));

/// What the frame shows: the template's look, as in the Electron references.
const BRAND: Brand = Brand {
    name: APP.product_name,
    tagline: "A starting template — Electron + React + Go.",
    version: APP.version,
    slug: APP.slug,
    accent: "#8b5cf6",
    icon: Ico::Box,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Examples,
    ScrollLab,
    Interaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Checking,
    Online,
    Offline,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Frame(frame::Msg<Page>),
    Showcase(showcase::Msg),
    Interaction(gallery::interaction::Msg),
    Name(String),
    AutoUpdate(bool),
    Ping,
    Health(bool),
    Version(Option<String>),
    Notify,
    NotifyResult(bool),
    Update(UpdatePhase),
    /// A check from About: like [`Msg::Update`], and installs what it finds.
    Checked(UpdatePhase),
    RestartFailed(String),
    /// `BATO_SELFTEST=dialog-parent`.
    ParentProbed(String),
    /// `BATO_SMOKE`: a frame event.
    SmokeFrame,
}

struct App {
    frame: Frame<Page>,
    showcase: showcase::State,
    interaction: gallery::interaction::State,
    notifier: Notifier,
    updater: Updater,
    selftest: Option<SelfTest>,
    name: String,
    auto_update: bool,
    status: Status,
    backend_version: Option<String>,
    smoke: Smoke,
    /// `BATO_SELFTEST=<name>`: a scripted input run (see `interaction`).
    script: Option<Vec<bato_kit::overlay::script::Step<Msg>>>,
}

/// The page, Showcase tab and `card=` anchor a `BATO_VIEW` asks for.
fn route(view: &BootView) -> (Page, Tab, Option<iced::widget::Id>) {
    let page = match view.view.as_deref() {
        Some("scroll-lab") => Page::ScrollLab,
        Some("interaction") => Page::Interaction,
        Some("examples") => Page::Examples,
        _ => Page::Home,
    };
    // `card=` selects its tab when `tab=` is missing.
    let tab = view
        .tab
        .as_deref()
        .and_then(Tab::parse)
        .or_else(|| view.card.as_deref().and_then(Tab::of_card))
        .unwrap_or_default();
    let anchor = view
        .card
        .as_deref()
        .filter(|c| page == Page::Examples && tab.cards().contains(c))
        .map(|c| showcase::card_id(tab, c));
    (page, tab, anchor)
}

impl App {
    fn boot(view: BootView) -> (Self, Task<Msg>) {
        hooks::mark("boot (window + renderer created)");
        let (page, tab, anchor) = route(&view);
        let nav = vec![
            NavItem::new(Page::Home, Ico::Home, "Home"),
            NavItem::new(Page::Examples, Ico::LayoutGrid, "Examples"),
            NavItem::new(Page::ScrollLab, Ico::Rows3, "Scroll lab"),
            NavItem::new(Page::Interaction, Ico::Command, "Interaction lab"),
        ];
        let start = Start {
            dialog: view.modal.as_deref().and_then(Dialog::parse),
            scroll: view.scroll,
            anchor,
            no_animations: hooks::smoke_enabled(),
        };
        let mut frame = Frame::new(BRAND, nav, APP.load_settings(), page).start(start);
        if let Some((x, y, cy)) = view.autoscroll {
            // Debug hook: BatoScroll engaged at (x, y) with the cursor at (x, cy).
            let mut bus = frame.autoscroll().borrow_mut();
            bus.arm(x, y, Instant::now());
            bus.auto.pointer_moved(x, cy);
        }
        if view.toast {
            frame.toast("Hello, Ada!", Tone::Success);
            frame.toast("Something went wrong", Tone::Error);
        }
        let mut app = App {
            frame,
            showcase: showcase::State::new(tab),
            interaction: Default::default(),
            notifier: Notifier::new(&APP),
            updater: Updater::new(&APP, updater::DEFAULT_REGISTRY),
            selftest: gaps::selftest(),
            name: String::new(),
            auto_update: true,
            status: Status::Checking,
            backend_version: None,
            smoke: Smoke::default(),
            script: std::env::var("BATO_SELFTEST").ok().and_then(|n| {
                let lab = gallery::interaction::selftest(&n)?
                    .into_iter()
                    .map(|s| s.map(Msg::Interaction));
                // `modals`: the frame's palette first, then the lab section.
                let frame = if n == "modals" { palette_selftest() } else { Vec::new() };
                Some(frame.into_iter().chain(lab).collect())
            }),
        };
        let mut task = app.ping();
        match app.selftest.clone() {
            Some(SelfTest::Update(s)) => app.frame.set_update_status(s),
            Some(SelfTest::DialogParent) => {
                let probe =
                    iced_ext::latest_dialog_parent().map(|p| Msg::ParentProbed(p.portal_string()));
                task = Task::batch([task, probe]);
            }
            _ => {}
        }
        if view.demo.as_deref() == Some("sidebar") {
            let toggle = iced_ext::blocking(|| std::thread::sleep(Duration::from_millis(600)))
                .map(|_| Msg::Frame(frame::Msg::ToggleSidebar));
            task = Task::batch([task, toggle]);
        }
        (app, task)
    }

    fn ping(&self) -> Task<Msg> {
        let api = Http::new(&self.frame.backend_url());
        iced_ext::blocking(move || api.health()).map(Msg::Health)
    }

    fn save_settings(&self) {
        if let Err(e) = APP.save_settings(self.frame.settings()) {
            eprintln!("settings: {e}");
        }
    }

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Frame(m) => match self.frame.update(m) {
                // The gallery registers no commands of its own.
                Action::None | Action::Navigated(_) | Action::Command(_) => {}
                Action::SaveSettings => self.save_settings(),
                Action::TestServer(url) => {
                    let api = Http::new(&url);
                    return iced_ext::blocking(move || api.health())
                        .map(|ok| Msg::Frame(frame::Msg::ServerTested(ok)));
                }
                Action::ServerSaved => {
                    self.save_settings();
                    return self.update(Msg::Ping);
                }
                Action::CheckUpdates => {
                    if !self.updater.is_supported() {
                        self.frame
                            .set_update_note(Some(updater::NOT_PACKAGED.into()));
                    } else {
                        return iced_ext::check_update(&self.updater).map(Msg::Checked);
                    }
                }
                Action::RestartToUpdate => {
                    return iced_ext::restart_to_update(&self.updater, Msg::RestartFailed);
                }
            },
            Msg::Update(phase) => {
                let downloaded = matches!(phase, UpdatePhase::Downloaded { .. });
                self.frame.set_update_status(update_status(phase));
                if downloaded && self.selftest == Some(SelfTest::Restart) {
                    // Leave the state up for a screenshot, then press Restart.
                    let press = iced_ext::blocking(|| std::thread::sleep(Duration::from_secs(3)))
                        .map(|_| Msg::Frame(frame::Msg::RestartToUpdate));
                    return press;
                }
            }
            Msg::Checked(phase) => {
                // A manual check installs what it finds, unless the background
                // loop is already on it.
                let apply = matches!(phase, UpdatePhase::Available { .. })
                    && !matches!(
                        self.frame.update_status(),
                        UpdateStatus::Downloading { .. } | UpdateStatus::Downloaded { .. }
                    );
                let t = self.update(Msg::Update(phase));
                if apply {
                    return Task::batch([
                        t,
                        iced_ext::apply_update(&self.updater).map(Msg::Update),
                    ]);
                }
                return t;
            }
            Msg::RestartFailed(e) => self
                .frame
                .toast(format!("Restart failed: {e}"), Tone::Error),
            Msg::ParentProbed(p) => {
                println!("dialog parent: {}", if p.is_empty() { "none" } else { &p });
                return iced::exit();
            }
            Msg::Interaction(m) => {
                if let Some(s) = m.gaps_status() {
                    self.frame.set_update_status(s.clone());
                }
                if let Some((text, tone)) = m.toast() {
                    self.frame.toast(text, tone);
                }
                return gallery::interaction::update(&mut self.interaction, m)
                    .map(Msg::Interaction);
            }
            Msg::Showcase(m) => match showcase::update(&mut self.showcase, m) {
                Some(Effect::Toast(text, tone)) => self.frame.toast(text, tone),
                Some(Effect::OpenFiles { multiple, reply }) => {
                    let o = bato_shell::dialogs::OpenFiles {
                        multiple,
                        ..Default::default()
                    };
                    return iced_ext::open_files(o).map(move |r| match r {
                        Ok(paths) => Msg::Showcase((reply.0)(paths)),
                        Err(e) => Msg::Frame(frame::Msg::Toast(e.to_string(), Tone::Error)),
                    });
                }
                None => {}
            },
            Msg::Name(n) => self.name = n,
            Msg::AutoUpdate(v) => self.auto_update = v,
            Msg::Ping => {
                self.status = Status::Checking;
                return self.ping();
            }
            Msg::Health(ok) => {
                if ok {
                    self.status = Status::Online;
                    let api = Http::new(&self.frame.backend_url());
                    return iced_ext::blocking(move || api.info_version()).map(Msg::Version);
                }
                self.status = Status::Offline;
                self.backend_version = None;
            }
            Msg::Version(v) => self.backend_version = v,
            Msg::Notify => {
                let body = if self.name.is_empty() {
                    "This is a native OS notification.".to_string()
                } else {
                    format!("Notifying you, {}.", self.name)
                };
                let n = Notification::new("Hello from the template", body);
                return iced_ext::notify(&self.notifier, n).map(Msg::NotifyResult);
            }
            Msg::NotifyResult(ok) => {
                if !ok {
                    self.frame
                        .toast("Notifications are blocked or unsupported", Tone::Error);
                }
            }
            Msg::SmokeFrame => {
                if self.smoke.on_frame() {
                    return iced::exit();
                }
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Msg> {
        Subscription::batch([
            self.frame.subscription().map(Msg::Frame),
            // Check on launch and every 30 min, install in the background.
            // Nothing for a dev build.
            iced_ext::update_status(&self.updater, true).map(Msg::Update),
            iced_ext::smoke_frames().map(|_| Msg::SmokeFrame),
            gallery::interaction::subscription(&self.interaction).map(Msg::Interaction),
        ])
    }

    fn view(&self) -> El<'_, Msg> {
        let cx = self.frame.ctx();
        let wide = self.frame.wide();
        let page: El<'_, Msg> = match self.frame.page() {
            Page::Home => home::view(
                cx,
                home::Home {
                    brand: self.frame.brand(),
                    backend: self.frame.backend_url(),
                    status: self.status,
                    version: self.backend_version.clone(),
                    name: &self.name,
                    auto_update: self.auto_update,
                },
                wide,
            ),
            Page::Examples => showcase::view(&self.showcase, cx, wide).map(Msg::Showcase),
            Page::ScrollLab => gallery::scroll_lab::view(cx, wide),
            Page::Interaction => {
                gallery::interaction::view(&self.interaction, cx, wide).map(Msg::Interaction)
            }
        };
        let levels = match self.frame.page() {
            Page::Interaction => gallery::interaction::levels(&self.interaction, cx)
                .into_iter()
                .map(|l| l.map(Msg::Interaction))
                .collect(),
            Page::Examples => showcase::levels(&self.showcase, cx)
                .into_iter()
                .map(|l| l.map(Msg::Showcase))
                .collect(),
            _ => Vec::new(),
        };
        let root = self.frame.view_with(page, levels, Msg::Frame);
        let root = match &self.script {
            Some(steps) => bato_kit::overlay::script::script(root, steps.clone()).into(),
            None => root,
        };
        // `BATO_SELFTEST=scroll`: scripted scroll input (otherwise a no-op).
        gallery::interaction::scroll::selftest(root)
    }
}

/// `BATO_SELFTEST=modals`, the frame's part: the command palette from
/// `mod+k` (the hotkey registry), from the sidebar search box and from
/// Home's Commands; filtering, ↑↓ and Enter running a default command.
fn palette_selftest() -> Vec<bato_kit::overlay::script::Step<Msg>> {
    use bato_kit::hotkeys::KeyPress;
    use bato_kit::iced::keyboard::{Modifiers, key::Named};
    use bato_kit::iced::{Point, mouse};
    use bato_kit::overlay::script::{Step, Target};
    let none = Modifiers::default();
    let key = |c| Step::Message(Msg::Frame(frame::Msg::Key(KeyPress::of(c).unwrap())));
    let chars = |s: &str| s.chars().map(|c| Step::Char(c, none)).collect::<Vec<_>>();
    let wait = |ms| Step::Wait(Duration::from_millis(ms));
    let mut v = vec![
        key("mod+k"),
        wait(300),
        Step::Shot("frame-palette"),
        Step::Key(Named::ArrowDown, none),
        Step::Key(Named::ArrowDown, none),
        Step::Key(Named::ArrowDown, none),
        Step::Key(Named::ArrowDown, none),
        wait(200),
        Step::Shot("frame-palette-down4"),
    ];
    v.extend(chars("dim"));
    v.extend([
        wait(200),
        Step::Shot("frame-palette-dim"),
        Step::Key(Named::Enter, none),
        wait(400),
        Step::Shot("theme-dim"),
        // The sidebar search box opens it too; "dark" + Enter restores.
        Step::Click(Target::At(Point::new(100.0, 80.0)), mouse::Button::Left),
        wait(300),
    ]);
    v.extend(chars("dark"));
    v.extend([
        Step::Key(Named::Enter, none),
        wait(300),
        // Esc closes it without running anything.
        Step::Message(Msg::Frame(frame::Msg::OpenPalette)),
        wait(200),
        Step::Key(Named::Escape, none),
        wait(300),
        Step::Shot("frame-palette-closed"),
    ]);
    v
}

/// bato-shell's phase as the frame shows it.
fn update_status(phase: UpdatePhase) -> UpdateStatus {
    match phase {
        UpdatePhase::Checking => UpdateStatus::Checking,
        UpdatePhase::Available { version } => UpdateStatus::Available { version },
        UpdatePhase::None => UpdateStatus::UpToDate,
        UpdatePhase::Downloading { percent } => UpdateStatus::Downloading { percent },
        UpdatePhase::Downloaded { version } => UpdateStatus::Downloaded { version },
        UpdatePhase::Error { message } => UpdateStatus::Error { message },
    }
}

fn main() -> iced::Result {
    bato_shell::gpu::prefer_vulkan();
    hooks::mark("main");
    let view = BootView::from_env();
    fonts::setup();
    hooks::mark("fonts ready");

    iced::application(move || App::boot(view.clone()), App::update, App::view)
        .title(|_: &App| String::from(APP.product_name))
        .window(iced_ext::window_settings(&APP))
        .default_font(fonts::sans())
        .subscription(App::subscription)
        .scale_factor(|a: &App| a.frame.scale_factor())
        .style(|a: &App, _| a.frame.style())
        .theme(|_: &App| iced::Theme::Dark)
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bato_view_routes_pages_tabs_and_cards() {
        let (p, t, a) = route(&BootView::parse("view=examples&tab=data&card=charts"));
        assert_eq!((p, t), (Page::Examples, Tab::Data));
        assert_eq!(a, Some(showcase::card_id(Tab::Data, "charts")));
        let (_, t, a) = route(&BootView::parse("view=examples&card=pickers"));
        assert_eq!(t, Tab::Date, "card= picks its tab");
        assert!(a.is_some());
        let (_, _, a) = route(&BootView::parse("view=examples&tab=inputs&card=charts"));
        assert_eq!(a, None, "a card of another tab is ignored");
        assert_eq!(
            route(&BootView::parse("view=scroll-lab")).0,
            Page::ScrollLab
        );
        assert_eq!(route(&BootView::parse("")).0, Page::Home);
    }

    #[test]
    fn x11_icon_decodes() {
        let icon = APP.icon_png.expect("the gallery has an icon");
        assert!(iced_ext::icon_from_png(icon).is_some());
        assert!(iced_ext::window_settings(&APP).icon.is_some());
    }
}
