use super::*;
use glyph_core::plugins::{self, Invalid, Listing, Manifest};

/// Settings -> Plugins. Sub-tab 0: installed, 1: browse the catalog (2+:
/// plugins' own pages, see `plugin_ui`); a plugin's detail view replaces
/// either (`app.plugins.detail`).
pub(super) fn plugins_page(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    if let Some(id) = app.plugins.detail.clone() {
        return detail(app, &id, cx);
    }
    if app.settings_sub == 1 {
        return browse(app, cx);
    }
    let updates = app.plugin_updates();
    let mut items: Vec<AnyElement> = Vec::new();
    let auto = app.config.plugins.auto_update;
    if !app.plugins.installed.is_empty() {
        items.push(row(
            t("plugins.auto_update"),
            Some(t("plugins.auto_update.desc")),
            switch("plugins-auto-update", auto).on_click(cx.listener(move |this, _, _, cx| this.set_plugin_auto_update(!auto, cx))),
        ));
    }
    if updates.len() > 1 {
        let all = updates.clone();
        items.push(row(
            tf("plugins.updates", &[("n", &updates.len().to_string())]),
            None,
            button("plugins-update-all", t("plugins.update_all")).on_click(cx.listener(move |this, _, _, cx| {
                for l in all.clone() {
                    this.install_plugin(l, cx);
                }
            })),
        ));
    }
    let cur_theme = app.config.visuals.theme.clone();
    for (i, p) in app.plugins.installed.clone().into_iter().enumerate() {
        let id = p.manifest.id.clone();
        let mut controls = div().flex().items_center().gap(px(8.0));
        if app.plugins.busy.contains(&id) {
            controls = controls.child(spin(app));
        } else {
            if let Some(l) = updates.iter().find(|l| l.manifest.id == id).cloned() {
                controls = controls.child(
                    primary_button(("plugin-update", i), tf("plugins.update_to", &[("v", &l.manifest.version)]))
                        .on_click(cx.listener(move |this, _, _, cx| this.install_plugin(l.clone(), cx))),
                );
            }
            if let Some(el) = use_control(&p.manifest, &cur_theme, ("plugin-use", i), cx) {
                controls = controls.child(el);
            }
            if app.plugins.pages.contains(&id) {
                let page = id.clone();
                controls = controls.child(
                    with_tip(icon_button(("plugin-settings", i), "sliders-horizontal", 15.0, white(0.7)), Some(t("plugins.settings")))
                        .on_click(cx.listener(move |this, _, _, cx| this.open_plugin_page(&page, cx))),
                );
            }
            if p.manifest.kind != "theme" {
                let on = app.plugin_enabled(&id);
                let pid = id.clone();
                controls = controls.child(
                    with_tip(div().id(("plugin-on-tip", i)).child(switch(("plugin-on", i), on)), Some(if on { t("plugins.turn_off") } else { t("plugins.turn_on") }))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_plugin_enabled(&pid, !on, cx))),
                );
            }
            let gone = id.clone();
            controls = controls.child(
                with_tip(icon_button(("plugin-remove", i), "trash-2", 15.0, white(0.7)), Some(t("plugins.remove")))
                    .on_click(cx.listener(move |this, _, _, cx| this.uninstall_plugin(gone.clone(), cx))),
            );
        }
        let status = meta_line(&p.manifest, p.origin.checked || p.origin.is_local(), None);
        let status = if p.origin.is_local() { SharedString::from(format!("{status} · {}", t("plugins.local"))) } else { status };
        let status = if app.plugins.rt.missing_credentials.contains_key(&id) {
            SharedString::from(format!("{} · {status}", t("plugins.setup.needed")))
        } else { status };
        let status = match app.plugins.rt.stopped.get(&id) {
            Some(_) if app.plugin_enabled(&id) => SharedString::from(format!("{} · {status}", t("plugins.stopped_short"))),
            _ if !app.plugin_enabled(&id) && p.manifest.kind != "theme" => SharedString::from(format!("{} · {status}", t("plugins.off"))),
            _ => status,
        };
        items.push(plugin_row(&p.manifest, status, ("plugin-open-i", i), controls, cx));
    }
    let mut col = div().flex().flex_col();
    if app.plugins.installed.is_empty() {
        col = col.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(10.0))
                .py(px(36.0))
                .child(icon("puzzle", 28.0, white(0.35)))
                .child(div().text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(t("plugins.empty")))
                .child(div().text_size(px(12.5)).text_color(white(0.55)).child(t("plugins.empty.desc")))
                .child(
                    primary_button("plugins-browse", t("plugins.browse"))
                        .on_click(cx.listener(|this, _, _, cx| this.set_settings_tab(SettingsPage::Plugins, 1, cx))),
                ),
        );
    } else {
        col = col.child(rows(items));
    }
    col.child(guide_row()).child(maker_row(cx))
}

