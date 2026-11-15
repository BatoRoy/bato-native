//! Wayland data-control, in-process: `ext-data-control-v1` (the
//! standardised one), else `zwlr-data-control-unstable-v1` (wlroots, KDE,
//! Hyprland). Both have the same shape, so one implementation is stamped out
//! for each.
//!
//! Unlike `wl-clipboard-rs`, which BatoFetch used: no fork, no MIME sniffing
//! (and no `tree_magic_mini`), every offer's bytes fixed up front, each data
//! request answered from its own short thread so a reader that never reads
//! cannot stall the owner, and every read bounded in time and size.

use wayland_client::Connection;

fn connect() -> Result<Connection, String> {
    Connection::connect_to_env().map_err(|e| format!("wayland: {e}"))
}

/// Own the selection with `offers`, served from a thread until replaced.
pub(super) fn copy(offers: Vec<(String, Vec<u8>)>) -> Result<(), String> {
    match ext::copy(connect()?, offers.clone()) {
        Ok(()) => Ok(()),
        Err(e1) => wlr::copy(connect()?, offers).map_err(|e2| format!("{e1}; {e2}")),
    }
}

/// The offered types, and the bytes of the ones `to_fetch` picks.
pub(super) fn paste(
    to_fetch: &dyn Fn(&[String]) -> Vec<&'static str>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    read(to_fetch).map(|(_, fetched)| fetched)
}

pub(super) fn types() -> Result<Vec<String>, String> {
    read(&|_| Vec::new()).map(|(types, _)| types)
}

type Read = (Vec<String>, Vec<(String, Vec<u8>)>);

fn read(to_fetch: &dyn Fn(&[String]) -> Vec<&'static str>) -> Result<Read, String> {
    match ext::read(connect()?, to_fetch) {
        Ok(r) => Ok(r),
        Err(e1) => wlr::read(connect()?, to_fetch).map_err(|e2| format!("{e1}; {e2}")),
    }
}

