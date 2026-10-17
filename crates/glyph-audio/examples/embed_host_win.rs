//! Windows check for the in-window embed: a GPUI-free parent window (tao, main
//! thread) holds a RefCell borrow while it calls the embed API on every frame,
//! like GPUI does. Any re-entrant window-procedure call during that borrow is
//! the bug this design avoids; the example counts them and fails on any.
//!
//! `cargo run -p glyph-audio --example embed_host_win -- [youtube-id]`

fn main() {
    #[cfg(windows)]
    win::main();
    #[cfg(not(windows))]
    eprintln!("embed_host_win only runs on Windows");
}

#[cfg(windows)]
mod win {
    use glyph_audio::*;
    use glyph_protocol::{PlaybackSource, ProviderId, Track};
    use raw_window_handle::HasWindowHandle;
    use std::cell::RefCell;
    use std::time::{Duration, Instant};
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::window::WindowBuilder;

    thread_local! { static STATE: RefCell<u64> = const { RefCell::new(0) }; }
    static REENTRANT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    static IN_BORROW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    pub fn main() {
        use std::sync::atomic::Ordering::SeqCst;
        let id = std::env::args().nth(1).unwrap_or_else(|| "jNQXAC9IVRw".into());
        let mut event_loop = EventLoop::new();
        let window = WindowBuilder::new().with_title("embed host test").with_inner_size(tao::dpi::LogicalSize::new(900.0, 600.0)).build(&event_loop).unwrap();
        let player = PlayerController::new().unwrap();
        let t0 = Instant::now();
        player.attach_embed_host(window.window_handle().unwrap().as_raw()).expect("attach");
        println!("attach call took {:?}", t0.elapsed());
        player.set_embed_bounds(300.0, 100.0, 480.0, 270.0);
        player.set_embed_visible(true);
        let t = Track {
            id: id.clone(), source: ProviderId::YouTube, title: "t".into(), artist: "-".into(), album: None,
            duration_ms: None, cover_url: None, isrc: None, permalink: None, external_ids: Default::default(), explicit: false,
        };
        player.load(t, PlaybackSource::Embed { provider: ProviderId::YouTube, embed_id: id });
        let start = Instant::now();
        let mut frame = 0u32;
        use tao::platform::run_return::EventLoopExtRunReturn;
        event_loop.run_return(|event, _, flow| {
            *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(16));
            if IN_BORROW.load(SeqCst) {
                REENTRANT.fetch_add(1, SeqCst); // any handler entry while "borrowed"
            }
            if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
                *flow = ControlFlow::Exit;
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
            if start.elapsed() > Duration::from_secs(14) {
                *flow = ControlFlow::Exit;
            }
        });
        let s = player.state();
        println!("status={:?} pos={:?} dur={:?} frames={frame} reentrant={}", s.status, s.position, s.duration, REENTRANT.load(SeqCst));
        assert_eq!(REENTRANT.load(SeqCst), 0, "re-entrant window events during borrow");
        assert!(matches!(s.status, Status::Playing), "embed did not reach Playing");
    }
}
