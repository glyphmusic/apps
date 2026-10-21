//! "Connect to a device": the side panel (queue slot) listing where music
//! can play: this computer, the other signed-in Glyph apps (first) and the
//! speakers, TVs and players Glyph found on the network (see
//! `crate::devices`). The Jam button in the header swaps the list for the
//! jam (see `crate::ui::jam`), with a back button to the devices.

use crate::app::Glyph;
use crate::devices::{Device, Kind};
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::views::QUEUE_W;
use crate::ui::widgets::*;
use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, Hsla, SharedString, Window};

/// Docs page: what to check when a device is missing.
const HELP_URL: &str = "https://docs.glyph.my/using-glyph/devices";

/// Protocol name under a device.
fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Chromecast => "Google Cast",
        Kind::Sonos => "Sonos",
        Kind::Upnp => "DLNA",
        Kind::Mpd => "MPD",
        Kind::AirPlay => "AirPlay",
    }
}

/// Icon of a signed-in Glyph app by the platform it runs on.
pub fn platform_icon(platform: &str) -> &'static str {
    match platform {
        "android" | "ios" => "smartphone",
        _ => "monitor",
    }
}

/// Icon in a rounded square, like a cover in the queue.
fn device_icon(name: &str, on: bool) -> AnyElement {
    div()
        .flex_none()
        .size(px(40.0))
        .rounded(px(6.0))
        .bg(theme::card())
        .flex()
        .items_center()
        .justify_center()
        .child(icon(name, 20.0, if on { theme::pink() } else { theme::text_dim() }))
        .into_any_element()
}

/// `[ LABEL ]` heading, as in the queue.
fn section(text: SharedString) -> AnyElement {
    mono().px(px(16.0)).pt(px(4.0)).pb(px(8.0)).text_size(px(11.0)).text_color(theme::text_faint()).child(format!("[ {} ]", text.to_uppercase())).into_any_element()
}

/// Small grey line under the list.
fn note(text: impl Into<SharedString>, color: Hsla) -> AnyElement {
    div().px(px(16.0)).py(px(8.0)).text_size(theme::text_sm()).text_color(color).child(text.into()).into_any_element()
}

/// One device row: icon, name, protocol / state. A click plays there.
fn row(id: impl Into<SharedString>, icon_name: &str, name: SharedString, sub: SharedString, sub_color: Hsla, enabled: bool) -> gpui::Stateful<gpui::Div> {
    let id: SharedString = id.into();
    div()
        .id(gpui::ElementId::Name(id.clone()))
        .flex()
        .items_center()
        .gap(px(12.0))
        .h(px(52.0))
        .px(px(6.0))
        .rounded(px(8.0))
        .relative()
        .when(enabled, |d| d.cursor_pointer().child(hover_fill(id.clone(), theme::card(), 8.0)))
        .when(!enabled, |d| d.opacity(0.45))
        .child(device_icon(icon_name, false))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().truncate().text_size(theme::text_sm()).font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(name))
                .child(div().truncate().text_size(theme::text_xs()).text_color(sub_color).child(sub)),
        )
}

