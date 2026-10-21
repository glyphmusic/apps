//! Now playing (Liner layout): left cover 280 px (or the official embed
//! player in its place), title/artist, thin seek bar, flat controls, volume;
//! right big lyrics with a small provider/offset pill on top.

use crate::app::{ControlMenu, Glyph};
use crate::ui::controls::on_right_click;
use crate::guard::guard;
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::widgets::*;
use glyph_core::RepeatMode;
use gpui::{canvas, div, prelude::*, px, Context, FontWeight, SharedString, Window};
use std::rc::Rc;
use std::time::Duration;

pub(crate) const COVER: f32 = 280.0;

pub fn now_playing(app: &mut Glyph, w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    // The cover grows with tall windows (280 px at most window sizes, up to
    // 360 on large screens) instead of a small square in a big empty page.
    let cover_size = (f32::from(w.viewport_size().height) * 0.36).clamp(COVER, 360.0);
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
            .size(px(cover_size))
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
        cover(app, &track, cover_size, 12.0, cx).into_any_element()
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
                .child(thin_bar("np-seek", cover_size - 30.0, frac, seek, dur.map(|d| Rc::new(move |f: f32| fmt_time(d.mul_f32(f))) as BarLabel), cx))
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
                })).on_mouse_down(gpui::MouseButton::Right, on_right_click(ControlMenu::Shuffle, cx)))
                .child(icon_button("np-prev", "skip-back", 19.0, theme::text()).on_click(cx.listener(|this, _, _, cx| this.previous(cx))).on_mouse_down(gpui::MouseButton::Right, on_right_click(ControlMenu::Back, cx)))
                .child(play_btn)
                .child(icon_button("np-next", "skip-forward", 19.0, theme::text()).on_click(cx.listener(|this, _, _, cx| this.next(false, cx))).on_mouse_down(gpui::MouseButton::Right, on_right_click(ControlMenu::Forward, cx)))
                .child(
                    toggle_button("np-rep", if repeat == RepeatMode::One { "repeat-1" } else { "repeat" }, 17.0, repeat != RepeatMode::Off, None)
                        .on_click(cx.listener(|this, _, _, cx| {
                            mark_toggled("np-rep");
                            this.cycle_repeat(cx)
                        }))
                        .on_mouse_down(gpui::MouseButton::Right, on_right_click(ControlMenu::Repeat, cx)),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(
                    icon_button("np-vol-icon", if app.volume() <= 0.001 { "volume-x" } else { "volume-2" }, 16.0, theme::text_dim())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_mute();
                            cx.notify()
                        }))
                        .on_mouse_down(gpui::MouseButton::Right, on_right_click(ControlMenu::Volume, cx)),
                )
                .child(thin_bar("np-vol", 150.0, app.volume(), vol, Some(Rc::new(|f: f32| format!("{:.0}%", f * 100.0)) as BarLabel), cx))
                .child(
                    toggle_button("np-like", "heart", 16.0, liked, Some(theme::pink()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            mark_toggled("np-like");
                            this.toggle_like(&t_like, cx)
                        })),
                )
                .child(
                    toggle_button("np-sleep", "timer", 16.0, app.sleep.is_some(), None)
                        .on_mouse_down(gpui::MouseButton::Left, on_right_click(ControlMenu::Sleep, cx))
                        .on_mouse_down(gpui::MouseButton::Right, on_right_click(ControlMenu::Sleep, cx)),
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
            .child(div().w_full().max_w(px(820.0)).h_full().child(lyrics(app, cx, false)))
            .into_any_element();
    }
    // Cover column + lyrics as one centred block (not pinned to the left
    // edge on wide windows).
    div()
        .id("np-root")
        .size_full()
        .flex()
        .justify_center()
        .px(px(40.0))
        .child(
            div()
                .w_full()
                .max_w(px(1180.0))
                .h_full()
                .flex()
                .items_center()
                .gap(px(56.0))
                .child(left)
                .when(app.show_lyrics, |d| d.child(lyrics(app, cx, false))),
        )
        .into_any_element()
}

