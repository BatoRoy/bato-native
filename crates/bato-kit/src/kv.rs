//! A small key-value store for UI state (INF-15), the native `localStorage`
//! for things like panel sizes and the dashboard layout
//! (`app:demo-panels-h`, `app:demo-dashboard`).
//!
//! One JSON object in `~/.config/<slug>/ui-state.json` (`XDG_CONFIG_HOME`
//! honoured), separate from `settings.json`. Every `set` writes the file
//! atomically; a missing or corrupt file reads as empty.
//!
//! ```ignore
//! let mut kv = UiState::load("app");
//! let sizes: Vec<f32> = kv.get("demo-panels-h").unwrap_or(vec![30.0, 70.0]);
//! kv.set("demo-panels-h", &sizes)?;
//! ```

use std::path::{Path, PathBuf};
use std::{fs, io};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

#[derive(Debug, Clone)]
pub struct UiState {
    path: PathBuf,
    map: Map<String, Value>,
}

impl UiState {
    /// `~/.config/<slug>/ui-state.json`.
    pub fn path(slug: &str) -> PathBuf {
        bato_theme::settings::config_dir(slug).join("ui-state.json")
    }

    pub fn load(slug: &str) -> Self {
        Self::at(Self::path(slug))
    }

    /// A store backed by any file.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let map = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, map }
    }

    pub fn file(&self) -> &Path {
        &self.path
    }

    /// The value at `key`, if present and of the right shape.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.map
            .get(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    pub fn contains(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    /// Store `value` at `key` and write the file.
    pub fn set<T: Serialize + ?Sized>(&mut self, key: &str, value: &T) -> io::Result<()> {
        self.map
            .insert(key.to_string(), serde_json::to_value(value)?);
        self.save()
    }

    pub fn remove(&mut self, key: &str) -> io::Result<()> {
        if self.map.remove(key).is_some() {
            self.save()
        } else {
            Ok(())
        }
    }

    fn save(&self) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(&self.map)?)?;
        fs::rename(tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_corrupt_file() {
        let dir = std::env::temp_dir().join(format!("bato-kit-kv-{}", std::process::id()));
        let file = dir.join("ui-state.json");
        let _ = fs::remove_dir_all(&dir);
        let mut kv = UiState::at(&file);
        assert_eq!(kv.get::<Vec<f32>>("panels"), None);
        kv.set("panels", &vec![30.0f32, 70.0]).unwrap();
        kv.set("name", "x").unwrap();
        let again = UiState::at(&file);
        assert_eq!(again.get::<Vec<f32>>("panels"), Some(vec![30.0, 70.0]));
        assert_eq!(again.get::<String>("name").as_deref(), Some("x"));
        assert_eq!(
            again.get::<u32>("name"),
            None,
            "wrong shape reads as absent"
        );
        fs::write(&file, "{not json").unwrap();
        assert!(!UiState::at(&file).contains("panels"));
        let _ = fs::remove_dir_all(&dir);
    }
}
