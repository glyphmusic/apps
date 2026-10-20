//! Right-click menus of the player controls (player bar, Now Playing): the
//! mini player button picks card or bar, repeat and shuffle pick their mode,
//! previous / next jump inside the song, the volume picks the output.

use crate::app::{ControlMenu, Glyph};
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::lists::menu_row;
use crate::ui::widgets::icon;
use glyph_core::config::{ChannelMode, ShuffleStyle};
use glyph_core::RepeatMode;
use gpui::{anchored, deferred, div, prelude::*, px, AnyElement, Context, ElementId, MouseButton, MouseDownEvent, SharedString, Window};

fn white(a: f32) -> gpui::Hsla {
    gpui::hsla(0.0, 0.0, 1.0, a)
}

/// One choice; `on` draws the check at its end.
fn choice(id: impl Into<ElementId>, icon_name: &str, label: SharedString, on: bool, cx: &mut Context<Glyph>) -> gpui::Stateful<gpui::Div> {
    menu_row(id.into(), icon_name, label, if on { theme::pink() } else { white(0.7) }, false, false, cx)
        .when(on, |d| d.child(icon("check", 14.0, theme::pink())))
}

/// A small caption above a group of choices.
fn caption(text: SharedString) -> AnyElement {
    div().px(px(10.0)).pt(px(6.0)).pb(px(2.0)).text_size(px(11.0)).text_color(white(0.45)).child(text).into_any_element()
}

fn sep() -> AnyElement {
    div().h(px(1.0)).mx(px(6.0)).my(px(4.0)).bg(white(0.07)).into_any_element()
}

/// Right-click handler that opens control menu `kind` at the mouse.
pub fn on_right_click(kind: ControlMenu, cx: &mut Context<Glyph>) -> impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static {
    let weak = cx.entity().downgrade();
    move |e: &MouseDownEvent, _: &mut Window, cx: &mut gpui::App| {
        cx.stop_propagation();
        weak.update(cx, |this, cx| this.open_control_menu(kind, e.position, cx)).ok();
    }
}

/// An AI assistant asks for access (OAuth sign-in through the server): the
/// user compares the code with the browser page and allows or declines.
pub fn mcp_approval(app: &mut Glyph, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let a = app.mcp.approvals.first()?.clone();
    let (id1, id2) = (a.id.clone(), a.id.clone());
    let panel = div()
        .id("mcp-approval")
        .occlude()
        .w(px(420.0))
        .p(px(22.0))
        .rounded(px(12.0))
        .bg(gpui::hsla(0.0, 0.0, 0.08, 0.99))
        .border_1()
        .border_color(white(0.1))
        .flex()
        .flex_col()
        .gap(px(14.0))
        .font_family(theme::FONT)
        .child(div().flex().items_center().gap(px(10.0)).child(icon("bot", 20.0, theme::pink())).child(div().text_size(px(17.0)).font_weight(gpui::FontWeight::SEMIBOLD).child(crate::i18n::tf("mcp.approve.title", &[("client", &a.client)]))))
        .child(div().text_size(px(13.5)).text_color(white(0.7)).child(t("mcp.approve.body")))
        .child(div().flex().justify_center().py(px(6.0)).child(crate::ui::widgets::mono().text_size(px(26.0)).text_color(theme::pink()).child(a.code.clone())))
        .child(div().text_size(px(12.5)).text_color(white(0.5)).child(t("mcp.approve.hint")))
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.0))
                .child(crate::ui::widgets::button("mcp-deny", t("mcp.approve.deny")).on_click(cx.listener(move |this, _, _, cx| this.answer_mcp_approval(&id1, false, cx))))
                .child(crate::ui::widgets::primary_button("mcp-allow", t("mcp.approve.allow")).on_click(cx.listener(move |this, _, _, cx| this.answer_mcp_approval(&id2, true, cx)))),
        );
    Some(
        deferred(div().id("mcp-approval-backdrop").absolute().inset_0().occlude().bg(gpui::hsla(0.0, 0.0, 0.0, 0.5)).flex().items_center().justify_center().child(panel))
            .with_priority(5)
            .into_any_element(),
    )
}

