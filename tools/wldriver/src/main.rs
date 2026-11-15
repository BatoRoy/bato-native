//! Test driver for a nested compositor: virtual pointer clicks and a real
//! wl_data_device drag of a file (text/uri-list) onto a point.
//!
//!   wldriver click X Y [left|middle]
//!   wldriver wheel N              (N wheel notches down; negative: up)
//!   wldriver drag PATH X Y        (source: a 100x100 layer surface, bottom-left)
//!   wldriver target X Y W H SECS  (a drop target: a W×H layer surface at X,Y that
//!                                  accepts any drag for SECS seconds and logs
//!                                  the offered types and the dropped data;
//!                                  ACTION=move prefers move over copy,
//!                                  REFUSE=1 accepts nothing)
//!   wldriver xtarget X Y W H SECS (the same, as an X11 XDND target on $DISPLAY)
//!   wldriver xkey KEYSYM          (an X11 key press + release through XTEST, hex keysym)
//!
//! OUT_W/OUT_H: the output size (default 1280x720).

use std::io::Write;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
    wl_data_source, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{
    delegate_noop, event_created_child, Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1,
};

mod xtarget;

const BTN_LEFT: u32 = 0x110;
const BTN_MIDDLE: u32 = 0x112;

#[derive(Default)]
struct State {
    configured: Option<u32>,
    entered: bool,
    press_serial: Option<u32>,
    uri: String,
    done: Option<&'static str>,
    /// Drop target mode: the target surface, the types of each offer, the
    /// offer over the target, and whether it was dropped.
    target: Option<wl_surface::WlSurface>,
    offered: std::collections::HashMap<u32, Vec<String>>,
    over: Option<wl_data_offer::WlDataOffer>,
    dropped: bool,
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
delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_data_device_manager::WlDataDeviceManager);

