//! The gallery: the Showcase and the kit's visual test bench, built exactly
//! as an app is: bato-kit's frame for the UI, bato-shell for the platform
//! (identity, settings, HTTP, notifications, `BATO_VIEW` / `BATO_SMOKE`).

mod home;

use std::time::{Duration, Instant};

use bato_kit::frame::{self, Action, Brand, Dialog, Frame, NavItem, Start};
use bato_kit::iced::{self, Subscription, Task};
use bato_kit::{El, Ico, Tone, fonts};
use bato_shell::hooks::{self, BootView, Smoke};
use bato_shell::http::Http;
use bato_shell::notify::{Notification, Notifier};
use bato_shell::{AppIdentity, iced_ext};
use gallery::showcase::{self, Effect, Tab};

/// The platform identity. The slug is the default Wayland app_id, so the
/// gallery is `batopoc-gallery` (routed off-screen in automated tests);
/// `BATO_APP_ID` overrides it.
const APP: AppIdentity = AppIdentity::new("App", "batopoc-gallery", "0.4.1");

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
    /// `BATO_SMOKE`: a frame event.
    SmokeFrame,
}

struct App {
    frame: Frame<Page>,
    showcase: showcase::State,
    notifier: Notifier,
    name: String,
    auto_update: bool,
    status: Status,
    backend_version: Option<String>,
    smoke: Smoke,
}

/// The page, Showcase tab and `card=` anchor a `BATO_VIEW` asks for.
fn route(view: &BootView) -> (Page, Tab, Option<iced::widget::Id>) {
    let page = match view.view.as_deref() {
        Some("scroll-lab") => Page::ScrollLab,
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
        let app = App {
            frame,
            showcase: showcase::State::new(tab),
            notifier: Notifier::new(&APP),
            name: String::new(),
            auto_update: true,
            status: Status::Checking,
            backend_version: None,
            smoke: Smoke::default(),
        };
        let mut task = app.ping();
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
                Action::None | Action::Navigated(_) => {}
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
            iced_ext::smoke_frames().map(|_| Msg::SmokeFrame),
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
        };
        self.frame.view(page, Msg::Frame)
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
}