pub fn control_menu(app: &mut Glyph, _window: &mut Window, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let (kind, pos, opened) = app.control_menu?;
    let k = (opened.elapsed().as_secs_f32() / 0.14).min(1.0);
    let alpha = 1.0 - (1.0 - k).powi(3);
    let mut body: Vec<AnyElement> = Vec::new();
    match kind {
        ControlMenu::Mini => {
            let shown = app.mini_shown;
            let big = app.mini_big;
            body.push(
                choice("ctl-mini-card", "picture-in-picture-2", t("ctl.mini.card"), shown && big, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_control_menu(cx);
                        this.open_mini_as(true, cx)
                    }))
                    .into_any_element(),
            );
            body.push(
                choice("ctl-mini-bar", "panel-bottom", t("ctl.mini.bar"), shown && !big, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_control_menu(cx);
                        this.open_mini_as(false, cx)
                    }))
                    .into_any_element(),
            );
            if shown {
                body.push(sep());
                body.push(
                    menu_row("ctl-mini-close".into(), "x", t("ctl.mini.close"), white(0.7), false, false, cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_control_menu(cx);
                            this.toggle_mini(cx)
                        }))
                        .into_any_element(),
                );
            }
        }
        ControlMenu::Repeat => {
            let cur = app.repeat();
            for (id, icon_name, label, mode) in [
                ("ctl-rep-off", "repeat", t("ctl.repeat.off"), RepeatMode::Off),
                ("ctl-rep-all", "repeat", t("ctl.repeat.all"), RepeatMode::All),
                ("ctl-rep-one", "repeat-1", t("ctl.repeat.one"), RepeatMode::One),
            ] {
                body.push(
                    choice(id, icon_name, label, cur == mode, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.close_control_menu(cx);
                            this.set_repeat_mode(mode, cx)
                        }))
                        .into_any_element(),
                );
            }
        }
        ControlMenu::Shuffle => {
            let on = app.shuffle();
            let style = app.config.playback.shuffle;
            body.push(
                choice("ctl-shuf-off", "arrow-right", t("ctl.shuffle.off"), !on, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_control_menu(cx);
                        if this.shuffle() {
                            this.toggle_shuffle(cx)
                        }
                    }))
                    .into_any_element(),
            );
            for (id, label, s) in [("ctl-shuf-smart", t("ctl.shuffle.smart"), ShuffleStyle::Smart), ("ctl-shuf-random", t("ctl.shuffle.random"), ShuffleStyle::Random)] {
                body.push(
                    choice(id, "shuffle", label, on && style == s, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.close_control_menu(cx);
                            this.shuffle_with(s, cx)
                        }))
                        .into_any_element(),
                );
            }
        }
        ControlMenu::Back | ControlMenu::Forward => {
            let back = kind == ControlMenu::Back;
            let sign = if back { -1.0 } else { 1.0 };
            let (i10, i30) = if back { ("rotate-ccw", "rotate-ccw") } else { ("rotate-cw", "rotate-cw") };
            for (id, icon_name, label, secs) in [
                ("ctl-seek-10", i10, if back { t("ctl.back_10") } else { t("ctl.fwd_10") }, 10.0),
                ("ctl-seek-30", i30, if back { t("ctl.back_30") } else { t("ctl.fwd_30") }, 30.0),
            ] {
                body.push(
                    menu_row(id.into(), icon_name, label, white(0.7), false, false, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.close_control_menu(cx);
                            this.seek_relative(sign * secs);
                            cx.notify()
                        }))
                        .into_any_element(),
                );
            }
            body.push(sep());
            if back {
                body.push(
                    menu_row("ctl-restart".into(), "skip-back", t("ctl.restart"), white(0.7), false, false, cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_control_menu(cx);
                            this.seek_to(std::time::Duration::ZERO);
                            cx.notify()
                        }))
                        .into_any_element(),
                );
                body.push(
                    menu_row("ctl-prev".into(), "skip-back", t("ctl.previous"), white(0.7), false, false, cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_control_menu(cx);
                            // Straight to the previous song, also late in this one.
                            this.seek_to(std::time::Duration::ZERO);
                            this.previous_song(cx)
                        }))
                        .into_any_element(),
                );
            } else {
                body.push(
                    menu_row("ctl-next".into(), "skip-forward", t("ctl.next"), white(0.7), false, false, cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_control_menu(cx);
                            this.next(false, cx)
                        }))
                        .into_any_element(),
                );
            }
        }
        ControlMenu::Volume => {
            let current = app.config.audio.output_device.clone();
            // While a remote output plays, no sound device here is "on";
            // picking one brings playback back to this computer.
            let remote = app.remote_chosen();
            body.push(
                choice("ctl-out-default", "speaker", t("sp.output.default"), current.is_none() && !remote, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_control_menu(cx);
                        this.use_local_output(cx);
                        this.set_output_device(None, cx)
                    }))
                    .into_any_element(),
            );
            let devices = app.output_devices.clone();
            if devices.is_empty() {
                body.push(caption(t("ctl.output.searching")));
            }
            // MPD: another computer plays (Settings › Playback sets its address).
            if let Some(a) = crate::mpd::Address::parse(&app.config.mpd.address) {
                let on = app.config.mpd.enabled;
                body.push(
                    choice("ctl-out-mpd", "radio", tf("ctl.output.mpd", &[("host", &a.label())]), on, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.close_control_menu(cx);
                            this.set_mpd_enabled(!on, cx)
                        }))
                        .into_any_element(),
                );
            }
            for (i, name) in devices.into_iter().enumerate() {
                let on = !remote && current.as_deref() == Some(name.as_str());
                let n = name.clone();
                body.push(
                    choice(("ctl-out", i), "headphones", name.into(), on, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.close_control_menu(cx);
                            this.use_local_output(cx);
                            this.set_output_device(Some(n.clone()), cx)
                        }))
                        .into_any_element(),
                );
            }
            // Speakers, TVs and other computers: the Connect panel.
            if let Some(d) = app.device_selected() {
                let name = d.name.clone();
                body.push(
                    choice("ctl-out-device", d.icon, name.into(), true, cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_control_menu(cx);
                            if !this.show_devices {
                                this.toggle_devices_panel(cx)
                            }
                        }))
                        .into_any_element(),
                );
            }
            body.push(
                menu_row("ctl-connect".into(), "monitor-speaker", t("tip.monitor-speaker"), white(0.7), false, false, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_control_menu(cx);
                        if !this.show_devices {
                            this.toggle_devices_panel(cx)
                        }
                    }))
                    .into_any_element(),
            );
            body.push(sep());
            let mode = app.config.audio.channels;
            for (id, icon_name, label, m) in [
                ("ctl-ch-mono", "circle", t("sp.audio.channels.mono"), ChannelMode::Mono),
                ("ctl-ch-stereo", "audio-lines", t("sp.audio.channels.stereo"), ChannelMode::Stereo),
                ("ctl-ch-spatial", "orbit", t("sp.audio.channels.spatial"), ChannelMode::Spatial),
            ] {
                body.push(
                    choice(id, icon_name, label, mode == m, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.close_control_menu(cx);
                            this.set_channel_mode(m, cx)
                        }))
                        .into_any_element(),
                );
            }
            body.push(sep());
            let muted = app.volume() <= 0.001;
            body.push(
                menu_row("ctl-mute".into(), if muted { "volume-2" } else { "volume-x" }, if muted { t("ctl.unmute") } else { t("ctl.mute") }, white(0.7), false, false, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_control_menu(cx);
                        this.toggle_mute();
                        cx.notify()
                    }))
                    .into_any_element(),
            );
        }
    }
    let menu = div()
        .id("control-menu")
        .w(px(236.0))
        .max_h(px(420.0))
        .overflow_y_scroll()
        .p(px(6.0))
        .rounded(px(10.0))
        .bg(gpui::hsla(0.0, 0.0, 0.075, 0.98))
        .border_1()
        .border_color(white(0.1))
        .flex()
        .flex_col()
        .font_family(theme::FONT)
        .opacity(alpha)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .children(body);
    // Opens upwards from the bar (it sits at the window's bottom edge);
    // `anchored` keeps it inside the window either way.
    let layer = div()
        .id("control-backdrop")
        .absolute()
        .inset_0()
        .occlude()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close_control_menu(cx)))
        .on_mouse_down(MouseButton::Right, cx.listener(|this, _, _, cx| this.close_control_menu(cx)))
        .on_scroll_wheel(cx.listener(|this, _, _, cx| this.close_control_menu(cx)))
        .child(anchored().position(pos).anchor(gpui::Anchor::BottomLeft).snap_to_window_with_margin(px(8.0)).child(menu));
    Some(deferred(layer).with_priority(3).into_any_element())
}
