//! Windows notifications: a balloon from a notification-area icon (shown as
//! a toast on Windows 10 and 11). One hidden window on its own thread owns
//! the icon; the icon is added for a notification and removed when the
//! balloon is clicked, closed or times out, so no tray icon lingers.

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use futures::stream::BoxStream;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_NOSOUND, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, IDI_APPLICATION, LoadIconW,
    MSG, PostMessageW, RegisterClassW, TranslateMessage, WM_APP, WM_USER, WNDCLASSW,
};

use super::{Click, DEFAULT_ACTION, Notification};
use crate::identity::AppIdentity;
use crate::win::wide;

const WM_SHOW: u32 = WM_APP + 1;
const WM_ICON: u32 = WM_APP + 2;
const NIN_BALLOONHIDE: u32 = WM_USER + 3;
const NIN_BALLOONTIMEOUT: u32 = WM_USER + 4;
const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;
const ICON_ID: u32 = 1;

struct Pending {
    tip: String,
    n: Notification,
    id: u32,
    reply: oneshot::Sender<bool>,
}

#[derive(Default)]
struct Shared {
    hwnd: Mutex<Option<isize>>,
    queue: Mutex<VecDeque<Pending>>,
    listeners: Mutex<Vec<mpsc::UnboundedSender<Click>>>,
    /// The notification the icon shows now.
    current: AtomicU32,
    next: AtomicU32,
}

fn shared() -> &'static Shared {
    static S: OnceLock<Shared> = OnceLock::new();
    S.get_or_init(Shared::default)
}

/// Sends notifications and reports clicks on them. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Notifier {
    app_name: String,
}

impl Hash for Notifier {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.app_name.hash(state);
    }
}

impl Notifier {
    pub fn new(app: &AppIdentity) -> Self {
        Self {
            app_name: app.product_name.to_string(),
        }
    }

    /// Show `n`; its id, or `None` when the shell refused it.
    pub async fn notify(&self, n: &Notification) -> Option<u32> {
        let hwnd = window()?;
        let s = shared();
        let id = s.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        s.queue.lock().unwrap().push_back(Pending {
            tip: self.app_name.clone(),
            n: n.clone(),
            id,
            reply: tx,
        });
        // SAFETY: posting to our own window; a failure only drops the request.
        unsafe { PostMessageW(hwnd as HWND, WM_SHOW, 0, 0) };
        rx.await.ok().filter(|ok| *ok).map(|_| id)
    }

    /// [`notify`](Self::notify) from a plain thread.
    pub fn notify_blocking(&self, n: &Notification) -> bool {
        futures::executor::block_on(self.notify(n)).is_some()
    }

    /// Clicks on this process's notifications.
    pub async fn clicks(&self) -> Option<BoxStream<'static, Click>> {
        let (tx, rx) = mpsc::unbounded();
        shared().listeners.lock().unwrap().push(tx);
        Some(rx.boxed())
    }
}

/// The hidden window, started once.
fn window() -> Option<isize> {
    let s = shared();
    let mut guard = s.hwnd.lock().unwrap();
    if let Some(h) = *guard {
        return Some(h);
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("bato-notify".into())
        .spawn(move || {
            let class = wide("BatoShellNotify");
            // SAFETY: a window class and window owned by this thread, pumped below.
            unsafe {
                let module = GetModuleHandleW(std::ptr::null());
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(proc_),
                    hInstance: module,
                    lpszClassName: class.as_ptr(),
                    ..std::mem::zeroed()
                };
                RegisterClassW(&wc);
                let hwnd = CreateWindowExW(
                    0,
                    class.as_ptr(),
                    class.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    module,
                    std::ptr::null(),
                );
                let _ = tx.send((!hwnd.is_null()).then_some(hwnd as isize));
                if hwnd.is_null() {
                    return;
                }
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        })
        .ok()?;
    let h = rx.recv().ok().flatten()?;
    *guard = Some(h);
    Some(h)
}

/// Copy `s` into a fixed UTF-16 field, truncated, NUL-terminated.
fn fill(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    // SAFETY: a plain-data struct; zero is its empty value.
    let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = hwnd;
    d.uID = ICON_ID;
    d
}

unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let s = shared();
    match msg {
        WM_SHOW => {
            let pending: Vec<Pending> = s.queue.lock().unwrap().drain(..).collect();
            for p in pending {
                let mut d = icon_data(hwnd);
                d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
                d.uCallbackMessage = WM_ICON;
                // SAFETY: the app's icon resource 1 when it has one, else the
                // stock application icon.
                unsafe {
                    d.hIcon = LoadIconW(GetModuleHandleW(std::ptr::null()), 1 as _);
                    if d.hIcon.is_null() {
                        d.hIcon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
                    }
                }
                fill(&mut d.szTip, &p.tip);
                // SAFETY: `d` is a valid NOTIFYICONDATAW for our window.
                unsafe {
                    if Shell_NotifyIconW(NIM_MODIFY, &d) == 0 {
                        Shell_NotifyIconW(NIM_ADD, &d);
                    }
                }
                let mut b = icon_data(hwnd);
                b.uFlags = NIF_INFO;
                fill(&mut b.szInfoTitle, &p.n.title);
                fill(
                    &mut b.szInfo,
                    if p.n.body.is_empty() { " " } else { &p.n.body },
                );
                b.dwInfoFlags = NIIF_INFO | if p.n.silent { NIIF_NOSOUND } else { 0 };
                // SAFETY: as above.
                let ok = unsafe { Shell_NotifyIconW(NIM_MODIFY, &b) } != 0;
                if ok {
                    s.current.store(p.id, Ordering::Relaxed);
                }
                let _ = p.reply.send(ok);
            }
            0
        }
        WM_ICON => {
            let event = (lp as u32) & 0xffff;
            match event {
                NIN_BALLOONUSERCLICK | NIN_BALLOONTIMEOUT | NIN_BALLOONHIDE => {
                    if event == NIN_BALLOONUSERCLICK {
                        let click = Click {
                            id: s.current.load(Ordering::Relaxed),
                            action: DEFAULT_ACTION.to_string(),
                            activation_token: None,
                        };
                        s.listeners
                            .lock()
                            .unwrap()
                            .retain(|l| l.unbounded_send(click.clone()).is_ok());
                    }
                    let d = icon_data(hwnd);
                    // SAFETY: removes our own icon.
                    unsafe { Shell_NotifyIconW(NIM_DELETE, &d) };
                }
                _ => {}
            }
            0
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_truncate() {
        let mut f = [7u16; 4];
        fill(&mut f, "abcdef");
        assert_eq!(f, [97, 98, 99, 0]);
    }
}
