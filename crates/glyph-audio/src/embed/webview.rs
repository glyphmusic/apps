//! Win32 + wry mini-player window on a dedicated thread (Windows).

use super::server::LocalServer;
use super::{parse_message, EmbedEvent, EventSink, Transport, MINI_SIZE};
use crate::MiniPlayerOptions;
use anyhow::{anyhow, Result};
use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use std::num::NonZeroIsize;
use std::sync::{Arc, Once};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, PeekMessageW, PostThreadMessageW, RegisterClassW, SendMessageW,
    SetWindowPos, ShowWindow, TranslateMessage, HTCAPTION, HWND_NOTOPMOST, HWND_TOPMOST, MSG,
    PM_NOREMOVE, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SW_HIDE, SW_SHOW, WINDOW_EX_STYLE,
    WM_APP, WM_CLOSE, WM_NCLBUTTONDOWN, WNDCLASSW, WS_CLIPCHILDREN, WS_EX_TOPMOST, WS_POPUP,
};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{WebViewBuilder, WebViewBuilderExtWindows};

const WM_MINI_CLOSED: u32 = WM_APP + 1;
const MARGIN: f64 = 24.0;

#[derive(Debug)]
enum Ue {
    Eval(String),
    Visible(bool),
    Options(MiniPlayerOptions),
    Drag,
    Quit,
}

pub struct WebviewTransport {
    tx: flume::Sender<Ue>,
    tid: u32,
}

struct Win32Parent(NonZeroIsize);

impl HasWindowHandle for Win32Parent {
    fn window_handle(&self) -> std::result::Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(self.0));
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CLOSE {
        let _ = ShowWindow(hwnd, SW_HIDE);
        let _ = PostThreadMessageW(GetCurrentThreadId(), WM_MINI_CLOSED, WPARAM(0), LPARAM(0));
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn dpi_scale(hwnd: Option<HWND>) -> f64 {
    let dpi = unsafe {
        match hwnd {
            Some(h) => {
                let d = GetDpiForWindow(h);
                if d != 0 { d } else { GetDpiForSystem() }
            }
            None => GetDpiForSystem(),
        }
    };
    if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 }
}

fn place(hwnd: HWND, opts: &MiniPlayerOptions) {
    let sf = dpi_scale(Some(hwnd));
    let w = (MINI_SIZE.0 * sf).round() as i32;
    let h = (MINI_SIZE.1 * sf).round() as i32;
    let (x, y) = match opts.position {
        Some((lx, ly)) => ((lx * sf).round() as i32, (ly * sf).round() as i32),
        None => {
            let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
            let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
            let margin = (MARGIN * sf).round() as i32;
            let taskbar = (40.0 * sf).round() as i32;
            (
                (screen_w - w - margin).max(0),
                (screen_h - h - margin - taskbar).max(0),
            )
        }
    };
    let z = if opts.always_on_top { HWND_TOPMOST } else { HWND_NOTOPMOST };
    unsafe {
        let _ = SetWindowPos(hwnd, Some(z), x, y, w, h, SWP_NOACTIVATE);
    }
}

impl WebviewTransport {
    pub fn spawn(srv: Arc<LocalServer>, sink: EventSink, options: MiniPlayerOptions) -> Result<Self> {
        let (init_tx, init_rx) = flume::bounded::<Result<(flume::Sender<Ue>, u32), String>>(1);
        std::thread::Builder::new().name("glyph-miniplayer".into()).spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                let mut msg = MSG::default();
                let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
            }
            let tid = unsafe { GetCurrentThreadId() };
            let (tx, rx) = flume::unbounded::<Ue>();

