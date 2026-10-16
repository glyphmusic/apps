//! Settings: language, sources (combo mode), server, keys, visuals, updates.
//!
//! Layout: one centred column; each group is a small title over a flat card
//! of rows (label + description left, control right), hairlines between rows.

use crate::app::{BackendState, ConnTest, Glyph};
use crate::i18n::{t, tf, Lang};
use crate::theme;
use crate::ui::widgets::*;
use glyph_core::{BackendMode, UpdateStatus};
use glyph_protocol::ProviderId;
use gpui::{div, hsla, prelude::*, px, AnyElement, Context, Div, FontWeight, SharedString, Window};
use std::rc::Rc;

const COLUMN_W: f32 = 680.0;
const INPUT_W: f32 = 300.0;

fn group(title: SharedString, rows: Vec<AnyElement>) -> Div {
    let n = rows.len();
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .px(px(4.0))
                .text_size(theme::text_sm())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text_dim())
                .child(title),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .rounded(px(10.0))
                .bg(hsla(0.0, 0.0, 0.07, 0.92))
                .children(rows.into_iter().enumerate().map(move |(i, r)| {
                    div()
                        .when(i + 1 < n, |d| d.border_b_1().border_color(hsla(0.0, 0.0, 1.0, 0.06)))
                        .child(r)
                })),
        )
}

/// Label (+ optional description) on the left, control on the right.
fn row(label: impl Into<SharedString>, desc: Option<SharedString>, control: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(16.0))
        .py(px(12.0))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap(px(2.0))
                .child(div().text_size(theme::text_md()).child(label.into()))
                .when_some(desc, |d, s| d.child(div().text_size(theme::text_sm()).text_color(theme::text_faint()).child(s))),
        )
        .child(div().flex_none().flex().items_center().gap(px(8.0)).child(control))
        .into_any_element()
}

/// Full-width row content (lists, wrapped chips).
fn block(content: impl IntoElement) -> AnyElement {
    div().px(px(16.0)).py(px(12.0)).child(content).into_any_element()
}

fn switch(id: impl Into<gpui::ElementId>, on: bool) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .cursor_pointer()
        .w(px(34.0))
        .h(px(20.0))
        .rounded_full()
        .p(px(2.0))
        .flex()
        .when(on, |d| d.justify_end())
        .bg(if on { gpui::white() } else { hsla(0.0, 0.0, 1.0, 0.14) })
        .child(div().size(px(16.0)).rounded_full().bg(if on { gpui::black() } else { hsla(0.0, 0.0, 1.0, 0.7) }))
}

fn input_box(input: impl IntoElement) -> Div {
    div()
        .w(px(INPUT_W))
        .h(px(32.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(8.0))
        .bg(hsla(0.0, 0.0, 0.0, 0.45))
        .border_1()
        .border_color(hsla(0.0, 0.0, 1.0, 0.08))
        .text_size(theme::text_sm())
        .font_family(theme::MONO)
        .child(input)
}

/// Floating preview while a priority entry is dragged.
#[derive(Clone)]
struct PrioDrag {
    from: usize,
    label: SharedString,
}

impl Render for PrioDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(12.0))
            .py(px(6.0))
            .rounded(px(8.0))
            .bg(hsla(0.0, 0.0, 0.16, 0.96))
            .border_1()
            .border_color(hsla(0.0, 0.0, 1.0, 0.14))
            .text_size(theme::text_sm())
            .text_color(gpui::white())
            .font_family(theme::FONT)
            .child(self.label.clone())
    }
}

