//! Desktop notifications (`org.freedesktop.Notifications` over the session
//! bus), with click-to-focus.
//!
//! Each notification carries the app's product name, the `desktop-entry`
//! hint `bato-<slug>` (the launcher `bato install` writes, so the server
//! shows the registry icon), `suppress-sound` when silent, and a `default`
//! action. Clicking it emits `ActionInvoked`; [`Notifier::clicks`] turns that
//! into a stream of [`Click`]s for *this* app's notifications only, with the
//! xdg-activation token when the server sends one. The app then focuses its
//! window (iced: `window::gain_focus`; see `iced_ext`).
//!
//! zbus runs on `async-io`, so these futures work on any executor (iced's
//! thread pool included) and need no tokio.
//!
//! Windows: a notification-area balloon (a toast on Windows 10 and 11) from
//! a hidden icon that exists only while a notification shows; a click on it
//! is a [`Click`] with the `default` action.

#[cfg(unix)]
use std::collections::{HashMap, HashSet};
#[cfg(unix)]
use std::hash::{Hash, Hasher};
#[cfg(unix)]
use std::sync::{Arc, Mutex};

#[cfg(unix)]
use futures::StreamExt;
#[cfg(unix)]
use futures::stream::BoxStream;
#[cfg(unix)]
use zbus::message::Type as MessageType;
#[cfg(unix)]
use zbus::zvariant::Value;
#[cfg(unix)]
use zbus::{Connection, MatchRule, MessageStream};

#[cfg(unix)]
use crate::identity::AppIdentity;

#[cfg(windows)]
#[path = "notify_win.rs"]
mod win;
#[cfg(windows)]
pub use win::Notifier;

pub const BUS_NAME: &str = "org.freedesktop.Notifications";
pub const PATH: &str = "/org/freedesktop/Notifications";
pub const INTERFACE: &str = "org.freedesktop.Notifications";
/// The action key a click on the notification body invokes.
pub const DEFAULT_ACTION: &str = "default";
#[cfg(unix)]
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Electron's `notify({ title, body, silent })`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub silent: bool,
}

impl Notification {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            silent: false,
        }
    }

    pub fn silent(mut self, silent: bool) -> Self {
        self.silent = silent;
        self
    }
}

/// A click on one of this app's notifications.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Click {
    pub id: u32,
    /// `default` for the body; another key if the app adds actions later.
    pub action: String,
    /// xdg-activation token sent just before the click (spec 1.2), for
    /// compositors that only let a window focus itself with one.
    pub activation_token: Option<String>,
}

#[cfg(unix)]
/// Sends notifications and listens for clicks on them. Cheap to clone: the
/// session-bus connection and the set of ids sent are shared.
#[derive(Clone)]
pub struct Notifier {
    app_name: String,
    desktop_entry: String,
    conn: Arc<Mutex<Option<Connection>>>,
    sent: Arc<Mutex<HashSet<u32>>>,
}

#[cfg(unix)]
impl std::fmt::Debug for Notifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notifier")
            .field("app_name", &self.app_name)
            .field("desktop_entry", &self.desktop_entry)
            .finish()
    }
}

#[cfg(unix)]
/// Identity for iced's `Subscription::run_with`: one click listener per app.
impl Hash for Notifier {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.app_name.hash(state);
        self.desktop_entry.hash(state);
    }
}

#[cfg(unix)]
impl Notifier {
    /// Connects to the session bus lazily, on the first notification.
    pub fn new(app: &AppIdentity) -> Self {
        Self {
            app_name: app.product_name.to_string(),
            desktop_entry: app.desktop_entry(),
            conn: Arc::default(),
            sent: Arc::default(),
        }
    }

    /// Use a given connection (a private bus or a peer-to-peer mock in tests).
    pub fn with_connection(app: &AppIdentity, conn: Connection) -> Self {
        let n = Self::new(app);
        *n.conn.lock().unwrap() = Some(conn);
        n
    }

