//! bato-native: desktop → window file drops on Wayland.
//!
//! winit 0.30 only reports file drops on X11, Windows and macOS. This binds
//! `wl_data_device_manager`, gets a `wl_data_device` per seat and turns a
//! drag that offers `text/uri-list` into the events the X11 backend sends:
//!
//! | wl_data_device | winit |
//! |---|---|
//! | `enter` (uri-list offered) | accept `text/uri-list`, action copy; read the list; `HoveredFile` per file once read |
//! | `enter`, `motion` | `CursorMoved` (the pointer is grabbed by the drag, so the window gets no `wl_pointer` motion) |
//! | `leave` before a drop | `HoveredFileCancelled`, `CursorLeft` |
//! | `drop` | `DroppedFile` per file (once the list is read), then `wl_data_offer.finish` |
//!
//! Only the copy action is offered: accepting "move" would let a file
//! manager delete the originals. The list is read through the event loop
//! (the pipe is a calloop source), never blocking.

use std::io::{ErrorKind, Read};
use std::path::PathBuf;

use sctk::compositor::SurfaceData;
use sctk::data_device_manager::data_device::{DataDeviceData, DataDeviceHandler};
use sctk::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use sctk::data_device_manager::ReadPipe;
use sctk::reexports::calloop::{PostAction, RegistrationToken};
use sctk::reexports::client::protocol::wl_data_device::WlDataDevice;
use sctk::reexports::client::protocol::wl_data_device_manager::DndAction;
use sctk::reexports::client::protocol::wl_surface::WlSurface;
use sctk::reexports::client::{Connection, Proxy, QueueHandle};
use tracing::warn;

use crate::dpi::{LogicalPosition, PhysicalPosition};
use crate::event::WindowEvent;
use crate::platform_impl::wayland::state::WinitState;
use crate::platform_impl::wayland::{self, DeviceId, WindowId};

const URI_LIST: &str = "text/uri-list";

/// The file drag in progress, if any.
#[derive(Debug, Default)]
pub struct DndState {
    /// The window under the drag, and the offer.
    target: Option<(WindowId, DragOffer)>,
    /// The offered files, once the list is read.
    paths: Option<Vec<PathBuf>>,
    /// The pipe being read.
    reading: Option<RegistrationToken>,
    /// The compositor's latest action for the offer.
    action: Option<DndAction>,
    /// Dropped: deliver and finish once the list is read.
    dropped: bool,
    /// `HoveredFile` was sent for this drag.
    hovered: bool,
    /// Bumped per drag, so a read of an older drag is ignored.
    serial: u64,
}

fn device_id() -> crate::event::DeviceId {
    crate::event::DeviceId(crate::platform_impl::DeviceId::Wayland(DeviceId))
}

fn hostname() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/hostname").ok().map(|h| h.trim().to_owned())
}

impl WinitState {
    fn dnd_push(&mut self, event: WindowEvent, window_id: WindowId) {
        self.events_sink.push_window_event(event, window_id);
        self.dispatched_events = true;
    }

    /// The pointer position during the drag (surface-local, logical).
    fn dnd_cursor(&mut self, window_id: WindowId, x: f64, y: f64) {
        let scale = match self.windows.get_mut().get(&window_id) {
            Some(w) => w.lock().unwrap().scale_factor(),
            None => return,
        };
        let position: PhysicalPosition<f64> = LogicalPosition::new(x, y).to_physical(scale);
        self.dnd_push(WindowEvent::CursorMoved { device_id: device_id(), position }, window_id);
    }

    /// Forget the drag (and stop reading its list).
    fn dnd_reset(&mut self) {
        if let Some(token) = self.dnd.reading.take() {
            self.loop_handle.remove(token);
        }
        let serial = self.dnd.serial;
        self.dnd = DndState { serial, ..DndState::default() };
    }