            static CLASS_ONCE: Once = Once::new();
            CLASS_ONCE.call_once(|| unsafe {
                let hinstance = GetModuleHandleW(None).unwrap_or_default();
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(wnd_proc),
                    hInstance: hinstance.into(),
                    lpszClassName: w!("GlyphMiniPlayer"),
                    ..Default::default()
                };
                let _ = RegisterClassW(&wc);
            });

            let built = (|| -> Result<(HWND, wry::WebView)> {
                let sf = dpi_scale(None);
                let w = (MINI_SIZE.0 * sf).round() as i32;
                let h = (MINI_SIZE.1 * sf).round() as i32;
                let ex_style = if options.always_on_top { WS_EX_TOPMOST } else { WINDOW_EX_STYLE(0) };
                let hinstance = unsafe { GetModuleHandleW(None).unwrap_or_default() };
                let hwnd = unsafe {
                    CreateWindowExW(
                        ex_style,
                        w!("GlyphMiniPlayer"),
                        w!("Glyph \u{b7} now playing"),
                        WS_POPUP | WS_CLIPCHILDREN,
                        0,
                        0,
                        w,
                        h,
                        None,
                        None,
                        Some(hinstance.into()),
                        None,
                    )
                }
                .map_err(|e| anyhow!("window: {e}"))?;
                let raw_hwnd = NonZeroIsize::new(hwnd.0 as isize)
                    .ok_or_else(|| anyhow!("window: null HWND"))?;
                place(hwnd, &options);

                let ipc_tx = tx.clone();
                let ipc_sink = sink.clone();
                let send_ue = move |ev: Ue| {
                    if ipc_tx.send(ev).is_ok() {
                        unsafe {
                            let _ = PostThreadMessageW(tid, WM_APP, WPARAM(0), LPARAM(0));
                        }
                    }
                };
                let wb = WebViewBuilder::new()
                    .with_devtools(false)
                    .with_browser_accelerator_keys(false)
                    .with_default_context_menus(false)
                    .with_bounds(wry::Rect {
                        position: LogicalPosition::new(0.0, 0.0).into(),
                        size: LogicalSize::new(MINI_SIZE.0, MINI_SIZE.1).into(),
                    })
                    .with_url(srv.page_url())
                    .with_ipc_handler(move |req| {
                        let body = req.body();
                        match body.as_str() {
                            "drag" => send_ue(Ue::Drag),
                            "close" => {
                                send_ue(Ue::Visible(false));
                                ipc_sink(0, EmbedEvent::Closed);
                            }
                            other => {
                                if let Some((s, ev)) = parse_message(other) {
                                    ipc_sink(s, ev);
                                }
                            }
                        }
                    })
                    .with_additional_browser_args(
                        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
                         --autoplay-policy=no-user-gesture-required",
                    );
                let webview = wb
                    .build(&Win32Parent(raw_hwnd))
                    .map_err(|e| anyhow!("webview: {e}"))?;
                super::harden_wry_webview(&webview);
                Ok((hwnd, webview))
            })();

            let (hwnd, webview) = match built {
                Ok(v) => v,
                Err(e) => {
                    let _ = init_tx.send(Err(format!("{e:#}")));
                    return;
                }
            };
            let _ = init_tx.send(Ok((tx, tid)));

            let mut msg = MSG::default();
            'loop_msg: while unsafe { GetMessageW(&mut msg, None, 0, 0).as_bool() } {
                if msg.message == WM_MINI_CLOSED {
                    sink(0, EmbedEvent::Closed);
                } else if msg.message != WM_APP {
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                while let Ok(ev) = rx.try_recv() {
                    match ev {
                        Ue::Eval(js) => {
                            if let Err(e) = webview.evaluate_script(&js) {
                                tracing::warn!("evaluate_script failed: {e}");
                            }
                        }
                        Ue::Visible(v) => unsafe {
                            let _ = ShowWindow(hwnd, if v { SW_SHOW } else { SW_HIDE });
                        },
                        Ue::Options(o) => place(hwnd, &o),
                        Ue::Drag => unsafe {
                            let _ = ReleaseCapture();
                            SendMessageW(
                                hwnd,
                                WM_NCLBUTTONDOWN,
                                Some(WPARAM(HTCAPTION as usize)),
                                Some(LPARAM(0)),
                            );
                        },
                        Ue::Quit => {
                            unsafe {
                                let _ = DestroyWindow(hwnd);
                            }
                            break 'loop_msg;
                        }
                    }
                }
            }
        })?;
        match init_rx.recv_timeout(std::time::Duration::from_secs(15)) {
            Ok(Ok((tx, tid))) => Ok(Self { tx, tid }),
            Ok(Err(e)) => Err(anyhow!(e)),
            Err(_) => Err(anyhow!("mini-player thread did not start")),
        }
    }

    fn send(&self, e: Ue) {
        if self.tx.send(e).is_ok() {
            unsafe {
                let _ = PostThreadMessageW(self.tid, WM_APP, WPARAM(0), LPARAM(0));
            }
        }
    }
}

impl Transport for WebviewTransport {
    fn eval(&self, js: String) {
        self.send(Ue::Eval(js));
    }
    fn set_visible(&self, visible: bool) {
        self.send(Ue::Visible(visible));
    }
    fn set_options(&self, options: MiniPlayerOptions) {
        self.send(Ue::Options(options));
    }
    fn ensure_open(&self) -> bool {
        false
    }
    fn name(&self) -> &'static str {
        "webview"
    }
}

impl Drop for WebviewTransport {
    fn drop(&mut self) {
        self.send(Ue::Quit);
    }
}
