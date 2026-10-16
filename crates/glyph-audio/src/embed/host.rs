//! Embed player as a child webview inside the app's main window ("combo mode").
//!
//! GPUI owns the main thread, and WebView2 / WKWebView / WebKitGTK objects
//! must be created and driven there. So:
//!
//! * [`attach`] (main thread) creates the `wry` child webview and parks it in a
//!   thread-local (a `WebView` is not `Send`).
//! * Everything else (script evaluation, navigation, bounds, visibility) is
//!   pushed from any thread into a coalescing [`CmdQueue`] ([`HostQueue`]).
//! * The UI thread calls [`pump`] (`PlayerController::pump_embed`) once per
//!   frame; it applies the queue to the webview. On Linux it also advances the
//!   GTK main loop, which WebKitGTK needs.
//!
//! **Windows is different**: Win32 calls on a child HWND (create, SetWindowPos,
//! ShowWindow, WebView2 controller) make Windows send synchronous messages
//! (`WM_NCHITTEST`, `WM_SETCURSOR`, `WM_WINDOWPOSCHANGED`, ...) to the parent.
//! Made from inside GPUI's window procedure those re-enter GPUI while it holds
//! its state borrowed ("RefCell already borrowed"). So on Windows the child
//! webview is created and driven on a dedicated thread with its own message
//! loop (the HWND is only used as the parent); cross-thread messages then
//! reach GPUI through its normal message loop. The UI thread never touches
//! the child: the queue pushes wake that thread, and `pump_embed` is a no-op.
//!
//! Wayland has no child-window support in wry; `attach` then returns an error
//! and the caller keeps the popup/browser fallback.

use super::EventSink;
use anyhow::{anyhow, Result};
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
#[cfg(not(target_os = "windows"))]
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;

/// YouTube's minimum player size.
pub const MIN_SIDE: f32 = 200.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Sanitise a UI rect: non-finite values become 0/min, size is raised to the
/// 200x200 minimum. Returns the rect and whether it had to be changed.
pub fn clamp_bounds(x: f32, y: f32, w: f32, h: f32) -> (Bounds, bool) {
    let fin = |v: f32| if v.is_finite() { v } else { 0.0 };
    let (x, y, w, h) = (fin(x), fin(y), fin(w), fin(h));
    let cw = w.max(MIN_SIDE);
    let ch = h.max(MIN_SIDE);
    (Bounds { x, y, w: cw, h: ch }, cw != w || ch != h)
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostCmd {
    Eval(String),
    Navigate(String),
    /// What the UI wants (`set_embed_visible`).
    UiVisible(bool),
    /// The engine parked the embed (native track playing / stopped).
    Parked(bool),
    Bounds(Bounds),
}

/// Ordered command queue; state-like commands keep only the latest value.
#[derive(Default, Debug)]
pub struct CmdQueue {
    cmds: Vec<HostCmd>,
}

impl CmdQueue {
    pub fn push(&mut self, c: HostCmd) {
        match &c {
            HostCmd::Bounds(_) | HostCmd::UiVisible(_) | HostCmd::Parked(_) => {
                let d = std::mem::discriminant(&c);
                self.cmds.retain(|o| std::mem::discriminant(o) != d);
            }
            HostCmd::Navigate(_) => {
                // A new page supersedes scripts aimed at the old one.
                self.cmds.retain(|o| !matches!(o, HostCmd::Eval(_) | HostCmd::Navigate(_)));
            }
            HostCmd::Eval(_) => {}
        }
        self.cmds.push(c);
    }
    pub fn drain(&mut self) -> Vec<HostCmd> {
        std::mem::take(&mut self.cmds)
    }
    pub fn len(&self) -> usize {
        self.cmds.len()
    }
    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }
}