/// One segment of the view switch: icon + label, the active one filled white.
fn mode_segment(id: &'static str, icon_name: &str, label: SharedString, on: bool) -> gpui::Stateful<gpui::Div> {
    let key = format!("np-seg-{id}");
    let k = hover_k(&key);
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(28.0))
        .px(px(10.0))
        .rounded(px(7.0))
        .cursor_pointer()
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .relative()
        .when(on, |d| d.bg(gpui::white()).text_color(gpui::black()))
        .when(!on, |d| d.text_color(gpui::hsla(0.0, 0.0, 1.0, 0.65 + 0.3 * k)).child(hover_fill(key, gpui::hsla(0.0, 0.0, 1.0, 0.08), 7.0)))
        .child(icon(icon_name, 14.0, if on { gpui::black() } else { gpui::hsla(0.0, 0.0, 1.0, 0.7 + 0.3 * k) }))
        .child(label)
}

/// "Song + lyrics" / "Lyrics only": one segmented control with labels.
fn mode_switch(app: &Glyph, cx: &mut Context<Glyph>) -> gpui::Div {
    let only = app.lyrics_only;
    div()
        .flex()
        .items_center()
        .gap(px(2.0))
        .p(px(3.0))
        .rounded(px(10.0))
        .bg(gpui::hsla(0.0, 0.0, 0.06, 0.82))
        .border_1()
        .border_color(gpui::hsla(0.0, 0.0, 1.0, 0.10))
        .child(mode_segment("np-mode-split", "layout-split", t("np.mode.split"), !only).on_click(cx.listener(|this, _, _, cx| {
            this.lyrics_only = false;
            cx.notify();
        })))
        .child(mode_segment("np-mode-lyrics", "text-center", t("np.mode.lyrics"), only).on_click(cx.listener(|this, _, _, cx| {
            this.lyrics_only = true;
            cx.notify();
        })))
}

thread_local! {
    /// Eased opacity per lyric line, for the song it belongs to.
    static LINE_ALPHA: std::cell::RefCell<(String, Vec<f32>, Option<std::time::Instant>)> = std::cell::RefCell::new((String::new(), Vec::new(), None));
    static FOLLOW: std::cell::Cell<(Option<std::time::Instant>, f32)> = const { std::cell::Cell::new((None, 0.0)) };
    /// Eased "is the active line" per lyric line (0..1): it grows a little.
    static LINE_BIG: std::cell::RefCell<(String, Vec<f32>, Option<std::time::Instant>)> = std::cell::RefCell::new((String::new(), Vec::new(), None));
    /// Eased fill of the active line (the karaoke sweep glides, no steps).
    static FILL: std::cell::Cell<(usize, f32)> = const { std::cell::Cell::new((usize::MAX, 0.0)) };
}

/// Exponential ease of every value in `c.1` towards `target(i)` with time
/// constant `tau` (s).
fn ease_all(c: &mut (String, Vec<f32>, Option<std::time::Instant>), key: String, n: usize, tau: f32, target: impl Fn(usize) -> f32) -> Vec<f32> {
    let now = std::time::Instant::now();
    if c.0 != key || c.1.len() != n {
        *c = (key, (0..n).map(&target).collect(), Some(now));
        return c.1.clone();
    }
    let dt = c.2.map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(0.016).clamp(0.0, 0.1);
    c.2 = Some(now);
    let k = 1.0 - (-dt / tau).exp();
    for i in 0..n {
        let t = target(i);
        c.1[i] += (t - c.1[i]) * k;
        if (t - c.1[i]).abs() < 0.002 {
            c.1[i] = t;
        }
    }
    c.1.clone()
}

/// 1 for the line being sung, easing to 0 for the others.
fn line_big(key: String, n: usize, active: Option<usize>) -> Vec<f32> {
    LINE_BIG.with(|c| ease_all(&mut c.borrow_mut(), key, n, 0.28, |i| if Some(i) == active { 1.0 } else { 0.0 }))
}

