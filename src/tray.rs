//! System tray icon (desktop): shown while Glyph runs in the background
//! after a start with the computer (Settings -> System).
//!
//! A click opens the window; the menu has Open, Play / Pause, Next,
//! Previous and Quit. Clicks become in-process requests (`tray show`,
//! `tray dock N`, `tray quit`, see [`crate::instance::post`]) that the app's
//! poll handles.
//! - Windows: `Shell_NotifyIcon` on a hidden window of its own thread.
//! - Linux: a StatusNotifierItem over D-Bus (KDE, GNOME with the
//!   AppIndicator extension, most other panels).
//! - macOS and phones: none (macOS starts hidden; the Dock icon opens it).

/// Menu texts in the app's language.
#[derive(Clone, Debug, Default)]
pub struct Labels {
    pub open: String,
    pub play_pause: String,
    pub next: String,
    pub previous: String,
    pub quit: String,
}

/// The platform has a tray icon.
pub const SUPPORTED: bool = cfg!(any(windows, target_os = "linux", target_os = "freebsd"));

/// Menu entries: label and the request they post.
fn entries(l: &Labels) -> [(&str, &'static str); 5] {
    [
        (&l.open, "tray show"),
        (&l.play_pause, "tray dock 5"),
        (&l.next, "tray dock 6"),
        (&l.previous, "tray dock 7"),
        (&l.quit, "tray quit"),
    ]
}

/// Shows the icon (once; later calls only update the texts).
pub fn show(labels: Labels) {
    imp::show(labels);
}

/// Removes the icon (before quitting, so none is left behind).
pub fn remove() {
    imp::remove();
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::instance::post;
    use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
    use std::sync::Mutex;
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW,
        GetSystemMetrics, LoadImageW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SendMessageW,
        SetForegroundWindow, TrackPopupMenu, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, LR_SHARED, MF_SEPARATOR, MF_STRING, MSG,
        SM_CXSMICON, SM_CYSMICON, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_DESTROY,
        WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
    };

    /// Mouse events on the icon arrive as this message.
    const CALLBACK: u32 = WM_APP + 0x47;
    const ICON_ID: u32 = 1;

    /// The tray's hidden window (0: none).
    static HWND_TRAY: AtomicIsize = AtomicIsize::new(0);
    /// `TaskbarCreated`: Explorer restarted, the icon must be added again.
    static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
    static LABELS: Mutex<Option<Labels>> = Mutex::new(None);

    pub fn show(labels: Labels) {
        let first = {
            let mut g = LABELS.lock().unwrap();
            let first = g.is_none();
            *g = Some(labels);
            first
        };
        if first {
            let _ = std::thread::Builder::new().name("glyph-tray".into()).spawn(|| {
                if let Err(e) = unsafe { run() } {
                    tracing::warn!("tray: {e}");
                }
            });
        }
    }

    pub fn remove() {
        let h = HWND_TRAY.load(Ordering::Acquire);
        if h != 0 {
            // Handled on the tray thread: destroys the window and the icon.
            unsafe { SendMessageW(HWND(h as *mut _), WM_CLOSE, None, None) };
        }
    }

    unsafe fn run() -> windows::core::Result<()> {
        let inst: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst, lpszClassName: w!("GlyphTray"), ..Default::default() };
        RegisterClassW(&class);
        // A top-level window that is never shown (message-only windows miss
        // the `TaskbarCreated` broadcast).
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE::default(), w!("GlyphTray"), w!("Glyph"), WS_OVERLAPPED, 0, 0, 0, 0, None, None, Some(inst), None)?;
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Release);
        HWND_TRAY.store(hwnd.0 as isize, Ordering::Release);
        add_icon(hwnd);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
        HWND_TRAY.store(0, Ordering::Release);
        Ok(())
    }

    fn data(hwnd: HWND) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: ICON_ID, ..Default::default() }
    }

    unsafe fn add_icon(hwnd: HWND) {
        // Icon resource 1 (build.rs), at the small-icon size.
        let inst: Option<HINSTANCE> = GetModuleHandleW(None).ok().map(Into::into);
        let (cx, cy) = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
        let icon = LoadImageW(inst, PCWSTR(1 as _), IMAGE_ICON, cx, cy, LR_DEFAULTCOLOR | LR_SHARED).map(|h| HICON(h.0)).unwrap_or_default();
        let mut nid = data(hwnd);
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = CALLBACK;
        nid.hIcon = icon;
        for (i, c) in "Glyph".encode_utf16().enumerate() {
            nid.szTip[i] = c;
        }
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }

    unsafe fn menu(hwnd: HWND) {
        let Some(labels) = LABELS.lock().unwrap().clone() else { return };
        let Ok(m) = CreatePopupMenu() else { return };
        let items = entries(&labels);
        for (i, (label, _)) in items.iter().enumerate() {
            // Separators around the transport entries.
            if i == 1 || i == 4 {
                let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
            }
            let _ = AppendMenuW(m, MF_STRING, i + 1, &HSTRING::from(*label));
        }
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Without this the menu does not close when clicking elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, None, hwnd, None).0 as usize;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(m);
        if let Some((_, req)) = cmd.checked_sub(1).and_then(|i| items.get(i)) {
            post(req);
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            CALLBACK => {
                match (lparam.0 & 0xffff) as u32 {
                    WM_LBUTTONUP => post("tray show"),
                    WM_RBUTTONUP => menu(hwnd),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data(hwnd));
                PostQuitMessage(0);
                LRESULT(0)
            }
            m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Acquire) => {
                add_icon(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod imp {
    use super::*;
    use crate::instance::post;
    use ksni::blocking::{Handle, TrayMethods};
    use std::sync::Mutex;

    struct GlyphTray {
        labels: Labels,
    }

    impl ksni::Tray for GlyphTray {
        fn id(&self) -> String {
            "glyph".into()
        }

        fn title(&self) -> String {
            "Glyph".into()
        }

        /// Installed by `integrate.rs`; the pixmap covers other installs.
        fn icon_name(&self) -> String {
            "glyph".into()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            [&include_bytes!("../assets/icons/icon-32.png")[..], &include_bytes!("../assets/icons/icon-64.png")[..]]
                .into_iter()
                .filter_map(|bytes| image::load_from_memory(bytes).ok())
                .map(|img| {
                    let img = img.to_rgba8();
                    // ARGB32 in network byte order.
                    let data = img.pixels().flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
                    ksni::Icon { width: img.width() as i32, height: img.height() as i32, data }
                })
                .collect()
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            post("tray show");
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::{MenuItem, StandardItem};
            let mut items = Vec::new();
            for (i, (label, req)) in entries(&self.labels).into_iter().enumerate() {
                if i == 1 || i == 4 {
                    items.push(MenuItem::Separator);
                }
                items.push(StandardItem { label: label.to_string(), activate: Box::new(move |_: &mut Self| post(req)), ..Default::default() }.into());
            }
            items
        }
    }

    static HANDLE: Mutex<Option<Handle<GlyphTray>>> = Mutex::new(None);

    pub fn show(labels: Labels) {
        let mut g = HANDLE.lock().unwrap();
        if let Some(h) = g.as_ref() {
            h.update(move |t| t.labels = labels);
            return;
        }
        match (GlyphTray { labels }).spawn() {
            Ok(h) => *g = Some(h),
            // No StatusNotifierItem host (e.g. GNOME without the extension).
            Err(e) => tracing::warn!("tray: {e}"),
        }
    }

    pub fn remove() {
        if let Some(h) = HANDLE.lock().unwrap().take() {
            let _ = h.shutdown();
        }
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "freebsd")))]
mod imp {
    use super::*;

    pub fn show(_labels: Labels) {
        let _ = entries;
    }

    pub fn remove() {}
}