/// Shared between the controller, the engine's transport and the UI thread.
#[derive(Default)]
pub struct HostQueue {
    q: Mutex<CmdQueue>,
    attached: AtomicBool,
    /// 0 = idle, 1 = attaching (Windows thread starting), 2 = settled (ok or failed).
    phase: AtomicU8,
    sink: Mutex<Option<EventSink>>,
    /// Wakes the thread that owns the webview (Windows).
    waker: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl HostQueue {
    pub fn push(&self, c: HostCmd) {
        self.q.lock().unwrap().push(c);
        if let Some(w) = self.waker.lock().unwrap().as_ref() {
            w();
        }
    }
    /// Windows attaches asynchronously (the UI thread must keep pumping its
    /// message loop meanwhile). Wait, off the UI thread, until it settled.
    pub fn wait_settled(&self, max: std::time::Duration) {
        let end = std::time::Instant::now() + max;
        while self.phase.load(Ordering::SeqCst) == 1 && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    pub fn is_attached(&self) -> bool {
        self.attached.load(Ordering::SeqCst)
    }
    pub fn set_sink(&self, sink: EventSink) {
        *self.sink.lock().unwrap() = Some(sink);
    }
    fn emit(&self, session: u64, ev: super::EmbedEvent) {
        let sink = self.sink.lock().unwrap().clone();
        if let Some(s) = sink {
            s(session, ev);
        }
    }
    fn drain(&self) -> Vec<HostCmd> {
        self.q.lock().unwrap().drain()
    }
}

struct Parent(RawWindowHandle);

impl HasWindowHandle for Parent {
    fn window_handle(&self) -> std::result::Result<WindowHandle<'_>, HandleError> {
        // SAFETY: the caller of `attach_embed_host` guarantees the handle
        // stays valid for the lifetime of the app window.
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

struct HostState {
    webview: wry::WebView,
    ui_visible: bool,
    parked: bool,
}

#[cfg(not(target_os = "windows"))]
thread_local! {
    static HOST: RefCell<Option<HostState>> = const { RefCell::new(None) };
}

fn to_rect(b: Bounds) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(b.x as f64, b.y as f64).into(),
        size: wry::dpi::LogicalSize::new(b.w as f64, b.h as f64).into(),
    }
}

fn build_webview(queue: &std::sync::Arc<HostQueue>, parent: RawWindowHandle) -> Result<wry::WebView> {
    let ipc_q = queue.clone();
    let builder = wry::WebViewBuilder::new()
        .with_bounds(to_rect(Bounds { x: 0.0, y: 0.0, w: MIN_SIDE, h: MIN_SIDE }))
        .with_visible(false)
        .with_background_color((0, 0, 0, 255))
        .with_url("about:blank")
        .with_ipc_handler(move |req| {
            eprintln!("DBG ipc {}", req.body());
            if let Some((s, ev)) = super::parse_message(req.body()) {
                ipc_q.emit(s, ev);
            }
        });
    #[cfg(target_os = "windows")]
    let builder = {
        use wry::WebViewBuilderExtWindows;
        builder.with_additional_browser_args(
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection              --autoplay-policy=no-user-gesture-required",
        )
    };
    builder.build_as_child(&Parent(parent)).map_err(|e| anyhow!("child webview: {e}"))
}

fn apply(st: &mut HostState, c: HostCmd) {
    eprintln!("DBG apply {:?}", c);
    match c {
        HostCmd::Eval(js) => {
            if let Err(e) = st.webview.evaluate_script(&js) {
                tracing::warn!("embed evaluate_script: {e}");
            }
        }
        HostCmd::Navigate(url) => {
            if let Err(e) = st.webview.load_url(&url) {
                tracing::warn!("embed load_url: {e}");
            }
        }
        HostCmd::Bounds(b) => {
            let _ = st.webview.set_bounds(to_rect(b));
        }
        HostCmd::UiVisible(v) => {
            st.ui_visible = v;
            let _ = st.webview.set_visible(st.ui_visible && !st.parked);
        }
        HostCmd::Parked(p) => {
            st.parked = p;
            let _ = st.webview.set_visible(st.ui_visible && !st.parked);
        }
    }
}

/// Create the child webview. On Windows this spawns the embed thread (see the
/// module docs); elsewhere it builds on the calling (main/UI) thread.
pub fn attach(queue: &std::sync::Arc<HostQueue>, parent: RawWindowHandle) -> Result<()> {
    #[cfg(target_os = "windows")]
    return attach_threaded(queue, parent);
    #[cfg(not(target_os = "windows"))]
    {
        #[cfg(target_os = "linux")]
        {
            if matches!(parent, RawWindowHandle::Wayland(_)) {
                return Err(anyhow!("child webviews are not supported on Wayland"));
            }
            gtk::init().map_err(|e| anyhow!("gtk init failed: {e}"))?;
        }
        let webview = build_webview(queue, parent)?;
        HOST.with(|h| *h.borrow_mut() = Some(HostState { webview, ui_visible: false, parked: true }));
        queue.attached.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn attach_threaded(queue: &std::sync::Arc<HostQueue>, parent: RawWindowHandle) -> Result<()> {
    use tao::event::Event;
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::platform::run_return::EventLoopExtRunReturn;
    use tao::platform::windows::EventLoopBuilderExtWindows;

    let RawWindowHandle::Win32(h) = parent else {
        return Err(anyhow!("expected a Win32 window handle"));
    };
    let hwnd = h.hwnd.get();
    if queue.is_attached() {
        return Ok(());
    }
    if queue.phase.swap(1, Ordering::SeqCst) == 1 {
        return Ok(());
    }
    let q = queue.clone();
    // NOTE: must not block the caller. Creating a child of a foreign-thread
    // parent sends messages to the parent, which only the (unblocked) UI
    // thread can answer.
    std::thread::Builder::new().name("glyph-embed-host".into()).spawn(move || {
        let mut builder = EventLoopBuilder::<()>::with_user_event();
        builder.with_any_thread(true);
        let mut event_loop = builder.build();
        let proxy = event_loop.create_proxy();
        let parent = match std::num::NonZeroIsize::new(hwnd) {
            Some(n) => RawWindowHandle::Win32(raw_window_handle::Win32WindowHandle::new(n)),
            None => {
                tracing::error!("embed host: null HWND");
                q.phase.store(2, Ordering::SeqCst);
                return;
            }
        };
        let webview = match build_webview(&q, parent) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("embed host attach failed: {e:#}");
                q.phase.store(2, Ordering::SeqCst);
                return;
            }
        };
        let mut st = HostState { webview, ui_visible: false, parked: true };
        let wake = Mutex::new(proxy);
        *q.waker.lock().unwrap() = Some(Box::new(move || {
            let _ = wake.lock().unwrap().send_event(());
        }));
        q.attached.store(true, Ordering::SeqCst);
        q.phase.store(2, Ordering::SeqCst);
        let q2 = q.clone();
        for c in q.drain() {
            apply(&mut st, c);
        }
        event_loop.run_return(move |event, _, flow| {
            *flow = ControlFlow::Wait;
            if let Event::UserEvent(()) = event {
                for c in q2.drain() {
                    apply(&mut st, c);
                }
            }
        });
    })?;
    Ok(())
}

/// Apply queued commands. Non-Windows: must run on the thread that called
/// [`attach`]. Windows: no-op (the embed thread applies them itself).
pub fn pump(queue: &HostQueue) {
    #[cfg(target_os = "windows")]
    {
        let _ = queue;
    }
    #[cfg(not(target_os = "windows"))]
    {
        #[cfg(target_os = "linux")]
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        HOST.with(|h| {
            let mut h = h.borrow_mut();
            let Some(st) = h.as_mut() else { return };
            for c in queue.drain() {
                apply(st, c);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_to_youtube_minimum() {
        let (b, changed) = clamp_bounds(10.0, 20.0, 150.0, 100.0);
        assert_eq!(b, Bounds { x: 10.0, y: 20.0, w: 200.0, h: 200.0 });
        assert!(changed);
        let (b, changed) = clamp_bounds(0.0, 0.0, 480.0, 270.0);
        assert_eq!((b.w, b.h), (480.0, 270.0));
        assert!(!changed);
    }

    #[test]
    fn clamp_handles_non_finite() {
        let (b, changed) = clamp_bounds(f32::NAN, f32::INFINITY, f32::NAN, 300.0);
        assert_eq!((b.x, b.y, b.w, b.h), (0.0, 0.0, 200.0, 300.0));
        assert!(changed);
    }

    #[test]
    fn queue_coalesces_state_and_keeps_script_order() {
        let mut q = CmdQueue::default();
        let b = |w| HostCmd::Bounds(Bounds { x: 0.0, y: 0.0, w, h: 200.0 });
        q.push(b(300.0));
        q.push(HostCmd::Eval("a".into()));
        q.push(b(400.0));
        q.push(HostCmd::UiVisible(true));
        q.push(HostCmd::Eval("b".into()));
        q.push(HostCmd::UiVisible(false));
        let d = q.drain();
        assert_eq!(
            d,
            vec![HostCmd::Eval("a".into()), b(400.0), HostCmd::Eval("b".into()), HostCmd::UiVisible(false)]
        );
        assert!(q.is_empty());
    }

    #[test]
    fn navigate_drops_stale_scripts() {
        let mut q = CmdQueue::default();
        q.push(HostCmd::Eval("old".into()));
        q.push(HostCmd::Navigate("u1".into()));
        q.push(HostCmd::Navigate("u2".into()));
        q.push(HostCmd::Eval("new".into()));
        assert_eq!(q.drain(), vec![HostCmd::Navigate("u2".into()), HostCmd::Eval("new".into())]);
    }

    #[test]
    fn pump_without_attach_is_a_noop() {
        let q = HostQueue::default();
        q.push(HostCmd::Eval("x".into()));
        pump(&q);
        // nothing attached on this thread: commands stay consumed-or-queued, never panic
        assert!(!q.is_attached());
    }
}
