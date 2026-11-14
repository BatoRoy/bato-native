//! `~/.config/<slug>/settings.json`, the file the Electron client also uses.
//!
//! [`Settings`] is `bato_theme`'s struct: the look settings plus `backendUrl`,
//! with every other key kept verbatim in `extra`, so an app switching between
//! the Electron and native clients never loses data. [`SettingsExt`] gives
//! typed access to an app's own keys in `extra`:
//!
//! ```
//! use bato_shell::{Settings, SettingsExt};
//! let mut s = Settings::default();
//! s.set_extra("lastProject", "/home/me/notes").unwrap();
//! assert_eq!(s.extra_or("lastProject", String::new()), "/home/me/notes");
//! assert!(s.set_extra("theme", "dark").is_err()); // a typed field, not an extra
//! ```

use std::sync::OnceLock;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

pub use bato_theme::settings::{Settings, config_dir};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtraError {
    /// The key belongs to a typed [`Settings`] field (`theme`, `backendUrl`, …).
    /// Writing it to `extra` would serialise the key twice.
    Reserved(String),
    Serialize(String),
}

impl std::fmt::Display for ExtraError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reserved(k) => write!(f, "'{k}' is a typed settings field, not an extra key"),
            Self::Serialize(e) => write!(f, "could not serialise the value: {e}"),
        }
    }
}

impl std::error::Error for ExtraError {}

/// Typed access to app-specific keys (`Settings::extra`).
pub trait SettingsExt {
    /// The value under `key`, or `None` when it is missing or has another shape.
    fn extra<T: DeserializeOwned>(&self, key: &str) -> Option<T>;
    fn extra_or<T: DeserializeOwned>(&self, key: &str, default: T) -> T {
        self.extra(key).unwrap_or(default)
    }
    fn set_extra<T: Serialize>(&mut self, key: &str, value: T) -> Result<(), ExtraError>;
    fn remove_extra(&mut self, key: &str) -> Option<Value>;
    /// Save a backend address the way `bridge.ts` `setBackendUrl` does
    /// (normalised); an empty string clears it.
    fn set_backend_url(&mut self, url: &str);
}

impl SettingsExt for Settings {
    fn extra<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        serde_json::from_value(self.extra.get(key)?.clone()).ok()
    }

    fn set_extra<T: Serialize>(&mut self, key: &str, value: T) -> Result<(), ExtraError> {
        if reserved_keys().iter().any(|k| k == key) {
            return Err(ExtraError::Reserved(key.to_string()));
        }
        let v = serde_json::to_value(value).map_err(|e| ExtraError::Serialize(e.to_string()))?;
        self.extra.insert(key.to_string(), v);
        Ok(())
    }

    fn remove_extra(&mut self, key: &str) -> Option<Value> {
        self.extra.remove(key)
    }

    fn set_backend_url(&mut self, url: &str) {
        let url = crate::backend::normalize(url);
        self.backend_url = (!url.is_empty()).then_some(url);
    }
}

/// The JSON keys of the typed fields, read from `Settings` itself so this list
/// follows `bato-theme` when it grows a field.
pub fn reserved_keys() -> &'static [String] {
    static KEYS: OnceLock<Vec<String>> = OnceLock::new();
    KEYS.get_or_init(|| {
        let full = Settings {
            backend_url: Some(String::new()),
            accent: Some(String::new()),
            ..Settings::default()
        };
        match serde_json::to_value(full) {
            Ok(Value::Object(m)) => m.keys().cloned().collect(),
            _ => Vec::new(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reserved_keys_cover_typed_fields() {
        let keys = reserved_keys();
        for k in [
            "backendUrl",
            "theme",
            "accent",
            "scale",
            "wide",
            "textSelect",
            "sidebarCollapsed",
        ] {
            assert!(keys.iter().any(|x| x == k), "{k} missing from {keys:?}");
        }
    }

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Recent {
        paths: Vec<String>,
        pinned: bool,
    }

    /// Electron writes a file, native reads it, sets its own keys, saves; the
    /// Electron-only keys survive and the typed extras read back.
    #[test]
    fn round_trip_with_typed_extras_and_foreign_keys() {
        let dir = std::env::temp_dir().join(format!("bato-shell-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &path,
            r#"{"backendUrl":"http://10.0.0.5:42100/","someElectronKey":[1,2],"windowWidth":1111}"#,
        )
        .unwrap();

        let mut s = Settings::load_from(&path);
        assert_eq!(s.extra::<u32>("windowWidth"), Some(1111));
        assert_eq!(
            s.extra::<String>("windowWidth"),
            None,
            "wrong shape is None, not a panic"
        );
        let recent = Recent {
            paths: vec!["/a".into(), "/b".into()],
            pinned: true,
        };
        s.set_extra("recent", &recent).unwrap();
        s.set_backend_url("  http://10.0.0.6:42100//  ");
        s.save_to(&path).unwrap();

        let back = Settings::load_from(&path);
        assert_eq!(back.extra::<Recent>("recent"), Some(recent));
        assert_eq!(back.backend_url.as_deref(), Some("http://10.0.0.6:42100"));
        let raw: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["someElectronKey"], serde_json::json!([1, 2]));
        assert_eq!(raw["windowWidth"], 1111);

        let mut cleared = back;
        cleared.set_backend_url("   ");
        assert_eq!(cleared.backend_url, None);
        assert!(cleared.remove_extra("recent").is_some());
        assert_eq!(cleared.extra_or("recent", 7), 7);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn typed_field_names_are_rejected_as_extras() {
        let mut s = Settings::default();
        assert_eq!(
            s.set_extra("backendUrl", "x"),
            Err(ExtraError::Reserved("backendUrl".into()))
        );
        assert!(s.extra.is_empty());
    }
}
