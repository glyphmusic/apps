use super::*;

pub(super) fn about(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let u = &app.update;
    let status: (String, Hsla) = if u.checking {
        (format!("{} {}", app.spinner(), t("update.checking")), white(0.6))
    } else if let Some((done, total)) = u.installing {
        let pct = if total > 0 { done * 100 / total } else { 0 };
        (tf("update.downloading", &[("p", &pct)]).to_string(), gpui::white())
    } else if u.installed {
        let key = if cfg!(target_os = "android") { "update.android_confirm" } else { "update.installed" };
        (t(key).to_string(), theme::ok())
    } else if let Some(e) = &u.error {
        (tf("update.error", &[("e", e)]).to_string(), theme::warn())
    } else {
        match &u.status {
            None => (t("update.not_checked").to_string(), white(0.55)),
            Some(UpdateStatus::UpToDate { current }) => (tf("update.up_to_date", &[("v", current)]).to_string(), theme::ok()),
            Some(UpdateStatus::Skipped) => (t("update.skipped").to_string(), white(0.55)),
            Some(UpdateStatus::Available(i)) => (tf("update.badge", &[("v", &i.version.trim_start_matches('v'))]).to_string(), theme::ok()),
        }
    };
    let avail = app.available_update().cloned();
    let installable = avail
        .clone()
        // Android: stays, so a cancelled installer dialog can be opened again.
        .filter(|i| i.can_self_install && (!app.update.installed || cfg!(target_os = "android")) && app.update.installing.is_none());
    let prerelease = app.config.updates.channel == "prerelease";

    let card = surface()
        .mt(px(10.0))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(10.0))
        .py(px(20.0))
        // The note from the landing page: assembles, then dodges the mouse.
        .child(crate::ui::splash::note_logo(4.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(div().text_size(px(20.0)).font_weight(FontWeight::BOLD).text_color(gpui::white()).child("Glyph"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .px(px(7.0))
                        .h(px(20.0))
                        .rounded(px(6.0))
                        .bg(white(0.08))
                        .child(div().size(px(6.0)).rounded_full().bg(theme::ok()))
                        .child(mono().text_size(px(11.5)).text_color(white(0.85)).child(format!("v{}", env!("CARGO_PKG_VERSION")))),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .pt(px(6.0))
                .child(
                    div()
                        .id("upd-check")
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .h(px(34.0))
                        .px(px(14.0))
                        .rounded(px(8.0))
                        .bg(white(0.08))
                        .cursor_pointer()
                        .relative()
                        .child(hover_fill("update-check", white(0.05), 8.0))
                        .text_size(px(13.5))
                        .text_color(gpui::white())
                        .on_click(cx.listener(|this, _, _, cx| this.check_updates(true, cx)))
                        .child(icon("refresh-cw", 14.0, white(0.85)))
                        .child(t("update.check")),
                )
                .when_some(installable, |d, info| {
                    d.child(primary_button("upd-install", t("update.install")).on_click(cx.listener(move |this, _, _, cx| this.install_update(info.clone(), cx))))
                })
                .when(app.update.installed && !cfg!(target_os = "android"), |d| {
                    d.child(primary_button("upd-restart", t("update.restart")).on_click(cx.listener(|this, _, _, cx| this.restart(cx))))
                }),
        )
        .child(mono().text_size(px(11.5)).text_color(status.1).child(status.0))
        .child(
            div()
                .flex()
                .gap(px(8.0))
                .pt(px(14.0))
                .child(link_button("lnk-gh", "github", "GitHub", REPO_URL))
                .child(link_button("lnk-rel", "external-link", t("sp.link.releases"), RELEASES_URL))
                .child(link_button("lnk-sup", "life-buoy", t("sp.support"), ISSUES_URL)),
        );

    let channel = segmented(
        vec![
            (t("update.channel.stable"), !prerelease, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_update_channel(false, cx))),
            (t("update.channel.prerelease"), prerelease, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_update_channel(true, cx))),
        ],
        cx,
        "seg-channel",
    );
    let mut items = vec![
        row(
            t("update.auto"),
            Some(t("sp.about.auto.desc")),
            switch("upd-auto", app.config.updates.auto_install).on_click(cx.listener(|this, _, _, cx| this.toggle_auto_update(cx))),
        ),
        row(t("update.channel"), Some(t("sp.about.channel.desc")), channel),
    ];
    if let Some(i) = avail.filter(|i| !i.can_self_install) {
        items.push(div().py(px(12.0)).text_size(px(12.5)).text_color(white(0.55)).child(tf("update.package_manager", &[("url", &i.html_url)])).into_any_element());
    }
    div().flex().flex_col().gap(px(10.0)).child(card).child(rows(items))
}

