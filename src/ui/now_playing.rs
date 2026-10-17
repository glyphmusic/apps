//! Now playing (Liner layout): left cover 280 px (or the official embed
//! player in its place), title/artist, thin seek bar, flat controls, volume;
//! right big lyrics with a small provider/offset pill on top.

use crate::app::Glyph;
use crate::guard::guard;
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::widgets::*;
use glyph_core::RepeatMode;
use gpui::{canvas, div, prelude::*, px, Context, FontWeight, SharedString, Window};
use std::rc::Rc;
use std::time::Duration;

const COVER: f32 = 280.0;

pub fn now_playing(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let Some(track) = app.current.clone() else {
        return div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(ascii_state(ART_EMPTY, t("np.empty.title"), t("np.empty.sub")))
            .into_any_element();
    };
    let liked = app.is_liked(&track);
    let pos = app.position();
    let dur = app.duration();
    let frac = dur
        .filter(|d| !d.is_zero())
        .map(|d| pos.as_secs_f32() / d.as_secs_f32())
        .unwrap_or(0.0);
    let weak = cx.entity().downgrade();
    let seek: Pick = {
        let weak = weak.clone();
        Rc::new(move |f, _, cx| {
            weak.update(cx, |this, cx| {
                if let Some(d) = this.duration() {
                    this.seek_to(Duration::from_secs_f32(d.as_secs_f32() * f));
                    cx.notify();
                }
            })
            .ok();
        })
    };
    let vol: Pick = {
        let weak = weak.clone();
        Rc::new(move |f, _, cx| {
            weak.update(cx, |this, cx| {
                this.set_volume(f);
                cx.notify();
            })
            .ok();
        })
    };

    // Cover slot: real cover, or an empty slot the embed webview is placed on.
    let slot: gpui::AnyElement = if app.embed_playing() {
        let weak = weak.clone();
        div()
            .size(px(COVER))
            .rounded(px(12.0))
            .bg(theme::card())
            .child(
                canvas(
                    move |bounds, _, cx| {
                        weak.update(cx, |this, _| {
                            if let Some(p) = this.player.clone() {
                                let o = bounds.origin;
                                let s = bounds.size;
                                this.embed.set_bounds(
                                    &p,
                                    (f32::from(o.x), f32::from(o.y), f32::from(s.width), f32::from(s.height)),
                                );
                            }
                        })
                        .ok();
                    },
                    |_, _, _, _| {},
                )
                .size_full(),
            )
            .into_any_element()
    } else {
        cover(app, &track, COVER, 12.0, cx).into_any_element()
    };

    let t_like = track.clone();
    let repeat = app.repeat();
    let on = |b: bool| if b { theme::text() } else { theme::text_dim() };
    let play_btn = div()
        .id("np-play")
        .size(px(44.0))
        .rounded_full()
        .bg(theme::text())
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.opacity(0.88))
        .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx)))
        .child(crate::ui::widgets::play_glyph(app.is_loading(), app.is_playing(), 18.0, 44.0));

    let left = div()
        .flex()
        .flex_col()
        .items_center()
        .flex_none()
        .w(px(380.0))
        .gap(px(16.0))
        .child(slot)
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.0))
                .child(song_title(track.title.clone(), track.explicit).text_size(px(16.0)).font_weight(FontWeight::SEMIBOLD))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(badges(&track))
                        .child(div().flex().text_size(theme::text_md()).text_color(theme::text_dim()).child(crate::ui::lists::artist_link("np-artist", &track.artist, cx)))
                        .when(app.current_preview, |d| d.child(mono().text_size(theme::text_xs()).text_color(theme::warn()).child("[PREVIEW 0:30]"))),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(mono().text_size(theme::text_xs()).text_color(theme::text_dim()).child(fmt_time(pos)))
                .child(thin_bar("np-seek", 250.0, frac, seek, dur.map(|d| Rc::new(move |f: f32| fmt_time(d.mul_f32(f))) as BarLabel), cx))
                .child(mono().text_size(theme::text_xs()).text_color(theme::text_dim()).child(dur.map(fmt_time).unwrap_or_else(|| "-:--".into()))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(14.0))
                .child(toggle_button("np-shuf", "shuffle", 17.0, app.shuffle(), None).on_click(cx.listener(|this, _, _, cx| {
                    mark_toggled("np-shuf");
                    this.toggle_shuffle(cx)
                })))
                .child(icon_button("np-prev", "skip-back", 19.0, theme::text()).on_click(cx.listener(|this, _, _, cx| this.previous(cx))))
                .child(play_btn)
                .child(icon_button("np-next", "skip-forward", 19.0, theme::text()).on_click(cx.listener(|this, _, _, cx| this.next(false, cx))))
                .child(
                    toggle_button("np-rep", if repeat == RepeatMode::One { "repeat-1" } else { "repeat" }, 17.0, repeat != RepeatMode::Off, None)
                        .on_click(cx.listener(|this, _, _, cx| {
                            mark_toggled("np-rep");
                            this.cycle_repeat(cx)
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(icon("volume-2", 16.0, theme::text_dim()))
                .child(thin_bar("np-vol", 150.0, app.volume(), vol, Some(Rc::new(|f: f32| format!("{:.0}%", f * 100.0)) as BarLabel), cx))
                .child(
                    toggle_button("np-like", "heart", 16.0, liked, Some(theme::pink()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            mark_toggled("np-like");
                            this.toggle_like(&t_like, cx)
                        })),
                ),
        );

    if app.show_lyrics && app.lyrics_only {
        // Lyrics only: centred column, the global player bar does transport.
        return div()
            .id("np-root")
            .size_full()
            .flex()
            .justify_center()
            .px(px(40.0))
            .child(div().w_full().max_w(px(820.0)).h_full().child(lyrics(app, cx)))
            .into_any_element();
    }
    div()
        .id("np-root")
        .size_full()
        .flex()
        .items_center()
        .gap(px(40.0))
        .px(px(40.0))
        .child(left)
        .when(app.show_lyrics, |d| d.child(lyrics(app, cx)))
        .into_any_element()
}

/// "Song + lyrics" / "Lyrics only" switch (filled icon = active mode).
fn mode_switch(app: &Glyph, cx: &mut Context<Glyph>) -> gpui::Div {
    let only = app.lyrics_only;
    pill()
        .px(px(3.0))
        .h(px(32.0))
        .gap(px(2.0))
        .child(
            toggle_button("np-mode-split", "layout-split", 15.0, !only, None).on_click(cx.listener(|this, _, _, cx| {
                mark_toggled("np-mode-split");
                this.lyrics_only = false;
                cx.notify();
            })),
        )
        .child(
            toggle_button("np-mode-lyrics", "text-center", 15.0, only, None).on_click(cx.listener(|this, _, _, cx| {
                mark_toggled("np-mode-lyrics");
                this.lyrics_only = true;
                cx.notify();
            })),
        )
}

thread_local! {
    /// Eased opacity per lyric line, for the song it belongs to.
    static LINE_ALPHA: std::cell::RefCell<(String, Vec<f32>, Option<std::time::Instant>)> = std::cell::RefCell::new((String::new(), Vec::new(), None));
    static FOLLOW_AT: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

/// Line opacities easing towards: active 1, the next few a soft step down,
/// sung lines quieter.
fn line_alphas(key: String, n: usize, active: Option<usize>) -> Vec<f32> {
    let target = |i: usize| -> f32 {
        match active {
            Some(a) if i == a => 1.0,
            Some(a) if i > a => (0.48 - 0.05 * (i - a - 1) as f32).max(0.26),
            Some(_) => 0.26,
            None => 0.4,
        }
    };
    LINE_ALPHA.with(|c| {
        let mut c = c.borrow_mut();
        let now = std::time::Instant::now();
        if c.0 != key || c.1.len() != n {
            *c = (key, (0..n).map(target).collect(), Some(now));
            return c.1.clone();
        }
        let dt = c.2.map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(0.016).clamp(0.0, 0.1);
        c.2 = Some(now);
        let k = 1.0 - (-dt / 0.16).exp();
        for i in 0..n {
            let t = target(i);
            c.1[i] += (t - c.1[i]) * k;
        }
        c.1.clone()
    })
}

/// 0..1 visibility of line `ix` near the top / bottom edge of the frame
/// (from last frame's layout, which is where it is drawn now give or take
/// the follow step).
fn edge_fade(handle: &gpui::ScrollHandle, ix: usize) -> f32 {
    const BAND: f32 = 64.0;
    let (Some(item), view) = (handle.bounds_for_item(ix), handle.bounds()) else { return 1.0 };
    let vh = f32::from(view.size.height);
    if vh <= 0.0 {
        return 1.0;
    }
    let mid = f32::from(item.origin.y - view.origin.y + handle.offset().y) + f32::from(item.size.height) / 2.0;
    let k = (mid / BAND).min((vh - mid) / BAND).clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

/// One easing step of the lyrics frame towards line `ix` (time-based).
fn follow_line(handle: &gpui::ScrollHandle, ix: usize) {
    let (Some(item), view) = (handle.bounds_for_item(ix), handle.bounds()) else { return };
    let vh = f32::from(view.size.height);
    if vh <= 0.0 {
        return;
    }
    let max = f32::from(handle.max_offset().y);
    let center = f32::from(item.origin.y - view.origin.y) + f32::from(item.size.height) / 2.0;
    let target = -(center - vh * 0.38).clamp(0.0, max.max(0.0));
    let cur = f32::from(handle.offset().y);
    let now = std::time::Instant::now();
    let dt = FOLLOW_AT.with(|f| f.replace(Some(now))).map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(0.016).clamp(0.0, 0.1);
    let k = 1.0 - (-dt / 0.22).exp();
    let next = if (target - cur).abs() < 0.5 { target } else { cur + (target - cur) * k };
    handle.set_offset(gpui::point(px(0.0), px(next)));
}

fn lyrics(app: &mut Glyph, cx: &mut Context<Glyph>) -> gpui::AnyElement {
    let ls = &app.lyrics;
    if ls.loading {
        return div()
            .flex_1()
            .pt(px(72.0))
            .child(skeleton_lyrics(7))
            .into_any_element();
    }
    let Some(l) = ls.lyrics.clone() else {
        let sub = ls.error.clone().map(Into::into).unwrap_or_else(|| t("lyrics.none.sub"));
        return div()
            .flex_1()
            .child(ascii_state(ART_EMPTY, t("lyrics.none.title"), sub))
            .into_any_element();
    };
    let offset = app.lyrics_offset();
    let pos_ms = app.display_position().as_millis() as u64;
    let active = guard(|| l.active_line(pos_ms, offset)).flatten();
    let progress = guard(|| l.line_progress(pos_ms, offset)).unwrap_or(0.0);
    let synced = !l.synced.is_empty();
    let centered = app.lyrics_only;

    // Mode switch on the left; provider / offset / match tucked behind an
    // info button on the right that grows open.
    let open_for = app.lyrics_info.map(|at| at.elapsed().as_secs_f32());
    let grow = open_for.map(|t| 1.0 - (1.0 - (t / 0.22).min(1.0)).powi(3)).unwrap_or(0.0);
    let details = pill()
            .px(px(6.0))
            .h(px(32.0))
            .gap(px(4.0))
            .text_size(theme::text_sm())
            .child(icon("chevron-left", 14.0, theme::text_faint()))
            .child(div().font_weight(FontWeight::SEMIBOLD).child(l.provider.clone()))
            .child(mono().text_size(theme::text_xs()).text_color(theme::text_dim()).child("1/1"))
            .child(icon("chevron-right", 14.0, theme::text_faint()))
            .child(div().w(px(1.0)).h(px(16.0)).mx(px(4.0)).bg(theme::border()))
            .child(icon_button("off-minus", "minus", 13.0, theme::text_dim()).on_click(cx.listener(|this, _, _, cx| this.shift_lyrics_offset(-100, cx))))
            .child(icon("timer", 14.0, theme::text_dim()))
            .child(mono().text_size(theme::text_xs()).child(format!("{:+.1}s", offset as f32 / 1000.0)))
            .child(icon_button("off-plus", "plus", 13.0, theme::text_dim()).on_click(cx.listener(|this, _, _, cx| this.shift_lyrics_offset(100, cx))))
            .child(
                mono()
                    .pl(px(6.0))
                    .pr(px(8.0))
                    .text_size(theme::text_xs())
                    .text_color(theme::text_faint())
                    .child(tf(
                        if synced { "lyrics.match_synced" } else { "lyrics.match_plain" },
                        &[("p", &format!("{:.1}", l.match_score * 100.0))],
                    )),
            );
    let info_btn = toggle_button("lyrics-info", "info", 15.0, open_for.is_some(), None).on_click(cx.listener(|this, _, _, cx| {
        mark_toggled("lyrics-info");
        this.lyrics_info = if this.lyrics_info.is_some() { None } else { Some(std::time::Instant::now()) };
        cx.notify();
    }));
    let info = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .when(centered, |d| d.justify_center())
        .child(mode_switch(app, cx))
        .child(div().flex_1())
        .when(open_for.is_some(), |d| d.child(div().opacity(grow).ml(px(24.0 * (1.0 - grow))).child(details)))
        .child(pill().px(px(3.0)).h(px(32.0)).child(info_btn));

    let body: Vec<gpui::AnyElement> = if synced {
        // Every line is a direct child of the scroll frame, so the frame can
        // glide to the active one; colours ease instead of jumping.
        let alphas = line_alphas(app.lyrics.key.clone().unwrap_or_default(), l.synced.len(), active);
        (0..l.synced.len())
            .map(|i| {
                let line = &l.synced[i];
                let tms = line.time_ms;
                let seek = cx.listener(move |this: &mut Glyph, _, _, cx| {
                    let off = this.lyrics_offset();
                    this.lyrics_user_scroll = None;
                    this.seek_to(Duration::from_millis((tms as i64 - off).max(0) as u64));
                    cx.notify();
                });
                let text: SharedString = if line.text.trim().is_empty() { "♪".into() } else { line.text.clone().into() };
                // Lines fade themselves out towards the frame's edges (no dark
                // band over the background, which looked wrong on light covers).
                let a = alphas.get(i).copied().unwrap_or(0.3) * edge_fade(&app.lyrics_scroll, i);
                let base = div()
                    .id(("lyric", i))
                    .w_full()
                    .py(px(11.0))
                    .cursor_pointer()
                    .on_click(seek)
                    .when(centered, |d| d.text_center())
                    .text_size(px(28.0))
                    .line_height(px(38.0))
                    .font_weight(FontWeight::BOLD);
                if Some(i) == active {
                    // Sung part fills in white inside one paragraph (wraps like the line).
                    let n = text.chars().count();
                    let k = (progress.clamp(0.0, 1.0) * n as f32).round() as usize;
                    let split = text.char_indices().nth(k).map(|(b, _)| b).unwrap_or(text.len());
                    let rest_c = theme::text().opacity(0.5);
                    let hl = vec![
                        (0..split, gpui::HighlightStyle { color: Some(theme::text().opacity(a)), ..Default::default() }),
                        (split..text.len(), gpui::HighlightStyle { color: Some(rest_c), ..Default::default() }),
                    ];
                    base.child(gpui::StyledText::new(text).with_highlights(hl)).into_any_element()
                } else {
                    base.text_color(theme::text().opacity(a)).hover(|s| s.text_color(theme::text().opacity(0.75))).child(text).into_any_element()
                }
            })
            .collect()
    } else {
        vec![div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .text_size(px(18.0))
            .text_color(theme::text_dim())
            .children(l.plain.unwrap_or_default().lines().map(|s| div().child(s.to_string())))
            .into_any_element()]
    };
    // Auto-follow: glide so the active line sits a bit above the middle.
    // A hand scroll pauses this for a few seconds.
    let follow = app.lyrics_user_scroll.is_none_or(|at| at.elapsed() > Duration::from_secs(4))
        && !crate::ui::widgets::wheeled_recently(&app.lyrics_scroll, Duration::from_secs(4));
    if synced && follow {
        if let Some(i) = active {
            follow_line(&app.lyrics_scroll, i);
        }
    }

    // The pills stay fixed above the lines; only the lines scroll, and their
    // edges fade only where there is more text.
    let scroll = div()
        .id("lyrics-scroll")
        .track_scroll(&app.lyrics_scroll)
        .size_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .when(centered, |d| d.items_center())
        .pt(px(if synced { 140.0 } else { 12.0 }))
        .pb(px(if synced { 320.0 } else { 24.0 }))
        .on_scroll_wheel(cx.listener(|this, _, _, _| this.lyrics_user_scroll = Some(std::time::Instant::now())))
        .children(body);
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .gap(px(16.0))
        .pt(px(20.0))
        .child(info)
        .child(div().relative().flex_1().min_h_0().w_full().child(scroll).child(scrollbar(&app.lyrics_scroll, true, cx)))
        .into_any_element()
}
