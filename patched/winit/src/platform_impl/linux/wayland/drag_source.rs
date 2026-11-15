//! bato-native: OS drags of files from a window on Wayland
//! ([`crate::bato_drag`]).
//!
//! A `wl_data_source` offering `text/uri-list`, the paths as `text/plain` and
//! the bato marker type, started with `wl_data_device.start_drag` on the seat's
//! one data device (the one file drops and paste use: Hyprland talks only to a
//! client's first `wl_data_device`), from the window's surface, with the serial
//! of the button press that holds the implicit grab. The icon is a `wl_shm`
//! buffer on its own surface.
//!
//! | wl_data_source | here |
//! |---|---|
//! | `send` | write the data from a thread (never blocks the loop) |
//! | `action` | remember the chosen action |
//! | `dnd_drop_performed` | wait for `dnd_finished` (or `cancelled`) |
//! | `dnd_finished` | [`Outcome::Dropped`] with the last action |
//! | `cancelled` | [`Outcome::Cancelled`] |

use std::io::Write;
use std::sync::Weak;

use sctk::compositor::Surface;
use sctk::data_device_manager::data_source::{DataSourceHandler, DragSource};
use sctk::data_device_manager::WritePipe;
use sctk::reexports::client::protocol::wl_data_device_manager::DndAction;
use sctk::reexports::client::protocol::wl_data_source::WlDataSource;
use sctk::reexports::client::protocol::wl_shm;
use sctk::reexports::client::{Connection, Proxy, QueueHandle};
use sctk::shell::WaylandSurface;
use sctk::shm::slot::{Buffer, SlotPool};
use tracing::warn;

use crate::bato_drag::{self, Action, Done, Icon, Outcome, Request};
use crate::platform_impl::wayland::seat::WinitPointerDataExt;
use crate::platform_impl::wayland::state::WinitState;
use crate::platform_impl::wayland::WindowId;

const URI_LIST: &str = "text/uri-list";
const TEXT: [&str; 3] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];

/// The drag this process is the source of, if any.
#[derive(Default)]
pub struct DragSourceState {
    active: Option<Active>,
}

impl std::fmt::Debug for DragSourceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DragSourceState").field("active", &self.active.is_some()).finish()
    }
}

struct Active {
    source: DragSource,
    /// The icon surface, and the pool and buffer behind it.
    icon: Option<(Surface, SlotPool, Buffer)>,
    marker: String,
    uri_list: Vec<u8>,
    text: Vec<u8>,
    action: DndAction,
    done: Done,
}

impl WinitState {
    /// Start the drags queued by [`bato_drag::start`].
    pub fn bato_drag_requests(&mut self, queue_handle: &QueueHandle<Self>) {
        for (id, request) in bato_drag::take() {
            if self.drag_source.active.is_some() {
                (request.done)(Outcome::Failed("another drag is in progress".into()));
                continue;
            }
            if let Err((done, why)) = self.bato_drag_start(queue_handle, id, request) {
                warn!("cannot start the OS drag: {why}");
                done(Outcome::Failed(why));
            }
        }
    }

    fn bato_drag_start(
        &mut self,
        queue_handle: &QueueHandle<Self>,
        id: u64,
        request: Request,
    ) -> Result<(), (Done, String)> {
        let Request { window, paths, icon, allow_move, done } = request;
        macro_rules! fail {
            ($($why:tt)*) => { return Err((done, format!($($why)*))) };
        }
        let Some(manager) = self.data_device_manager.as_ref() else {
            fail!("no wl_data_device_manager");
        };
        let window_id = WindowId::from(window);
        let Some(window) = self.windows.borrow().get(&window_id).cloned() else {
            fail!("no window {window:#x}");
        };
        let window = window.lock().unwrap();
        let surface = window.window.wl_surface().clone();
        // The pointer holding a button on the window: its seat and press serial.
        let grab = window.pointers.iter().filter_map(Weak::upgrade).find_map(|pointer| {
            let data = pointer.pointer().winit_data();
            let winit = data.data();
            (winit.focused_window() == Some(window_id) && winit.bato_pressed())
                .then(|| data.latest_button_serial().map(|s| (data.seat().clone(), s)))
                .flatten()
        });
        drop(window);
        let Some((seat, serial)) = grab else {
            fail!("no pointer button is held on the window");
        };
        let Some(device) = self.seats.get(&seat.id()).and_then(|s| s.data_device.as_ref()) else {
            fail!("the seat has no data device");
        };

        let marker = bato_drag::marker(id);
        let mut mimes = vec![URI_LIST.to_owned()];
        mimes.extend(TEXT.iter().map(|t| t.to_string()));
        mimes.push(marker.clone());
        let actions = if allow_move { DndAction::Copy | DndAction::Move } else { DndAction::Copy };
        let source = manager.create_drag_and_drop_source(queue_handle, mimes, actions);

        let icon = icon.and_then(|icon| match self.bato_drag_icon(queue_handle, &icon) {
            Ok(icon) => Some(icon),
            Err(e) => {
                warn!("no drag icon: {e}");
                None
            },
        });
        source.start_drag(device, &surface, icon.as_ref().map(|i| i.0.wl_surface()), serial);
        if let Some((icon_surface, _, _)) = &icon {
            // After start_drag gave it the dnd-icon role.
            icon_surface.wl_surface().commit();
        }
        flush(&surface);
        self.drag_source.active = Some(Active {
            source,
            icon,
            marker,
            uri_list: bato_drag::uri_list(&paths).into_bytes(),
            text: bato_drag::plain_text(&paths).into_bytes(),
            action: DndAction::empty(),
            done,
        });
        Ok(())
    }