pub fn panel(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let jam_view = app.devices_jam;
    let search_on = app.discovery.is_some();
    let jam_on = app.jam_active();

    let header = div().flex().items_center().gap(px(2.0)).pl(px(if jam_view { 8.0 } else { 16.0 })).pr(px(10.0)).pt(px(16.0)).pb(px(8.0));
    let header = if jam_view {
        header
            .child(
                with_tip(icon_button_plain("dev-back", "arrow-left", 16.0, theme::text_dim()), Some(t("connect.back"))).on_click(cx.listener(|this, _, _, cx| {
                    this.devices_jam = false;
                    cx.notify();
                })),
            )
            .child(div().flex_1().pl(px(4.0)).text_size(theme::text_lg()).font_weight(FontWeight::SEMIBOLD).text_color(if jam_on { theme::pink() } else { theme::text() }).child(t("jam.title")))
    } else {
        header
            .child(div().flex_1().text_size(theme::text_lg()).font_weight(FontWeight::SEMIBOLD).child(t("connect.title")))
            // Listening together: its own page of this panel, pink while in one.
            .child(
                with_tip(icon_button_plain("dev-jam", "users", 15.0, if jam_on { theme::pink() } else { theme::text_dim() }), Some(t("jam.title"))).on_click(cx.listener(|this, _, _, cx| {
                    this.devices_jam = true;
                    cx.notify();
                })),
            )
            .when(search_on, |d| {
                d.child(
                    with_tip(icon_button_plain("dev-rescan", "refresh-cw", 15.0, theme::text_dim()), Some(t("connect.rescan")))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.rescan_devices_now();
                            cx.notify()
                        })),
                )
            })
    }
    .child(icon_button("dev-close", "x", 16.0, theme::text_dim()).on_click(cx.listener(|this, _, _, cx| {
        this.show_devices = false;
        cx.notify();
    })));

    let content: AnyElement = if jam_view { crate::ui::jam::section_el(app, cx) } else { devices(app, cx) };

    let help = div()
        .id("dev-help")
        .mx(px(10.0))
        .h(px(44.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .relative()
        .cursor_pointer()
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(hover_fill("dev-help", theme::card(), 8.0))
        .child(div().flex_1().min_w_0().truncate().underline().text_size(theme::text_sm()).text_color(theme::text()).child(t("connect.help")))
        .child(icon("external-link", 14.0, theme::text_dim()))
        .on_click(|_, _, cx| cx.open_url(HELP_URL));

    crate::ui::widgets::panel()
        .flex_none()
        .w(px(QUEUE_W))
        .h_full()
        .bg(theme::panel_strong())
        .flex()
        .flex_col()
        .child(header)
        .child(
            fade_edges(
                div().id("dev-scroll").track_scroll(&app.devices_scroll).size_full().overflow_y_scroll().touch_axis().pb(px(10.0)).child(content),
                theme::panel_strong(),
                18.0,
                Some(&app.devices_scroll),
            )
            .flex_1()
            .min_h_0()
            .child(scrollbar(&app.devices_scroll, true, cx)),
        )
        .when(!jam_view, |d| d.child(div().pt(px(6.0)).pb(px(10.0)).border_t_1().border_color(theme::border_faint()).child(help)))
}

/// Where music plays now, then one list: this computer, the other Glyph
/// apps, the devices on the network.
pub(crate) fn devices(app: &mut Glyph, cx: &mut Context<Glyph>) -> AnyElement {
    let devices = app.network_devices();
    let selected = app.device_selected().cloned();
    let mpd = app.mpd.as_ref().map(|m| m.address.clone());
    let searching = app.devices_searching();
    let search_on = app.discovery.is_some();
    let target = app.account.output_target.clone();
    let spinner = app.spinner();
    let local_out = app.config.audio.output_device.clone().map(SharedString::from).unwrap_or_else(|| t("sp.output.default"));

    // Where music plays now.
    let mut cur_sub_color = theme::text_dim();
    let (cur_icon, cur_name, cur_sub): (&str, SharedString, SharedString) = if let Some(o) = app.account_output() {
        let sub: SharedString = if let Some(e) = app.account.output_error.clone().filter(|_| app.account.output_error_at == target) {
            cur_sub_color = theme::warn();
            e.into()
        } else if !app.account.output_commands.borrow().is_empty() {
            format!("{spinner} {}", t("connect.account.confirming")).into()
        } else if !o.online {
            format!("{spinner} {}", t("connect.account.reconnecting")).into()
        } else {
            t("connect.account.control")
        };
        (platform_icon(&o.device.platform), o.device.name.clone().into(), sub)
    } else if let Some(o) = &app.device_out {
        let sub = match (o.error(), o.status()) {
            (Some(_), _) => t("connect.unreachable"),
            (None, None) => t("connect.connecting"),
            (None, Some(_)) => kind_label(o.device.kind).into(),
        };
        (o.device.icon, o.device.name.clone().into(), sub)
    } else if let Some(a) = &mpd {
        let sub = match app.mpd.as_ref().and_then(|m| m.error()) {
            Some(_) => t("connect.unreachable"),
            None => "MPD".into(),
        };
        let name = devices.iter().find(|d| d.kind == Kind::Mpd && crate::mpd::Address::parse(&d.mpd_address()).is_some_and(|x| x.host == a.host && x.port == a.port)).map(|d| d.name.clone()).unwrap_or_else(|| a.label());
        ("radio", name.into(), sub)
    } else {
        (this_icon(), this_device(), local_out.clone())
    };
    let current = div()
        .mx(px(10.0))
        .mb(px(12.0))
        .p(px(6.0))
        .rounded(px(10.0))
        .bg(theme::card())
        .flex()
        .items_center()
        .gap(px(12.0))
        .child(device_icon(cur_icon, true))
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::pink()).child(cur_name))
                .child(div().truncate().text_size(theme::text_sm()).text_color(cur_sub_color).child(cur_sub)),
        );

    let mut rows: Vec<AnyElement> = Vec::new();
    // Back to this computer.
    if app.remote_chosen() || target.is_some() {
        rows.push(
            row("dev-local", this_icon(), this_device(), local_out, theme::text_dim(), true)
                .on_click(cx.listener(|this, _, _, cx| this.use_local_output(cx)))
                .into_any_element(),
        );
    }

    // The other Glyph apps of this account come first.
    let signed_in = app.signed_in().is_some();
    let sending = app.account.transfer_to.clone();
    let mut glyph_apps = 0;
    for output in app.account.outputs.iter().filter(|o| o.online && !o.device.current && Some(&o.device.id) != target.as_ref()) {
        glyph_apps += 1;
        let id = output.device.id.clone();
        let mut color = theme::text_dim();
        let sub: SharedString = if sending.as_ref() == Some(&id) {
            format!("{spinner} {}", t("connect.connecting")).into()
        } else if let Some(e) = app.account.output_error.clone().filter(|_| app.account.output_error_at.as_ref() == Some(&id)) {
            color = theme::warn();
            e.into()
        } else if !output.state.available {
            t("connect.account.busy")
        } else {
            output.state.track.as_ref().map(|t| SharedString::from(format!("{} · {}", t.artist, t.title))).unwrap_or_else(|| t("connect.account.ready"))
        };
        let enabled = output.state.available && sending.is_none();
        let playing = output.state.playing && output.state.track.is_some();
        rows.push(
            row(format!("account-device-{id}"), platform_icon(&output.device.platform), output.device.name.clone().into(), sub, color, enabled)
                .when(playing, |d| d.child(div().flex_none().pr(px(6.0)).child(icon("audio-lines", 14.0, theme::text_dim()))))
                .when(enabled, |d| d.on_click(cx.listener(move |s, _, _, cx| s.connect_account_output(id.clone(), cx))))
                .into_any_element(),
        );
    }

    // Speakers, TVs and players on the network.
    let is_on = |d: &Device| match d.kind {
        Kind::Mpd => mpd.as_ref().is_some_and(|a| crate::mpd::Address::parse(&d.mpd_address()).is_some_and(|x| x.host == a.host && x.port == a.port)),
        _ => selected.as_ref().is_some_and(|s| s.id == d.id),
    };
    if search_on {
        for (i, d) in devices.iter().filter(|d| !is_on(d)).enumerate() {
            let dev = d.clone();
            rows.push(
                row(format!("dev-{i}"), d.icon, d.name.clone().into(), kind_label(d.kind).into(), theme::text_dim(), true)
                    .on_click(cx.listener(move |this, _, _, cx| this.select_device(dev.clone(), cx)))
                    .into_any_element(),
            );
        }
    }

    let general_error = app.account.output_error.clone().filter(|_| app.account.output_error_at.is_none());
    div()
        .flex()
        .flex_col()
        .child(current)
        .child(section(t("connect.devices")))
        .child(div().px(px(10.0)).flex().flex_col().children(rows))
        .when(signed_in && glyph_apps == 0, |d| d.child(note(t("connect.account.none"), theme::text_dim())))
        .when_some(general_error, |d, e| d.child(note(e, theme::warn())))
        .when(search_on && searching, |d| d.child(note(t("ctl.output.searching"), theme::text_dim())))
        .when(search_on && !searching && devices.is_empty(), |d| d.child(note(t("ctl.output.none"), theme::text_dim())))
        .when(!search_on, |d| {
            d.child(
                div()
                    .px(px(16.0))
                    .pt(px(8.0))
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(div().text_size(theme::text_sm()).text_color(theme::text_dim()).child(t("connect.off")))
                    .child(div().flex().child(button("dev-search-on", t("connect.turn_on")).on_click(cx.listener(|this, _, _, cx| this.set_discovery(true, cx))))),
            )
        })
        .into_any_element()
}