/// Key cap, like a keyboard key.
pub(super) fn key_cap(k: &str) -> Div {
    mono()
        .flex_none()
        .min_w(px(24.0))
        .h(px(24.0))
        .px(px(7.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(white(0.07))
        .border_1()
        .border_color(white(0.10))
        .text_size(px(11.5))
        .text_color(white(0.85))
        .child(k.to_string())
}

/// Keys of one shortcut: `+` joins keys pressed together, `/` separates
/// alternatives (`Ctrl+1 / … / 9`).
pub(super) fn key_combo(keys: &str) -> Div {
    let mut d = div().flex().items_center().gap(px(4.0));
    for (n, alt) in keys.split(" / ").enumerate() {
        if n > 0 {
            d = d.child(div().text_size(px(12.0)).text_color(white(0.4)).child("/"));
        }
        for (m, k) in alt.split('+').enumerate() {
            if m > 0 {
                d = d.child(div().text_size(px(12.0)).text_color(white(0.4)).child("+"));
            }
            d = d.child(key_cap(k));
        }
    }
    d
}

/// Settings -> About -> Shortcuts: every keyboard shortcut, grouped.
pub(super) fn shortcuts() -> Div {
    let m = if cfg!(target_os = "macos") { "\u{2318}" } else { "Ctrl" };
    let alt = if cfg!(target_os = "macos") { "\u{2325}" } else { "Alt" };
    let groups: [(&str, Vec<(&str, String)>); 3] = [
        (
            "sp.keys.group.playback",
            vec![
                ("sp.keys.play", "Space".into()),
                ("sp.keys.seek", "\u{2190} / \u{2192}".into()),
                ("sp.keys.prev", format!("{m}+\u{2190}")),
                ("sp.keys.next", format!("{m}+\u{2192}")),
                ("sp.keys.loop_start", format!("{alt}+1")),
                ("sp.keys.loop_end", format!("{alt}+2")),
                ("sp.keys.loop_clear", format!("{alt}+0")),
            ],
        ),
        (
            "sp.keys.group.nav",
            vec![
                ("sp.keys.search", format!("{m}+K")),
                ("sp.keys.lyrics", format!("{m}+L")),
                ("sp.keys.sidebar", format!("{m}+1 / 9")),
                ("sp.keys.settings", format!("{m}+0")),
                ("sp.keys.close", "Esc".into()),
            ],
        ),
        ("sp.keys.group.app", vec![("sp.keys.quit", format!("{m}+Q"))]),
    ];
    let mut col = div().flex().flex_col().gap(px(14.0));
    for (title, items) in groups {
        col = col.child(
            div()
                .flex()
                .flex_col()
                .child(section_label(t(title)))
                .child(rows(items.into_iter().map(|(label, keys)| row(t(label), None, key_combo(&keys))).collect())),
        );
    }
    col
}

pub(super) fn support(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let diag = app.diagnostics();
    let data_dir = glyph_core::paths::data_dir();
    rows(vec![
        row(
            t("sp.support.issue"),
            Some(t("sp.support.issue.desc")),
            link_button("sup-issue", "external-link", t("sp.support.open"), ISSUES_URL),
        ),
        row(
            t("sp.support.diag"),
            Some(t("sp.support.diag.desc")),
            button("sup-diag", t("sp.support.copy")).on_click(cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(diag.clone()));
                this.toast(t("sp.support.copied").to_string());
                cx.notify();
            })),
        ),
        row(
            t("sp.docs"),
            Some(t("sp.support.docs.desc")),
            link_button("sup-docs", "book-open", t("sp.support.open"), DOCS_URL),
        ),
        row(
            t("sp.support.data"),
            Some(t("sp.support.data.desc")),
            button("sup-data", t("sp.storage.open")).on_click(move |_, _, cx| cx.reveal_path(&data_dir)),
        ),
    ])
}
