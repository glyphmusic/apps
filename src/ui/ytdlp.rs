//! yt-dlp row of the settings (Playback page): status of the local yt-dlp
//! that plays YouTube from this machine, plus "Install / Update" for the
//! app-managed copy. Also the prefetching that keeps yt-dlp out of the way:
//! the next songs of the queue and the row the pointer rests on.

use crate::app::Glyph;
use crate::guard::{guard, guard_async};
use crate::i18n::{t, tf};
use crate::ui::widgets::icon;
use crate::app::track_key;
use glyph_core::ytdlp::YtDlpStatus;
use glyph_protocol::{ProviderId, Track};
use gpui::{div, hsla, prelude::*, px, AnyElement, Context, Hsla, SharedString};
use std::time::Duration;

#[derive(Default)]
pub struct YtDlpUi {
    pub status: Option<YtDlpStatus>,
    pub checking: bool,
    pub installing: bool,
    pub error: Option<String>,
    /// Row the pointer rests on (hover prefetch).
    pub hovered: Option<String>,
}

/// How long the pointer has to rest on a row before yt-dlp starts.
const HOVER_DELAY: Duration = Duration::from_millis(400);
/// Songs of the queue resolved ahead.
const AHEAD: usize = 3;

fn white(a: f32) -> Hsla {
    hsla(0.0, 0.0, 1.0, a)
}

impl Glyph {
    /// Resolves the next songs of the queue in the background (local yt-dlp
    /// only; one batch run), so they start without waiting.
    pub fn prefetch_upcoming(&mut self) {
        let Some(b) = self.backend.as_ref() else { return };
        let next: Vec<Track> = self.queue.upcoming().into_iter().take(AHEAD).cloned().collect();
        guard(|| b.prefetch(next));
    }

    /// Hover over a YouTube song: after a short rest start resolving it, so
    /// a click plays at once.
    pub fn prefetch_on_hover(&mut self, t: &Track, hovered: bool, cx: &mut Context<Self>) {
        let key = track_key(t);
        if !hovered {
            if self.ytdlp.hovered.as_deref() == Some(key.as_str()) {
                self.ytdlp.hovered = None;
            }
            return;
        }
        let youtube = t.source == ProviderId::YouTube || t.external_ids.contains_key(&ProviderId::YouTube);
        let Some(b) = self.backend.clone().filter(|b| youtube && b.local_youtube()) else { return };
        self.ytdlp.hovered = Some(key.clone());
        let t = t.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOVER_DELAY).await;
            this.update(cx, |s, _| {
                if s.ytdlp.hovered.as_deref() == Some(key.as_str()) {
                    guard(|| b.prefetch(vec![t]));
                }
            })
            .ok();
        })
        .detach();
    }

    /// Loads the status once (the settings page calls this while drawing).
    pub fn ensure_ytdlp_status(&mut self, cx: &mut Context<Self>) {
        if self.ytdlp.status.is_some() || self.ytdlp.checking || self.ytdlp.installing {
            return;
        }
        let Some(b) = self.backend.clone() else { return };
        self.ytdlp.checking = true;
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.ytdlp_status()).await;
            this.update(cx, |s, cx| {
                s.ytdlp.checking = false;
                match r {
                    Ok(st) => s.ytdlp.status = Some(st),
                    Err(e) => {
                        s.ytdlp.status = Some(YtDlpStatus::default());
                        s.ytdlp.error = Some(e);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn install_ytdlp(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        if self.ytdlp.installing {
            return;
        }
        self.ytdlp.installing = true;
        self.ytdlp.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.install_ytdlp()).await;
            this.update(cx, |s, cx| {
                s.ytdlp.installing = false;
                match r {
                    Ok(st) => {
                        let v = st.version.clone().unwrap_or_default();
                        s.ytdlp.status = Some(st);
                        s.toast_info(tf("ytdlp.installed", &[("v", &v)]));
                    }
                    Err(e) => {
                        s.toast_info(tf("ytdlp.install_error", &[("e", &e)]));
                        s.ytdlp.error = Some(e);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// Description line under "yt-dlp": found / version / JS runtime hint.
pub fn description(app: &Glyph) -> SharedString {
    let ui = &app.ytdlp;
    if ui.installing {
        return t("ytdlp.installing");
    }
    let Some(st) = &ui.status else { return t("ytdlp.checking") };
    let mut s = match (&st.path, &st.version) {
        (None, _) => t("ytdlp.missing").to_string(),
        (Some(_), None) => t("ytdlp.broken").to_string(),
        (Some(_), Some(v)) => {
            let js = st.js_runtime.as_deref().unwrap_or("-");
            tf("ytdlp.found", &[("v", &v.as_str()), ("js", &js)]).to_string()
        }
    };
    if st.path.is_some() && st.js_runtime.is_none() {
        s.push_str(" · ");
        s.push_str(&t("ytdlp.no_js"));
    }
    if let Some(e) = &ui.error {
        s.push_str(" · ");
        s.push_str(e);
    }
    s.into()
}

/// Right side of the row: a status dot and, where it applies, the
/// Install / Update button for the app-managed copy.
pub fn control(app: &mut Glyph, cx: &mut Context<Glyph>) -> AnyElement {
    app.ensure_ytdlp_status(cx);
    let ui = &app.ytdlp;
    let st = ui.status.clone().unwrap_or_default();
    let ok = st.version.is_some();
    let dot = div().size(px(8.0)).rounded_full().bg(if ok { gpui::white() } else { white(0.25) });
    // A yt-dlp from PATH or the config belongs to the user: no button.
    let show_button = ui.status.is_some() && st.can_install && (st.path.is_none() || st.managed);
    let label = if ui.installing {
        t("ytdlp.installing_short")
    } else if st.managed {
        t("ytdlp.update")
    } else {
        t("ytdlp.install")
    };
    let busy = ui.installing;
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(dot)
        .when(show_button, |d| {
            d.child(
                div()
                    .id("ytdlp-install")
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .h(px(32.0))
                    .px(px(12.0))
                    .rounded(px(8.0))
                    .bg(white(0.07))
                    .text_size(px(13.0))
                    .text_color(if busy { white(0.5) } else { gpui::white() })
                    .when(!busy, |d| d.cursor_pointer().hover(|s| s.bg(white(0.12))))
                    .child(icon("download", 14.0, white(0.8)))
                    .child(label)
                    .on_click(cx.listener(|this, _, _, cx| this.install_ytdlp(cx))),
            )
        })
        .into_any_element()
}
