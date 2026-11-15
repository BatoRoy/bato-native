//! Which backend the app talks to. Same rule as `bridge.ts` and the QML
//! flavor, first match wins:
//!
//! 1. `BATO_BACKEND_URL` in the environment (a supervisor, a dev shell);
//! 2. the saved `backendUrl` (the user chose a server: **spawn nothing**);
//! 3. the bundled server, when the app has one and it started;
//! 4. `http://localhost:8080` (dev: `make dev-server`).
//!
//! Every URL is normalised: trimmed, trailing slashes removed. A blank value
//! counts as absent.
//!
//! ```
//! use bato_shell::backend::{resolve, Source};
//! let b = resolve(None, Some(" http://nas.lan:42100/ "), Some("http://127.0.0.1:40123"));
//! assert_eq!((b.url.as_str(), b.source), ("http://nas.lan:42100", Source::Saved));
//! ```

use crate::settings::Settings;

pub const ENV_BACKEND_URL: &str = "BATO_BACKEND_URL";
pub const DEFAULT_BACKEND: &str = "http://localhost:8080";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    Env,
    Saved,
    Bundled,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Backend {
    pub url: String,
    pub source: Source,
}

/// `bridge.ts` `normalize`: trim, drop trailing slashes.
pub fn normalize(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn present(v: Option<&str>) -> Option<String> {
    v.map(normalize).filter(|s| !s.is_empty())
}

/// The pure rule; [`resolve_for`] reads the environment and settings.
pub fn resolve(env: Option<&str>, saved: Option<&str>, bundled: Option<&str>) -> Backend {
    let pick = |url, source| Backend { url, source };
    present(env)
        .map(|u| pick(u, Source::Env))
        .or_else(|| present(saved).map(|u| pick(u, Source::Saved)))
        .or_else(|| present(bundled).map(|u| pick(u, Source::Bundled)))
        .unwrap_or_else(|| pick(DEFAULT_BACKEND.to_string(), Source::Default))
}

/// A bundled server may be spawned only when nothing outranks it: no
/// `BATO_BACKEND_URL` and no saved address (the QML rule; otherwise "point at
/// my daemon" silently starts a second server).
pub fn may_spawn_bundled(env: Option<&str>, saved: Option<&str>) -> bool {
    present(env).is_none() && present(saved).is_none()
}

pub fn env_url() -> Option<String> {
    std::env::var(ENV_BACKEND_URL).ok()
}

/// [`resolve`] with `BATO_BACKEND_URL` and `settings.backend_url`.
pub fn resolve_for(settings: &Settings, bundled: Option<&str>) -> Backend {
    resolve(
        env_url().as_deref(),
        settings.backend_url.as_deref(),
        bundled,
    )
}

/// [`may_spawn_bundled`] with `BATO_BACKEND_URL` and `settings.backend_url`.
pub fn may_spawn_bundled_for(settings: &Settings) -> bool {
    may_spawn_bundled(env_url().as_deref(), settings.backend_url.as_deref())
}

/// Whether the saved `backendUrl` takes part in the rule.
///
/// An app without the Server address card (`Frame::without_server_card`:
/// BatoFetch, BatoBrowse, whose remotes are connections, not "the backend")
/// has no way to show or clear a saved address, so one left by an older
/// client must neither retarget the app nor stop the bundled spawn (R19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SavedUrl {
    /// Steps 1-4 as above (the template, with the card).
    #[default]
    Honor,
    /// Step 2 is skipped: `BATO_BACKEND_URL`, then the bundled server.
    Ignore,
}

impl SavedUrl {
    fn pick(self, settings: &Settings) -> Option<&str> {
        match self {
            Self::Honor => settings.backend_url.as_deref(),
            Self::Ignore => None,
        }
    }
}

/// [`resolve_for`], with the saved address honoured or ignored.
pub fn resolve_with(settings: &Settings, bundled: Option<&str>, saved: SavedUrl) -> Backend {
    resolve(env_url().as_deref(), saved.pick(settings), bundled)
}

/// [`may_spawn_bundled_for`], with the saved address honoured or ignored.
pub fn may_spawn_bundled_with(settings: &Settings, saved: SavedUrl) -> bool {
    may_spawn_bundled(env_url().as_deref(), saved.pick(settings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence() {
        let env = Some("http://env:1");
        let saved = Some("http://saved:2");
        let bundled = Some("http://127.0.0.1:3");
        assert_eq!(
            resolve(env, saved, bundled),
            Backend {
                url: "http://env:1".into(),
                source: Source::Env
            }
        );
        assert_eq!(resolve(None, saved, bundled).source, Source::Saved);
        assert_eq!(
            resolve(None, None, bundled),
            Backend {
                url: "http://127.0.0.1:3".into(),
                source: Source::Bundled
            }
        );
        assert_eq!(
            resolve(None, None, None),
            Backend {
                url: DEFAULT_BACKEND.into(),
                source: Source::Default
            }
        );
    }

    #[test]
    fn blank_counts_as_absent_and_urls_are_normalised() {
        assert_eq!(resolve(Some("   "), Some(""), None).source, Source::Default);
        assert_eq!(
            resolve(Some(" http://a:1/// "), None, None).url,
            "http://a:1"
        );
        assert_eq!(
            resolve(None, Some("http://nas.lan:42100/api/"), None).url,
            "http://nas.lan:42100/api"
        );
        assert_eq!(normalize("https://x.bato.lan/"), "https://x.bato.lan");
    }

    #[test]
    fn spawn_only_when_nothing_outranks_the_bundled_server() {
        assert!(may_spawn_bundled(None, None));
        assert!(may_spawn_bundled(Some(" "), Some("")));
        assert!(!may_spawn_bundled(Some("http://env:1"), None));
        assert!(!may_spawn_bundled(None, Some("http://saved:2")));
    }

    #[test]
    fn an_ignored_saved_url_neither_retargets_nor_blocks_the_spawn() {
        let s = Settings {
            backend_url: Some("http://old-daemon:42110".into()),
            ..Settings::default()
        };
        if env_url().is_some_and(|v| !v.trim().is_empty()) {
            return; // the environment outranks both; nothing to compare
        }
        assert!(!may_spawn_bundled_with(&s, SavedUrl::Honor));
        assert!(may_spawn_bundled_with(&s, SavedUrl::Ignore));
        let b = resolve_with(&s, Some("http://127.0.0.1:4"), SavedUrl::Ignore);
        assert_eq!(b.source, Source::Bundled);
        assert_eq!(
            resolve_with(&s, Some("http://127.0.0.1:4"), SavedUrl::Honor).source,
            Source::Saved
        );
    }
}