/// For plugin makers: the plugins folder (drop a folder with a
/// glyph-plugin.json in to try it) and reloading it without a restart.
fn maker_row(cx: &mut Context<Glyph>) -> AnyElement {
    let dir = plugins::plugins_dir();
    row(
        t("plugins.maker"),
        Some(t("plugins.maker.desc")),
        div()
            .flex()
            .gap(px(8.0))
            .child(button("plugins-folder", t("sp.storage.open")).on_click(move |_, _, cx| {
                let _ = std::fs::create_dir_all(&dir);
                cx.reveal_path(&dir);
            }))
            .child(button("plugins-reload", t("plugins.reload")).on_click(cx.listener(|this, _, _, cx| this.reload_plugins(cx)))),
    )
}

fn browse(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let mut col = div().flex().flex_col().child(row_status(
        t("plugins.catalog"),
        Some(catalog_status(app)),
        button("plugins-refresh", t("plugins.refresh"))
            .on_click(cx.listener(|this, _, _, cx| this.refresh_plugin_catalog(true, cx))),
    ));
    col = col.child(risk_note());
    let Some(catalog) = app.plugins.catalog.clone() else {
        return col.child(skeleton_rows("plugins-skel", 4));
    };
    let query = app.plugins_search.read(cx).value.trim().to_string();
    let sort = app.config.plugins.sort;
    let pick = |s: plugins::Sort| -> Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> { Rc::new(move |this: &mut Glyph, cx: &mut Context<Glyph>| this.set_plugin_sort(s, cx)) };
    let sorts = segmented(
        vec![
            (t("plugins.sort.stars"), sort == plugins::Sort::Stars, pick(plugins::Sort::Stars)),
            (t("plugins.sort.updated"), sort == plugins::Sort::Updated, pick(plugins::Sort::Updated)),
            (t("plugins.sort.name"), sort == plugins::Sort::Name, pick(plugins::Sort::Name)),
        ],
        cx,
        "plugins-sort",
    );
    // Search field (settings nav search look) and the order, one line.
    col = col.child(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.0))
            .pt(px(16.0))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(200.0))
                    .items_center()
                    .gap(px(8.0))
                    .h(px(34.0))
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .bg(white(0.06))
                    .text_size(px(13.0))
                    .child(icon("search", 14.0, white(0.5)))
                    .child(div().flex_1().min_w_0().child(app.plugins_search.clone())),
            )
            .child(sorts),
    );
    let checked_hits: Vec<Listing> = plugins::filter_sort(&catalog.checked, &query, sort).into_iter().cloned().collect();
    let community_hits: Vec<Listing> = plugins::filter_sort(&catalog.community, &query, sort).into_iter().cloned().collect();
    if !query.is_empty() && checked_hits.is_empty() && community_hits.is_empty() {
        return col.child(empty_line(tf("plugins.no_match", &[("q", &query)]))).child(guide_row());
    }
    let section = |title: SharedString, tip: SharedString, tip_id: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(section_label(title))
            .child(div().pt(px(16.0)).child(info_tip(tip_id, tip)))
    };
    col = col.child(section(t("plugins.checked"), t("plugins.checked.desc"), "info-plugins-checked"));
    col = col.child(if checked_hits.is_empty() {
        empty_line(if query.is_empty() { t("plugins.none") } else { t("plugins.none_match") })
    } else {
        rows(checked_hits.iter().enumerate().map(|(i, l)| listing_row(app, l, i, cx)).collect()).into_any_element()
    });
    col = col.child(section(t("plugins.community"), t("plugins.community.desc"), "info-plugins-community"));
    col = col.child(if community_hits.is_empty() {
        empty_line(if query.is_empty() { t("plugins.none_community") } else { t("plugins.none_match") })
    } else {
        let off = checked_hits.len();
        rows(community_hits.iter().enumerate().map(|(i, l)| listing_row(app, l, off + i, cx)).collect()).into_any_element()
    });
    col.child(guide_row())
}

