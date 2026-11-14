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
//!
//! The same data device serves the clipboard: Hyprland (`dataDeviceForClient`)
//! sends the selection, like drags, only to a client's first
//! `wl_data_device`. This one is created when the event loop starts, before
//! iced's clipboard (window_clipboard → smithay-clipboard) creates its own,
//! which then never sees a selection, so paste found nothing. Clipboard reads
//! go through [`read_selection_text`] (iced_winit, patched, calls it);
//! writes still go through smithay-clipboard, whose `wl_data_source` the
//! compositor accepts from any device.

use std::io::{ErrorKind, Read};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use sctk::compositor::SurfaceData;
use sctk::data_device_manager::data_device::{DataDevice, DataDeviceData, DataDeviceHandler};
use sctk::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use sctk::data_device_manager::{DataDeviceManagerState, ReadPipe};
use sctk::reexports::calloop::{PostAction, RegistrationToken};
use sctk::reexports::client::protocol::wl_data_device::WlDataDevice;
use sctk::reexports::client::protocol::wl_data_device_manager::DndAction;
use sctk::reexports::client::protocol::wl_seat::WlSeat;
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

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, device: &WlDataDevice) {
        // The seat that saw the latest selection is the one to paste from.
        // SCTK keeps the offer (and drops it when the selection is cleared,
        // without calling this), so reads always see the current one.
        *selection_device() = Some(device.clone());
    }

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

// bato-native: the clipboard selection, read through the data device above.

/// The data device to read the selection from (the latest seat to see one).
static SELECTION_DEVICE: Mutex<Option<WlDataDevice>> = Mutex::new(None);

fn selection_device() -> std::sync::MutexGuard<'static, Option<WlDataDevice>> {
    SELECTION_DEVICE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A seat's data device: file drops and the clipboard selection.
pub fn data_device(
    manager: &DataDeviceManagerState,
    queue_handle: &QueueHandle<WinitState>,
    seat: &WlSeat,
) -> DataDevice {
    let device = manager.get_data_device(queue_handle, seat);
    selection_device().get_or_insert_with(|| device.inner().clone());
    device
}

/// The seat is gone: stop reading the selection through its device.
pub fn forget_seat(seat: &WlSeat) {
    let mut device = selection_device();
    if device.as_ref().and_then(|d| d.data::<DataDeviceData>()).is_some_and(|d| d.seat() == seat) {
        *device = None;
    }
}

/// Text types, preferred first: smithay-clipboard's, then X11's older names.
const TEXT_MIME_TYPES: [&str; 5] =
    ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "TEXT", "STRING"];

/// Give up when the selection owner sends nothing for this long...
const READ_IDLE: Duration = Duration::from_secs(1);
/// ...or has not finished after this long.
const READ_TOTAL: Duration = Duration::from_secs(5);

/// See [`crate::platform::wayland::read_selection_text`].
pub fn read_selection_text() -> Result<Option<String>, ()> {
    let device = selection_device().clone().ok_or(())?;
    let connection = Connection::from_backend(device.backend().upgrade().ok_or(())?);
    let Some(offer) = device.data::<DataDeviceData>().and_then(|d| d.selection_offer()) else {
        return Ok(None);
    };
    let Some(mime) = offer.with_mime_types(|offered| {
        TEXT_MIME_TYPES.into_iter().find(|t| offered.iter().any(|o| o == t))
    }) else {
        return Ok(None);
    };
    let mut pipe = match offer.receive(mime.to_owned()) {
        Ok(pipe) => pipe,
        Err(e) => {
            warn!("cannot receive the clipboard: {e}");
            return Ok(None);
        },
    };
    // The request has to reach the compositor before the owner can answer.
    // The owner may be this process (smithay-clipboard holds our copies): it
    // writes from its own thread and queue, so blocking here is fine. The
    // event loop holds no wl_display read intent outside its poll, so that
    // thread can read the `send` event while this one waits.
    if let Err(e) = connection.flush() {
        warn!("cannot request the clipboard: {e}");
        return Ok(None);
    }
    Ok(read_with_timeout(&mut pipe).map(|bytes| decode_text(mime, bytes)))
}

/// The pipe's contents, or `None` on an error or a stalled owner.
fn read_with_timeout(pipe: &mut ReadPipe) -> Option<Vec<u8>> {
    use rustix::event::{poll, PollFd, PollFlags, Timespec};
    let start = Instant::now();
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let Some(left) = READ_TOTAL.checked_sub(start.elapsed()) else {
            warn!("the clipboard owner did not finish within {READ_TOTAL:?}");
            return None;
        };
        let wait = left.min(READ_IDLE);
        let timeout = Timespec { tv_sec: wait.as_secs() as _, tv_nsec: wait.subsec_nanos() as _ };
        match poll(&mut [PollFd::new(&*pipe, PollFlags::IN)], Some(&timeout)) {
            Ok(0) => {
                warn!("the clipboard owner sent nothing for {wait:?}");
                return None;
            },
            Ok(_) => {},
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => {
                warn!("reading the clipboard failed: {e}");
                return None;
            },
        }
        match pipe.read(&mut chunk) {
            Ok(0) => return Some(bytes),
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == ErrorKind::Interrupted => {},
            Err(e) => {
                warn!("reading the clipboard failed: {e}");
                return None;
            },
        }
    }
}

/// Text from `mime` data: `STRING` is Latin-1, the rest UTF-8 (lossy);
/// `text/plain` line ends (CRLF by RFC 2046) become LF, as in
/// smithay-clipboard.
fn decode_text(mime: &str, bytes: Vec<u8>) -> String {
    let text = if mime == "STRING" {
        bytes.into_iter().map(char::from).collect()
    } else {
        String::from_utf8(bytes)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
    };
    if mime.starts_with("text/plain") {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text
    }
}