/// The other device music plays on (icon, name), if any.
pub fn playing_on(app: &Glyph) -> Option<(&'static str, String)> {
    if let Some(o) = app.account_output() {
        Some((platform_icon(&o.device.platform), o.device.name.clone()))
    } else if let Some(o) = app.device_out.as_ref().filter(|_| app.remote_on()) {
        Some((o.device.icon, o.device.name.clone()))
    } else if app.remote_on() {
        app.mpd.as_ref().map(|m| ("radio", m.address.label()))
    } else {
        None
    }
}

/// "This phone" / "This computer".
pub fn this_device() -> SharedString {
    t(if crate::ui::compact::TOUCH { "connect.this_phone" } else { "connect.this_computer" })
}

/// Icon of this device.
fn this_icon() -> &'static str {
    if crate::ui::compact::TOUCH { "smartphone" } else { "monitor" }
}

/// The strip under the player bar: pink while music plays on another
/// device ("Playing on …"), grey while another Glyph app plays on its own
/// ("Connect to …"). A click opens the Connect panel or takes control.
pub fn status_strip(app: &mut Glyph, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let playing_on = playing_on(app);
    let strip = |id: &'static str, icon_name: &str, label: SharedString, on: bool| {
        let fg = if on { gpui::hsla(0.0, 0.0, 0.04, 1.0) } else { theme::text_dim() };
        div()
            .id(id)
            .relative()
            .h(px(STRIP_H))
            .w_full()
            .px(px(12.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .bg(if on { theme::pink() } else { gpui::hsla(0.0, 0.0, 0.06, 0.94) })
            .when(!on, |d| d.child(hover_fill(id, theme::card(), 8.0)))
            .flex()
            .items_center()
            .justify_end()
            .gap(px(8.0))
            .child(icon(icon_name, 13.0, fg))
            .child(div().truncate().text_size(theme::text_xs()).font_weight(FontWeight::SEMIBOLD).text_color(fg).child(label))
    };
    if let Some((icon_name, name)) = playing_on {
        return Some(
            strip("bar-playing-on", icon_name, tf("connect.playing_on", &[("name", &name)]), true)
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.show_devices {
                        this.toggle_devices_panel(cx);
                    }
                }))
                .into_any_element(),
        );
    }
    // Another Glyph app plays while this one does not: one click to control it.
    if app.is_playing() || app.jam_active() {
        return None;
    }
    let o = app.account.outputs.iter().find(|o| o.online && !o.device.current && o.state.available && o.state.playing && o.state.track.is_some())?;
    let (id, icon_name, name) = (o.device.id.clone(), platform_icon(&o.device.platform), o.device.name.clone());
    Some(
        strip("bar-connect-to", icon_name, tf("connect.connect_to", &[("name", &name)]), false)
            .on_click(cx.listener(move |this, _, _, cx| this.select_account_output(id.clone(), cx)))
            .into_any_element(),
    )
}

/// Height of [`status_strip`].
pub const STRIP_H: f32 = 26.0;
