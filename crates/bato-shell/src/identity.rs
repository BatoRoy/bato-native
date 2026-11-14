//! Who the app is: names, version, window defaults, install location.
//!
//! The template stamps one `AppIdentity` into `src/brand.rs` (`new-app.sh`),
//! with the version from `../VERSION` via `build.rs`:
//!
//! ```
//! use bato_shell::AppIdentity;
//! pub const APP: AppIdentity = AppIdentity::new("Bato Notes", "batonotes", "1.4.0");
//! assert_eq!(APP.desktop_entry(), "bato-batonotes");
//! ```

use std::path::{Path, PathBuf};

use crate::settings::Settings;

/// Overrides the Wayland app_id / X11 WM_CLASS (off-screen test routing,
/// side-by-side dev builds).
pub const ENV_APP_ID: &str = "BATO_APP_ID";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AppIdentity {
    /// Window title, notification app name (e.g. "Bato Notes").
    pub product_name: &'static str,
    /// Registry name, config dir, binary name, default app_id (e.g. "batonotes").
    pub slug: &'static str,
    /// The running version (`x.y.z`), compared with the registry's `latest`.
    pub version: &'static str,
    /// PNG for the X11 window icon. Wayland takes the icon from the
    /// `.desktop` file `bato install` writes, so this is optional.
    pub icon_png: Option<&'static [u8]>,
}

/// The template's name for it (`brand.rs`).
pub type Brand = AppIdentity;

impl AppIdentity {
    pub const fn new(
        product_name: &'static str,
        slug: &'static str,
        version: &'static str,
    ) -> Self {
        Self {
            product_name,
            slug,
            version,
            icon_png: None,
        }
    }

    pub const fn with_icon(mut self, png: &'static [u8]) -> Self {
        self.icon_png = Some(png);
        self
    }

    /// Wayland app_id and X11 WM_CLASS: `BATO_APP_ID`, else the slug.
    ///
    /// iced 0.14 sets both from `application_id`, so `bato.json`'s
    /// `desktop.startupWMClass` is the slug too.
    pub fn app_id(&self) -> String {
        std::env::var(ENV_APP_ID)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| self.slug.to_string())
    }

    /// The launcher `bato install` writes is `bato-<slug>.desktop`; its id is
    /// what the notification server uses to find the app's icon and name.
    pub fn desktop_entry(&self) -> String {
        format!("bato-{}", self.slug)
    }

    /// `~/.config/<slug>/settings.json`, shared with the Electron client.
    pub fn settings_path(&self) -> PathBuf {
        Settings::path(self.slug)
    }

    /// Never fails: a missing or corrupt file gives defaults.
    pub fn load_settings(&self) -> Settings {
        Settings::load(self.slug)
    }

    pub fn save_settings(&self, settings: &Settings) -> std::io::Result<()> {
        settings.save(self.slug)
    }

    /// `~/.local/share/bato/apps/<slug>`, where `bato install` unpacks a
    /// `desktop` release.
    pub fn install_dir(&self) -> PathBuf {
        home().join(".local/share/bato/apps").join(self.slug)
    }

    /// True when this process runs from the `bato install` location, i.e. it
    /// is a packaged build that can update itself.
    pub fn is_installed_build(&self) -> bool {
        current_exe().is_some_and(|exe| is_inside(&exe, &self.install_dir()))
    }
}

/// Window geometry and chrome, same as the Electron `BrowserWindow`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowDefaults {
    pub width: f32,
    pub height: f32,
    pub min_width: f32,
    pub min_height: f32,
    /// `true`: the compositor decides. GNOME and KDE draw a title bar,
    /// Hyprland draws none. (Decision 2026-11-14; the PoC had `false`, which
    /// leaves a GNOME window without title bar or close button.)
    pub decorations: bool,
}

pub const WINDOW: WindowDefaults = WindowDefaults {
    width: 1280.0,
    height: 800.0,
    min_width: 900.0,
    min_height: 600.0,
    decorations: true,
};

pub(crate) fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// The running binary, with the ` (deleted)` suffix Linux appends to
/// `/proc/self/exe` once `bato install` has unlinked it for an upgrade.
pub(crate) fn current_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let s = exe.to_str()?;
    Some(PathBuf::from(s.strip_suffix(" (deleted)").unwrap_or(s)))
}

fn is_inside(path: &Path, dir: &Path) -> bool {
    !dir.as_os_str().is_empty() && path.starts_with(dir) && path != dir
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: AppIdentity = AppIdentity::new("Bato Notes", "batonotes", "1.4.0");

    #[test]
    fn names() {
        assert_eq!(APP.desktop_entry(), "bato-batonotes");
        assert!(APP.settings_path().ends_with("batonotes/settings.json"));
        assert!(
            APP.install_dir()
                .ends_with(".local/share/bato/apps/batonotes")
        );
        // The test binary lives in target/, not in the install dir.
        assert!(!APP.is_installed_build());
    }

    #[test]
    fn inside() {
        let dir = Path::new("/h/.local/share/bato/apps/x");
        assert!(is_inside(Path::new("/h/.local/share/bato/apps/x/x"), dir));
        assert!(!is_inside(Path::new("/h/.local/share/bato/apps/xy/x"), dir));
        assert!(!is_inside(dir, dir));
    }

    #[test]
    fn window_defaults_match_electron() {
        assert_eq!((WINDOW.width, WINDOW.height), (1280.0, 800.0));
        assert_eq!((WINDOW.min_width, WINDOW.min_height), (900.0, 600.0));
        const { assert!(WINDOW.decorations) };
    }
}
