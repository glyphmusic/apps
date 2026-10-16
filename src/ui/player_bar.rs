//! Player bar: rounded, tinted in the song colour, ASCII controls.

use crate::app::{Glyph, View};
use crate::theme;
use crate::ui::widgets::{ascii_bar, ascii_button, badge, fmt_time, mini_cover, panel, pill};
use crate::visuals::Rgb;
use glyph_core::RepeatMode;
use gpui::{div, prelude::*, px, Context, FontWeight, Window};
use std::rc::Rc;
use std::time::Duration;

pub fn player_bar(app: &mut Glyph, _window: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let accent = app.palette.map(|p| p.accent()).unwrap_or(Rgb::hex(0xff5fd2));
    let playing = app.is_playing();
    let pos = app.position();
    let dur = app.duration();
    let frac = dur
        .filter(|d| !d.is_zero())
        .map(|d| pos.as_secs_f32() / d.as_secs_f32())
        .unwrap_or(0.0);
    let weak = cx.entity().downgrade();
    let seek: Rc<dyn Fn(f32, &mut Window, &mut gpui::App)> = {
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
    let vol: Rc<dyn Fn(f32, &mut Window, &mut gpui::App)> = Rc::new(move |f, _, cx| {
        weak.update(cx, |this, cx| {
            this.set_volume(f);
            cx.notify();
        })
        .ok();
    });

    let now = match app.current.clone() {
        Some(t) => div()
            .id("np-open")
            .flex()
            .items_center()
            .gap(px(12.0))
            .min_w_0()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, w, cx| this.navigate(View::NowPlaying, w, cx)))
            .child(mini_cover(&t, 46.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .gap(px(3.0))
                    .child(div().truncate().text_color(theme::text()).child(t.title.clone()))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .items_center()
                            .text_size(theme::text_sm())
                            .child(div().truncate().text_color(theme::text_dim()).child(t.artist.clone()))
                            .when_some(app.current_source, |d, p| d.child(badge(p)))
                            .when(app.current_preview, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .text_size(theme::text_xs())
                                        .text_color(theme::warn())
                                        .child("[PREVIEW 0:30]"),
                                )
                            })
                            .when(app.resolving, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .text_size(theme::text_xs())
                                        .text_color(theme::text_faint())
                                        .child(format!("{} resolve", app.spinner())),
                                )
                            }),
                    ),
            ),
        None => div()
            .id("np-open")
            .text_size(theme::text_sm())
            .text_color(theme::text_faint())
            .child("— nichts läuft · space spielt zuletzt gehört"),
    };

    let repeat = match app.repeat() {
        RepeatMode::Off => "rep",
        RepeatMode::All => "rep:all",
        RepeatMode::One => "rep:one",
    };
    let shuffle = app.shuffle();
    let on = |b: bool| if b { accent.lighten(0.35).hsla(1.0) } else { theme::text_dim() };

    panel()
        .flex_none()
        .h(px(theme::PLAYER_H))
        .w_full()
        .bg(theme::panel())
        .rounded(px(14.0))
        .px(px(16.0))
        .flex()
        .items_center()
        .gap(px(18.0))
        .child(div().flex().w(px(280.0)).min_w_0().child(now))
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            ascii_button("prev", "|◀◀|")
                                .on_click(cx.listener(|this, _, _, cx| this.previous(cx))),
                        )
                        .child(
                            ascii_button("play", if playing { "[ ❚❚ ]" } else { "[ ▶ ]" })
                                .text_color(theme::text())
                                .font_weight(FontWeight::BOLD)
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx))),
                        )
                        .child(
                            ascii_button("next", "▶▶|")
                                .on_click(cx.listener(|this, _, _, cx| this.next(false, cx))),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .text_size(theme::text_sm())
                        .child(div().w(px(36.0)).text_color(theme::text_faint()).child(fmt_time(pos)))
                        .child(ascii_bar("seek", 46, frac, accent.lighten(0.35).hsla(0.95), seek))
                        .child(
                            div()
                                .w(px(36.0))
                                .text_color(theme::text_faint())
                                .child(dur.map(fmt_time).unwrap_or_else(|| "-:--".into())),
                        ),
                ),
        )
        .child(
            pill()
                .flex_none()
                .px(px(6.0))
                .py(px(3.0))
                .gap(px(2.0))
                .text_size(theme::text_sm())
                .child(
                    ascii_button("shuffle", "shuf")
                        .text_color(on(shuffle))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_shuffle(cx))),
                )
                .child(
                    ascii_button("repeat", repeat)
                        .text_color(on(app.repeat() != RepeatMode::Off))
                        .on_click(cx.listener(|this, _, _, cx| this.cycle_repeat(cx))),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .px(px(4.0))
                        .child(div().text_color(theme::text_faint()).child("vol"))
                        .child(ascii_bar("vol", 8, app.volume(), theme::text_dim(), vol)),
                )
                .child(
                    ascii_button("mini", "mini")
                        .text_color(on(app.mini_on_top))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_mini(cx))),
                )
                .child(
                    ascii_button("queue", "queue")
                        .text_color(on(app.show_queue))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_queue = !this.show_queue;
                            cx.notify();
                        })),
                ),
        )
}