/// Loading / error / source line of the catalog.
fn catalog_status(app: &Glyph) -> SharedString {
    let p = &app.plugins;
    if p.loading {
        format!("{} {}", app.spinner(), t("plugins.loading")).into()
    } else if let Some(e) = &p.error {
        tf("plugins.error", &[("e", e)])
    } else if p.catalog.as_ref().is_some_and(|c| c.partial) {
        t("plugins.partial")
    } else {
        t("plugins.source")
    }
}

/// One catalog entry: open on GitHub + its install state.
fn listing_row(app: &Glyph, l: &Listing, i: usize, cx: &mut Context<Glyph>) -> AnyElement {
    let url = l.repo_url();
    let controls = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(
            with_tip(icon_button(("plugin-repo", i), "github", 15.0, white(0.7)), Some(t("plugins.repo")))
                .on_click(move |_, _, cx| cx.open_url(&url)),
        )
        .child(action_control(app, l, ("plugin-get", i), cx));
    let status = meta_line(&l.manifest, l.checked, l.stars);
    let status = match app.plugins.failed.get(&l.manifest.id) {
        Some(e) => SharedString::from(format!("{status} · {e}")),
        None => status,
    };
    plugin_row(&l.manifest, status, ("plugin-open-b", i), controls, cx)
}

