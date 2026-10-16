//! Settings: server, providers + priority, API keys, visuals, updates.

use crate::app::{BackendState, ConnTest, Glyph};
use crate::theme;
use crate::ui::widgets::*;
use glyph_core::{BackendMode, UpdateStatus};
use glyph_protocol::ProviderId;
use gpui::{div, prelude::*, px, Context, Div, Window};
use std::rc::Rc;

fn section(name: &str, title: &str) -> Div {
    panel()
        .p(px(18.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(10.0))
                .child(div().text_size(theme::text_lg()).text_color(theme::text()).child(title.to_string()))
                .child(tag(name)),
        )
}

fn field(label: &str, input: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .child(div().w(px(180.0)).flex_none().text_size(theme::text_sm()).text_color(theme::text_dim()).child(label.to_string()))
        .child(
            div()
                .flex_1()
                .h(px(28.0))
                .px(px(10.0))
                .rounded(px(6.0))
                .bg(theme::panel_strong())
                .border_1()
                .border_color(theme::border())
                .text_size(theme::text_sm())
                .child(input),
        )
}

pub fn settings(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let accent = app.palette.map(|p| p.accent()).unwrap_or(crate::visuals::Rgb::hex(0xff5fd2));

    // ---- server
    let mode = match &app.backend_state {
        BackendState::Connecting => format!("{} verbinde ...", app.spinner()),
        BackendState::Ready(BackendMode::Server { url, server_version }) => format!("MODE server {url} · v{server_version}"),
        BackendState::Ready(BackendMode::Direct) => "MODE direct".into(),
        BackendState::Ready(BackendMode::Incompatible { protocol_version, .. }) => {
            format!("[ SERVER v{protocol_version} · INKOMPATIBEL ] -> direct")
        }
        BackendState::Offline(e) => format!("[ BACKEND OFFLINE ] {e}"),
    };
    let conn = match &app.conn_test {
        None => None,
        Some(ConnTest::Running) => Some((format!("{} teste ...", app.spinner()), theme::text_dim())),
        Some(ConnTest::Ok(s)) => Some((s.clone(), theme::ok())),
        Some(ConnTest::Failed(s)) => Some((s.clone(), theme::warn())),
    };
    let server = section("server", "server")
        .child(field("server-url", app.server_input.clone()))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(outline_button("conn-test", "[ VERBINDUNG TESTEN ]").on_click(cx.listener(|this, _, _, cx| this.test_connection(cx))))
                .child(outline_button("conn-apply", "[ ÜBERNEHMEN ]").on_click(cx.listener(|this, _, _, cx| this.apply_server(cx))))
                .when_some(conn, |d, (s, c)| d.child(div().text_size(theme::text_xs()).text_color(c).child(s))),
        )
        .child(div().text_size(theme::text_xs()).text_color(theme::text_faint()).child(mode));

    // ---- providers
    let enabled = app.config.providers.enabled.clone();
    let providers = section("providers", "quellen")
        .child(
            div().flex().flex_wrap().gap(px(6.0)).children(ProviderId::ALL.into_iter().map(|p| {
                let on = enabled.contains(&p);
                let cooling = app.backend.as_ref().and_then(|b| crate::guard::guard(|| b.provider_cooling_down(p))).unwrap_or(false);
                div()
                    .id(("prov", p as usize))
                    .cursor_pointer()
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(if on { theme::border_strong() } else { theme::border_faint() })
                    .text_size(theme::text_sm())
                    .text_color(if on { theme::text() } else { theme::text_faint() })
                    .hover(|s| s.bg(theme::hover()))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_provider(p, cx)))
                    .child(format!(
                        "{} [{}] {}{}",
                        if on { "[x]" } else { "[ ]" },
                        p.badge(),
                        p.display_name(),
                        if cooling { " · pause" } else { "" }
                    ))
            })),
        )
        .child(div().text_size(theme::text_xs()).text_color(theme::text_faint()).child("priorität (volle quellen, oben zuerst)"))
        .child(div().flex().flex_col().gap(px(2.0)).children(app.config.providers.priority.clone().into_iter().enumerate().map(|(i, p)| {
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(theme::text_sm())
                .child(div().w(px(22.0)).text_color(theme::text_ghost()).child(format!("{}.", i + 1)))
                .child(div().w(px(180.0)).child(format!("[{}] {}", p.badge(), p.display_name())))
                .child(ascii_button(("prio-up", i), "↑").on_click(cx.listener(move |this, _, _, cx| this.move_priority(i, true, cx))))
                .child(ascii_button(("prio-down", i), "↓").on_click(cx.listener(move |this, _, _, cx| this.move_priority(i, false, cx))))
        })));

    // ---- keys
    let keys = section("keys", "api-keys")
        .child(div().text_size(theme::text_xs()).text_color(theme::text_faint()).child("nur lokal in config.toml · env-variablen (GLYPH_*) haben vorrang · enter speichert"))
        .children(app.key_inputs.iter().map(|(name, input)| field(name, input.clone())));

    // ---- visuals
    let weak = cx.entity().downgrade();
    let set_int: Rc<dyn Fn(f32, &mut Window, &mut gpui::App)> = Rc::new(move |f, _, cx| {
        weak.update(cx, |this, cx| this.set_intensity(f, cx)).ok();
    });
    let intensity = app.config.visuals.intensity;
    let visuals = section("visuals", "visuals")
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .text_size(theme::text_sm())
                .child(div().w(px(180.0)).text_color(theme::text_dim()).child("intensität"))
                .child(ascii_bar("intensity", 24, intensity, accent.lighten(0.3).hsla(0.95), set_int))
                .child(div().text_color(theme::text_faint()).child(format!("{:.0}%", intensity * 100.0))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .text_size(theme::text_sm())
                .child(div().w(px(180.0)).text_color(theme::text_dim()).child("preset"))
                .child(div().text_color(theme::text_faint()).child(app.glow.read(cx).preset().name())),
        );

    // ---- updates
    let u = &app.update;
    let status: (String, gpui::Hsla) = if u.checking {
        (format!("{} GET api.github.com/.../releases", app.spinner()), theme::text_dim())
    } else if let Some((done, total)) = u.installing {
        let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
        let n = 20;
        let k = (frac * n as f32) as usize;
        (format!("DOWNLOAD {}{} {}%", "█".repeat(k), "░".repeat(n - k), (frac * 100.0) as u32), theme::text())
    } else if u.installed {
        ("UPDATE installiert -> neustart nötig".into(), theme::ok())
    } else if let Some(e) = &u.error {
        (format!("ERR update -> {e}"), theme::warn())
    } else {
        match &u.status {
            None => ("noch nicht geprüft".into(), theme::text_faint()),
            Some(UpdateStatus::UpToDate { current }) => (format!("aktuell · v{current}"), theme::ok()),
            Some(UpdateStatus::Skipped) => ("letzte prüfung < 24 h".into(), theme::text_faint()),
            Some(UpdateStatus::Available(i)) => (format!("[ UPDATE v{} VERFÜGBAR ]", i.version.trim_start_matches('v')), theme::ok()),
        }
    };
    let avail = app.available_update().cloned();
    let prerelease = app.config.updates.channel == "prerelease";
    let updates = section("updates", "updates")
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(outline_button("upd-check", "[ Nach Updates suchen ]").on_click(cx.listener(|this, _, _, cx| this.check_updates(true, cx))))
                .when_some(avail.clone().filter(|i| i.can_self_install && !app.update.installed && app.update.installing.is_none()), |d, info| {
                    d.child(outline_button("upd-install", "[ INSTALLIEREN ]").on_click(cx.listener(move |this, _, _, cx| this.install_update(info.clone(), cx))))
                })
                .when(app.update.installed, |d| {
                    d.child(outline_button("upd-restart", "[ NEU STARTEN ]").on_click(cx.listener(|this, _, _, cx| this.restart(cx))))
                })
                .child(div().text_size(theme::text_xs()).text_color(status.1).child(status.0)),
        )
        .when_some(avail.filter(|i| !i.can_self_install), |d, i| {
            d.child(div().text_size(theme::text_xs()).text_color(theme::text_faint()).child(format!("paketmanager-installation -> bitte dort aktualisieren · {}", i.html_url)))
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(14.0))
                .text_size(theme::text_sm())
                .child(
                    ascii_button("upd-auto", format!("{} automatisch installieren", if app.config.updates.auto_install { "[x]" } else { "[ ]" }))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_auto_update(cx))),
                )
                .child(div().text_color(theme::text_faint()).child("kanal"))
                .child(ascii_button("upd-stable", if !prerelease { "(•) stable" } else { "( ) stable" }).on_click(cx.listener(|this, _, _, cx| this.set_update_channel(false, cx))))
                .child(ascii_button("upd-pre", if prerelease { "(•) prerelease" } else { "( ) prerelease" }).on_click(cx.listener(|this, _, _, cx| this.set_update_channel(true, cx)))),
        )
        .child(div().text_size(theme::text_xs()).text_color(theme::text_faint()).child(format!("glyph v{} · keine telemetrie", env!("CARGO_PKG_VERSION"))));

    div()
        .id("settings-scroll")
        .size_full()
        .overflow_y_scroll()
        .px(px(28.0))
        .py(px(24.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(14.0))
                .max_w(px(820.0))
                .child(tag("settings"))
                .child(heading("einstellungen"))
                .child(server)
                .child(providers)
                .child(keys)
                .child(visuals)
                .child(updates),
        )
}