/// Line opacities easing towards: active 1, the next lines a soft step down
/// each, sung lines quieter the further back they are.
fn line_alphas(key: String, n: usize, active: Option<usize>) -> Vec<f32> {
    let target = |i: usize| -> f32 {
        match active {
            Some(a) if i == a => 1.0,
            Some(a) if i > a => (0.5 - 0.06 * (i - a - 1) as f32).max(0.22),
            Some(a) => (0.62 - 0.08 * (a - i - 1) as f32).max(0.3),
            None => 0.42,
        }
    };
    LINE_ALPHA.with(|c| ease_all(&mut c.borrow_mut(), key, n, 0.32, target))
}

/// The active line's fill, gliding towards `progress` (jumps back on a new line).
fn eased_fill(line: usize, progress: f32) -> f32 {
    FILL.with(|f| {
        let (l, v) = f.get();
        let v = if l != line || progress < v - 0.2 { 0.0 } else { v + (progress - v) * 0.35 };
        f.set((line, v));
        v
    })
}

/// 0..1 visibility of line `ix` near the top / bottom edge of the frame
/// (from last frame's layout, which is where it is drawn now give or take
/// the follow step).
fn edge_fade(handle: &gpui::ScrollHandle, ix: usize) -> f32 {
    const BAND: f32 = 96.0;
    let (Some(item), view) = (handle.bounds_for_item(ix), handle.bounds()) else { return 1.0 };
    let vh = f32::from(view.size.height);
    if vh <= 0.0 {
        return 1.0;
    }
    let mid = f32::from(item.origin.y - view.origin.y + handle.offset().y) + f32::from(item.size.height) / 2.0;
    let k = (mid / BAND).min((vh - mid) / BAND).clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

/// One step of the lyrics frame towards line `ix`, centred in the frame: a
/// critically damped spring, so it starts softly and settles without
/// overshoot.
fn follow_line(handle: &gpui::ScrollHandle, ix: usize) {
    let (Some(item), view) = (handle.bounds_for_item(ix), handle.bounds()) else { return };
    let vh = f32::from(view.size.height);
    if vh <= 0.0 {
        return;
    }
    let max = f32::from(handle.max_offset().y);
    let center = f32::from(item.origin.y - view.origin.y) + f32::from(item.size.height) / 2.0;
    let target = -(center - vh * 0.5).clamp(0.0, max.max(0.0));
    let cur = f32::from(handle.offset().y);
    let now = std::time::Instant::now();
    let (last, vel) = FOLLOW.with(|f| f.get());
    let dt = last.map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(0.016).clamp(0.0, 0.05);
    // ω = 9 rad/s: about 0.45 s to settle.
    let w = 9.0;
    let x = cur - target;
    let a = -w * w * x - 2.0 * w * vel;
    let mut v = vel + a * dt;
    let mut next = cur + v * dt;
    if (next - target).abs() < 0.4 && v.abs() < 4.0 {
        next = target;
        v = 0.0;
    }
    FOLLOW.with(|f| f.set((Some(now), v)));
    handle.set_offset(gpui::point(px(0.0), px(next)));
}

/// Small round icon button of the lyrics tool pill.
fn tool_btn(id: &'static str, name: &str, cx: &mut Context<Glyph>, f: impl Fn(&mut Glyph, &mut Context<Glyph>) + 'static) -> gpui::Stateful<gpui::Div> {
    icon_button(id, name, 13.0, gpui::hsla(0.0, 0.0, 1.0, 0.75)).size(px(24.0)).on_click(cx.listener(move |this, _, _, cx| f(this, cx)))
}

/// The lyrics options panel: which version (with a preview to compare),
/// the timing offset, the match.
fn options_panel(app: &mut Glyph, l: &glyph_core::Lyrics, offset: i64, grow: f32, cx: &mut Context<Glyph>) -> gpui::Stateful<gpui::Div> {
    let synced = !l.synced.is_empty();
    let n = app.lyrics.alts.len().max(1);
    let idx = app.lyrics_alt_index().unwrap_or(0);
    let white = |a: f32| gpui::hsla(0.0, 0.0, 1.0, a);
    // Label left, value right (like a settings row).
    let row = |label: SharedString, body: gpui::AnyElement| {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .min_h(px(34.0))
            .child(div().flex_none().text_size(px(12.5)).text_color(white(0.55)).child(label))
            .child(div().min_w_0().flex().justify_end().child(body))
    };
    let versions: gpui::AnyElement = if app.lyrics.alts_loading {
        div().text_size(px(12.5)).text_color(white(0.6)).child(format!("{} {}", app.spinner(), t("lyrics.alts.loading"))).into_any_element()
    } else if !app.lyrics.alts_loaded {
        button("lyrics-find", t("lyrics.alts.find")).on_click(cx.listener(|this, _, _, cx| this.load_lyrics_alternatives(cx))).into_any_element()
    } else if app.lyrics.alts.len() < 2 {
        div().text_size(px(12.5)).text_color(white(0.6)).child(t("lyrics.alts.none")).into_any_element()
    } else {
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(tool_btn("lyrics-prev", "chevron-left", cx, |this, cx| this.cycle_lyrics(-1, cx)))
            .child(mono().text_size(px(12.0)).child(format!("{}/{}", idx + 1, n)))
            .child(tool_btn("lyrics-next", "chevron-right", cx, |this, cx| this.cycle_lyrics(1, cx)))
            .into_any_element()
    };
    let list: Vec<gpui::AnyElement> = if app.lyrics.alts.len() >= 2 {
        app.lyrics
            .alts
            .clone()
            .into_iter()
            .enumerate()
            .map(|(i, a)| {
                let on = i == idx;
                let key = format!("lyrics-alt-{i}");
                div()
                    .id(("lyrics-alt", i))
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .relative()
                    .when(on, |d| d.bg(white(0.10)))
                    .when(!on, |d| d.child(hover_fill(key, white(0.05), 8.0)))
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_lyrics(i, cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(px(12.0))
                            .text_color(white(0.85))
                            .child(mono().child(format!("{:.0}%", a.match_score * 100.0)))
                            .child(div().text_color(white(0.5)).child(if a.synced.is_empty() { t("lyrics.plain") } else { t("lyrics.synced") }))
                            .child(div().flex_1())
                            .when(on, |d| d.child(icon("check", 13.0, theme::pink()))),
                    )
                    .child(div().w_full().min_w_0().text_size(px(12.0)).line_height(px(17.0)).text_color(white(0.6)).line_clamp(2).child(a.preview(3)))
                    .into_any_element()
            })
            .collect()
    } else {
        Vec::new()
    };
    div()
        .id("lyrics-options")
        .occlude()
        .w(px(340.0))
        .max_h(px(440.0))
        .overflow_y_scroll().touch_axis()
        .px(px(14.0))
        .py(px(10.0))
        .rounded(px(12.0))
        .bg(gpui::hsla(0.0, 0.0, 0.07, 0.97))
        .border_1()
        .border_color(white(0.10))
        .opacity(grow)
        .mt(px(6.0 * (1.0 - grow)))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        // Source: the provider, and under it how well it matches.
        .child(row(
            t("lyrics.source"),
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .items_end()
                .child(div().text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD).child(l.provider.clone()))
                .child(mono().truncate().text_size(px(11.0)).text_color(white(0.5)).child(tf(if synced { "lyrics.match_synced" } else { "lyrics.match_plain" }, &[("p", &format!("{:.0}", l.match_score * 100.0))])))
                .into_any_element(),
        ))
        .child(row(t("lyrics.version"), versions))
        .child(row(
            t("lyrics.timing"),
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(tool_btn("off-minus", "minus", cx, |this, cx| this.shift_lyrics_offset(-100, cx)))
                .child(mono().w(px(48.0)).flex().justify_center().text_size(px(12.0)).child(format!("{:+.1}s", offset as f32 / 1000.0)))
                .child(tool_btn("off-plus", "plus", cx, |this, cx| this.shift_lyrics_offset(100, cx)))
                .when(offset != 0, |d| d.child(text_button("off-reset", t("lyrics.timing.reset")).text_size(px(12.0)).on_click(cx.listener(|this, _, _, cx| this.reset_lyrics_offset(cx)))))
                .into_any_element(),
        ))
        .when(!list.is_empty(), |d| {
            d.child(div().h(px(1.0)).my(px(8.0)).bg(white(0.08)))
                .child(div().px(px(2.0)).pb(px(6.0)).text_size(px(12.0)).line_height(px(17.0)).text_color(white(0.5)).child(t("lyrics.alts.pick")))
                .child(div().flex().flex_col().gap(px(2.0)).children(list))
        })
}

