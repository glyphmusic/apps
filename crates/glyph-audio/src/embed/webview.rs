//! tao + wry mini-player window on a dedicated thread (Windows, Linux).

use super::server::LocalServer;
use super::{parse_message, EmbedEvent, EventSink, Transport, MINI_SIZE};
use crate::MiniPlayerOptions;
use anyhow::{anyhow, Result};
use std::sync::{Arc, Mutex};
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::{Window, WindowBuilder};
use wry::WebViewBuilder;

#[cfg(target_os = "linux")]
use tao::platform::unix::{EventLoopBuilderExtUnix, WindowExtUnix};
#[cfg(target_os = "windows")]
use tao::platform::windows::EventLoopBuilderExtWindows;
#[cfg(target_os = "linux")]
use wry::WebViewBuilderExtUnix;
#[cfg(target_os = "windows")]
use wry::WebViewBuilderExtWindows;

#[derive(Debug)]
enum Ue {
    Eval(String),
    Visible(bool),
    Options(MiniPlayerOptions),
    Drag,
    Quit,
}

pub struct WebviewTransport {
    proxy: Mutex<EventLoopProxy<Ue>>,
}

const MARGIN: f64 = 24.0;

fn place(window: &Window, opts: &MiniPlayerOptions) {
    let pos = opts.position.or_else(|| {
        let m = window.primary_monitor()?;
        let sf = m.scale_factor();
        let size = m.size().to_logical::<f64>(sf);
        let origin = m.position().to_logical::<f64>(sf);
        Some((
            origin.x + size.width - MINI_SIZE.0 - MARGIN,
            origin.y + size.height - MINI_SIZE.1 - MARGIN - 40.0,
        ))
    });
    if let Some((x, y)) = pos {
        window.set_outer_position(LogicalPosition::new(x, y));
    }
}

impl WebviewTransport {
    pub fn spawn(srv: Arc<LocalServer>, sink: EventSink, options: MiniPlayerOptions) -> Result<Self> {
        let (init_tx, init_rx) = flume::bounded::<Result<EventLoopProxy<Ue>, String>>(1);
        std::thread::Builder::new().name("glyph-miniplayer".into()).spawn(move || {
            let mut builder = EventLoopBuilder::<Ue>::with_user_event();
            builder.with_any_thread(true);
            let mut event_loop = builder.build();
            let proxy = event_loop.create_proxy();

            let built = (|| -> Result<(Window, wry::WebView)> {
                let window = WindowBuilder::new()
                    .with_title("Glyph \u{b7} now playing")
                    .with_decorations(false)
                    .with_resizable(false)
                    .with_inner_size(LogicalSize::new(MINI_SIZE.0, MINI_SIZE.1))
                    .with_always_on_top(options.always_on_top)
                    .with_visible(false)
                    .build(&event_loop)
                    .map_err(|e| anyhow!("window: {e}"))?;
                place(&window, &options);

                let ipc_proxy = proxy.clone();
                let ipc_sink = sink.clone();
                let wb = WebViewBuilder::new()
                    .with_url(srv.page_url())
                    .with_ipc_handler(move |req| {
                        let body = req.body();
                        match body.as_str() {
                            "drag" => {
                                let _ = ipc_proxy.send_event(Ue::Drag);
                            }
                            "close" => {
                                let _ = ipc_proxy.send_event(Ue::Visible(false));
                                ipc_sink(0, EmbedEvent::Closed);
                            }
                            other => {
                                if let Some((s, ev)) = parse_message(other) {
                                    ipc_sink(s, ev);
                                }
                            }
                        }
                    });
                #[cfg(target_os = "windows")]
                let wb = wb.with_additional_browser_args(
                    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
                     --autoplay-policy=no-user-gesture-required",
                );
                #[cfg(target_os = "linux")]
                let webview = {
                    let vbox = window.default_vbox().ok_or_else(|| anyhow!("no gtk container"))?;
                    wb.build_gtk(vbox)
                };
                #[cfg(not(target_os = "linux"))]
                let webview = wb.build(&window);
                let webview = webview.map_err(|e| anyhow!("webview: {e}"))?;
                Ok((window, webview))
            })();

            let (window, webview) = match built {
                Ok(v) => v,
                Err(e) => {
                    let _ = init_tx.send(Err(format!("{e:#}")));
                    return;
                }
            };
            let _ = init_tx.send(Ok(proxy));

            event_loop.run_return(move |event, _, flow| {
                *flow = ControlFlow::Wait;
                match event {
                    Event::UserEvent(Ue::Eval(js)) => {
                        if let Err(e) = webview.evaluate_script(&js) {
                            tracing::warn!("evaluate_script failed: {e}");
                        }
                    }
                    Event::UserEvent(Ue::Visible(v)) => window.set_visible(v),
                    Event::UserEvent(Ue::Options(o)) => {
                        window.set_always_on_top(o.always_on_top);
                        place(&window, &o);
                    }
                    Event::UserEvent(Ue::Drag) => {
                        let _ = window.drag_window();
                    }
                    Event::UserEvent(Ue::Quit) => *flow = ControlFlow::Exit,
                    Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                        // Hide instead of destroying; the embed pauses.
                        window.set_visible(false);
                        sink(0, EmbedEvent::Closed);
                    }
                    _ => {}
                }
            });
        })?;
        match init_rx.recv_timeout(std::time::Duration::from_secs(15)) {
            Ok(Ok(proxy)) => Ok(Self { proxy: Mutex::new(proxy) }),
            Ok(Err(e)) => Err(anyhow!(e)),
            Err(_) => Err(anyhow!("mini-player thread did not start")),
        }
    }

    fn send(&self, e: Ue) {
        let _ = self.proxy.lock().unwrap().send_event(e);
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