    /// The icon surface (not yet committed), its pool and buffer.
    fn bato_drag_icon(
        &self,
        queue_handle: &QueueHandle<Self>,
        icon: &Icon,
    ) -> Result<(Surface, SlotPool, Buffer), String> {
        let (w, h) = (icon.width as i32, icon.height as i32);
        if w <= 0 || h <= 0 || icon.rgba.len() != (w * h * 4) as usize {
            return Err(format!("bad icon: {w}x{h}, {} bytes", icon.rgba.len()));
        }
        let mut pool = SlotPool::new((w * h * 4) as usize, &self.shm).map_err(|e| e.to_string())?;
        let (buffer, canvas) =
            pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888).map_err(|e| e.to_string())?;
        bato_drag::premultiplied_argb(&icon.rgba, canvas);
        let surface =
            Surface::new(&*self.compositor_state, queue_handle).map_err(|e| e.to_string())?;
        let wl_surface = surface.wl_surface();
        let scale = icon.scale.max(1) as i32;
        if wl_surface.version() >= 3 {
            wl_surface.set_buffer_scale(scale);
        }
        buffer.attach_to(wl_surface).map_err(|e| e.to_string())?;
        // The icon's top-left starts at the pointer; move the hotspot under it.
        let (hx, hy) = icon.hotspot;
        if wl_surface.version() >= 5 {
            wl_surface.offset(-hx, -hy);
        }
        if wl_surface.version() >= 4 {
            wl_surface.damage_buffer(0, 0, w, h);
        } else {
            wl_surface.damage(0, 0, w / scale, h / scale);
        }
        Ok((surface, pool, buffer))
    }

    fn bato_drag_end(&mut self, source: &WlDataSource, outcome: impl FnOnce(DndAction) -> Outcome) {
        let Some(active) = self.drag_source.active.take_if(|a| a.source.inner() == source) else {
            return;
        };
        let outcome = outcome(active.action);
        // Dropping the DragSource destroys the source; the icon surface goes too.
        drop(active.source);
        if let Some((surface, _, _)) = &active.icon {
            surface.wl_surface().attach(None, 0, 0);
        }
        drop(active.icon);
        (active.done)(outcome);
    }
}

fn flush(proxy: &impl Proxy) {
    if let Some(backend) = proxy.backend().upgrade() {
        let _ = Connection::from_backend(backend).flush();
    }
}

impl DataSourceHandler for WinitState {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        pipe: WritePipe,
    ) {
        let Some(active) = self.drag_source.active.as_ref().filter(|a| a.source.inner() == source)
        else {
            return;
        };
        let data = if mime == URI_LIST {
            active.uri_list.clone()
        } else if TEXT.contains(&mime.as_str()) {
            active.text.clone()
        } else if mime == active.marker {
            Vec::new()
        } else {
            return;
        };
        // A pipe holds 64 KiB; a reader that is slow (or this very process,
        // reading its own drop through the loop) must not stall the loop.
        let _ = std::thread::Builder::new().name("bato-drag-send".into()).spawn(move || {
            let mut pipe = pipe;
            if let Err(e) = pipe.write_all(&data) {
                warn!("sending the dragged {mime} failed: {e}");
            }
        });
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        self.bato_drag_end(source, |_| Outcome::Cancelled);
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        self.bato_drag_end(source, |action| {
            Outcome::Dropped(if action == DndAction::Move { Action::Move } else { Action::Copy })
        });
    }

    fn action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        action: DndAction,
    ) {
        if let Some(active) =
            self.drag_source.active.as_mut().filter(|a| a.source.inner() == source)
        {
            active.action = action;
        }
    }
}
