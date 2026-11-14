//! Windows check for the in-window embed: a GPUI-free parent window (Win32,
//! main thread) holds a RefCell borrow while it calls the embed API on every
//! frame, like GPUI does. Any re-entrant window-procedure call during that
//! borrow is the bug this design avoids; the example counts them and fails on
//! any.
//!
//! `cargo run -p glyph-audio --features embed-host-example --example embed_host_win -- [youtube-id]`

#[cfg(not(windows))]
fn main() {
    eprintln!("embed_host_win is a Windows-only example");
}

#[cfg(windows)]
fn main() {
    use glyph_audio::*;
    use glyph_protocol::{PlaybackSource, ProviderId, Track};
    use raw_window_handle::{RawWindowHandle, Win32WindowHandle};
    use std::cell::RefCell;
    use std::num::NonZeroIsize;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};
    use std::time::{Duration, Instant};
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, PeekMessageW, RegisterClassW,
        TranslateMessage, CW_USEDEFAULT, MSG, PM_REMOVE, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY,
        WNDCLASSW, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    thread_local! { static STATE: RefCell<u64> = const { RefCell::new(0) }; }
    static REENTRANT: AtomicU32 = AtomicU32::new(0);
    static IN_BORROW: AtomicBool = AtomicBool::new(false);
    static CLOSED: AtomicBool = AtomicBool::new(false);

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if IN_BORROW.load(SeqCst) {
            REENTRANT.fetch_add(1, SeqCst);
        }
        if msg == WM_CLOSE || msg == WM_DESTROY {
            CLOSED.store(true, SeqCst);
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    let id = std::env::args().nth(1).unwrap_or_else(|| "jNQXAC9IVRw".into());
    let hinstance = unsafe { GetModuleHandleW(None).unwrap_or_default() };
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance.into(),
        lpszClassName: w!("GlyphEmbedHostTest"),
        ..Default::default()
    };
    unsafe {
        let _ = RegisterClassW(&wc);
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("GlyphEmbedHostTest"),
            w!("embed host test"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            900,
            600,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
    }
    .expect("window");
    let raw = RawWindowHandle::Win32(Win32WindowHandle::new(
        NonZeroIsize::new(hwnd.0 as isize).expect("non-null HWND"),
    ));

    let player = PlayerController::new().unwrap();
    let t0 = Instant::now();
    player.attach_embed_host(raw).expect("attach");
    println!("attach call took {:?}", t0.elapsed());
    player.set_embed_bounds(300.0, 100.0, 480.0, 270.0);
    player.set_embed_visible(true);
    let t = Track {
        id: id.clone(),
        source: ProviderId::YouTube,
        title: "t".into(),
        artist: "-".into(),
        album: None,
        duration_ms: None,
        cover_url: None,
        isrc: None,
        permalink: None,
        external_ids: Default::default(),
        explicit: false,
    };
    player.load(t, PlaybackSource::Embed { provider: ProviderId::YouTube, embed_id: id });
    let start = Instant::now();
    let mut frame = 0u32;
    let mut msg = MSG::default();
    while !CLOSED.load(SeqCst) && start.elapsed() <= Duration::from_secs(14) {
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() } {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        STATE.with(|s| {
            let mut g = s.borrow_mut();
            IN_BORROW.store(true, SeqCst);
            frame += 1;
            *g += 1;
            let w = 400.0 + (frame % 100) as f32;
            player.set_embed_bounds(300.0, 100.0, w, 270.0);
            player.set_embed_visible(frame % 400 < 380);
            player.pump_embed();
            std::thread::sleep(Duration::from_millis(1));
            IN_BORROW.store(false, SeqCst);
        });
        std::thread::sleep(Duration::from_millis(15));
    }
    let s = player.state();
    println!(
        "status={:?} pos={:?} dur={:?} frames={frame} reentrant={}",
        s.status,
        s.position,
        s.duration,
        REENTRANT.load(SeqCst)
    );
    assert_eq!(REENTRANT.load(SeqCst), 0, "re-entrant window events during borrow");
    assert!(matches!(s.status, Status::Playing), "embed did not reach Playing");
}
