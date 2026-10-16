//! Now playing: big ASCII cover, title in large monospace, synced lyrics.

use crate::app::Glyph;
use crate::guard::guard;
use crate::theme;
use crate::ui::widgets::*;
use crate::visuals::Rgb;
use gpui::{div, prelude::*, px, Context, FontWeight, Window};
use std::time::Duration;

pub fn now_playing(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let Some(track) = app.current.clone() else {
        return div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(ascii_state(ART_EMPTY, "nichts läuft", "wähle einen song oder drücke space"))
            .into_any_element();
    };
    let accent = app.palette.map(|p| p.accent()).unwrap_or(Rgb::hex(0xff5fd2));
    let liked = app.is_liked(&track);

    let cover = div()
        .size(px(280.0))
        .rounded(px(12.0))
        .overflow_hidden()
        .bg(theme::panel_strong())
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .text_size(px(8.5))
        .line_height(px(8.5))
        .children(app.cover.iter().flat_map(|c| c.ascii.iter()).map(|(line, tint)| {
            let col = tint.map(|p| p.0[0]).unwrap_or(accent).lighten(0.25).hsla(0.9);
            div().whitespace_nowrap().text_color(col).child(line.clone())
        }));

    let pos = app.position();
    let dur = app.duration();
    let frac = dur.filter(|d| !d.is_zero()).map(|d| pos.as_secs_f32() / d.as_secs_f32()).unwrap_or(0.0);
    let seek_cells: std::rc::Rc<dyn Fn(f32, &mut Window, &mut gpui::App)> = {
        let weak = cx.entity().downgrade();
        std::rc::Rc::new(move |f, _, cx| {
            weak.update(cx, |this, cx| {
                if let Some(d) = this.duration() {
                    this.seek_to(Duration::from_secs_f32(d.as_secs_f32() * f));
                    cx.notify();
                }
            })
            .ok();
        })
    };
    let vol_cells: std::rc::Rc<dyn Fn(f32, &mut Window, &mut gpui::App)> = {
        let weak = cx.entity().downgrade();
        std::rc::Rc::new(move |f, _, cx| {
            weak.update(cx, |this, cx| { this.set_volume(f); cx.notify(); }).ok();
        })
    };

    let t_like = track.clone();
    let left = div()
        .flex()
        .flex_col()
        .items_center()
        .flex_none()
        .w(px(360.0))
        .gap(px(14.0))
        .child(cover)
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(3.0))
                .child(div().text_color(theme::text()).font_weight(FontWeight::MEDIUM).child(track.title.clone()))
                .child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .text_size(theme::text_sm())
                        .text_color(theme::text_dim())
                        .child(badges(&track))
                        .child(track.artist.clone())
                        .when(app.current_preview, |d| d.child(div().text_color(theme::warn()).child("[PREVIEW 0:30]"))),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(theme::text_xs())
                .text_color(theme::text_faint())
                .child(fmt_time(pos))
                .child(thin_seek("np-seek", 220.0, frac, seek_cells))
                .child(dur.map(fmt_time).unwrap_or_else(|| "-:--".into())),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(14.0))
                .child(ascii_button("np-shuf", "shuf").on_click(cx.listener(|this, _, _, cx| this.toggle_shuffle(cx))))
                .child(ascii_button("np-prev", "|◀◀").on_click(cx.listener(|this, _, _, cx| this.previous(cx))))
                .child(
                    ascii_button("np-play", if app.is_playing() { "[ ❚❚ ]" } else { "[ ▶ ]" })
                        .text_color(theme::text())
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx))),
                )
                .child(ascii_button("np-next", "▶▶|").on_click(cx.listener(|this, _, _, cx| this.next(false, cx))))
                .child(ascii_button("np-rep", "rep").on_click(cx.listener(|this, _, _, cx| this.cycle_repeat(cx)))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(theme::text_xs())
                .text_color(theme::text_faint())
                .child("vol")
                .child(thin_seek("np-vol", 140.0, app.volume(), vol_cells))
                .child(
                    ascii_button("np-like", if liked { "♥" } else { "♡" })
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_like(&t_like, cx))),
                ),
        );

    div()
        .id("np-scroll")
        .size_full()
        .overflow_y_scroll()
        .px(px(36.0))
        .py(px(28.0))
        .flex()
        .gap(px(48.0))
        .child(left)
        .when(app.show_lyrics, |d| d.child(lyrics(app, accent, cx)))
        .into_any_element()
}