/// The lyrics frame. `compact`: the phone's lyrics screen (smaller lines,
/// no view switch: it is its own screen).
pub(crate) fn lyrics(app: &mut Glyph, cx: &mut Context<Glyph>, compact: bool) -> gpui::AnyElement {
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
        // Nothing found: offer the search for other versions right here.
        let find = (!ls.alts_loaded || ls.alts.len() > 0).then(|| {
            if !ls.alts.is_empty() {
                let n = ls.alts.len();
                button("lyrics-use-alt", tf("lyrics.alts.found", &[("n", &n)])).on_click(cx.listener(|this, _, _, cx| this.pick_lyrics(0, cx))).into_any_element()
            } else if ls.alts_loading {
                div().text_size(theme::text_sm()).text_color(theme::text_dim()).child(t("lyrics.alts.loading")).into_any_element()
            } else {
                button("lyrics-search-alt", t("lyrics.alts.find")).on_click(cx.listener(|this, _, _, cx| this.load_lyrics_alternatives(cx))).into_any_element()
            }
        });
        return div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(ascii_state(ART_EMPTY, t("lyrics.none.title"), sub))
            .children(find.map(|f| div().pl(px(4.0)).child(f)))
            .into_any_element();
    };
    let offset = app.lyrics_offset();
    let pos_ms = app.display_position().as_millis() as u64;
    let active = guard(|| l.active_line(pos_ms, offset)).flatten();
    let progress = guard(|| l.line_progress(pos_ms, offset)).unwrap_or(0.0);
    let synced = !l.synced.is_empty();
    let centered = app.lyrics_only;

    // Top: the view switch on the left; on the right one "options" button
    // that opens a panel (version, timing, match).
    let open_for = app.lyrics_info.map(|at| at.elapsed().as_secs_f32());
    let grow = open_for.map(|t| 1.0 - (1.0 - (t / 0.18).min(1.0)).powi(3)).unwrap_or(0.0);
    let open = open_for.is_some();
    let options_btn = div()
        .id("lyrics-info")
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(34.0))
        .px(px(11.0))
        .rounded(px(10.0))
        .cursor_pointer()
        .bg(if open { gpui::white() } else { gpui::hsla(0.0, 0.0, 0.06, 0.82) })
        .border_1()
        .border_color(gpui::hsla(0.0, 0.0, 1.0, if open { 0.0 } else { 0.10 }))
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(if open { gpui::black() } else { gpui::hsla(0.0, 0.0, 1.0, 0.85) })
        .on_click(cx.listener(|this, _, _, cx| {
            this.lyrics_info = if this.lyrics_info.is_some() { None } else { Some(std::time::Instant::now()) };
            cx.notify();
        }))
        .child(icon("sliders-horizontal", 14.0, if open { gpui::black() } else { gpui::hsla(0.0, 0.0, 1.0, 0.8) }))
        .child(l.provider.clone())
        .when(offset != 0, |d| d.child(mono().text_size(px(11.0)).opacity(0.7).child(format!("{:+.1}s", offset as f32 / 1000.0))));
    let panel = open.then(|| options_panel(app, &l, offset, grow, cx));
    let info = div()
        .relative()
        .flex()
        .items_center()
        .gap(px(8.0))
        .when(!compact, |d| d.child(mode_switch(app, cx)))
        .child(div().flex_1())
        .child(options_btn)
        .children(panel.map(|p| gpui::deferred(div().absolute().top(px(40.0)).right_0().child(p)).with_priority(2)));

    let body: Vec<gpui::AnyElement> = if synced {
        // Every line is a direct child of the scroll frame, so the frame can
        // glide to the active one; size and colour ease instead of jumping.
        let alphas = line_alphas(app.lyrics.key.clone().unwrap_or_default(), l.synced.len(), active);
        let bigs = line_big(app.lyrics.key.clone().unwrap_or_default(), l.synced.len(), active);
        let fill = active.map(|a| eased_fill(a, progress)).unwrap_or(0.0);
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
                let big = bigs.get(i).copied().unwrap_or(0.0);
                let base = div()
                    .id(("lyric", i))
                    .w_full()
                    .py(px(10.0 + 2.0 * big))
                    .cursor_pointer()
                    .on_click(seek)
                    .when(centered, |d| d.text_center())
                    // The line being sung is a size bigger, so it is clear
                    // where the song is.
                    .text_size(px(if compact { 24.0 + 4.0 * big } else { 28.0 + 8.0 * big }))
                    .line_height(px(if compact { 32.0 + 5.0 * big } else { 38.0 + 9.0 * big }))
                    .font_weight(FontWeight::BOLD);
                if Some(i) == active {
                    // The sung part fills in white with a soft edge that sweeps
                    // across the line (wraps like the line itself).
                    let n = text.chars().count().max(1);
                    let pos = fill.clamp(0.0, 1.0) * n as f32;
                    let edge = 3.0f32;
                    let mut hl = Vec::new();
                    let rest = theme::text().opacity(0.42 * a.max(0.5));
                    for (ci, (b, ch)) in text.char_indices().enumerate() {
                        let k = ((pos - ci as f32) / edge + 0.5).clamp(0.0, 1.0);
                        let c = mix(rest, theme::text().opacity(a), k);
                        hl.push((b..b + ch.len_utf8(), gpui::HighlightStyle { color: Some(c), ..Default::default() }));
                    }
                    base.child(gpui::StyledText::new(text).with_highlights(hl)).into_any_element()
                } else {
                    base.text_color(theme::text().opacity(a)).hover(|s| s.text_color(theme::text().opacity(0.8))).child(text).into_any_element()
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
            .when(centered, |d| d.items_center().text_center())
            .children(l.plain.clone().unwrap_or_default().lines().map(|s| div().child(s.to_string())))
            .into_any_element()]
    };
    // Auto-follow: glide so the active line sits in the middle of the frame.
    // A hand scroll pauses this for a few seconds.
    let follow = app.lyrics_user_scroll.is_none_or(|at| at.elapsed() > Duration::from_secs(4))
        && !crate::ui::widgets::wheeled_recently(&app.lyrics_scroll, Duration::from_secs(4));
    if synced && follow {
        if let Some(i) = active {
            follow_line(&app.lyrics_scroll, i);
        }
    }

    // Half a frame of room above the first and below the last line, so
    // every line (the first and last too) can sit in the middle.
    let half = (f32::from(app.lyrics_scroll.bounds().size.height) / 2.0 - 30.0).max(120.0);
    let scroll = div()
        .id("lyrics-scroll")
        .track_scroll(&app.lyrics_scroll)
        .size_full()
        .overflow_y_scroll().touch_axis()
        .flex()
        .flex_col()
        .when(centered, |d| d.items_center())
        .pt(px(if synced { half } else { 12.0 }))
        .pb(px(if synced { half } else { 24.0 }))
        .on_scroll_wheel(cx.listener(|this, _, _, _| this.lyrics_user_scroll = Some(std::time::Instant::now())))
        .children(body);
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(div().relative().size_full().child(scroll).child(scrollbar(&app.lyrics_scroll, true, cx)))
        .child(div().absolute().top(px(20.0)).left_0().right_0().child(info))
        .into_any_element()
}
