//! Player bar (Liner layout): its own flat surface. Left cover/title/like,
//! transport + thin seek row truly centred in the bar, right a pill with
//! shuffle/repeat/volume/mini/queue. Fixed light ink: it is a surface, not
//! text on the background.

use crate::app::{ControlMenu, Glyph, View};
use crate::ui::controls::on_right_click;
use crate::i18n::t;
use crate::theme;
use crate::ui::widgets::*;
use glyph_core::RepeatMode;
use gpui::{div, hsla, prelude::*, px, Context, FontWeight, Window};
use std::rc::Rc;
use std::time::Duration;

const SIDE_W: f32 = 300.0;
/// Width of the seek bar.
const SEEK_W: f32 = 340.0;

thread_local! {
    /// A-B loop end being dragged (`true`: the start), and the seek bar's
    /// left edge in window coordinates (measured while painting).
    static LOOP_DRAG: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static SEEK_X: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
    /// Width of the whole bar (measured while painting).
    static BAR_W: std::cell::Cell<f32> = const { std::cell::Cell::new(f32::MAX) };
}

/// Below this bar width the volume slider folds into its button (it would
/// run into the seek bar) and opens above it on hover.
const VOL_FOLD_W: f32 = 1072.0;

/// One end of the A-B loop: a tick that grows under the mouse and can be
/// dragged along the seek bar.
fn loop_handle(start: bool, x: f32, cx: &mut Context<Glyph>) -> gpui::Stateful<gpui::Div> {
    let key = if start { "loop-a" } else { "loop-b" };
    let k = if LOOP_DRAG.with(|d| d.get()) == Some(start) { 1.0 } else { hover_k(key) };
    div()
        .id(key)
        .absolute()
        .top(px(-8.0))
        .left(px(x - 7.0))
        .w(px(14.0))
        .h(px(20.0))
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .cursor(gpui::CursorStyle::ResizeLeftRight)
        .on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(move |_, _, _, cx| {
                cx.stop_propagation();
                LOOP_DRAG.with(|d| d.set(Some(start)));
                cx.notify();
            }),
        )
        .child(div().w(px(2.0 + 2.0 * k)).h(px(12.0 + 6.0 * k)).rounded(px(2.0)).bg(theme::pink()))
        .child(hover_fill(key, gpui::transparent_black(), 0.0))
}
/// Cover size of the current song (its ASCII reveal replays on every song change).
pub(crate) const COVER: f32 = 48.0;

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

    let explicit = app.current.as_ref().is_some_and(|t| t.explicit);
    let title_lines = move |title: gpui::SharedString, sub: gpui::AnyElement| {
        div()
            .flex()
            .flex_col()
            .justify_center()
            .min_w_0()
            .gap(px(2.0))
            .child(crate::ui::widgets::song_title(title, explicit).text_size(theme::text_md()).font_weight(FontWeight::MEDIUM))
            .child(sub)
    };

    let now = match app.current.clone() {
        Some(tr) => {
            let liked = app.is_liked(&tr);
            let art = cover(app, &tr, COVER, 6.0, cx);
            let reveal = reveal_k(&format!("bar-meta:{}", glyph_core::dedupe::track_key(&tr)), 0.9);
            let t_like = tr.clone();
            let sub = div()
                .flex()
                .gap(px(6.0))
                .items_center()
                .min_w_0()
                .child(div().flex().min_w_0().text_size(theme::text_sm()).text_color(theme::text_dim()).child(crate::ui::lists::artist_link_label("bar-artist", &tr.artist, decode(&tr.artist, reveal), cx)))
                .when_some(app.current_source, |d, p| d.child(badge(p)))
                .when(app.current_preview, |d| {
                    d.child(mono().flex_none().text_size(theme::text_xs()).text_color(theme::warn()).child("[PREVIEW 0:30]"))
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
                        .child(title_lines(decode(&tr.title, reveal).into(), sub)),
                )
                .child(
                    toggle_button("bar-like", "heart", 16.0, liked, Some(theme::pink()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            mark_toggled("bar-like");
                            this.toggle_like(&t_like, cx)
                        })),
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

    // Pressed buttons shrink a little and spring back.
    use crate::ui::widgets::{press, press_k};
    let pk = press_k("play");
    let play_d = 34.0 - 5.0 * pk;
    let play_btn = div()
        .id("play")
        .flex_none()
        .size(px(34.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
            press("play");
            this.toggle_play(cx)
        }))
        .child(
            div()
                .size(px(play_d))
                .rounded_full()
                .bg(gpui::white())
                .flex()
                .items_center()
                .justify_center()
                .hover(|s| s.opacity(0.88))
                .child(crate::ui::widgets::play_glyph(app.is_loading(), playing, 15.0 - 2.0 * pk, play_d)),
        );

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
                .child(icon_button("prev", "skip-back", 17.0 - 4.0 * press_k("prev"), theme::text()).size(px(31.0)).on_click(cx.listener(|this, _, _, cx| {
                    press("prev");
                    this.previous(cx)
                })).on_mouse_down(gpui::MouseButton::Right, on_right_click(ControlMenu::Back, cx)))
                .child(play_btn)
                .child(icon_button("next", "skip-forward", 17.0 - 4.0 * press_k("next"), theme::text()).size(px(31.0)).on_click(cx.listener(|this, _, _, cx| {
                    press("next");
                    this.next(false, cx)
                })).on_mouse_down(gpui::MouseButton::Right, on_right_click(ControlMenu::Forward, cx))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(time(fmt_time(pos), true))
                .child({
                    // A-B loop: the range tinted pink with a tick at each end.
                    let at = |p: Duration| dur.filter(|d| !d.is_zero()).map(|d| (p.as_secs_f32() / d.as_secs_f32()).clamp(0.0, 1.0) * SEEK_W);
                    let (la, lb) = (app.loop_ab.0.and_then(at), app.loop_ab.1.and_then(at));
                    // Dragging an end: follow the mouse anywhere in the window
                    // until the button is released.
                    let me = cx.entity().downgrade();
                    let track = gpui::canvas(
                        |_, _, _| (),
                        move |b, _, window, _| {
                            SEEK_X.with(|x| x.set(f32::from(b.origin.x)));
                            let Some(start) = LOOP_DRAG.with(|d| d.get()) else { return };
                            let me2 = me.clone();
                            window.on_mouse_event(move |e: &gpui::MouseMoveEvent, phase, _, cx| {
                                if phase != gpui::DispatchPhase::Bubble {
                                    return;
                                }
                                let f = ((f32::from(e.position.x) - SEEK_X.with(|x| x.get())) / SEEK_W).clamp(0.0, 1.0);
                                me2.update(cx, |g, cx| g.drag_loop_point(start, f, cx)).ok();
                            });
                            let me3 = me.clone();
                            window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                                if phase == gpui::DispatchPhase::Bubble && LOOP_DRAG.with(|d| d.take()).is_some() {
                                    me3.update(cx, |_, cx| cx.notify()).ok();
                                }
                            });
                        },
                    )
                    .absolute()
                    .inset_0();
                    div()
                        .relative()
                        .child(thin_bar("seek", SEEK_W, frac, seek, app.duration().map(|d| Rc::new(move |f: f32| fmt_time(d.mul_f32(f))) as BarLabel), cx))
                        .child(track)
                        .when_some(la.zip(lb), |d, (a, b)| {
                            d.child(div().absolute().top(px(0.0)).bottom(px(0.0)).left(px(a)).w(px((b - a).max(0.0))).bg(theme::pink().opacity(0.25)))
                        })
                        .when_some(la, |d, x| d.child(loop_handle(true, x, cx)))
                        .when_some(lb, |d, x| d.child(loop_handle(false, x, cx)))
                })
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
        .child(toggle_button("shuffle", "shuffle", 15.0, app.shuffle(), None).on_click(cx.listener(|this, _, _, cx| {
            mark_toggled("shuffle");
            this.toggle_shuffle(cx)
        })).on_mouse_down(gpui::MouseButton::Right, on_right_click(ControlMenu::Shuffle, cx)))
        .child(
            toggle_button("repeat", if repeat == RepeatMode::One { "repeat-1" } else { "repeat" }, 15.0, repeat != RepeatMode::Off, None)
                .on_click(cx.listener(|this, _, _, cx| {
                    mark_toggled("repeat");
                    this.cycle_repeat(cx)
                }))
                .on_mouse_down(gpui::MouseButton::Right, on_right_click(ControlMenu::Repeat, cx)),
        )
        // Connect: where music plays (this computer or a network device);
        // pink while it plays on another device or in a jam.
        .child(
            toggle_button("devices", "monitor-speaker", 15.0, app.show_devices || app.remote_on() || app.jam_active(), (app.remote_on() || app.jam_active()).then(theme::pink)).on_click(cx.listener(|this, _, _, cx| {
                mark_toggled("devices");
                this.toggle_devices_panel(cx)
            })),
        )
        .child({
            let folded = BAR_W.with(|w| w.get()) < VOL_FOLD_W;
            let icon_name = if app.volume() <= 0.001 { "volume-x" } else { "volume-2" };
            // Folded, hovering opens the slider: no tooltip over it.
            let mute = if folded { icon_button_plain("mute", icon_name, 15.0, theme::text_dim()) } else { icon_button("mute", icon_name, 15.0, theme::text_dim()) }.on_click(cx.listener(|this, _, _, cx| {
                this.toggle_mute();
                cx.notify();
            })).on_mouse_down(gpui::MouseButton::Right, on_right_click(ControlMenu::Volume, cx));
            let vol_label = Some(Rc::new(|f: f32| format!("{:.0}%", f * 100.0)) as BarLabel);
            if folded {
                // Hovering the button (or the open slider) shows the slider
                // above it; the box reaches down to the button so the mouse
                // can move up into it.
                let k = hover_k("vol-btn").max(hover_k("vol-pop"));
                div()
                    .relative()
                    .child(mute)
                    .when(k > 0.0, |d| {
                        // Deferred: the bar's slot clips its contents.
                        d.child(gpui::deferred(
                            div()
                                .id("vol-pop")
                                .absolute()
                                // Same as the mini player card's: a slim upright
                                // slider that reaches down to the button.
                                .bottom(px(29.0))
                                .left(px(1.0))
                                .w(px(28.0))
                                .h(px(108.0))
                                .pt(px(10.0))
                                .pb(px(14.0))
                                .opacity(k)
                                .occlude()
                                .flex()
                                .justify_center()
                                .child(div().absolute().top_0().left_0().right_0().bottom(px(4.0)).rounded(px(8.0)).bg(hsla(0.0, 0.0, 0.1, 0.98)))
                                .child({
                                    let w = cx.entity().downgrade();
                                    crate::ui::mini::frac_bar(
                                        "vol-bar",
                                        true,
                                        app.volume(),
                                        hsla(0.0, 0.0, 1.0, 0.2),
                                        theme::pink(),
                                        Rc::new(move |f, cx| {
                                            w.update(cx, |g, cx| {
                                                g.set_volume(f);
                                                cx.notify();
                                            })
                                            .ok();
                                        }),
                                    )
                                })
                                .child(hover_fill("vol-pop", gpui::transparent_black(), 0.0)),
                        ))
                    })
                    .child(hover_fill("vol-btn", gpui::transparent_black(), 0.0))
            } else {
                div().flex().items_center().gap(px(6.0)).px(px(4.0)).child(mute).child(thin_bar("vol", 80.0, app.volume(), vol, vol_label, cx))
            }
        })
        // Sleep timer: a click opens its menu (lengths, end of song, off).
        .child(
            toggle_button("sleep", "timer", 15.0, app.sleep.is_some(), None)
                .on_mouse_down(gpui::MouseButton::Left, on_right_click(ControlMenu::Sleep, cx))
                .on_mouse_down(gpui::MouseButton::Right, on_right_click(ControlMenu::Sleep, cx)),
        )
        .child(toggle_button("mini", "picture-in-picture-2", 15.0, app.mini_shown, None).on_click(cx.listener(|this, _, _, cx| {
            mark_toggled("mini");
            this.toggle_mini(cx)
        })).on_mouse_down(gpui::MouseButton::Right, on_right_click(ControlMenu::Mini, cx)))
        .child(
            toggle_button("queue", "list-music", 15.0, app.show_queue, None).on_click(cx.listener(|this, _, _, cx| {
                mark_toggled("queue");
                this.show_queue = !this.show_queue;
                cx.notify();
            })),
        );

    let measure = gpui::canvas(|_, _, _| (), |b, _, _, _| BAR_W.with(|w| w.set(f32::from(b.size.width)))).absolute().inset_0();
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
        .child(measure)
        .child(center)
        .child(div().relative().flex().w(px(SIDE_W)).min_w_0().child(now))
        .child(div().relative().flex().w(px(SIDE_W)).justify_end().child(right))
}