/// Install / update / installed / why not, for a catalog listing.
fn action_control(app: &Glyph, l: &Listing, id: (&'static str, usize), cx: &mut Context<Glyph>) -> AnyElement {
    let pid = &l.manifest.id;
    let installed = app.plugins.installed.iter().find(|p| &p.manifest.id == pid);
    let foreign = installed.is_some_and(|p| !p.origin.repo.eq_ignore_ascii_case(&l.repo));
    let newer = installed.is_some_and(|p| !foreign && plugins::is_newer(&l.manifest.version, &p.manifest.version));
    match l.installable() {
        _ if app.plugins.busy.contains(pid) => spin(app).into_any_element(),
        Err(Invalid::NeedsNewerApp) => kind_chip(t("plugins.needs_update")).into_any_element(),
        Err(Invalid::Bad(_)) => kind_chip(t("plugins.broken")).into_any_element(),
        Ok(()) if foreign => kind_chip(t("plugins.id_taken")).into_any_element(),
        Ok(()) if newer => {
            let l = l.clone();
            primary_button(id, tf("plugins.update_to", &[("v", &l.manifest.version)]))
                .on_click(cx.listener(move |this, _, _, cx| this.install_plugin(l.clone(), cx)))
                .into_any_element()
        }
        Ok(()) if installed.is_some() => div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(px(12.5))
            .text_color(white(0.6))
            .child(icon("check", 14.0, white(0.6)))
            .child(t("plugins.installed"))
            .into_any_element(),
        Ok(()) => {
            let l = l.clone();
            button(id, t("plugins.install"))
                .on_click(cx.listener(move |this, _, _, cx| this.install_plugin(l.clone(), cx)))
                .into_any_element()
        }
    }
}

/// "Use" / "In use" for an installed plugin with a theme; `None` for others.
fn use_control(m: &Manifest, cur_theme: &str, id: (&'static str, usize), cx: &mut Context<Glyph>) -> Option<AnyElement> {
    m.theme.as_ref()?;
    let theme_id = plugins::theme_id(&m.id);
    Some(if cur_theme == theme_id {
        kind_chip(t("plugins.in_use")).into_any_element()
    } else {
        button(id, t("plugins.use"))
            .on_click(cx.listener(move |this, _, _, cx| this.set_theme(&theme_id, cx)))
            .into_any_element()
    })
}

fn spin(app: &Glyph) -> Div {
    mono().text_size(px(12.0)).text_color(white(0.6)).child(app.spinner().to_string())
}

// ------------------------------------------------------------------ detail

/// One plugin: preview, description, actions and every manifest field.
/// Opened by `glyph://plugin/<id>` (glyph.my/plugins) or a click on a name.
fn detail(app: &mut Glyph, id: &str, cx: &mut Context<Glyph>) -> Div {
    let listing = app.plugins.catalog.as_ref().and_then(|c| c.find(id)).cloned();
    let installed = app.plugins.installed.iter().find(|p| p.manifest.id == id).cloned();
    let back = div()
        .id("plugin-back")
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(28.0))
        .px(px(8.0))
        .ml(px(-8.0))
        .rounded(px(7.0))
        .cursor_pointer()
        .relative()
        .child(hover_fill("plugin-back", white(0.05), 7.0))
        .text_size(px(13.0))
        .text_color(white(0.7))
        .child(icon("chevron-left", 14.0, white(0.7)))
        .child(t("plugins.back"))
        .on_click(cx.listener(|this, _, _, cx| this.close_plugin_detail(cx)));
    let col = div().flex().flex_col().child(div().flex().child(back));

    let (manifest, checked, repo, path, rev, stars) = match (&listing, &installed) {
        (Some(l), _) => (l.manifest.clone(), l.checked, l.repo.clone(), l.path.clone(), l.rev.clone(), l.stars),
        (None, Some(p)) => (p.manifest.clone(), p.origin.checked, p.origin.repo.clone(), p.origin.path.clone(), p.origin.rev.clone(), None),
        (None, None) => {
            // Not in the catalog (yet): still loading, or unknown.
            if app.plugins.loading || app.plugins.catalog.is_none() {
                return col.child(div().pt(px(14.0)).child(skeleton_rows("plugin-detail-skel", 3)));
            }
            return col.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.0))
                    .py(px(36.0))
                    .child(icon("puzzle", 28.0, white(0.35)))
                    .child(div().text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(t("plugins.not_found")))
                    .child(div().text_size(px(12.5)).text_color(white(0.55)).child(tf("plugins.not_found.desc", &[("id", &id)])))
                    .child(button("plugin-detail-refresh", t("plugins.refresh")).on_click(cx.listener(|this, _, _, cx| this.refresh_plugin_catalog(true, cx)))),
            );
        }
    };

    // Header (preview 64, name, meta, chips), like a bigger plugin_row.
    let mut chips = div().flex().items_center().gap(px(6.0)).child(kind_chip(if checked { t("plugins.checked") } else { t("plugins.community") }));
    if manifest.kind == "theme" {
        chips = chips.child(kind_chip(t("plugins.kind.theme")));
    } else {
        chips = chips.child(kind_chip(t("plugins.kind.extension")));
    }
    if !manifest.data_only() {
        chips = chips.child(kind_chip(t("plugins.kind.code")));
    }
    let mut sub = vec![format!("v{}", manifest.version)];
    if !manifest.author.trim().is_empty() {
        sub.push(manifest.author.clone());
    }
    if let Some(s) = stars {
        sub.push(format!("★ {}", compact_count(s)));
    }
    let header = div()
        .flex()
        .items_center()
        .gap(px(16.0))
        .pt(px(14.0))
        .child(preview(&manifest, 64.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(div().truncate().text_size(px(20.0)).font_weight(FontWeight::SEMIBOLD).text_color(gpui::white()).child(manifest.name.clone()))
                .child(div().truncate().text_size(px(12.5)).text_color(white(0.55)).child(sub.join(" · ")))
                .child(chips),
        );

    // Actions: install / update (catalog), use / remove (installed), GitHub.
    let cur_theme = app.config.visuals.theme.clone();
    let mut actions = div().flex().flex_wrap().items_center().gap(px(8.0)).pt(px(16.0));
    if let Some(l) = &listing {
        actions = actions.child(action_control(app, l, ("plugin-detail-get", 0), cx));
    }
    if let Some(p) = &installed {
        if !app.plugins.busy.contains(id) {
            if let Some(el) = use_control(&p.manifest, &cur_theme, ("plugin-detail-use", 0), cx) {
                actions = actions.child(el);
            }
            if app.plugins.pages.iter().any(|x| x == id) {
                let page = id.to_string();
                actions = actions.child(button("plugin-detail-settings", t("plugins.settings")).on_click(cx.listener(move |this, _, _, cx| this.open_plugin_page(&page, cx))));
            }
            if p.manifest.kind != "theme" {
                let on = app.plugin_enabled(id);
                let pid = id.to_string();
                actions = actions.child(
                    button("plugin-detail-toggle", if on { t("plugins.turn_off") } else { t("plugins.turn_on") })
                        .on_click(cx.listener(move |this, _, _, cx| this.set_plugin_enabled(&pid, !on, cx))),
                );
            }
            let gone = id.to_string();
            actions = actions.child(
                button("plugin-detail-remove", t("plugins.remove")).on_click(cx.listener(move |this, _, _, cx| this.uninstall_plugin(gone.clone(), cx))),
            );
        }
    }
    let url = match &listing {
        Some(l) => l.repo_url(),
        None => format!("https://github.com/{repo}"),
    };
    actions = actions.child(
        button("plugin-detail-repo", t("plugins.repo")).child(icon("external-link", 13.0, white(0.7))).on_click(move |_, _, cx| cx.open_url(&url)),
    );

    let mut col = col.child(header);
    if !manifest.description.trim().is_empty() {
        col = col.child(div().pt(px(14.0)).text_size(px(13.5)).line_height(px(20.0)).text_color(white(0.72)).child(manifest.description.clone()));
    }
    let credentials: Vec<String> = manifest.settings.iter().flat_map(|s| &s.items).filter(|i| i.required)
        .map(|i| match &i.credential {
            Some(c) => format!("{} (https://{}{}, {}: {})", i.label.get(crate::i18n::lang().code()), c.host, c.path, c.location, c.name),
            None => i.label.get(crate::i18n::lang().code()).to_string(),
        }).collect();
    if !credentials.is_empty() {
        col = col.child(div().pt(px(12.0)).text_size(px(12.5)).line_height(px(18.0)).text_color(white(0.7))
            .child(tf("plugins.setup.requires", &[("items", &credentials.join(", "))])));
    }
    if installed.is_some() && app.plugins.rt.missing_credentials.contains_key(id) {
        col = col.child(div().pt(px(10.0)).text_size(px(12.5)).text_color(theme::warn()).child(t("plugins.setup.missing")));
    }
    let confirming = app.plugins.confirm.as_ref().is_some_and(|l| l.manifest.id == id);
    // What the warning lists: everything for a first install, only what is
    // new for an update.
    let asks = match &installed {
        Some(p) => manifest.new_permissions(&p.manifest),
        None => all_permissions(&manifest),
    };
    col = col.child(if confirming { warning(app, &manifest, checked, installed.is_some(), &asks, cx).into_any_element() } else { actions.into_any_element() });
    if !confirming {
        col = col.child(risk_note());
    }
    if let Some(e) = app.plugins.failed.get(id) {
        col = col.child(div().pt(px(10.0)).text_size(px(12.5)).text_color(theme::warn()).child(e.clone()));
    }
    if let Some(why) = app.plugins.rt.stopped.get(id).filter(|_| app.plugin_enabled(id)) {
        col = col.child(div().pt(px(10.0)).text_size(px(12.5)).text_color(theme::warn()).child(tf("plugins.stopped", &[("e", why)])));
    }

    // Every field of the manifest.
    let value = |s: String| div().text_size(px(13.0)).text_color(white(0.75)).child(s);
    let dash = || "–".to_string();
    let source = if path.is_empty() { repo.clone() } else { format!("{repo}/{path}") };
    let pinned = if rev.len() == 40 { format!("{source} @ {}", &rev[..7]) } else { source };
    let mut info = vec![
        row(t("plugins.version"), None, value(manifest.version.clone())),
        row(t("plugins.author"), None, value(if manifest.author.trim().is_empty() { dash() } else { manifest.author.clone() })),
        row(t("plugins.license"), None, value(if manifest.license.trim().is_empty() { dash() } else { manifest.license.clone() })),
        row(t("plugins.min_app"), None, value(manifest.min_app.clone().unwrap_or_else(dash))),
        row(t("plugins.repo_source"), None, mono().text_size(px(12.0)).text_color(white(0.75)).child(pinned)),
    ];
    if let Some(s) = stars {
        info.push(row(t("plugins.stars"), Some(t("plugins.stars.desc")), value(s.to_string())));
    }
    if let Some(day) = listing.as_ref().and_then(|l| l.updated.as_deref()).and_then(|u| u.get(..10)) {
        info.push(row(t("plugins.updated"), None, value(day.to_string())));
    }
    if let Some(p) = installed.as_ref().filter(|p| p.manifest.version != manifest.version) {
        info.insert(1, row(t("plugins.installed_version"), None, value(p.manifest.version.clone())));
    }
    col = col.child(section_label(t("plugins.details"))).child(rows(info));
    // What the plugin does to Glyph and what its code may use.
    let mut does = Vec::new();
    for p in all_permissions(&manifest) {
        does.push(row(permission_label(&p, &manifest), None, div()));
    }
    let lang = crate::i18n::lang().code();
    let sections: Vec<String> = manifest
        .settings
        .iter()
        .map(|s| {
            let page = if s.page == "own" { t("plugins.does.own_page").to_string() } else { s.page.clone() };
            let title = s.title.get(lang);
            if title.is_empty() { page } else { format!("{title} ({page})") }
        })
        .collect();
    if !sections.is_empty() {
        does.push(row(t("plugins.does.settings"), None, value(sections.join(", "))));
    }
    if !manifest.menus.is_empty() {
        let names: Vec<String> = manifest.menus.iter().map(|m| m.label.get(lang).to_string()).collect();
        does.push(row(t("plugins.does.menus"), None, value(names.join(", "))));
    }
    if !manifest.strings.is_empty() {
        let n: usize = manifest.strings.values().map(|m| m.len()).max().unwrap_or(0);
        does.push(row(t("plugins.does.strings"), None, value(n.to_string())));
    }
    if !manifest.hide.is_empty() {
        let names: Vec<String> = manifest.hide.iter().map(|k| t(k).to_string()).collect();
        does.push(row(t("plugins.does.hide"), None, value(names.join(", "))));
    }
    if !does.is_empty() {
        col = col.child(section_label(t("plugins.does"))).child(rows(does));
    }
    // Plugin makers: what their code logged.
    if installed.as_ref().is_some_and(|p| p.origin.is_local()) {
        let lines = app.plugin_logs(id);
        if !lines.is_empty() {
            let shown: Vec<String> = lines.iter().rev().take(14).rev().cloned().collect();
            col = col.child(section_label(t("plugins.log"))).child(
                surface().child(mono().text_size(px(11.5)).line_height(px(17.0)).text_color(white(0.7)).flex().flex_col().children(shown.into_iter().map(|l| div().child(l)))),
            );
        }
    }
    if let Some(c) = manifest.theme_colors() {
        let swatch = |hex: u32| {
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(div().size(px(14.0)).rounded_full().bg(gpui::rgb(hex)).border_1().border_color(white(0.12)))
                .child(mono().text_size(px(12.0)).text_color(white(0.75)).child(format!("#{hex:06x}")))
        };
        let mut colours = vec![row(t("plugins.accent"), None, swatch(c.accent))];
        if let Some(g) = c.glow {
            colours.push(row(t("plugins.glow"), Some(t("plugins.glow.desc")), div().flex().gap(px(12.0)).children(g.map(swatch))));
        }
        colours.push(row(t("plugins.base"), None, swatch(c.base)));
        col = col.child(section_label(t("plugins.colours"))).child(rows(colours));
    }
    col
}