fn lyrics(app: &Glyph, accent: Rgb, cx: &mut Context<Glyph>) -> gpui::AnyElement {
    let ls = &app.lyrics;
    if ls.loading {
        return div()
            .flex_1()
            .pt(px(40.0))
            .child(spinner_line(app.spinner(), "GET lrclib /api/get ..."))
            .into_any_element();
    }
    let Some(l) = ls.lyrics.clone() else {
        let sub = ls.error.clone().unwrap_or_else(|| "lrclib kennt diesen song nicht".into());
        return div().flex_1().child(ascii_state(ART_EMPTY, "keine lyrics", sub)).into_any_element();
    };
    let offset = app.lyrics_offset();
    let pos_ms = app.position().as_millis() as u64;
    let active = guard(|| l.active_line(pos_ms, offset)).flatten();
    let progress = guard(|| l.line_progress(pos_ms, offset)).unwrap_or(0.0);
    let synced = !l.synced.is_empty();

    let info = div().flex().child(
        pill()
            .px(px(8.0))
            .py(px(2.0))
            .gap(px(8.0))
            .text_size(theme::text_xs())
            .text_color(theme::text_dim())
            .child(div().text_color(theme::text()).child(format!("‹ {} 1/1 ›", l.provider)))
            .child(div().text_color(theme::text_faint()).child(format!("{:.1}% match · {}", l.match_score * 100.0, if synced { "line-synced" } else { "plain" })))
            .child(ascii_button("off-minus", "–").on_click(cx.listener(|this, _, _, cx| this.shift_lyrics_offset(-100, cx))))
            .child(div().child(format!("⏱ {:+.1}s", offset as f32 / 1000.0)))
            .child(ascii_button("off-plus", "+").on_click(cx.listener(|this, _, _, cx| this.shift_lyrics_offset(100, cx)))),
    );

    let body: gpui::AnyElement = if synced {
        let center = active.unwrap_or(0);
        let start = center.saturating_sub(4);
        let end = (start + 12).min(l.synced.len());
        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .children((start..end).map(|i| {
                let line = &l.synced[i];
                let t = line.time_ms;
                let seek = cx.listener(move |this: &mut Glyph, _, _, cx| {
                    let off = this.lyrics_offset();
                    let ms = (t as i64 - off).max(0) as u64;
                    this.seek_to(Duration::from_millis(ms));
                    cx.notify();
                });
                let base = div()
                    .id(("lyric", i))
                    .cursor_pointer()
                    .on_click(seek)
                    .hover(|s| s.opacity(1.0));
                let text = if line.text.trim().is_empty() { "♪".to_string() } else { line.text.clone() };
                if Some(i) == active {
                    let n = text.chars().count();
                    let k = ((progress.clamp(0.0, 1.0)) * n as f32).round() as usize;
                    let filled: String = text.chars().take(k).collect();
                    let rest: String = text.chars().skip(k).collect();
                    base.flex()
                        .flex_wrap()
                        .text_size(px(28.0))
                        .line_height(px(36.0))
                        .font_weight(FontWeight::BOLD)
                        .child(div().text_color(theme::text()).child(filled))
                        .child(div().text_color(theme::text().opacity(0.6)).child(rest))
                } else {
                    let dist = (i as i64 - center as i64).unsigned_abs() as f32;
                    base.text_size(px(26.0))
                        .line_height(px(34.0))
                        .text_color(theme::text().opacity(if dist <= 3.0 { 0.35 } else { 0.2 }))
                        .child(text)
                }
            }))
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .text_size(px(15.0))
            .text_color(theme::text_dim())
            .children(l.plain.unwrap_or_default().lines().map(|s| div().child(s.to_string())))
            .into_any_element()
    };

    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(22.0))
        .pt(px(8.0))
        .child(info)
        .child(body)
        .into_any_element()
}