pub fn settings(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    // ---- language
    let cur = app.lang;
    let language = group(
        t("settings.language"),
        vec![block(div().flex().gap(px(8.0)).children(Lang::ALL.into_iter().map(|l| {
            div()
                .id(("lang", l as usize))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.set_language(l, cx)))
                .child(chip(l.native_name(), cur == l))
        })))],
    );

    // ---- sources
    let enabled = app.config.providers.enabled.clone();
    let audio_only = [ProviderId::Audius, ProviderId::Jamendo, ProviderId::Archive];
    let mut source_rows: Vec<AnyElement> = audio_only
        .into_iter()
        .map(|p| {
            let on = enabled.contains(&p);
            let cooling = app
                .backend
                .as_ref()
                .and_then(|b| crate::guard::guard(|| b.provider_cooling_down(p)))
                .unwrap_or(false);
            row(
                p.display_name(),
                cooling.then(|| t("settings.sources.paused")),
                switch(("prov", p as usize), on).on_click(cx.listener(move |this, _, _, cx| this.toggle_provider(p, cx))),
            )
        })
        .collect();
    source_rows.push(row(
        t("settings.sources.fallback"),
        Some(t("settings.sources.fallback.hint")),
        switch("embed-fallback", app.embed_fallback()).on_click(cx.listener(|this, _, _, cx| this.toggle_embed_fallback(cx))),
    ));
    let sources = group(t("settings.sources"), source_rows);

    // ---- priority (drag to reorder)
    let prio_rows: Vec<AnyElement> = app
        .config
        .providers
        .priority
        .clone()
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let drag = PrioDrag { from: i, label: p.display_name().into() };
            div()
                .id(("prio", i))
                .flex()
                .items_center()
                .gap(px(12.0))
                .px(px(16.0))
                .py(px(10.0))
                .cursor_grab()
                .hover(|s| s.bg(hsla(0.0, 0.0, 1.0, 0.03)))
                .on_drag(drag, |d, _, _, cx| cx.new(|_| d.clone()))
                .drag_over::<PrioDrag>(|s, _, _, _| s.bg(hsla(0.0, 0.0, 1.0, 0.07)))
                .on_drop(cx.listener(move |this, d: &PrioDrag, _, cx| this.reorder_priority(d.from, i, cx)))
                .child(icon("grip-vertical", 15.0, theme::text_faint()))
                .child(mono().w(px(18.0)).text_size(theme::text_sm()).text_color(theme::text_faint()).child(format!("{}", i + 1)))
                .child(div().flex_1().text_size(theme::text_md()).child(p.display_name()))
                .child(badge(p))
                .into_any_element()
        })
        .collect();
    let priority = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(8.0))
                .px(px(4.0))
                .child(div().text_size(theme::text_sm()).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_dim()).child(t("settings.priority")))
                .child(div().text_size(theme::text_xs()).text_color(theme::text_faint()).child(t("settings.priority.hint"))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .rounded(px(10.0))
                .bg(hsla(0.0, 0.0, 0.07, 0.92))
                .children(prio_rows),
        );

    // ---- server
    let mode: String = match &app.backend_state {
        BackendState::Connecting => format!("{} {}", app.spinner(), t("status.connecting")),
        BackendState::Ready(BackendMode::Server { url, server_version }) => format!("server {url} · v{server_version}"),
        BackendState::Ready(BackendMode::Direct) => t("status.direct").to_string(),
        BackendState::Ready(BackendMode::Incompatible { protocol_version, .. }) => tf("status.incompatible", &[("v", protocol_version)]).to_string(),
        BackendState::Offline(e) => format!("{} {e}", t("status.backend_offline")),
    };
    let conn = match &app.conn_test {
        None => None,
        Some(ConnTest::Running) => Some((format!("{} {}", app.spinner(), t("settings.server.testing")), theme::text_dim())),
        Some(ConnTest::Ok(s)) => Some((s.clone(), theme::ok())),
        Some(ConnTest::Failed(s)) => Some((s.clone(), theme::warn())),
    };
    let server = group(
        t("settings.server"),
        vec![
            row(t("settings.server.url"), Some(mode.into()), input_box(app.server_input.clone())),
            block(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(button("conn-test", t("settings.server.test")).on_click(cx.listener(|this, _, _, cx| this.test_connection(cx))))
                    .child(button("conn-apply", t("settings.server.apply")).on_click(cx.listener(|this, _, _, cx| this.apply_server(cx))))
                    .when_some(conn, |d, (s, c)| d.child(mono().text_size(theme::text_xs()).text_color(c).child(s))),
            ),
        ],
    );

    // ---- keys
    let mut key_rows: Vec<AnyElement> = vec![block(div().text_size(theme::text_sm()).text_color(theme::text_faint()).child(t("settings.keys.hint")))];
    key_rows.extend(
        app.key_inputs
            .iter()
            .map(|(name, input)| row(t(&format!("settings.keys.{name}")), None, input_box(input.clone()))),
    );
    let keys = group(t("settings.keys"), key_rows);

    // ---- visuals
    let weak = cx.entity().downgrade();
    let set_int: Pick = Rc::new(move |f, _, cx| {
        weak.update(cx, |this, cx| this.set_intensity(f, cx)).ok();
    });
    let intensity = app.config.visuals.intensity;
    let visuals = group(
        t("settings.visuals"),
        vec![
            row(
                t("settings.visuals.intensity"),
                None,
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(thin_bar("intensity", 180.0, intensity, set_int))
                    .child(mono().w(px(36.0)).flex().justify_end().text_size(theme::text_xs()).text_color(theme::text_dim()).child(format!("{:.0}%", intensity * 100.0))),
            ),
            row(
                t("settings.visuals.ascii_covers"),
                None,
                switch("ascii-covers", app.config.visuals.ascii_covers).on_click(cx.listener(|this, _, _, cx| {
                    this.config.visuals.ascii_covers = !this.config.visuals.ascii_covers;
                    this.save_config(cx);
                    cx.notify();
                })),
            ),
        ],
    );

    // ---- updates
    let u = &app.update;
    let status: (String, gpui::Hsla) = if u.checking {
        (format!("{} {}", app.spinner(), t("update.checking")), theme::text_dim())
    } else if let Some((done, total)) = u.installing {
        let pct = if total > 0 { done * 100 / total } else { 0 };
        (tf("update.downloading", &[("p", &pct)]).to_string(), theme::text())
    } else if u.installed {
        (t("update.installed").to_string(), theme::ok())
    } else if let Some(e) = &u.error {
        (tf("update.error", &[("e", e)]).to_string(), theme::warn())
    } else {
        match &u.status {
            None => (t("update.not_checked").to_string(), theme::text_dim()),
            Some(UpdateStatus::UpToDate { current }) => (tf("update.up_to_date", &[("v", current)]).to_string(), theme::ok()),
            Some(UpdateStatus::Skipped) => (t("update.skipped").to_string(), theme::text_dim()),
            Some(UpdateStatus::Available(i)) => (tf("update.badge", &[("v", &i.version.trim_start_matches('v'))]).to_string(), theme::ok()),
        }
    };
    let avail = app.available_update().cloned();
    let prerelease = app.config.updates.channel == "prerelease";
    let installable = avail
        .clone()
        .filter(|i| i.can_self_install && !app.update.installed && app.update.installing.is_none());
    let mut update_rows = vec![
        row(
            tf("settings.version", &[("v", &env!("CARGO_PKG_VERSION"))]),
            Some(status.0.into()),
            div()
                .flex()
                .gap(px(8.0))
                .child(button("upd-check", t("update.check")).on_click(cx.listener(|this, _, _, cx| this.check_updates(true, cx))))
                .when_some(installable, |d, info| {
                    d.child(primary_button("upd-install", t("update.install")).on_click(cx.listener(move |this, _, _, cx| this.install_update(info.clone(), cx))))
                })
                .when(app.update.installed, |d| {
                    d.child(primary_button("upd-restart", t("update.restart")).on_click(cx.listener(|this, _, _, cx| this.restart(cx))))
                }),
        ),
        row(
            t("update.auto"),
            None,
            switch("upd-auto", app.config.updates.auto_install).on_click(cx.listener(|this, _, _, cx| this.toggle_auto_update(cx))),
        ),
        row(
            t("update.channel"),
            None,
            div()
                .flex()
                .gap(px(6.0))
                .child(div().id("upd-stable").cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.set_update_channel(false, cx))).child(chip(t("update.channel.stable"), !prerelease)))
                .child(div().id("upd-pre").cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.set_update_channel(true, cx))).child(chip(t("update.channel.prerelease"), prerelease))),
        ),
    ];
    if let Some(i) = avail.filter(|i| !i.can_self_install) {
        update_rows.push(block(div().text_size(theme::text_sm()).text_color(theme::text_faint()).child(tf("update.package_manager", &[("url", &i.html_url)]))));
    }
    let updates = group(t("settings.updates"), update_rows);

    div()
        .id("settings-scroll")
        .size_full()
        .overflow_y_scroll()
        .px(px(32.0))
        .child(
            div()
                .w_full()
                .max_w(px(COLUMN_W))
                .mx_auto()
                .py(px(24.0))
                .flex()
                .flex_col()
                .gap(px(24.0))
                .child(heading(t("nav.settings")))
                .child(language)
                .child(sources)
                .child(priority)
                .child(server)
                .child(keys)
                .child(visuals)
                .child(updates),
        )
}