// ------------------------------------------------------------------ pieces

/// 1234 -> "1.2k", 1200000 -> "1.2M" (star counts in lists).
fn compact_count(n: u32) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f32 / 1_000.0).replace(".0k", "k"),
        _ => format!("{:.1}M", n as f32 / 1_000_000.0).replace(".0M", "M"),
    }
}

/// "v1.0.0 · author · ★ 12 · description".
fn meta_line(m: &Manifest, checked: bool, stars: Option<u32>) -> SharedString {
    let mut parts = vec![format!("v{}", m.version)];
    if !m.author.trim().is_empty() {
        parts.push(m.author.clone());
    }
    if let Some(s) = stars {
        parts.push(format!("★ {}", compact_count(s)));
    }
    if !checked {
        parts.push(t("plugins.unchecked").to_string());
    }
    if !m.description.trim().is_empty() {
        parts.push(m.description.clone());
    }
    parts.join(" · ").into()
}

/// The plugin's picture: a theme's swatch (glow ring, accent inside, as in
/// Appearance) or the puzzle icon for other kinds.
fn preview(m: &Manifest, size: f32) -> Div {
    match m.theme_colors() {
        Some(c) => {
            let accent: Hsla = gpui::rgb(c.accent).into();
            let ring: Hsla = c.glow.map_or(gpui::rgb(0x1d5cff).into(), |g| gpui::rgb(g[1]).into());
            div()
                .flex_none()
                .size(px(size))
                .rounded_full()
                .bg(ring)
                .flex()
                .items_center()
                .justify_center()
                .child(div().size(px(size * 0.57)).rounded_full().bg(accent))
        }
        None => div()
            .flex_none()
            .size(px(size))
            .rounded(px(size * 0.28))
            .bg(white(0.07))
            .flex()
            .items_center()
            .justify_center()
            .child(icon("puzzle", size * 0.54, white(0.7))),
    }
}