    fn dnd_read(&mut self, pipe: ReadPipe) {
        let serial = self.dnd.serial;
        let mut list = Vec::new();
        let token = self.loop_handle.insert_source(pipe, move |_, file, state| {
            let mut chunk = [0u8; 4096];
            // SAFETY: the file is only read, never replaced or closed here.
            match unsafe { file.get_mut() }.read(&mut chunk) {
                Ok(0) => {
                    state.dnd_list(serial, std::mem::take(&mut list));
                    PostAction::Remove
                },
                Ok(n) => {
                    list.extend_from_slice(&chunk[..n]);
                    PostAction::Continue
                },
                Err(e) if matches!(e.kind(), ErrorKind::Interrupted | ErrorKind::WouldBlock) => {
                    PostAction::Continue
                },
                Err(e) => {
                    warn!("reading a dropped text/uri-list failed: {e}");
                    state.dnd_list(serial, Vec::new());
                    PostAction::Remove
                },
            }
        });
        match token {
            Ok(token) => self.dnd.reading = Some(token),
            Err(e) => warn!("cannot read a dropped text/uri-list: {e}"),
        }
    }

    /// The list of drag `serial` is in.
    fn dnd_list(&mut self, serial: u64, list: Vec<u8>) {
        if serial != self.dnd.serial {
            return;
        }
        self.dnd.reading = None;
        let Some((window_id, _)) = &self.dnd.target else { return };
        let window_id = *window_id;
        let paths = crate::bato_uri_list::parse(&list, hostname().as_deref());
        if !self.dnd.dropped {
            for p in &paths {
                self.dnd_push(WindowEvent::HoveredFile(p.clone()), window_id);
            }
            self.dnd.hovered = !paths.is_empty();
        }
        self.dnd.paths = Some(paths);
        self.dnd_finish();
    }

    /// After the drop, once the list is read: deliver the files, finish.
    fn dnd_finish(&mut self) {
        if !self.dnd.dropped || self.dnd.paths.is_none() {
            return;
        }
        let paths = self.dnd.paths.take().unwrap_or_default();
        if let Some((window_id, offer)) = self.dnd.target.take() {
            for p in paths {
                self.dnd_push(WindowEvent::DroppedFile(p), window_id);
            }
            // `finish` is only valid for an accepted copy (else a protocol error).
            if self.dnd.action == Some(DndAction::Copy) {
                offer.finish();
            }
            offer.destroy();
        }
        self.dnd_reset();
    }
}

impl DataDeviceHandler for WinitState {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        device: &WlDataDevice,
        x: f64,
        y: f64,
        surface: &WlSurface,
    ) {
        self.dnd_reset();
        let Some(offer) = device.data::<DataDeviceData>().and_then(|d| d.drag_offer()) else {
            return;
        };
        let parent = surface
            .data::<SurfaceData<()>>()
            .and_then(|d| d.parent_surface())
            .unwrap_or(surface);
        let window_id = wayland::make_wid(parent);
        let files = offer.with_mime_types(|m| m.iter().any(|t| t == URI_LIST));
        if !files || !self.windows.get_mut().contains_key(&window_id) {
            offer.accept_mime_type(offer.serial, None);
            offer.set_actions(DndAction::empty(), DndAction::empty());
            return;
        }
        offer.accept_mime_type(offer.serial, Some(URI_LIST.to_owned()));
        offer.set_actions(DndAction::Copy, DndAction::Copy);
        self.dnd.serial += 1;
        self.dnd.target = Some((window_id, offer.clone()));
        self.dnd_cursor(window_id, x, y);
        // Reading before the drop is allowed; it lets HoveredFile name the files.
        match offer.receive(URI_LIST.to_owned()) {
            Ok(pipe) => self.dnd_read(pipe),
            Err(e) => warn!("cannot receive a dropped text/uri-list: {e}"),
        }
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        if self.dnd.dropped {
            // The drop finishes (and destroys) the offer.
            return;
        }
        if let Some((window_id, _)) = self.dnd.target.take() {
            if self.dnd.hovered {
                self.dnd_push(WindowEvent::HoveredFileCancelled, window_id);
            }
            self.dnd_push(WindowEvent::CursorLeft { device_id: device_id() }, window_id);
        }
        self.dnd_reset();
    }

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, x: f64, y: f64) {
        if let Some((window_id, _)) = &self.dnd.target {
            let window_id = *window_id;
            self.dnd_cursor(window_id, x, y);
        }
    }

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        if self.dnd.target.is_none() {
            return;
        }
        self.dnd.dropped = true;
        self.dnd_finish();
    }
}

impl DataOfferHandler for WinitState {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }

    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        offer: &mut DragOffer,
        action: DndAction,
    ) {
        if self.dnd.target.as_ref().is_some_and(|(_, o)| o.inner() == offer.inner()) {
            self.dnd.action = Some(action);
        }
    }
}
