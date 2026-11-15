//! Seat handling.

use std::sync::Arc;

use ahash::AHashMap;
use tracing::warn;

use sctk::reexports::client::backend::ObjectId;
use sctk::reexports::client::protocol::wl_seat::WlSeat;
use sctk::reexports::client::protocol::wl_touch::WlTouch;
use sctk::reexports::client::{Connection, Proxy, QueueHandle};
use sctk::reexports::protocols::wp::relative_pointer::zv1::client::zwp_relative_pointer_v1::ZwpRelativePointerV1;
use sctk::reexports::protocols::wp::text_input::zv3::client::zwp_text_input_v3::ZwpTextInputV3;

use sctk::seat::pointer::{ThemeSpec, ThemedPointer};
use sctk::seat::{Capability as SeatCapability, SeatHandler, SeatState};

use crate::event::WindowEvent;
use crate::keyboard::ModifiersState;
use crate::platform_impl::wayland::state::WinitState;

mod keyboard;
mod pointer;
mod text_input;
mod touch;

pub use pointer::relative_pointer::RelativePointerState;
pub use pointer::{PointerConstraintsState, WinitPointerData, WinitPointerDataExt};
pub use text_input::{TextInputState, ZwpTextInputV3Ext};

use keyboard::{KeyboardData, KeyboardState};
use text_input::TextInputData;
use touch::TouchPoint;

#[derive(Debug, Default)]
pub struct WinitSeatState {
    /// The pointer bound on the seat.
    pointer: Option<Arc<ThemedPointer<WinitPointerData>>>,

    /// The touch bound on the seat.
    touch: Option<WlTouch>,

    /// The mapping from touched points to the surfaces they're present.
    touch_map: AHashMap<i32, TouchPoint>,

    /// The text input bound on the seat.
    text_input: Option<Arc<ZwpTextInputV3>>,

    /// The relative pointer bound on the seat.
    relative_pointer: Option<ZwpRelativePointerV1>,

    /// The keyboard bound on the seat.
    keyboard_state: Option<KeyboardState>,

    /// The current modifiers state on the seat.
    modifiers: ModifiersState,

    /// Whether we have pending modifiers.
    modifiers_pending: bool,

    /// bato-native: the seat's data device (file drops).
    pub data_device: Option<sctk::data_device_manager::data_device::DataDevice>,
}

impl WinitSeatState {
    pub fn new() -> Self {
        Default::default()
    }
}

