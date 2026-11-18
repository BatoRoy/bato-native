//! Windows single instance: a named pipe instead of the unix socket.
//!
//! The first launch creates `\\.\pipe\<%TEMP%\app_id with / for \>` with
//! `FILE_FLAG_FIRST_PIPE_INSTANCE` and serves it on a thread; a later launch
//! gets `ERROR_ACCESS_DENIED` creating it, connects instead, sends a
//! length-prefixed request and waits for the acknowledgement. Remote
//! clients are rejected; the per-user temp folder keeps users apart.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

use super::{ACK, CONNECT_RETRY, Inner, Instance, MAGIC, MAX_REQUEST, Primary, Request};
use crate::win::wide;

/// `\\.\pipe\` + the folder path with `/` for `\` (a pipe name may hold
/// anything but a backslash).
pub(super) fn pipe_name(dir: &Path) -> String {
    format!(
        r"\\.\pipe\bato-{}",
        dir.to_string_lossy().replace('\\', "/")
    )
}

impl Request {
    /// `MAGIC`, then NUL-terminated UTF-8 fields: token, cwd, argv….
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        let mut field = |b: &str| {
            out.extend_from_slice(b.as_bytes());
            out.push(0);
        };
        field(self.activation_token.as_deref().unwrap_or(""));
        field(&self.cwd.to_string_lossy());
        for a in &self.argv {
            field(&a.to_string_lossy());
        }
        out
    }

    pub(super) fn decode(b: &[u8]) -> Option<Self> {
        let rest = b.strip_prefix(MAGIC)?;
        let body = rest.strip_suffix(b"\0")?;
        let mut fields = body.split(|&c| c == 0);
        let text = |f: &[u8]| String::from_utf8(f.to_vec()).ok();
        let token = text(fields.next()?)?;
        let cwd = PathBuf::from(text(fields.next()?)?);
        let argv = fields
            .map(|f| text(f).map(OsString::from))
            .collect::<Option<_>>()?;
        Some(Self {
            argv,
            cwd,
            activation_token: (!token.is_empty()).then_some(token),
        })
    }
}

fn create(name: &[u16], first: bool) -> io::Result<isize> {
    let flags = PIPE_ACCESS_DUPLEX
        | if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    // SAFETY: a NUL-terminated name; default security (the creator, SYSTEM
    // and administrators get full access).
    let h = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            flags,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            4096,
            4096,
            0,
            std::ptr::null(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(h as isize)
}

pub(super) fn acquire_at(dir: &Path, request: Request) -> io::Result<Instance> {
    let name = pipe_name(dir);
    let wname = wide(&name);
    match create(&wname, true) {
        Ok(h) => Ok(Instance::Primary(serve(name, wname, h))),
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
            forward(&name, &request)?;
            Ok(Instance::Forwarded)
        }
        Err(e) => Err(e),
    }
}

fn serve(name: String, wname: Vec<u16>, first: isize) -> Primary {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::unbounded();
    {
        let stop = stop.clone();
        let _ = std::thread::Builder::new()
            .name("bato-instance".into())
            .spawn(move || {
                let mut h = first;
                loop {
                    // SAFETY: our own pipe handle; ERROR_PIPE_CONNECTED means
                    // a client got in between create and connect.
                    let ok = unsafe { ConnectNamedPipe(h as _, std::ptr::null_mut()) } != 0
                        || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
                    if stop.load(Ordering::SeqCst) {
                        // SAFETY: closing our own handle.
                        unsafe { CloseHandle(h as _) };
                        break;
                    }
                    if ok && let Some(req) = receive(h) {
                        // Queued before the acknowledgement, as on unix.
                        let _ = tx.unbounded_send(req);
                        // SAFETY: borrowed for one write; not closed here.
                        let mut f =
                            std::mem::ManuallyDrop::new(unsafe { File::from_raw_handle(h as _) });
                        let _ = f.write_all(ACK);
                        let _ = f.flush();
                    }
                    // SAFETY: ready the same instance for the next client.
                    unsafe { DisconnectNamedPipe(h as _) };
                    if !ok {
                        // A broken instance: replace it.
                        // SAFETY: closing our own handle.
                        unsafe { CloseHandle(h as _) };
                        match create(&wname, false) {
                            Ok(n) => h = n,
                            Err(_) => break,
                        }
                    }
                }
            });
    }
    Primary {
        inner: Arc::new(Inner {
            socket: PathBuf::from(name),
            stop,
            requests: Mutex::new(Some(rx)),
        }),
    }
}

/// A 4-byte little-endian length, then the encoded request.
fn receive(h: isize) -> Option<Request> {
    // SAFETY: borrowed for these reads; not closed here.
    let mut f = std::mem::ManuallyDrop::new(unsafe { File::from_raw_handle(h as _) });
    let mut len = [0u8; 4];
    f.read_exact(&mut len).ok()?;
    let len = u32::from_le_bytes(len) as u64;
    if len > MAX_REQUEST {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf).ok()?;
    Request::decode(&buf)
}

fn forward(name: &str, request: &Request) -> io::Result<()> {
    // The running instance may raise its window for this launch.
    // SAFETY: no preconditions.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    let end = Instant::now() + CONNECT_RETRY;
    let mut pipe = loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(name)
        {
            Ok(f) => break f,
            // Busy (another launch is talking to it) or not created yet.
            Err(_) if Instant::now() < end => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => {
                return Err(io::Error::new(
                    e.kind(),
                    format!("the running instance does not answer on {name}: {e}"),
                ));
            }
        }
    };
    let body = request.encode();
    let mut msg = (body.len() as u32).to_le_bytes().to_vec();
    msg.extend_from_slice(&body);
    pipe.write_all(&msg)?;
    let mut ack = [0u8; 3];
    pipe.read_exact(&mut ack)?;
    if ack != ACK {
        return Err(io::Error::other(
            "the running instance did not take the request",
        ));
    }
    Ok(())
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag and closes the pipe.
        let _ = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.socket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        let r = Request {
            argv: vec!["nem://join?x=1".into(), "C:\\a b".into()],
            cwd: "C:\\Users\\me".into(),
            activation_token: None,
        };
        assert_eq!(Request::decode(&r.encode()), Some(r));
        assert!(pipe_name(Path::new("C:\\T\\app")).ends_with("bato-C:/T/app"));
    }
}