/// Settings row with the plugin's preview in front; preview and name open
/// the detail view.
fn plugin_row(m: &Manifest, status: SharedString, open: (&'static str, usize), controls: impl IntoElement, cx: &mut Context<Glyph>) -> AnyElement {
    let id = m.id.clone();
    let key = format!("{}-{}", open.0, open.1);
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .py(px(14.0))
        .child(
            div()
                .id(open)
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(14.0))
                .mx(px(-8.0))
                .px(px(8.0))
                .py(px(4.0))
                .rounded(px(8.0))
                .cursor_pointer()
                .relative()
                .child(hover_fill(key, white(0.04), 8.0))
                .on_click(cx.listener(move |this, _, _, cx| this.show_plugin(id.clone(), cx)))
                .child(preview(m, 28.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(div().truncate().text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(m.name.clone()))
                        .child(div().truncate().text_size(px(12.5)).text_color(white(0.55)).child(status)),
                ),
        )
        .child(div().flex_none().child(controls))
        .into_any_element()
}

/// Always on the catalog and detail views: plugins are other people's work.
fn risk_note() -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap(px(8.0))
        .pt(px(12.0))
        .child(div().pt(px(1.0)).child(icon("info", 14.0, white(0.45))))
        .child(div().flex_1().min_w_0().text_size(px(12.5)).line_height(px(18.0)).text_color(white(0.5)).child(t("plugins.risk_note")))
        .into_any_element()
}

