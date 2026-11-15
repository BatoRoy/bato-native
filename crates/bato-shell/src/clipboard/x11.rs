//! X11 CLIPBOARD, in-process (x11rb's pure-Rust connection, the crate winit
//! already builds): an owner on a hidden 1×1 input-only window that answers
//! `TARGETS`, `TIMESTAMP` and every offered type until another client takes
//! the selection, and a reader that converts `TARGETS` then the wanted types.
//!
//! Not supported, deliberately: `MULTIPLE` requests (refused; clients fall
//! back to one at a time) and `INCR` transfers (a file list never needs
//! one; an owner that insists gets an error).

use std::time::{Duration, Instant};

use x11rb::connection::{Connection as _, RequestConnection as _};
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode,
    SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

use crate::clipboard::{MAX_READ, READ_TIMEOUT};

struct Atoms {
    clipboard: Atom,
    targets: Atom,
    timestamp: Atom,
    incr: Atom,
    /// Our scratch property.
    prop: Atom,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("x11: {e}")
}

fn intern(conn: &RustConnection, name: &str) -> Result<Atom, String> {
    Ok(conn
        .intern_atom(false, name.as_bytes())
        .map_err(err)?
        .reply()
        .map_err(err)?
        .atom)
}

fn open(display: Option<&str>) -> Result<(RustConnection, Window, Atoms), String> {
    let (conn, screen) = x11rb::connect(display).map_err(err)?;
    let root = conn.setup().roots[screen].root;
    let win = conn.generate_id().map_err(err)?;
    conn.create_window(
        0,
        win,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )
    .map_err(err)?;
    let atoms = Atoms {
        clipboard: intern(&conn, "CLIPBOARD")?,
        targets: intern(&conn, "TARGETS")?,
        timestamp: intern(&conn, "TIMESTAMP")?,
        incr: intern(&conn, "INCR")?,
        prop: intern(&conn, "BATO_CLIPBOARD")?,
    };
    Ok((conn, win, atoms))
}