impl Dispatch<wl_data_offer::WlDataOffer, ()> for State {
    fn event(
        s: &mut Self,
        offer: &wl_data_offer::WlDataOffer,
        e: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_client::Proxy;
        match e {
            wl_data_offer::Event::Offer { mime_type } => s
                .offered
                .entry(offer.id().protocol_id())
                .or_default()
                .push(mime_type),
            wl_data_offer::Event::SourceActions { source_actions } if s.target.is_some() => {
                eprintln!("target: source actions {source_actions:?}")
            }
            wl_data_offer::Event::Action { dnd_action } if s.target.is_some() => {
                eprintln!("target: action {dnd_action:?}")
            }
            _ => {}
        }
    }
}
delegate_noop!(State: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(State: ignore zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ignore zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1);

impl Dispatch<wl_data_device::WlDataDevice, ()> for State {
    fn event(
        s: &mut Self,
        _: &wl_data_device::WlDataDevice,
        e: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_client::Proxy;
        match e {
            wl_data_device::Event::Enter {
                serial,
                surface,
                id: Some(offer),
                ..
            } if s.target.as_ref() == Some(&surface) => {
                let types = s
                    .offered
                    .get(&offer.id().protocol_id())
                    .cloned()
                    .unwrap_or_default();
                eprintln!("target: enter, offered {types:?}");
                let refuse = std::env::var_os("REFUSE").is_some();
                let pick = ["text/uri-list", "text/plain;charset=utf-8", "text/plain"]
                    .into_iter()
                    .find(|t| !refuse && types.iter().any(|o| o == t));
                offer.accept(serial, pick.map(str::to_owned));
                let preferred = if std::env::var("ACTION").as_deref() == Ok("move") {
                    wl_data_device_manager::DndAction::Move
                } else {
                    wl_data_device_manager::DndAction::Copy
                };
                if refuse {
                    offer.set_actions(
                        wl_data_device_manager::DndAction::empty(),
                        wl_data_device_manager::DndAction::empty(),
                    );
                } else {
                    offer.set_actions(
                        wl_data_device_manager::DndAction::Copy
                            | wl_data_device_manager::DndAction::Move,
                        preferred,
                    );
                }
                s.over = Some(offer);
            }
            wl_data_device::Event::Leave if s.target.is_some() => {
                if !s.dropped {
                    eprintln!("target: leave");
                    s.over = None;
                }
            }
            wl_data_device::Event::Drop if s.over.is_some() => s.dropped = true,
            wl_data_device::Event::Enter { .. } | wl_data_device::Event::Leave => {
                eprintln!("driver: own data device {e:?}");
            }
            _ => {}
        }
    }
    event_created_child!(State, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        s: &mut Self,
        _: &wl_pointer::WlPointer,
        e: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            wl_pointer::Event::Enter { .. } => s.entered = true,
            wl_pointer::Event::Button { serial, state, .. } => {
                if state == wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed) {
                    s.press_serial = Some(serial);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for State {
    fn event(
        s: &mut Self,
        _: &wl_data_source::WlDataSource,
        e: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            wl_data_source::Event::Send { mime_type, fd } => {
                eprintln!("driver: source send {mime_type}");
                let mut f = std::fs::File::from(fd);
                let _ = f.write_all(s.uri.as_bytes());
            }
            wl_data_source::Event::Target { mime_type } => {
                eprintln!("driver: target {mime_type:?}")
            }
            wl_data_source::Event::Action { dnd_action } => {
                eprintln!("driver: action {dnd_action:?}")
            }
            wl_data_source::Event::DndDropPerformed => eprintln!("driver: drop performed"),
            wl_data_source::Event::DndFinished => s.done = Some("finished"),
            wl_data_source::Event::Cancelled => s.done = Some("cancelled"),
            _ => {}
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        s: &mut Self,
        ls: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        e: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = e {
            ls.ack_configure(serial);
            s.configured = Some(serial);
        }
    }
}

fn now_ms(t0: Instant) -> u32 {
    t0.elapsed().as_millis() as u32
}

fn pump(q: &mut EventQueue<State>, s: &mut State, for_ms: u64) {
    let end = Instant::now() + Duration::from_millis(for_ms);
    loop {
        q.roundtrip(s).unwrap();
        if Instant::now() >= end {
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("xtarget") {
        return xtarget::run(&args[2..]);
    }
    if args.get(1).map(String::as_str) == Some("xkey") {
        return xtarget::key(&args[2..]);
    }
    let ow: u32 = std::env::var("OUT_W")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1280);
    let oh: u32 = std::env::var("OUT_H")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(720);
    let conn = Connection::connect_to_env().expect("connect");
    let (globals, mut q) = registry_queue_init::<State>(&conn).unwrap();
    let qh = q.handle();
    let mut s = State::default();
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).unwrap();
    let vpm: zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1 =
        globals.bind(&qh, 1..=2, ()).unwrap();
    let vp = vpm.create_virtual_pointer(Some(&seat), &qh, ());
    let t0 = Instant::now();
    let motion = |x: f64, y: f64| {
        vp.motion_absolute(now_ms(t0), x as u32, y as u32, ow, oh);
        vp.frame();
    };
    let button = |b: u32, down: bool| {
        let st = if down {
            wl_pointer::ButtonState::Pressed
        } else {
            wl_pointer::ButtonState::Released
        };
        vp.button(now_ms(t0), b, st);
        vp.frame();
    };
    q.roundtrip(&mut s).unwrap();

    let actions: Vec<Vec<String>> = if args.get(1).map(String::as_str) == Some("script") {
        args[2..]
            .iter()
            .map(|a| a.split_whitespace().map(str::to_owned).collect())
            .collect()
    } else {
        vec![args[1..].to_vec()]
    };
    let mut globals_drag = Some(());
    for mut args in actions {
        args.insert(0, String::new());
        match args.get(1).map(String::as_str) {
            Some("shot") => {
                let mut c = std::process::Command::new("grim");
                if args.len() >= 7 {
                    c.arg("-g")
                        .arg(format!("{},{} {}x{}", args[3], args[4], args[5], args[6]));
                }
                let _ = c.arg(&args[2]).status();
            }
            Some("sleep") => pump(&mut q, &mut s, args[2].parse().unwrap()),
            Some("exec") => {
                let st = std::process::Command::new(&args[2])
                    .args(&args[3..])
                    .status();
                eprintln!("driver: exec {:?} -> {st:?}", &args[2..]);
                pump(&mut q, &mut s, 50);
            }
            Some("press") | Some("release") => {
                let b = if args.get(2).map(String::as_str) == Some("middle") {
                    BTN_MIDDLE
                } else {
                    BTN_LEFT
                };
                button(b, args[1] == "press");
                pump(&mut q, &mut s, 60);
            }
            Some("wheel") => {
                // N notches down (negative: up), one per 40 ms.
                let n: i32 = args[2].parse().unwrap();
                for _ in 0..n.abs() {
                    vp.axis_source(wl_pointer::AxisSource::Wheel);
                    vp.axis_discrete(
                        now_ms(t0),
                        wl_pointer::Axis::VerticalScroll,
                        15.0 * n.signum() as f64,
                        n.signum(),
                    );
                    vp.frame();
                    pump(&mut q, &mut s, 40);
                }
            }
            Some("move") => {
                let (x, y): (f64, f64) = (args[2].parse().unwrap(), args[3].parse().unwrap());
                motion(x, y);
                pump(&mut q, &mut s, 100);
            }
            Some("click") => {
                let (x, y): (f64, f64) = (args[2].parse().unwrap(), args[3].parse().unwrap());
                let b = if args.get(4).map(String::as_str) == Some("middle") {
                    BTN_MIDDLE
                } else {
                    BTN_LEFT
                };
                motion(x, y);
                pump(&mut q, &mut s, 80);
                button(b, true);
                pump(&mut q, &mut s, 60);
                button(b, false);
                pump(&mut q, &mut s, 100);
            }
            Some("drag") if globals_drag.take().is_some() => {
                let path = std::fs::canonicalize(&args[2]).unwrap();
                s.uri = format!("file://{}\r\n", path.display());
                let (tx, ty): (f64, f64) = (args[3].parse().unwrap(), args[4].parse().unwrap());
                s.done = None;
                let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=4, ()).unwrap();
                let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
                let layer: zwlr_layer_shell_v1::ZwlrLayerShellV1 =
                    globals.bind(&qh, 1..=4, ()).unwrap();
                let ddm: wl_data_device_manager::WlDataDeviceManager =
                    globals.bind(&qh, 3..=3, ()).unwrap();
                let dd = ddm.get_data_device(&seat, &qh, ());
                let pointer = seat.get_pointer(&qh, ());
                let _ = &pointer;
                let surface = compositor.create_surface(&qh, ());
                let ls = layer.get_layer_surface(
                    &surface,
                    None,
                    zwlr_layer_shell_v1::Layer::Overlay,
                    "wldriver".into(),
                    &qh,
                    (),
                );
                ls.set_size(100, 100);
                ls.set_anchor(
                    zwlr_layer_surface_v1::Anchor::Bottom | zwlr_layer_surface_v1::Anchor::Left,
                );
                surface.commit();
                while s.configured.is_none() {
                    q.blocking_dispatch(&mut s).unwrap();
                }
                let (w, h) = (100i32, 100i32);
                let size = (w * h * 4) as usize;
                let path_buf = std::env::var("XDG_RUNTIME_DIR").unwrap() + "/wldriver-shm";
                let mut file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&path_buf)
                    .unwrap();
                std::fs::remove_file(&path_buf).unwrap();
                file.write_all(&[0x40u8, 0xa0, 0x20, 0xff].repeat((w * h) as usize))
                    .unwrap();
                let pool = shm.create_pool(file.as_fd(), size as i32, &qh, ());
                let buf = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, &qh, ());
                surface.attach(Some(&buf), 0, 0);
                surface.damage(0, 0, w, h);
                surface.commit();
                pump(&mut q, &mut s, 300);
                let (sx, sy) = (50.0, oh as f64 - 50.0);
                motion(sx, sy);
                pump(&mut q, &mut s, 200);
                eprintln!("driver: pointer entered source: {}", s.entered);
                button(BTN_LEFT, true);
                pump(&mut q, &mut s, 100);
                let serial = s
                    .press_serial
                    .expect("no button serial on the source surface");
                let source = ddm.create_data_source(&qh, ());
                source.offer("text/uri-list".into());
                source.set_actions(
                    wl_data_device_manager::DndAction::Copy
                        | wl_data_device_manager::DndAction::Move,
                );
                dd.start_drag(Some(&source), &surface, None, serial);
                pump(&mut q, &mut s, 100);
                let steps = 25;
                for i in 1..=steps {
                    let f = i as f64 / steps as f64;
                    motion(sx + (tx - sx) * f, sy + (ty - sy) * f);
                    pump(&mut q, &mut s, 30);
                }
                let hover_ms: u64 = std::env::var("HOVER_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(600);
                pump(&mut q, &mut s, hover_ms);
                if std::env::var_os("SHOT_DURING").is_some() {
                    let st = std::process::Command::new("grim")
                        .arg(std::env::var("SHOT_DURING").unwrap())
                        .status();
                    eprintln!("driver: hover screenshot {st:?}");
                }
                button(BTN_LEFT, false);
                let end = Instant::now() + Duration::from_secs(3);
                while s.done.is_none() && Instant::now() < end {
                    pump(&mut q, &mut s, 50);
                }
                eprintln!("driver: drag {}", s.done.unwrap_or("timed out"));
                ls.destroy();
                surface.destroy();
                q.roundtrip(&mut s).unwrap();
            }
            Some("target") => {
                let n = |i: usize| -> i32 { args[i].parse().unwrap() };
                let (x, y, w, h, secs) = (n(2), n(3), n(4), n(5), n(6));
                run_target(&conn, &globals, &mut q, &mut s, &seat, (x, y, w, h), secs as u64);
            }
            _ => eprintln!(
                "usage: wldriver click X Y [middle] | move X Y | drag PATH X Y | target X Y W H SECS"
            ),
        }
    }
}

/// A drop target: a `w × h` layer surface at `x, y` (top-left anchored) that
/// accepts any drag, logs the offered types, and on a drop reads and logs
/// `text/uri-list` and `text/plain;charset=utf-8`, then finishes the offer.
fn run_target(
    conn: &Connection,
    globals: &wayland_client::globals::GlobalList,
    q: &mut EventQueue<State>,
    s: &mut State,
    seat: &wl_seat::WlSeat,
    (x, y, w, h): (i32, i32, i32, i32),
    secs: u64,
) {
    use std::io::Read;
    let qh = q.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=4, ()).unwrap();
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
    let layer: zwlr_layer_shell_v1::ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ()).unwrap();
    let ddm: wl_data_device_manager::WlDataDeviceManager = globals.bind(&qh, 3..=3, ()).unwrap();
    let _dd = ddm.get_data_device(seat, &qh, ());
    let surface = compositor.create_surface(&qh, ());
    let ls = layer.get_layer_surface(
        &surface,
        None,
        zwlr_layer_shell_v1::Layer::Top,
        "wldriver-target".into(),
        &qh,
        (),
    );
    ls.set_size(w as u32, h as u32);
    ls.set_anchor(zwlr_layer_surface_v1::Anchor::Top | zwlr_layer_surface_v1::Anchor::Left);
    ls.set_margin(y, 0, 0, x);
    ls.set_exclusive_zone(-1);
    surface.commit();
    while s.configured.is_none() {
        q.blocking_dispatch(s).unwrap();
    }
    let path_buf = std::env::var("XDG_RUNTIME_DIR").unwrap() + "/wldriver-target-shm";
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path_buf)
        .unwrap();
    std::fs::remove_file(&path_buf).unwrap();
    // A magenta-ish panel: easy to spot in screenshots.
    file.write_all(&[0xa0u8, 0x30, 0xc0, 0xff].repeat((w * h) as usize))
        .unwrap();
    let pool = shm.create_pool(file.as_fd(), w * h * 4, &qh, ());
    let buf = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, &qh, ());
    surface.attach(Some(&buf), 0, 0);
    surface.damage(0, 0, w, h);
    surface.commit();
    s.target = Some(surface.clone());
    eprintln!("target: ready at {x},{y} {w}x{h}");
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        pump(q, s, 30);
        if !s.dropped {
            continue;
        }
        s.dropped = false;
        let Some(offer) = s.over.take() else { continue };
        use wayland_client::Proxy;
        let types = s
            .offered
            .get(&offer.id().protocol_id())
            .cloned()
            .unwrap_or_default();
        for mime in ["text/uri-list", "text/plain;charset=utf-8"] {
            if !types.iter().any(|t| t == mime) {
                continue;
            }
            let (mut reader, writer) = std::io::pipe().unwrap();
            offer.receive(mime.into(), writer.as_fd());
            drop(writer);
            conn.flush().unwrap();
            let mut data = String::new();
            let _ = reader.read_to_string(&mut data);
            eprintln!("target: dropped {mime}: {data:?}");
        }
        offer.finish();
        offer.destroy();
        conn.flush().unwrap();
        eprintln!("target: finished");
    }
    ls.destroy();
    surface.destroy();
    let _ = q.roundtrip(s);
}
