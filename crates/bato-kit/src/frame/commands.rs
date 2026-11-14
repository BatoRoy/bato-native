//! The frame's command palette (`App.tsx`): `mod+k`, the sidebar search
//! box and [`Msg::OpenPalette`] open it; the default commands are
//! Navigate (one per nav item), Theme ×3 and the app group (App Options,
//! About, Check for updates), followed by the app's own
//! ([`Frame::commands`]), which come back as [`Action::Command`].

use bato_theme::ThemeId;

use super::{Dialog, Frame, Msg};
use crate::icon::Ico;
use crate::modal::Modal;
use crate::overlay::palette::{Command, Outcome};

/// The palette's placeholder (React's default).
pub const PLACEHOLDER: &str = "Type a command…";

impl<P: Copy + PartialEq + std::fmt::Debug + Send + 'static> Frame<P> {
    /// Add the app's own commands to the palette (after the defaults). Each
    /// `run` is an id the app gets back as [`Action::Command`](super::Action::Command).
    pub fn commands(mut self, commands: Vec<Command<String>>) -> Self {
        self.app_commands = commands;
        self
    }

    /// Replace the app's own commands.
    pub fn set_commands(&mut self, commands: Vec<Command<String>>) {
        self.app_commands = commands;
    }

    pub fn open_palette(&mut self) {
        self.palette.open();
    }

    pub fn palette_open(&self) -> bool {
        self.palette.is_open()
    }

    /// Every command of the palette, in order (`App.tsx`'s `commands`).
    pub fn palette_commands(&self) -> Vec<Command<Msg<P>>> {
        let app = self.brand.name;
        let mut v: Vec<Command<Msg<P>>> = self
            .nav
            .iter()
            .map(|n| {
                Command::new(
                    format!("nav-{}", n.label.to_lowercase().replace(' ', "-")),
                    n.label,
                    Msg::Navigate(n.page),
                )
                .group("Navigate")
                .icon(n.icon)
            })
            .collect();
        for (id, label, icon, theme) in [
            ("theme-dark", "Dark theme", Ico::Moon, ThemeId::Dark),
            ("theme-dim", "Dim theme", Ico::CloudMoon, ThemeId::Dim),
            ("theme-light", "Light theme", Ico::SunMedium, ThemeId::Light),
        ] {
            v.push(
                Command::new(id, label, Msg::Theme(theme))
                    .group("Theme")
                    .icon(icon)
                    .keywords("appearance"),
            );
        }
        v.push(
            Command::new("app-options", "App Options", Msg::Open(Dialog::Options))
                .group(app)
                .icon(Ico::Settings)
                .keywords("settings preferences"),
        );
        v.push(
            Command::new("app-about", format!("About {app}"), Msg::Open(Dialog::About))
                .group(app)
                .icon(Ico::Info)
                .keywords("version"),
        );
        v.push(
            Command::new("app-update", "Check for updates", Msg::Open(Dialog::About))
                .group(app)
                .icon(Ico::RefreshCw),
        );
        v.extend(
            self.app_commands
                .iter()
                .cloned()
                .map(|c| c.map(Msg::Command)),
        );
        v
    }

    /// A palette message: the command to run next, if any.
    pub(super) fn palette_update(&mut self, m: crate::overlay::palette::Msg) -> Option<Msg<P>> {
        let cmds = self.palette_commands();
        match self.palette.update(m, &cmds) {
            Some(Outcome::Run(msg)) => Some(msg),
            _ => None,
        }
    }

    pub(super) fn palette_level<'a, M: Clone + 'a>(&self, wrap: fn(Msg<P>) -> M) -> Modal<'a, M> {
        self.palette.level(
            &self.cx,
            &self.palette_commands(),
            PLACEHOLDER,
            move |m| wrap(Msg::Palette(m)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::hotkeys::KeyPress;
    use crate::overlay::palette::{Command, Msg as P};

    const BRAND: Brand = Brand {
        name: "App",
        tagline: "t",
        version: "1.0.0",
        slug: "bato-kit-test",
        accent: "#8b5cf6",
        icon: Ico::Box,
    };

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Page {
        Home,
        Examples,
        ScrollLab,
    }

    fn frame() -> Frame<Page> {
        Frame::new(
            BRAND,
            vec![
                NavItem::new(Page::Home, Ico::Home, "Home"),
                NavItem::new(Page::Examples, Ico::LayoutGrid, "Examples"),
                NavItem::new(Page::ScrollLab, Ico::Rows3, "Scroll lab"),
            ],
            Settings::default(),
            Page::Home,
        )
    }

    #[test]
    fn default_commands_are_app_tsx() {
        let f = frame();
        let c = f.palette_commands();
        let ids: Vec<&str> = c.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "nav-home",
                "nav-examples",
                "nav-scroll-lab",
                "theme-dark",
                "theme-dim",
                "theme-light",
                "app-options",
                "app-about",
                "app-update"
            ]
        );
        assert_eq!(c[7].label, "About App");
        assert_eq!(c[6].group.as_deref(), Some("App"));
        assert_eq!(c[2].icon, Some(Ico::Rows3));
    }

    #[test]
    fn mod_k_opens_and_commands_run_through_update() {
        let mut f = frame();
        assert!(!f.palette_open());
        f.update(Msg::Key(KeyPress::of("mod+k").unwrap()));
        assert!(f.palette_open(), "mod+k");
        // Typing in a field: React's useHotkeys ignores the press.
        f.update(Msg::Palette(P::Close));
        let mut typing = KeyPress::of("mod+k").unwrap();
        typing.typing = true;
        f.update(Msg::Key(typing));
        assert!(!f.palette_open());

        f.update(Msg::OpenPalette);
        f.update(Msg::Palette(P::Query("light".into())));
        assert_eq!(f.update(Msg::Palette(P::Enter)), Action::SaveSettings);
        assert_eq!(f.settings().theme, ThemeId::Light);
        assert!(!f.palette_open());

        f.update(Msg::OpenPalette);
        f.update(Msg::Palette(P::Query("scroll".into())));
        assert_eq!(
            f.update(Msg::Palette(P::Enter)),
            Action::Navigated(Page::ScrollLab)
        );

        f.update(Msg::OpenPalette);
        f.update(Msg::Palette(P::Query("updates".into())));
        f.update(Msg::Palette(P::Enter));
        assert_eq!(f.dialog(), Some(Dialog::About));
    }

    #[test]
    fn app_commands_come_back_as_actions() {
        let mut f = frame().commands(vec![
            Command::new("new-file", "New File", "new-file".to_string())
                .group("Files")
                .hint("Ctrl N"),
        ]);
        assert_eq!(f.palette_commands().len(), 10);
        f.open_palette();
        f.update(Msg::Palette(P::Query("new".into())));
        assert_eq!(
            f.update(Msg::Palette(P::Run(0))),
            Action::Command("new-file".into())
        );
    }
}
