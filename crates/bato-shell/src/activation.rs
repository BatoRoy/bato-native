//! Raise an existing window (feature `activation`).
//!
//! What winit 0.30 (and so iced 0.14) offers: an activation token only for a
//! window being *created* (`with_activation_token`), and
//! `request_user_attention`, which asks for a token for itself (the
//! compositor usually only marks the window urgent). `focus_window`, behind
//! iced's `window::gain_focus`, is a no-op on Wayland. Nothing takes a
//! token for a window that already exists, which is what a forwarded launch
//! or a notification click carries.
//!
//! So this does `xdg_activation_v1.activate(token, surface)` itself, on the
//! window's own Wayland connection (proxies on the `wl_display` winit
//! opened, as ashpd does for xdg-foreign). Without a token it asks the
//! compositor for one first, for its own surface.
//!
//! Whether focus actually moves is the compositor's policy: KDE and GNOME
//! honour a launcher's token; Hyprland focuses only with
//! `misc:focus_on_activate = true` and otherwise marks the window urgent.
//! On X11 [`activate`] returns `false`: use `window::gain_focus`
//! (`_NET_ACTIVE_WINDOW`), which `iced_ext::activate` does.

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

/// Ask the compositor to activate `window`, with `token` when there is one.
/// `true` when the request was sent (Wayland with xdg-activation).
///
/// Call it where the window is borrowed (iced: `window::run`).
pub fn activate<W>(window: &W, token: Option<&str>) -> bool
where
    W: HasWindowHandle + HasDisplayHandle + ?Sized,
{
    let (Ok(w), Ok(d)) = (window.window_handle(), window.display_handle()) else {
        return false;
    };
    // SAFETY: both handles are borrowed from `window` for this whole call.
    unsafe { activate_raw(w.as_raw(), d.as_raw(), token) }
}

/// [`activate`] on raw handles.
///
/// # Safety
///
/// For Wayland handles, `surface` and `display` must be a live `wl_surface`
/// and `wl_display` for the duration of the call.
pub unsafe fn activate_raw(
    window: RawWindowHandle,
    display: RawDisplayHandle,
    token: Option<&str>,
) -> bool {
    match (window, display) {
        (RawWindowHandle::Wayland(w), RawDisplayHandle::Wayland(d)) => {
            // SAFETY: forwarded from the caller.
            unsafe { wayland::activate(w.surface.as_ptr(), d.display.as_ptr(), token) }
                .inspect_err(|e| eprintln!("[bato-shell] activation: {e}"))
                .is_ok()
        }
        _ => false,
    }
}

mod wayland {
    use wayland_backend::sys::client::{Backend, ObjectId};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_registry, wl_surface::WlSurface};
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
    use wayland_protocols::xdg::activation::v1::client::{
        xdg_activation_token_v1::{self, XdgActivationTokenV1},
        xdg_activation_v1::XdgActivationV1,
    };

    #[derive(Default)]
    struct State {
        token: Option<String>,
    }

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<XdgActivationV1, ()> for State {
        fn event(
            _: &mut Self,
            _: &XdgActivationV1,
            _: <XdgActivationV1 as Proxy>::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<XdgActivationTokenV1, ()> for State {
        fn event(
            state: &mut Self,
            _: &XdgActivationTokenV1,
            event: xdg_activation_token_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_activation_token_v1::Event::Done { token } = event {
                state.token = Some(token);
            }
        }
    }

    /// # Safety
    /// Live `wl_surface` and `wl_display` pointers.
    pub(super) unsafe fn activate(
        surface: *mut std::ffi::c_void,
        display: *mut std::ffi::c_void,
        token: Option<&str>,
    ) -> Result<(), String> {
        // SAFETY: the caller guarantees a live display; the backend only
        // borrows it (winit keeps owning it).
        let backend = unsafe { Backend::from_foreign_display(display.cast()) };
        let conn = Connection::from_backend(backend);
        // SAFETY: the caller guarantees a live wl_surface.
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }
            .map_err(|e| e.to_string())?;
        let surface = WlSurface::from_id(&conn, id).map_err(|e| e.to_string())?;
        let (globals, mut queue) =
            registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let activation: XdgActivationV1 = globals
            .bind(&qh, 1..=1, ())
            .map_err(|_| "the compositor has no xdg_activation_v1".to_string())?;
        let mut state = State::default();
        let token = match token.filter(|t| !t.is_empty()) {
            Some(t) => t.to_string(),
            None => {
                let req = activation.get_activation_token(&qh, ());
                req.set_surface(&surface);
                req.commit();
                for _ in 0..4 {
                    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
                    if state.token.is_some() {
                        break;
                    }
                }
                req.destroy();
                state
                    .token
                    .take()
                    .ok_or("the compositor sent no activation token")?
            }
        };
        activation.activate(token, &surface);
        activation.destroy();
        conn.flush().map_err(|e| e.to_string())
    }
}