/// The next event `pick` accepts, polling until `deadline`.
fn wait_for<T>(
    conn: &RustConnection,
    deadline: Instant,
    mut pick: impl FnMut(&Event) -> Option<T>,
) -> Result<T, String> {
    loop {
        while let Some(ev) = conn.poll_for_event().map_err(err)? {
            if let Some(t) = pick(&ev) {
                return Ok(t);
            }
        }
        if Instant::now() >= deadline {
            return Err("the clipboard owner did not answer".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Own CLIPBOARD with `offers`; served from a thread until another client
/// takes it.
pub(super) fn copy(display: Option<&str>, offers: Vec<(String, Vec<u8>)>) -> Result<(), String> {
    let (conn, win, a) = open(display)?;
    let mut served = Vec::with_capacity(offers.len());
    for (mime, data) in offers {
        served.push((intern(&conn, &mime)?, data));
    }
    // ICCCM: own with a real server time, not CurrentTime. A zero-length
    // append to our own window yields one in its PropertyNotify.
    conn.change_property8(PropMode::APPEND, win, a.prop, AtomEnum::STRING, &[])
        .map_err(err)?;
    conn.flush().map_err(err)?;
    let time = wait_for(&conn, Instant::now() + READ_TIMEOUT, |e| match e {
        Event::PropertyNotify(p) if p.window == win => Some(p.time),
        _ => None,
    })?;
    conn.set_selection_owner(win, a.clipboard, time)
        .map_err(err)?;
    let owner = conn
        .get_selection_owner(a.clipboard)
        .map_err(err)?
        .reply()
        .map_err(err)?
        .owner;
    if owner != win {
        return Err("another client kept CLIPBOARD".into());
    }
    std::thread::Builder::new()
        .name("bato-clip-x11".into())
        .spawn(move || serve(conn, win, a, served, time))
        .map_err(err)?;
    Ok(())
}

fn serve(conn: RustConnection, win: Window, a: Atoms, served: Vec<(Atom, Vec<u8>)>, time: u32) {
    // Leave room for the request header.
    let max = conn.maximum_request_bytes().saturating_sub(64);
    while let Ok(ev) = conn.wait_for_event() {
        match ev {
            Event::SelectionClear(e) if e.selection == a.clipboard => break,
            Event::SelectionRequest(e) => {
                // Obsolete clients pass no property: use the target's name.
                let prop = if e.property == NONE {
                    e.target
                } else {
                    e.property
                };
                let ok = if e.selection != a.clipboard {
                    false
                } else if e.target == a.targets {
                    let mut list = vec![a.targets, a.timestamp];
                    list.extend(served.iter().map(|(t, _)| *t));
                    conn.change_property32(
                        PropMode::REPLACE,
                        e.requestor,
                        prop,
                        AtomEnum::ATOM,
                        &list,
                    )
                    .is_ok()
                } else if e.target == a.timestamp {
                    conn.change_property32(
                        PropMode::REPLACE,
                        e.requestor,
                        prop,
                        AtomEnum::INTEGER,
                        &[time],
                    )
                    .is_ok()
                } else if let Some((_, data)) = served.iter().find(|(t, _)| *t == e.target) {
                    data.len() <= max
                        && conn
                            .change_property8(PropMode::REPLACE, e.requestor, prop, e.target, data)
                            .is_ok()
                } else {
                    false
                };
                let notify = SelectionNotifyEvent {
                    response_type: SELECTION_NOTIFY_EVENT,
                    sequence: 0,
                    time: e.time,
                    requestor: e.requestor,
                    selection: e.selection,
                    target: e.target,
                    property: if ok { prop } else { NONE },
                };
                let _ = conn.send_event(false, e.requestor, EventMask::NO_EVENT, notify);
                let _ = conn.flush();
            }
            _ => {}
        }
    }
    let _ = conn.destroy_window(win);
    let _ = conn.flush();
}

/// Convert CLIPBOARD to `target` into our property; `None` when refused.
fn convert(
    conn: &RustConnection,
    win: Window,
    a: &Atoms,
    target: Atom,
) -> Result<Option<(u8, Vec<u8>)>, String> {
    conn.convert_selection(win, a.clipboard, target, a.prop, CURRENT_TIME)
        .map_err(err)?;
    conn.flush().map_err(err)?;
    let property = wait_for(conn, Instant::now() + READ_TIMEOUT, |e| match e {
        Event::SelectionNotify(n) if n.requestor == win && n.target == target => Some(n.property),
        _ => None,
    })?;
    if property == NONE {
        return Ok(None);
    }
    let reply = conn
        .get_property(true, win, property, AtomEnum::ANY, 0, (MAX_READ / 4) as u32)
        .map_err(err)?
        .reply()
        .map_err(err)?;
    if reply.type_ == a.incr {
        return Err("the clipboard owner sent an INCR transfer (unsupported)".into());
    }
    if reply.bytes_after > 0 {
        return Err("clipboard content too large".into());
    }
    Ok(Some((reply.format, reply.value)))
}

pub(super) fn paste(
    display: Option<&str>,
    to_fetch: &dyn Fn(&[String]) -> Vec<&'static str>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    read(display, to_fetch).map(|(_, f)| f)
}

pub(super) fn types(display: Option<&str>) -> Result<Vec<String>, String> {
    read(display, &|_| Vec::new()).map(|(t, _)| t)
}

type Read = (Vec<String>, Vec<(String, Vec<u8>)>);

fn read(
    display: Option<&str>,
    to_fetch: &dyn Fn(&[String]) -> Vec<&'static str>,
) -> Result<Read, String> {
    let (conn, win, a) = open(display)?;
    let result = (|| {
        let owner = conn
            .get_selection_owner(a.clipboard)
            .map_err(err)?
            .reply()
            .map_err(err)?
            .owner;
        if owner == NONE {
            return Ok((Vec::new(), Vec::new()));
        }
        let Some((32, raw)) = convert(&conn, win, &a, a.targets)? else {
            return Ok((Vec::new(), Vec::new()));
        };
        let mut names = Vec::new();
        for c in raw.as_chunks::<4>().0 {
            let atom = u32::from_ne_bytes(*c);
            if atom == a.targets || atom == a.timestamp {
                continue;
            }
            let name = conn
                .get_atom_name(atom)
                .map_err(err)?
                .reply()
                .map_err(err)?
                .name;
            names.push(String::from_utf8_lossy(&name).into_owned());
        }
        let mut fetched = Vec::new();
        for want in to_fetch(&names) {
            let Some(name) = names.iter().find(|n| n.eq_ignore_ascii_case(want)) else {
                continue;
            };
            let atom = intern(&conn, name)?;
            if let Some((8, data)) = convert(&conn, win, &a, atom)? {
                fetched.push((want.to_string(), data));
            }
        }
        Ok((names, fetched))
    })();
    let _ = conn.destroy_window(win);
    let _ = conn.flush();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::{Files, Op, offers, parse};
    use std::io::BufRead;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    /// A private, headless X server (Xvfb: nothing is shown anywhere).
    struct Xvfb(std::process::Child, String);

    impl Xvfb {
        fn start() -> Option<Self> {
            let mut child = Command::new("Xvfb")
                .args([
                    "-displayfd",
                    "1",
                    "-nolisten",
                    "tcp",
                    "-screen",
                    "0",
                    "16x16x24",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            let mut line = String::new();
            std::io::BufReader::new(child.stdout.take()?)
                .read_line(&mut line)
                .ok()?;
            let n: u32 = line.trim().parse().ok()?;
            Some(Self(child, format!(":{n}")))
        }
    }

    impl Drop for Xvfb {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn copy_and_paste_files_over_x11() {
        let Some(x) = Xvfb::start() else {
            eprintln!("skipped: no Xvfb");
            return;
        };
        let d = Some(x.1.as_str());
        assert_eq!(types(d).unwrap(), Vec::<String>::new(), "empty at first");
        let paths = vec![PathBuf::from("/d/a b.txt"), PathBuf::from("/d/c")];
        copy(d, offers(&paths, Op::Cut)).unwrap();
        let t = types(d).unwrap();
        for want in [
            "text/uri-list",
            "x-special/gnome-copied-files",
            "application/x-kde-cutselection",
            "UTF8_STRING",
        ] {
            assert!(t.iter().any(|x| x == want), "{want} missing from {t:?}");
        }
        let fetched = paste(d, &crate::clipboard::to_fetch).unwrap();
        assert_eq!(
            parse(&fetched),
            Some(Files {
                op: Op::Cut,
                paths: paths.clone(),
                other: vec![]
            })
        );
        // Text consumers get the paths.
        let text = paste(d, &|_| vec!["UTF8_STRING"]).unwrap();
        assert_eq!(text[0].1, b"/d/a b.txt\n/d/c");
        // A second copy takes over (the first owner's thread ends).
        let one = vec![PathBuf::from("/e")];
        copy(d, offers(&one, Op::Copy)).unwrap();
        let fetched = paste(d, &crate::clipboard::to_fetch).unwrap();
        assert_eq!(
            parse(&fetched).map(|f| (f.op, f.paths)),
            Some((Op::Copy, one))
        );
    }
}
