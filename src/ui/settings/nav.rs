use super::*;

pub(super) fn nav(app: &mut Glyph, cx: &mut Context<Glyph>) -> gpui::Stateful<Div> {
    let q = app.settings_search.read(cx).value.trim().to_lowercase();
    let (current, current_sub) = (app.settings_page, app.settings_sub);
    let signed_in = app.signed_in().is_some();
    // (page, extra search words)
    let groups: [(SharedString, Vec<(SettingsPage, &'static str)>); 2] = [
        (
            t("sp.group.prefs"),
            vec![
                (SettingsPage::Playback, "sources source quelle youtube soundcloud priority radio quality qualität audio"),
                (SettingsPage::Audio, "equalizer eq bass treble höhen tiefen klang sound preset teilen share import export code datei file"),
                (SettingsPage::Appearance, "language sprache theme ascii cover glow"),
            ],
        ),
        (
            t("sp.group.app"),
            vec![
                (SettingsPage::Account, "account konto login sign in anmelden sync devices geräte handle passkey"),
                (SettingsPage::Profile, "profile profil privacy privatsphäre picture bild banner public öffentlich"),
                (SettingsPage::Server, "url api key keys schlüssel"),
                (SettingsPage::Integrations, "discord rich presence integration integrations status activity listening mcp ai assistant claude"),
                (SettingsPage::Storage, "cache download downloads speicher"),
                (SettingsPage::Migrate, "transfer übertragen umzug move import spotify youtube soundcloud deezer last.fm lastfm apple bandcamp tidal library bibliothek playlists"),
                (SettingsPage::Transfer, "backup export import share teilen code datei file"),
                (SettingsPage::About, "update version support hilfe help"),
            ],
        ),
    ];
    let hit = |text: &str| q.is_empty() || text.to_lowercase().contains(&q);

    // Single options matching the search (with their page / sub-tab).
    let found: Vec<(SettingsPage, u8, SharedString)> = if q.is_empty() {
        Vec::new()
    } else {
        OPTIONS
            .iter()
            .filter(|(p, _, _, _)| !matches!(p, SettingsPage::Account | SettingsPage::Profile) || signed_in)
            .filter(|(_, _, key, words)| hit(&t(key)) || words.contains(&q))
            .map(|(p, s, key, _)| (*p, *s, t(key)))
            .take(14)
            .collect()
    };

    // Search stays on top; only the list below it scrolls.
    let search = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .h(px(34.0))
        .mx(px(6.0))
        .mt(px(6.0))
        .mb(px(6.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .bg(white(0.06))
        .text_size(px(13.0))
        .child(icon("search", 14.0, white(0.5)))
        .child(div().flex_1().min_w_0().child(app.settings_search.clone()));
    let nav_scroll = crate::ui::widgets::handle_for("sp-nav");
    let mut col = div()
        .id("sp-nav")
        .track_scroll(&nav_scroll)
        .size_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .px(px(6.0))
        .pb(px(6.0));
    for (group, items) in groups {
        let mut visible = Vec::new();
        for (page, words) in items {
            if page == SettingsPage::Profile && !signed_in {
                continue;
            }
            let (label, _, icon_name) = page_info(page);
            let subs = subtabs(app, page);
            let sub_hit: Vec<bool> = subs.iter().map(|k| !q.is_empty() && hit(&t(k))).collect();
            let page_hit = hit(&label) || words.contains(&q) || found.iter().any(|f| f.0 == page);
            if page_hit || sub_hit.contains(&true) {
                visible.push((page, label, icon_name, subs, sub_hit));
            }
        }
        if visible.is_empty() {
            continue;
        }
        col = col.child(div().flex_none().px(px(10.0)).pt(px(10.0)).pb(px(4.0)).text_size(px(12.0)).text_color(white(0.45)).child(group));
        for (page, label, icon_name, subs, sub_hit) in visible {
            let on = page == current;
            let open = !subs.is_empty() && !app.settings_collapsed.contains(&(page as usize)) && (on || sub_hit.contains(&true));
            let has_subs = !subs.is_empty();
            // A page left for another keeps folding its sub-tabs away (motion to 0).
            let group_k = if on || sub_hit.contains(&true) { app.settings_group_k(page, open) } else { app.settings_group_k(page, false) };
            col = col.child(
                div()
                    .id(("sp-nav", page as usize))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.0))
                    .h(px(36.0))
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .text_size(px(14.0))
                    .when(on, |d| d.bg(white(0.08)).text_color(gpui::white()))
                    .when(!on, |d| {
                        let key = format!("sp-nav-{}", page as usize);
                        d.text_color(mix(white(0.75), gpui::white(), hover_k(&key))).relative().child(hover_fill(key, white(0.04), 8.0))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.settings_page != page {
                            this.set_settings_page(page, cx);
                            if has_subs { this.settings_group_motion.insert(page as usize, (std::time::Instant::now(), 0.0, 1.0)); }
                        } else if has_subs {
                            let from = this.settings_group_k(page, !this.settings_collapsed.contains(&(page as usize)));
                            if !this.settings_collapsed.insert(page as usize) { this.settings_collapsed.remove(&(page as usize)); }
                            let target = if this.settings_collapsed.contains(&(page as usize)) { 0.0 } else { 1.0 };
                            this.settings_group_motion.insert(page as usize, (std::time::Instant::now(), from, target));
                            cx.notify();
                        }
                    }))
                    .child(icon(icon_name, 16.0, if on { gpui::white() } else { white(0.7) }))
                    .child(div().flex_1().child(label))
                    .when(!subs.is_empty(), |d| d.child(icon(if open { "chevron-down" } else { "chevron-right" }, 14.0, white(0.45)))),
            );
            if has_subs && group_k > 0.001 {
                // Sub-tabs hang off a thin line under the page's icon.
                col = col.child(
                    div()
                        .flex_none()
                        .ml(px(18.0))
                        .h(px((subs.len() as f32 * 34.0 - 2.0) * group_k))
                        .overflow_hidden()
                        .opacity(group_k)
                        .my(px(2.0 * group_k))
                        .pl(px(8.0))
                        .border_l_1()
                        .border_color(white(0.10))
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .children(subs.iter().enumerate().map(|(i, key)| {
                            let sel = on && current_sub as usize == i;
                            let sub = i as u8;
                            div()
                                .id(("sp-sub", page as usize * 8 + i))
                                .flex_none()
                                .flex()
                                .items_center()
                                .h(px(32.0))
                                .px(px(10.0))
                                .rounded(px(7.0))
                                .cursor_pointer()
                                .text_size(px(13.5))
                                .when(sel, |d| d.bg(white(0.08)).text_color(gpui::white()).font_weight(FontWeight::MEDIUM))
                                .when(!sel, |d| {
                                    let key = format!("sp-sub-{}", page as usize * 8 + i);
                                    d.text_color(mix(white(0.6), gpui::white(), hover_k(&key))).relative().child(hover_fill(key, white(0.04), 7.0))
                                })
                                .on_click(cx.listener(move |this, _, _, cx| this.set_settings_tab(page, sub, cx)))
                                .child(t(key))
                        })),
                );
            }
        }
    }
    if !found.is_empty() {
        col = col.child(div().flex_none().px(px(10.0)).pt(px(14.0)).pb(px(4.0)).text_size(px(12.0)).text_color(white(0.45)).child(t("sp.group.options")));
        for (i, (page, sub, label)) in found.into_iter().enumerate() {
            let (page_label, _, _) = page_info(page);
            let crumb: SharedString = match subtabs(app, page).get(sub as usize) {
                Some(k) => format!("{page_label} › {}", t(k)).into(),
                None => page_label,
            };
            let l = label.clone();
            col = col.child(
                div()
                    .id(("sp-found", i))
                    .flex()
                    .flex_none()
                    .flex_col()
                    .justify_center()
                    .gap(px(1.0))
                    .py(px(6.0))
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .relative()
                    .child(hover_fill(format!("sp-found-{i}"), white(0.05), 8.0))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        flash_option(l.to_string());
                        this.set_settings_tab(page, sub, cx);
                    }))
                    .child(div().text_size(px(13.5)).text_color(white(0.9)).truncate().child(label))
                    .child(div().text_size(px(11.5)).text_color(white(0.45)).truncate().child(crumb)),
            );
        }
    }
    let wheel = crate::ui::widgets::smooth_wheel(&nav_scroll, false, cx);
    div()
        .id("sp-nav-wrap")
        .flex_none()
        .w(px(NAV_W))
        .h_full()
        .flex()
        .flex_col()
        .child(search)
        .child(div().relative().flex_1().min_h_0().child(col).child(wheel))
}