    async fn connection(&self) -> Option<Connection> {
        if let Some(c) = self.conn.lock().unwrap().clone() {
            return Some(c);
        }
        // A timeout, so a wedged notification server cannot leave the call
        // (and the task awaiting it) pending forever.
        let c = zbus::connection::Builder::session()
            .ok()?
            .method_timeout(CALL_TIMEOUT)
            .build()
            .await
            .ok()?;
        Some(self.conn.lock().unwrap().get_or_insert(c).clone())
    }

    /// Show a notification. `None` when there is no session bus or no
    /// notification server answers (Electron's `notify` returning false).
    pub async fn notify(&self, n: &Notification) -> Option<u32> {
        let conn = self.connection().await?;
        let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
        hints.insert("desktop-entry", Value::from(self.desktop_entry.as_str()));
        if n.silent {
            hints.insert("suppress-sound", Value::from(true));
        }
        let actions = [DEFAULT_ACTION, "Open"];
        let reply = conn
            .call_method(
                Some(BUS_NAME),
                PATH,
                Some(INTERFACE),
                "Notify",
                &(
                    self.app_name.as_str(),
                    0u32,
                    "",
                    n.title.as_str(),
                    n.body.as_str(),
                    &actions[..],
                    hints,
                    -1i32,
                ),
            )
            .await
            .ok()?;
        let id: u32 = reply.body().deserialize().ok()?;
        self.sent.lock().unwrap().insert(id);
        Some(id)
    }

    /// [`notify`](Self::notify) for callers without an executor; blocks
    /// until the server answers. Returns whether it was shown.
    pub fn notify_blocking(&self, n: &Notification) -> bool {
        futures::executor::block_on(self.notify(n)).is_some()
    }

    /// Clicks on notifications this `Notifier` (or a clone) sent. `None`
    /// without a session bus. The stream ends when the bus connection does.
    pub async fn clicks(&self) -> Option<BoxStream<'static, Click>> {
        let conn = self.connection().await?;
        let rule = MatchRule::builder()
            .msg_type(MessageType::Signal)
            .interface(INTERFACE)
            .ok()?
            .path(PATH)
            .ok()?
            .build();
        let stream = MessageStream::for_match_rule(rule, &conn, Some(64))
            .await
            .ok()?;
        let sent = self.sent.clone();
        let tokens: Arc<Mutex<HashMap<u32, String>>> = Arc::default();
        Some(
            stream
                .filter_map(move |msg| {
                    let sent = sent.clone();
                    let tokens = tokens.clone();
                    async move {
                        let msg = msg.ok()?;
                        let member = msg.header().member()?.to_string();
                        let body = msg.body();
                        let mine = |id: &u32| sent.lock().unwrap().contains(id);
                        match member.as_str() {
                            "ActivationToken" => {
                                let (id, token): (u32, String) = body.deserialize().ok()?;
                                if mine(&id) {
                                    tokens.lock().unwrap().insert(id, token);
                                }
                                None
                            }
                            "ActionInvoked" => {
                                let (id, action): (u32, String) = body.deserialize().ok()?;
                                if !mine(&id) {
                                    return None;
                                }
                                let activation_token = tokens.lock().unwrap().remove(&id);
                                Some(Click {
                                    id,
                                    action,
                                    activation_token,
                                })
                            }
                            "NotificationClosed" => {
                                let (id, _reason): (u32, u32) = body.deserialize().ok()?;
                                sent.lock().unwrap().remove(&id);
                                tokens.lock().unwrap().remove(&id);
                                None
                            }
                            _ => None,
                        }
                    }
                })
                .boxed(),
        )
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::sync::Mutex as StdMutex;
    use zbus::object_server::SignalEmitter;

    const APP: AppIdentity = AppIdentity::new("Bato Notes", "batonotes", "1.0.0");

    type Calls = Arc<StdMutex<Vec<(String, String, String, Vec<String>, HashMap<String, String>)>>>;

    /// A notification server on a peer-to-peer connection: records calls,
    /// emits signals on demand. Nothing reaches the real desktop.
    struct MockServer {
        calls: Calls,
        next: u32,
    }

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl MockServer {
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &mut self,
            app_name: String,
            _replaces_id: u32,
            _app_icon: String,
            summary: String,
            body: String,
            actions: Vec<String>,
            hints: HashMap<String, zbus::zvariant::OwnedValue>,
            _expire_timeout: i32,
        ) -> u32 {
            let hints = hints
                .into_iter()
                .map(|(k, v)| (k, format!("{v:?}")))
                .collect();
            self.calls
                .lock()
                .unwrap()
                .push((app_name, summary, body, actions, hints));
            self.next += 1;
            self.next
        }

