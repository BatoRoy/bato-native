//! The gallery: the Showcase and the kit's visual test bench, built on the
//! kit's app frame exactly as an app is (Home, Examples, Scroll lab, App
//! Options, About). See README "Develop".

mod home;
mod platform_tmp;

use std::time::{Duration, Instant};

use bato_kit::frame::{self, Action, Brand, Frame, NavItem};
use bato_kit::iced::{self, Subscription, Task, window};
use bato_kit::{El, Ico, Tone, fonts};
use gallery::showcase::{self, Effect, Tab};
use platform_tmp as platform;

const BRAND: Brand = Brand {
    name: "App",
    tagline: "A starting template — Electron + React + Go.",
    version: "0.4.1",
    slug: "app",
    accent: "#8b5cf6",
    icon: Ico::Box,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Examples,
    ScrollLab,
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
    Name(String),
    AutoUpdate(bool),
    Ping,
    Health(bool),
    Version(Option<String>),
    Notify,
    NotifyResult(bool),
    /// `BATO_SMOKE`: a presented frame.
    SmokeFrame,
}

struct App {
    frame: Frame<Page>,
    showcase: showcase::State,
    name: String,
    auto_update: bool,
    status: Status,
    backend_version: Option<String>,
    smoke: bool,
    frames: u32,
}

impl App {
    fn boot(boot: platform::Boot) -> (Self, Task<Msg>) {
        platform::mark("boot (window + renderer created)");
        let page = match boot.view.as_deref() {
            Some("scroll-lab") => Page::ScrollLab,
            Some("examples") => Page::Examples,
            _ => Page::Home,
        };
        // `card=` selects its tab when `tab=` is missing.
        let tab = boot
            .tab
            .as_deref()
            .and_then(Tab::parse)
            .or_else(|| boot.card.as_deref().and_then(Tab::of_card))
            .unwrap_or_default();
        let anchor = boot
            .card
            .as_deref()
            .filter(|c| page == Page::Examples && tab.cards().contains(c))
            .map(|c| showcase::card_id(tab, c));
        let nav = vec![
            NavItem::new(Page::Home, Ico::Home, "Home"),
            NavItem::new(Page::Examples, Ico::LayoutGrid, "Examples"),
            NavItem::new(Page::ScrollLab, Ico::Rows3, "Scroll lab"),
        ];
        let mut frame = Frame::new(BRAND, nav, platform::load_settings(BRAND.slug), page)
            .start(boot.start(anchor));
        if let Some((x, y, cy)) = boot.autoscroll {
            // Debug hook: BatoScroll engaged at (x, y) with the cursor at (x, cy).
            let mut bus = frame.autoscroll().borrow_mut();
            bus.arm(x, y, Instant::now());
            bus.auto.pointer_moved(x, cy);
        }
        if boot.toast {
            frame.toast("Hello, Ada!", Tone::Success);
            frame.toast("Something went wrong", Tone::Error);
        }
        let app = App {
            frame,
            showcase: showcase::State::new(tab),
            name: String::new(),
            auto_update: true,
            status: Status::Checking,
            backend_version: None,
            smoke: boot.smoke,
            frames: 0,
        };
        let mut task = app.ping();
        if boot.demo_sidebar {
            let toggle = Task::perform(
                platform::spawn(|| std::thread::sleep(Duration::from_millis(600))),
                |_| Msg::Frame(frame::Msg::ToggleSidebar),
            );
            task = Task::batch([task, toggle]);
        }
        (app, task)
    }

    fn ping(&self) -> Task<Msg> {
        let base = self.frame.backend_url();
        Task::perform(
            platform::spawn(move || platform::health(&base)),
            Msg::Health,
        )
    }

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Frame(m) => match self.frame.update(m) {
                Action::None | Action::Navigated(_) => {}
                Action::SaveSettings => platform::save_settings(BRAND.slug, self.frame.settings()),
                Action::TestServer(url) => {
                    return Task::perform(platform::spawn(move || platform::health(&url)), |ok| {
                        Msg::Frame(frame::Msg::ServerTested(ok))
                    });
                }
                Action::ServerSaved => {
                    platform::save_settings(BRAND.slug, self.frame.settings());
                    return self.update(Msg::Ping);
                }
                Action::CheckUpdates => self
                    .frame
                    .set_update_note(Some("Updates apply to packaged builds only.".into())),
            },
            Msg::Showcase(m) => {
                if let Some(Effect::Toast(text, tone)) = showcase::update(&mut self.showcase, m) {
                    self.frame.toast(text, tone);
                }
            }
            Msg::Name(n) => self.name = n,
            Msg::AutoUpdate(v) => self.auto_update = v,
            Msg::Ping => {
                self.status = Status::Checking;
                return self.ping();
            }
            Msg::Health(ok) => {
                if ok {
                    self.status = Status::Online;
                    let base = self.frame.backend_url();
                    return Task::perform(
                        platform::spawn(move || platform::info_version(&base)),
                        Msg::Version,
                    );
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
                return Task::perform(
                    platform::spawn(move || {
                        platform::notify(BRAND.name, "Hello from the template", &body)
                    }),
                    Msg::NotifyResult,
                );
            }
            Msg::NotifyResult(ok) => {
                if !ok {
                    self.frame
                        .toast("Notifications are blocked or unsupported", Tone::Error);
                }
            }
            Msg::SmokeFrame => {
                self.frames += 1;
                platform::mark(if self.frames == 1 {
                    "first redraw event"
                } else {
                    "frame presented"
                });
                if self.frames >= 2 {
                    platform::smoke_report();
                    return iced::exit();
                }
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Msg> {
        let mut subs = vec![self.frame.subscription().map(Msg::Frame)];
        if self.smoke {
            subs.push(window::frames().map(|_| Msg::SmokeFrame));
        }
        Subscription::batch(subs)
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
        };
        self.frame.view(page, Msg::Frame)
    }
}

fn main() -> iced::Result {
    platform::prefer_vulkan();
    let boot = platform::Boot::from_env();
    platform::mark("main");
    fonts::setup();
    platform::mark("fonts ready");

    iced::application(move || App::boot(boot.clone()), App::update, App::view)
        .title(|_: &App| String::from(BRAND.name))
        .window(platform::window_settings())
        .default_font(fonts::sans())
        .subscription(App::subscription)
        .scale_factor(|a: &App| a.frame.scale_factor())
        .style(|a: &App, _| a.frame.style())
        .theme(|_: &App| iced::Theme::Dark)
        .run()
}