/// Permissions of a plugin as the warning lists them: `code`, then each
/// permission (`network` once per host, as `network:<host>`).
fn all_permissions(m: &Manifest) -> Vec<String> {
    let mut out = Vec::new();
    if !m.data_only() {
        out.push("code".to_string());
    }
    out.extend(m.permissions.iter().filter(|p| *p != "network").cloned());
    out.extend(m.network.iter().map(|h| format!("network:{h}")));
    out
}

/// "May control playback", "May contact api.example.com", ...
fn permission_label(p: &str, _m: &Manifest) -> SharedString {
    match p.strip_prefix("network:") {
        Some(host) => tf("plugins.perm.network", &[("host", &host)]),
        None => t(&format!("plugins.perm.{p}")),
    }
}

/// Shown before a first install (and before an update that asks for more),
/// in place of the actions: who made the plugin, that Glyph cannot vouch
/// for it, what its code may do, and that installing is at the user's own
/// risk.
fn warning(app: &Glyph, m: &Manifest, checked: bool, update: bool, asks: &[String], cx: &mut Context<Glyph>) -> Div {
    let skip = app.plugins.confirm_skip;
    let author = if m.author.trim().is_empty() { t("plugins.warn.someone").to_string() } else { m.author.clone() };
    let perms = (!asks.is_empty()).then(|| {
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(div().text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(if update { t("plugins.warn.more") } else { t("plugins.warn.may") }))
            .children(asks.iter().map(|p| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(icon("check", 13.0, white(0.6)))
                    .child(div().text_size(px(13.0)).text_color(white(0.75)).child(permission_label(p, m)))
            }))
    });
    // Code plugins ask every time; "don't show again" is for pure data only.
    let can_skip = m.data_only() && !update;
    div()
        .mt(px(16.0))
        .rounded(px(10.0))
        .bg(white(0.035))
        .p(px(18.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(icon("info", 16.0, theme::warn()))
                .child(div().text_size(px(14.0)).font_weight(FontWeight::SEMIBOLD).text_color(gpui::white()).child(t("plugins.warn.title"))),
        )
        .child(div().text_size(px(13.0)).line_height(px(19.0)).text_color(white(0.72)).child(tf("plugins.warn.body", &[("author", &author)])))
        .child(div().text_size(px(13.0)).line_height(px(19.0)).text_color(white(0.72)).child(if checked { t("plugins.warn.listed") } else { t("plugins.warn.community") }))
        .children(perms)
        .when(can_skip, |d| d.child(
            div()
                .id("plugin-warn-skip")
                .flex()
                .items_center()
                .gap(px(10.0))
                .pt(px(4.0))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.plugins.confirm_skip = !this.plugins.confirm_skip;
                    cx.notify();
                }))
                .child(switch("plugin-warn-skip-switch", skip))
                .child(div().text_size(px(13.0)).text_color(white(0.75)).child(t("plugins.warn.skip"))),
        ))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.0))
                .pt(px(6.0))
                .child(primary_button("plugin-warn-ok", t("plugins.warn.accept")).on_click(cx.listener(|this, _, _, cx| this.accept_plugin_warning(cx))))
                .child(button("plugin-warn-cancel", t("plugins.warn.cancel")).on_click(cx.listener(|this, _, _, cx| this.cancel_plugin_warning(cx)))),
        )
}

fn empty_line(text: SharedString) -> AnyElement {
    div().py(px(14.0)).text_size(px(12.5)).text_color(white(0.5)).child(text).into_any_element()
}

/// "Make your own": how to publish on GitHub, and the web catalog.
fn guide_row() -> AnyElement {
    div()
        .mt(px(10.0))
        .pt(px(4.0))
        .border_t_1()
        .border_color(white(0.06))
        .child(row(
            t("plugins.publish"),
            Some(t("plugins.publish.desc")),
            div()
                .flex()
                .gap(px(8.0))
                .child(link_button("plugins-web", "globe", t("plugins.web"), plugins::WEB_URL))
                .child(link_button("plugins-guide", "book-open", t("plugins.publish.open"), plugins::GUIDE_URL)),
        ))
        .into_any_element()
}
