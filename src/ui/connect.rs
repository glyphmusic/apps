//! "Connect to a device": the side panel (queue slot) listing where music
//! can play: this computer and the speakers, TVs and players Glyph found on
//! the network (see `crate::devices`).

use crate::app::Glyph;
use crate::devices::{Device, Kind};
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::views::QUEUE_W;
use crate::ui::widgets::*;
use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, SharedString, Window};

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

/// One device row: icon, name, protocol / state.
fn row(id: impl Into<SharedString>, icon_name: &str, name: SharedString, sub: SharedString, enabled: bool) -> gpui::Stateful<gpui::Div> {
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
                .child(div().truncate().text_size(theme::text_xs()).text_color(theme::text_dim()).child(sub)),
        )
}

pub fn panel(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let devices = app.network_devices();
    let selected = app.device_selected().cloned();
    let mpd = app.mpd.as_ref().map(|m| m.address.clone());
    let searching = app.devices_searching();
    let search_on = app.discovery.is_some();

    // Where music plays now.
    let (cur_icon, cur_name, cur_sub): (&str, SharedString, SharedString) = if let Some(o) = &app.device_out {
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
        let out = app.config.audio.output_device.clone().map(SharedString::from).unwrap_or_else(|| t("sp.output.default"));
        ("monitor", t("connect.this_computer"), out)
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
                .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(cur_sub)),
        );

    let mut rows: Vec<AnyElement> = Vec::new();
    // Back to this computer.
    if app.remote_chosen() {
        rows.push(
            row("dev-local", "monitor", t("connect.this_computer"), app.config.audio.output_device.clone().map(SharedString::from).unwrap_or_else(|| t("sp.output.default")), true)
                .on_click(cx.listener(|this, _, _, cx| this.use_local_output(cx)))
                .into_any_element(),
        );
    }
    let is_on = |d: &Device| match d.kind {
        Kind::Mpd => mpd.as_ref().is_some_and(|a| crate::mpd::Address::parse(&d.mpd_address()).is_some_and(|x| x.host == a.host && x.port == a.port)),
        _ => selected.as_ref().is_some_and(|s| s.id == d.id),
    };
    for (i, d) in devices.iter().filter(|d| !is_on(d)).enumerate() {
        let playable = d.kind.playable();
        let sub: SharedString = if playable { kind_label(d.kind).into() } else { tf("ctl.output.unsupported", &[("name", &kind_label(d.kind))]) };
        let r = row(format!("dev-{i}"), d.icon, d.name.clone().into(), sub, playable);
        rows.push(if playable {
            let dev = d.clone();
            r.on_click(cx.listener(move |this, _, _, cx| this.select_device(dev.clone(), cx))).into_any_element()
        } else {
            r.into_any_element()
        });
    }

    let body: AnyElement = if !search_on {
        div()
            .px(px(16.0))
            .pt(px(8.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(div().text_size(theme::text_sm()).text_color(theme::text_dim()).child(t("connect.off")))
            .child(
                div().flex().child(
                    button("dev-search-on", t("connect.turn_on")).on_click(cx.listener(|this, _, _, cx| this.set_discovery(true, cx))),
                ),
            )
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .when(!rows.is_empty(), |d| d.child(section(t("connect.others"))))
            .child(div().px(px(10.0)).flex().flex_col().children(rows))
            .when(searching, |d| d.child(div().px(px(16.0)).py(px(10.0)).text_size(theme::text_sm()).text_color(theme::text_dim()).child(t("ctl.output.searching"))))
            .when(!searching && devices.is_empty(), |d| d.child(div().px(px(16.0)).py(px(10.0)).text_size(theme::text_sm()).text_color(theme::text_dim()).child(t("ctl.output.none"))))
            .into_any_element()
    };

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
        .child(
            div()
                .flex()
                .items_center()
                .pl(px(16.0))
                .pr(px(10.0))
                .pt(px(16.0))
                .pb(px(8.0))
                .child(div().flex_1().text_size(theme::text_lg()).font_weight(FontWeight::SEMIBOLD).child(t("connect.title")))
                .when(search_on, |d| {
                    d.child(
                        with_tip(icon_button_plain("dev-rescan", "refresh-cw", 15.0, theme::text_dim()), Some(t("connect.rescan")))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.rescan_devices_now();
                                cx.notify()
                            })),
                    )
                })
                .child(icon_button("dev-close", "x", 16.0, theme::text_dim()).on_click(cx.listener(|this, _, _, cx| {
                    this.show_devices = false;
                    cx.notify();
                }))),
        )
        .child(current)
        .child(
            fade_edges(
                div().id("dev-scroll").track_scroll(&app.devices_scroll).size_full().overflow_y_scroll().pb(px(10.0)).child(body),
                theme::panel_strong(),
                18.0,
                Some(&app.devices_scroll),
            )
            .flex_1()
            .min_h_0()
            .child(scrollbar(&app.devices_scroll, true, cx)),
        )
        .child(div().pt(px(6.0)).pb(px(10.0)).border_t_1().border_color(theme::border_faint()).child(help))
}
