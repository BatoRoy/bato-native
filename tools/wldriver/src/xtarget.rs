//! `wldriver xtarget X Y W H SECS`: an X11 XDND (version 5) drop target. A
//! W×H window at X,Y (`$DISPLAY`, no window manager needed) that accepts any
//! drag for SECS seconds, logs the offered types, and on a drop reads and logs
//! `text/uri-list` and `text/plain;charset=utf-8`, then sends `XdndFinished`.
//! REFUSE=1 refuses every drag; ACTION=move answers with the move action.

use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;

pub fn run(args: &[String]) {
    let n = |i: usize| -> i32 { args[i].parse().expect("xtarget X Y W H SECS") };
    let (x, y, w, h, secs) = (n(0), n(1), n(2), n(3), n(4));
    let (conn, screen) = x11rb::connect(None).expect("connect to $DISPLAY");
    let root = conn.setup().roots[screen].root;
    let atom = |name: &str| {
        conn.intern_atom(false, name.as_bytes())
            .unwrap()
            .reply()
            .unwrap()
            .atom
    };
    let aware = atom("XdndAware");
    let [enter, position, status, leave, drop, finished, selection, type_list] = [
        "XdndEnter",
        "XdndPosition",
        "XdndStatus",
        "XdndLeave",
        "XdndDrop",
        "XdndFinished",
        "XdndSelection",
        "XdndTypeList",
    ]
    .map(atom);
    let (copy, mv) = (atom("XdndActionCopy"), atom("XdndActionMove"));
    let uri_list = atom("text/uri-list");
    let text = atom("text/plain;charset=utf-8");
    let prop = atom("WLDRIVER_DROP");
    let name = |a: u32| {
        conn.get_atom_name(a)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| String::from_utf8_lossy(&r.name).into_owned())
            .unwrap_or_else(|| format!("#{a}"))
    };
    let refuse = std::env::var_os("REFUSE").is_some();
    let action = if std::env::var("ACTION").as_deref() == Ok("move") {
        mv
    } else {
        copy
    };

    let win = conn.generate_id().unwrap();
    let aux = xproto::CreateWindowAux::new()
        .background_pixel(0x00a0_30c0)
        .override_redirect(1)
        .event_mask(xproto::EventMask::PROPERTY_CHANGE);
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        win,
        root,
        x as i16,
        y as i16,
        w as u16,
        h as u16,
        0,
        xproto::WindowClass::INPUT_OUTPUT,
        0,
        &aux,
    )
    .unwrap();
    conn.change_property32(
        xproto::PropMode::REPLACE,
        win,
        aware,
        xproto::AtomEnum::ATOM,
        &[5],
    )
    .unwrap();
    conn.map_window(win).unwrap();
    conn.flush().unwrap();
    eprintln!("xtarget: ready at {x},{y} {w}x{h}");

    let send = |to: u32, ty: u32, data: [u32; 5]| {
        let ev = xproto::ClientMessageEvent::new(32, to, ty, data);
        conn.send_event(false, to, xproto::EventMask::NO_EVENT, ev)
            .unwrap();
        conn.flush().unwrap();
    };
    let mut source = 0;
    let mut wanted: Vec<u32> = Vec::new();
    let mut reading: Vec<u32> = Vec::new();
    let end = Instant::now() + Duration::from_secs(secs as u64);
    while Instant::now() < end {
        let Some(event) = conn.poll_for_event().unwrap() else {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        match event {
            Event::ClientMessage(e) if e.type_ == enter => {
                let d = e.data.as_data32();
                source = d[0];
                let types: Vec<u32> = if d[1] & 1 == 1 {
                    conn.get_property(false, source, type_list, xproto::AtomEnum::ATOM, 0, 64)
                        .unwrap()
                        .reply()
                        .unwrap()
                        .value32()
                        .map(|v| v.collect())
                        .unwrap_or_default()
                } else {
                    d[2..].iter().copied().filter(|&a| a != 0).collect()
                };
                let names: Vec<String> = types.iter().map(|&a| name(a)).collect();
                eprintln!("xtarget: enter, version {}, offered {names:?}", d[1] >> 24);
                wanted = [uri_list, text]
                    .into_iter()
                    .filter(|a| types.contains(a))
                    .collect();
            }
            Event::ClientMessage(e) if e.type_ == position => {
                let d = e.data.as_data32();
                let ok = !refuse && !wanted.is_empty();
                eprintln!(
                    "xtarget: position {},{} action {} -> {}",
                    d[2] >> 16,
                    d[2] & 0xffff,
                    name(d[4]),
                    if ok { "accept" } else { "refuse" }
                );
                send(
                    d[0],
                    status,
                    [win, u32::from(ok) | 2, 0, 0, if ok { action } else { 0 }],
                );
            }
            Event::ClientMessage(e) if e.type_ == leave => eprintln!("xtarget: leave"),
            Event::ClientMessage(e) if e.type_ == drop => {
                let d = e.data.as_data32();
                eprintln!("xtarget: drop");
                if refuse || wanted.is_empty() {
                    send(d[0], finished, [win, 0, 0, 0, 0]);
                    continue;
                }
                reading = wanted.clone();
                conn.convert_selection(win, selection, reading[0], prop, d[2])
                    .unwrap();
                conn.flush().unwrap();
            }
            Event::SelectionNotify(e) if !reading.is_empty() => {
                let mime = name(reading.remove(0));
                if e.property == x11rb::NONE {
                    eprintln!("xtarget: dropped {mime}: refused by the source");
                } else {
                    let reply = conn
                        .get_property(true, win, prop, xproto::AtomEnum::ANY, 0, 1 << 20)
                        .unwrap()
                        .reply()
                        .unwrap();
                    let data = String::from_utf8_lossy(&reply.value).into_owned();
                    eprintln!("xtarget: dropped {mime}: {data:?}");
                }
                if let Some(&next) = reading.first() {
                    conn.convert_selection(win, selection, next, prop, x11rb::CURRENT_TIME)
                        .unwrap();
                    conn.flush().unwrap();
                } else {
                    send(source, finished, [win, 1, action, 0, 0]);
                    eprintln!("xtarget: finished ({})", name(action));
                }
            }
            _ => {}
        }
    }
}

/// `wldriver xkey KEYSYM`: press and release a key on `$DISPLAY` through
/// XTEST (hex keysym, e.g. ff1b for Escape).
pub fn key(args: &[String]) {
    use x11rb::protocol::xtest::ConnectionExt as _;
    let keysym = u32::from_str_radix(args[0].trim_start_matches("0x"), 16).expect("xkey KEYSYM");
    let (conn, screen) = x11rb::connect(None).expect("connect to $DISPLAY");
    let root = conn.setup().roots[screen].root;
    let (min, max) = (conn.setup().min_keycode, conn.setup().max_keycode);
    let map = conn
        .get_keyboard_mapping(min, max - min + 1)
        .unwrap()
        .reply()
        .unwrap();
    let per = map.keysyms_per_keycode as usize;
    let index = map
        .keysyms
        .iter()
        .position(|&k| k == keysym)
        .expect("keysym not mapped");
    let code = min + (index / per) as u8;
    for ty in [xproto::KEY_PRESS_EVENT, xproto::KEY_RELEASE_EVENT] {
        conn.xtest_fake_input(ty, code, x11rb::CURRENT_TIME, root, 0, 0, 0)
            .unwrap();
        conn.flush().unwrap();
        std::thread::sleep(Duration::from_millis(30));
    }
    conn.sync().unwrap();
    eprintln!("xkey: {keysym:#x} (keycode {code})");
}