        #[zbus(signal)]
        async fn action_invoked(
            e: &SignalEmitter<'_>,
            id: u32,
            action_key: &str,
        ) -> zbus::Result<()>;
        #[zbus(signal)]
        async fn activation_token(e: &SignalEmitter<'_>, id: u32, token: &str) -> zbus::Result<()>;
    }

    async fn pair(serve: bool) -> (Connection, Connection, Calls) {
        let (a, b) = UnixStream::pair().unwrap();
        let calls: Calls = Arc::default();
        let guid = zbus::Guid::generate();
        let mut server = zbus::connection::Builder::async_io_unix_stream(a)
            .server(guid)
            .unwrap()
            .p2p();
        if serve {
            server = server
                .serve_at(
                    PATH,
                    MockServer {
                        calls: calls.clone(),
                        next: 0,
                    },
                )
                .unwrap();
        }
        // A served call must never time out, however loaded the machine
        // (a parallel build can stall the mock server past 300 ms); only the
        // unserved peer relies on the timeout.
        let timeout = if serve { 60_000 } else { 300 };
        let client = zbus::connection::Builder::async_io_unix_stream(b)
            .p2p()
            .method_timeout(std::time::Duration::from_millis(timeout));
        let (s, c) = futures::join!(server.build(), client.build());
        (s.unwrap(), c.unwrap(), calls)
    }

    #[test]
    fn notify_sends_identity_hints_and_default_action() {
        futures::executor::block_on(async {
            let (_server, client, calls) = pair(true).await;
            let n = Notifier::with_connection(&APP, client);
            assert_eq!(
                n.notify(&Notification::new("Saved", "3 notes").silent(true))
                    .await,
                Some(1)
            );
            assert_eq!(n.notify(&Notification::new("Loud", "")).await, Some(2));
            let calls = calls.lock().unwrap();
            let (app, title, body, actions, hints) = &calls[0];
            assert_eq!(
                (app.as_str(), title.as_str(), body.as_str()),
                ("Bato Notes", "Saved", "3 notes")
            );
            assert_eq!(actions, &["default", "Open"]);
            assert!(
                hints["desktop-entry"].contains("bato-batonotes"),
                "{hints:?}"
            );
            assert!(hints["suppress-sound"].contains("true"), "{hints:?}");
            assert!(!calls[1].4.contains_key("suppress-sound"));
        });
    }

    #[test]
    fn no_server_means_false() {
        futures::executor::block_on(async {
            // A peer that serves no Notifications object never answers; the
            // call times out instead of hanging. (A bus without a server
            // answers ServiceUnknown at once.)
            let (_server, client, _) = pair(false).await;
            assert_eq!(
                Notifier::with_connection(&APP, client)
                    .notify(&Notification::new("x", "y"))
                    .await,
                None
            );
        });
    }

    #[test]
    fn clicks_on_own_notifications_only_with_token() {
        futures::executor::block_on(async {
            let (server, client, _) = pair(true).await;
            let n = Notifier::with_connection(&APP, client);
            let mut clicks = Box::pin(n.clicks().await.unwrap());
            let id = n.notify(&Notification::new("Hi", "")).await.unwrap();

            let iface = server
                .object_server()
                .interface::<_, MockServer>(PATH)
                .await
                .unwrap();
            let e = iface.signal_emitter();
            // Another app's notification: ignored.
            MockServer::action_invoked(e, 999, "default").await.unwrap();
            MockServer::activation_token(e, id, "tok-1").await.unwrap();
            MockServer::action_invoked(e, id, "default").await.unwrap();

            let click = clicks.next().await.unwrap();
            assert_eq!(
                click,
                Click {
                    id,
                    action: "default".into(),
                    activation_token: Some("tok-1".into())
                }
            );
        });
    }
}