macro_rules! data_control {
    ($name:ident, $($proto:ident)::+, $max:expr,
     $mgr_mod:ident :: $Mgr:ident,
     $dev_mod:ident :: $Dev:ident,
     $src_mod:ident :: $Src:ident,
     $off_mod:ident :: $Off:ident) => {
        mod $name {
            use std::collections::HashMap;
            use std::os::fd::AsFd;
            use std::time::Instant;

            use wayland_client::backend::ObjectId;
            use wayland_client::globals::{GlobalListContents, registry_queue_init};
            use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
            use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
            use $($proto)::+::{
                $dev_mod::{self, $Dev},
                $mgr_mod::$Mgr,
                $off_mod::{self, $Off},
                $src_mod::{self, $Src},
            };

            use crate::clipboard::{READ_TIMEOUT, lookup, pipe, read_pipe, serve_fd};

            #[derive(Default)]
            struct State {
                /// What a copy serves.
                offers: Vec<(String, Vec<u8>)>,
                cancelled: bool,
                finished: bool,
                /// Offered types per offer object.
                mimes: HashMap<ObjectId, Vec<String>>,
                selection: Option<$Off>,
                got_selection: bool,
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

            impl Dispatch<WlSeat, ()> for State {
                fn event(
                    _: &mut Self,
                    _: &WlSeat,
                    _: <WlSeat as Proxy>::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }

            impl Dispatch<$Mgr, ()> for State {
                fn event(
                    _: &mut Self,
                    _: &$Mgr,
                    _: <$Mgr as Proxy>::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }

            impl Dispatch<$Dev, ()> for State {
                fn event(
                    state: &mut Self,
                    _: &$Dev,
                    event: $dev_mod::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                    match event {
                        $dev_mod::Event::Selection { id } => {
                            if let Some(old) = state.selection.take() {
                                state.mimes.remove(&old.id());
                                old.destroy();
                            }
                            state.selection = id;
                            state.got_selection = true;
                        }
                        $dev_mod::Event::PrimarySelection { id: Some(o) } => {
                            state.mimes.remove(&o.id());
                            o.destroy();
                        }
                        $dev_mod::Event::Finished => state.finished = true,
                        _ => {}
                    }
                }

                wayland_client::event_created_child!(State, $Dev, [
                    $dev_mod::EVT_DATA_OFFER_OPCODE => ($Off, ()),
                ]);
            }

            impl Dispatch<$Off, ()> for State {
                fn event(
                    state: &mut Self,
                    offer: &$Off,
                    event: $off_mod::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                    if let $off_mod::Event::Offer { mime_type } = event {
                        state.mimes.entry(offer.id()).or_default().push(mime_type);
                    }
                }
            }

            impl Dispatch<$Src, ()> for State {
                fn event(
                    state: &mut Self,
                    _: &$Src,
                    event: $src_mod::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                    match event {
                        // An unknown type: dropping the fd closes it (an
                        // empty answer).
                        $src_mod::Event::Send { mime_type, fd } => {
                            if let Some(d) = lookup(&state.offers, &mime_type) {
                                serve_fd(fd, d.to_vec());
                            }
                        }
                        $src_mod::Event::Cancelled => state.cancelled = true,
                        _ => {}
                    }
                }
            }

            fn setup(conn: &Connection) -> Result<(EventQueue<State>, $Mgr, WlSeat), String> {
                let (globals, queue) =
                    registry_queue_init::<State>(conn).map_err(|e| e.to_string())?;
                let qh = queue.handle();
                let mgr: $Mgr = globals
                    .bind(&qh, 1..=$max, ())
                    .map_err(|_| format!("no {}", <$Mgr as Proxy>::interface().name))?;
                let seat: WlSeat = globals
                    .bind(&qh, 1..=1, ())
                    .map_err(|_| "no wl_seat".to_string())?;
                Ok((queue, mgr, seat))
            }

            pub(super) fn copy(
                conn: Connection,
                offers: Vec<(String, Vec<u8>)>,
            ) -> Result<(), String> {
                let (mut queue, mgr, seat) = setup(&conn)?;
                let qh = queue.handle();
                let source = mgr.create_data_source(&qh, ());
                for (mime, _) in &offers {
                    source.offer(mime.clone());
                }
                let device = mgr.get_data_device(&seat, &qh, ());
                device.set_selection(Some(&source));
                let mut state = State {
                    offers,
                    ..State::default()
                };
                queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
                if state.cancelled || state.finished {
                    return Err("the compositor did not take the selection".into());
                }
                std::thread::Builder::new()
                    .name("bato-clip-owner".into())
                    .spawn(move || {
                        while !state.cancelled && !state.finished {
                            if queue.blocking_dispatch(&mut state).is_err() {
                                break;
                            }
                        }
                        source.destroy();
                        if let Some(o) = state.selection.take() {
                            o.destroy();
                        }
                        device.destroy();
                        mgr.destroy();
                        let _ = conn.flush();
                    })
                    .map_err(|e| e.to_string())?;
                Ok(())
            }

            pub(super) fn read(
                conn: Connection,
                to_fetch: &dyn Fn(&[String]) -> Vec<&'static str>,
            ) -> Result<super::Read, String> {
                let (mut queue, mgr, seat) = setup(&conn)?;
                let qh = queue.handle();
                let device = mgr.get_data_device(&seat, &qh, ());
                let mut state = State::default();
                // The current selection is sent on get_data_device; one
                // round-trip brings it, a second covers a slow compositor.
                for _ in 0..2 {
                    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
                    if state.got_selection {
                        break;
                    }
                }
                let mut types = Vec::new();
                let mut fetched = Vec::new();
                let mut result = Ok(());
                if let Some(offer) = state.selection.take() {
                    types = state.mimes.remove(&offer.id()).unwrap_or_default();
                    for want in to_fetch(&types) {
                        let Some(name) = types.iter().find(|t| t.eq_ignore_ascii_case(want))
                        else {
                            continue;
                        };
                        let (r, w) = match pipe() {
                            Ok(p) => p,
                            Err(e) => {
                                result = Err(e);
                                break;
                            }
                        };
                        offer.receive(name.clone(), w.as_fd());
                        if let Err(e) = conn.flush() {
                            result = Err(e.to_string());
                            break;
                        }
                        drop(w);
                        match read_pipe(r, Instant::now() + READ_TIMEOUT) {
                            Ok(data) => fetched.push((want.to_string(), data)),
                            Err(e) => {
                                result = Err(e);
                                break;
                            }
                        }
                    }
                    offer.destroy();
                }
                device.destroy();
                mgr.destroy();
                let _ = conn.flush();
                result.map(|()| (types, fetched))
            }
        }
    };
}

data_control!(
    ext,
    wayland_protocols::ext::data_control::v1::client,
    1,
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_device_v1::ExtDataControlDeviceV1,
    ext_data_control_source_v1::ExtDataControlSourceV1,
    ext_data_control_offer_v1::ExtDataControlOfferV1
);

data_control!(
    wlr,
    wayland_protocols_wlr::data_control::v1::client,
    2,
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
    zwlr_data_control_source_v1::ZwlrDataControlSourceV1,
    zwlr_data_control_offer_v1::ZwlrDataControlOfferV1
);