impl SeatHandler for WinitState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_capability(
        &mut self,
        _: &Connection,
        queue_handle: &QueueHandle<Self>,
        seat: WlSeat,
        capability: SeatCapability,
    ) {
        let seat_state = match self.seats.get_mut(&seat.id()) {
            Some(seat_state) => seat_state,
            None => {
                warn!("Received wl_seat::new_capability for unknown seat");
                return;
            },
        };

        match capability {
            SeatCapability::Touch if seat_state.touch.is_none() => {
                seat_state.touch = self.seat_state.get_touch(queue_handle, &seat).ok();
            },
            SeatCapability::Keyboard if seat_state.keyboard_state.is_none() => {
                let keyboard = seat.get_keyboard(queue_handle, KeyboardData::new(seat.clone()));
                seat_state.keyboard_state =
                    Some(KeyboardState::new(keyboard, self.loop_handle.clone()));
            },
            SeatCapability::Pointer if seat_state.pointer.is_none() => {
                let surface = self.compositor_state.create_surface(queue_handle);
                let viewport = self
                    .viewporter_state
                    .as_ref()
                    .map(|state| state.get_viewport(&surface, queue_handle));
                let surface_id = surface.id();
                let pointer_data = WinitPointerData::new(viewport);
                let themed_pointer = self
                    .seat_state
                    .get_pointer_with_theme_and_data(
                        queue_handle,
                        &seat,
                        self.shm.wl_shm(),
                        surface,
                        ThemeSpec::System,
                        pointer_data,
                    )
                    .expect("failed to create pointer with present capability.");

                seat_state.relative_pointer = self.relative_pointer.as_ref().map(|manager| {
                    manager.get_relative_pointer(
                        themed_pointer.pointer(),
                        queue_handle,
                        sctk::globals::GlobalData,
                    )
                });

                let themed_pointer = Arc::new(themed_pointer);

                // Register cursor surface.
                self.pointer_surfaces.insert(surface_id, themed_pointer.clone());

                seat_state.pointer = Some(themed_pointer);
            },
            _ => (),
        }

        if let Some(text_input_state) =
            seat_state.text_input.is_none().then_some(self.text_input_state.as_ref()).flatten()
        {
            seat_state.text_input = Some(Arc::new(text_input_state.get_text_input(
                &seat,
                queue_handle,
                TextInputData::default(),
            )));
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _queue_handle: &QueueHandle<Self>,
        seat: WlSeat,
        capability: SeatCapability,
    ) {
        let seat_state = match self.seats.get_mut(&seat.id()) {
            Some(seat_state) => seat_state,
            None => {
                warn!("Received wl_seat::remove_capability for unknown seat");
                return;
            },
        };

        if let Some(text_input) = seat_state.text_input.take() {
            text_input.destroy();
        }

        match capability {
            SeatCapability::Touch => {
                if let Some(touch) = seat_state.touch.take() {
                    if touch.version() >= 3 {
                        touch.release();
                    }
                }
            },
            SeatCapability::Pointer => {
                if let Some(relative_pointer) = seat_state.relative_pointer.take() {
                    relative_pointer.destroy();
                }

                if let Some(pointer) = seat_state.pointer.take() {
                    // Remove the cursor from the mapping.
                    let surface_id = pointer.surface().id();
                    let _ = self.pointer_surfaces.remove(&surface_id);

                    if let Some(pointer_data) = pointer.pointer().winit_data() {
                        // Remove the inner locks/confines before dropping the pointer.
                        pointer_data.data().unlock_pointer();
                        pointer_data.data().unconfine_pointer();

                        // bato-native: no leave or release will come for this
                        // pointer; end its presses and hover on the window it
                        // was over, so no button stays held there.
                        if let Some((window_id, held)) = pointer_data.data().bato_take_focus() {
                            self.bato_pointer_gone(&pointer, window_id, held);
                        }
                    }

                    if pointer.pointer().version() >= 3 {
                        pointer.pointer().release();
                    }
                }
            },
            SeatCapability::Keyboard => {
                seat_state.keyboard_state = None;
                self.on_keyboard_destroy(&seat.id());
            },
            _ => (),
        }
    }

    fn new_seat(
        &mut self,
        _connection: &Connection,
        queue_handle: &QueueHandle<Self>,
        seat: WlSeat,
    ) {
        let mut state = WinitSeatState::new();
        state.data_device = self
            .data_device_manager
            .as_ref()
            .map(|m| crate::platform_impl::wayland::dnd::data_device(m, queue_handle, &seat));
        self.seats.insert(seat.id(), state);
    }

    fn remove_seat(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        seat: WlSeat,
    ) {
        let _ = self.seats.remove(&seat.id());
        crate::platform_impl::wayland::dnd::forget_seat(&seat);
        self.on_keyboard_destroy(&seat.id());
    }
}

impl WinitState {
    /// bato-native: the seat's pointer went away while over `window_id` with
    /// `held` buttons down: send their releases and `CursorLeft`.
    fn bato_pointer_gone(
        &mut self,
        pointer: &Arc<ThemedPointer<WinitPointerData>>,
        window_id: crate::platform_impl::wayland::WindowId,
        held: Vec<u32>,
    ) {
        let device_id = crate::event::DeviceId(crate::platform_impl::DeviceId::Wayland(
            crate::platform_impl::wayland::DeviceId,
        ));
        if let Some(window) = self.windows.get_mut().get(&window_id) {
            window.lock().unwrap().pointer_left(Arc::downgrade(pointer));
        }
        for button in held {
            self.events_sink.push_window_event(
                WindowEvent::MouseInput {
                    device_id,
                    state: crate::event::ElementState::Released,
                    button: pointer::wayland_button_to_winit(button),
                },
                window_id,
            );
        }
        self.events_sink.push_window_event(WindowEvent::CursorLeft { device_id }, window_id);
    }

    fn on_keyboard_destroy(&mut self, seat: &ObjectId) {
        for (window_id, window) in self.windows.get_mut() {
            let mut window = window.lock().unwrap();
            let had_focus = window.has_focus();
            window.remove_seat_focus(seat);
            if had_focus != window.has_focus() {
                self.events_sink.push_window_event(WindowEvent::Focused(false), *window_id);
            }
        }
    }
}
