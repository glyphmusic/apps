//! Player bar (Liner layout): its own flat surface. Left cover/title/like,
//! transport + thin seek row truly centred in the bar, right a pill with
//! shuffle/repeat/volume/mini/queue. Fixed light ink: it is a surface, not
//! text on the background.

use crate::app::{Glyph, View};
use crate::i18n::t;
use crate::theme;
use crate::ui::widgets::*;
use glyph_core::RepeatMode;
use gpui::{div, hsla, prelude::*, px, Context, FontWeight, Window};
use std::rc::Rc;
use std::time::Duration;

const SIDE_W: f32 = 300.0;

pub fn player_bar(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    theme::on_panel(|| bar(app, window, cx))
}

fn bar(app: &mut Glyph, _window: &mut Window, cx: &mut Context<Glyph>) -> gpui::Div {
    let playing = app.is_playing();
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
    let vol: Pick = Rc::new(move |f, _, cx| {
        weak.update(cx, |this, cx| {
            this.set_volume(f);
            cx.notify();
        })
        .ok();
    });

    let title_lines = |title: gpui::SharedString, sub: gpui::AnyElement| {
        div()
            .flex()
            .flex_col()
            .justify_center()
            .min_w_0()
            .gap(px(2.0))
            .child(div().truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).child(title))
            .child(sub)
    };

    let now = match app.current.clone() {
        Some(tr) => {
            let liked = app.is_liked(&tr);
            let art = cover(app, &tr, 48.0, 6.0, cx);
            let t_like = tr.clone();
            let sub = div()
                .flex()
                .gap(px(6.0))
                .items_center()
                .min_w_0()
                .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(tr.artist.clone()))
                .when_some(app.current_source, |d, p| d.child(badge(p)))
                .when(app.current_preview, |d| {
                    d.child(mono().flex_none().text_size(theme::text_xs()).text_color(theme::warn()).child("[PREVIEW 0:30]"))
                })
                .when(app.resolving, |d| {
                    d.child(mono().flex_none().text_size(theme::text_xs()).text_color(theme::text_faint()).child(app.spinner().to_string()))
                })
                .into_any_element();
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .min_w_0()
                .child(
                    div()
                        .id("np-open")
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .min_w_0()
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, w, cx| this.navigate(View::NowPlaying, w, cx)))
                        .child(art)
                        .child(title_lines(tr.title.clone().into(), sub)),
                )
                .child(
                    icon_button("bar-like", "heart", 16.0, if liked { theme::pink() } else { theme::text_dim() })
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_like(&t_like, cx))),
                )
        }
        None => div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .min_w_0()
            .child(
                div()
                    .flex_none()
                    .size(px(48.0))
                    .rounded(px(6.0))
                    .bg(hsla(0.0, 0.0, 1.0, 0.06))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("music-2", 18.0, theme::text_faint())),
            )
            .child(title_lines(
                t("player.idle.title"),
                div().truncate().text_size(theme::text_sm()).text_color(theme::text_faint()).child(t("player.idle.sub")).into_any_element(),
            )),
    };

    let repeat = app.repeat();
    let on = |b: bool| if b { theme::text() } else { theme::text_dim() };

    let play_btn = div()
        .id("play")
        .flex_none()
        .size(px(34.0))
        .rounded_full()
        .bg(gpui::white())
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.opacity(0.88))
        .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx)))
        .child(icon(if playing { "pause" } else { "play" }, 15.0, gpui::black()));

    let time = |s: String, right: bool| {
        mono()
            .flex_none()
            .w(px(38.0))
            .flex()
            .when(right, |d| d.justify_end())
            .text_size(px(11.0))
            .text_color(theme::text_faint())
            .child(s)
    };

    // Centre column, positioned over the whole bar so it is centred on the
    // bar itself, independent of how wide the left/right blocks are.
    let center = div()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .right_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(18.0))
                .child(icon_button("prev", "skip-back", 17.0, theme::text()).on_click(cx.listener(|this, _, _, cx| this.previous(cx))))
                .child(play_btn)
                .child(icon_button("next", "skip-forward", 17.0, theme::text()).on_click(cx.listener(|this, _, _, cx| this.next(false, cx)))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(time(fmt_time(pos), true))
                .child(thin_bar("seek", 340.0, frac, seek))
                .child(time(dur.map(fmt_time).unwrap_or_else(|| "-:--".into()), false)),
        );

    let right = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.0))
        .px(px(4.0))
        .h(px(36.0))
        .rounded(px(10.0))
        .bg(hsla(0.0, 0.0, 1.0, 0.04))
        .child(icon_button("shuffle", "shuffle", 15.0, on(app.shuffle())).on_click(cx.listener(|this, _, _, cx| this.toggle_shuffle(cx))))
        .child(
            icon_button("repeat", if repeat == RepeatMode::One { "repeat-1" } else { "repeat" }, 15.0, on(repeat != RepeatMode::Off))
                .on_click(cx.listener(|this, _, _, cx| this.cycle_repeat(cx))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(4.0))
                .child(icon(if app.volume() <= 0.001 { "volume-x" } else { "volume-2" }, 15.0, theme::text_dim()))
                .child(thin_bar("vol", 80.0, app.volume(), vol)),
        )
        .child(icon_button("mini", "picture-in-picture-2", 15.0, on(app.mini_on_top)).on_click(cx.listener(|this, _, _, cx| this.toggle_mini(cx))))
        .child(
            icon_button("queue", "list-music", 15.0, on(app.show_queue)).on_click(cx.listener(|this, _, _, cx| {
                this.show_queue = !this.show_queue;
                cx.notify();
            })),
        );

    div()
        .relative()
        .flex_none()
        .h(px(theme::PLAYER_H))
        .w_full()
        .bg(hsla(0.0, 0.0, 0.06, 0.94))
        .rounded(px(12.0))
        .px(px(12.0))
        .flex()
        .items_center()
        .justify_between()
        .child(center)
        .child(div().relative().flex().w(px(SIDE_W)).min_w_0().child(now))
        .child(div().relative().flex().w(px(SIDE_W)).justify_end().child(right))
}
