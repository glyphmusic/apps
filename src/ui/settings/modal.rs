use super::*;

// ------------------------------------------------------------------ modal

pub fn modal(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    theme::on_panel(|| modal_inner(app, window, cx))
}

/// Title, subtitle and nav icon of a settings page.
pub fn page_info(page: SettingsPage) -> (SharedString, SharedString, &'static str) {
    match page {
        SettingsPage::Playback => (t("sp.playback"), t("sp.playback.sub"), "play"),
        SettingsPage::Audio => (t("sp.eq"), t("sp.eq.sub"), "volume-2"),
        SettingsPage::Appearance => (t("sp.appearance"), t("sp.appearance.sub"), "palette"),
        SettingsPage::Account => (t("sp.account"), t("sp.account.sub"), "user-round"),
        SettingsPage::Profile => (t("sp.profile"), t("sp.profile.sub"), "eye"),
        SettingsPage::Server => (t("sp.server"), t("sp.server.sub"), "server"),
        SettingsPage::Storage => (t("sp.storage"), t("sp.storage.sub"), "database"),
        SettingsPage::Transfer => (t("sp.transfer"), t("sp.transfer.sub"), "copy"),
        SettingsPage::Migrate => (t("sp.migrate"), t("sp.migrate.sub"), "arrow-right-left"),
        SettingsPage::Integrations => (t("sp.integrations"), t("sp.integrations.sub"), "link-2"),
        SettingsPage::Plugins => (t("sp.plugins"), t("sp.plugins.sub"), "puzzle"),
        SettingsPage::About => (t("sp.about"), t("sp.about.sub"), "info"),
        SettingsPage::System => (t("sp.system"), t("sp.system.sub"), "monitor"),
    }
}

/// Sub-tabs of a settings page (label keys); empty: the page has none.
/// `app.settings_sub` indexes this list.
pub fn subtabs(app: &Glyph, page: SettingsPage) -> &'static [&'static str] {
    match page {
        SettingsPage::Playback => &["sp.sub.general", "sp.sub.content", "settings.sources", "sp.sub.player"],
        SettingsPage::Account if app.signed_in().is_some() => &["sp.sub.overview", "sp.sub.devices", "account.security"],
        SettingsPage::Profile => &["sp.sub.profile", "sp.sub.privacy"],
        SettingsPage::Server => &["sp.sub.connection", "settings.keys", "privacy.tab"],
        // Installed, Browse, then one tab per plugin with its own page.
        SettingsPage::Plugins => super::plugin_ui::plugin_tabs(),
        SettingsPage::About => &["sp.sub.version", "sp.support", "sp.sub.shortcuts"],
        _ => &[],
    }
}

/// Every option the settings search finds on its own: page, sub-tab, label
/// key (the row's label, so the row can light up), extra search words.
pub(super) const OPTIONS: &[(SettingsPage, u8, &str, &str)] = &[
    (SettingsPage::Playback, 0, "sp.audio.quality", "streaming quality qualität bitrate"),
    (SettingsPage::Server, 2, "privacy.title", "privacy datenschutz transparency transparenz encryption verschlüsselung e2e end-to-end ende-zu-ende server sees sieht"),
    (SettingsPage::Playback, 0, "sp.audio.dl_quality", "download quality qualität flac"),
    (SettingsPage::Playback, 0, "sp.crossfade", "crossfade überblenden fade"),
    (SettingsPage::Playback, 0, "sp.output", "output ausgabe device gerät speaker lautsprecher headphones kopfhörer"),
    (SettingsPage::Playback, 0, "sp.level", "volume lautstärke level"),
    (SettingsPage::Playback, 0, "sp.normalize", "normalize normalisieren loudness lautheit"),
    (SettingsPage::Playback, 0, "sp.devices", "network netzwerk devices geräte chromecast cast nest google upnp dlna tv fernseher sonos airplay mpd receiver lautsprecher speaker"),
    (SettingsPage::Playback, 0, "sp.audio.channels", "mono stereo spatial surround 5.1 7.1 hrtf binaural 3d räumlich raumklang channels kanäle"),
    (SettingsPage::Playback, 1, "sp.shuffle", "shuffle zufall random smart"),
    (SettingsPage::Playback, 1, "sp.stats", "stats statistics statistik analytics hörzeit listening time tracking"),
    (SettingsPage::Playback, 2, "sp.extractor", "r-dlp rdlp yt-dlp extractor youtube schneller faster"),
    (SettingsPage::Playback, 2, "sp.search.mode", "search suche lokal local server rate limit ban quellen sources"),
    (SettingsPage::Playback, 1, "sp.explicit", "explicit explizit"),
    (SettingsPage::Playback, 1, "sp.radio", "radio autoplay"),
    (SettingsPage::Playback, 1, "sp.hidden", "hidden ausgeblendet blocked artists künstler"),
    (SettingsPage::Playback, 2, "settings.sources.fallback", "youtube soundcloud embed fallback"),
    (SettingsPage::Playback, 2, "ytdlp.title", "yt-dlp ytdlp youtube"),
    (SettingsPage::Playback, 2, "settings.priority", "priority priorität order reihenfolge audius jamendo archive"),
    (SettingsPage::Playback, 3, "sp.player.wheel_seek", "mouse maus wheel mausrad scroll scrollen seek spulen vorspulen progress fortschritt"),
    (SettingsPage::Playback, 3, "sp.player.wheel_volume", "mouse maus wheel mausrad scroll scrollen volume lautstärke"),
    (SettingsPage::Playback, 3, "sp.player.lyrics", "lyrics songtext liedtext now playing hide ausblenden"),
    (SettingsPage::Playback, 3, "sp.player.np_download", "download herunterladen now playing button"),
    (SettingsPage::Playback, 3, "sp.player.keys", "hotkey shortcut tastenkürzel taste ändern change rebind belegen zen download"),
    (SettingsPage::Audio, 0, "sp.eq.bass", "bass boost tiefen low"),
    (SettingsPage::Audio, 0, "sp.eq.headroom", "headroom preamp clipping übersteuern verzerrung distortion pegel gain"),
    (SettingsPage::Appearance, 0, "sp.appearance.shape", "visualizer visualisierer hintergrund background shape form glow standard default favorite liebling"),
    (SettingsPage::Appearance, 0, "sp.appearance.text", "text size schrift größe ui scale skalierung zoom font"),
    (SettingsPage::Storage, 0, "sp.storage.dl_dir", "download folder ordner pfad path speicherort location"),
    (SettingsPage::Storage, 0, "sp.storage.reveal", "download folder ordner öffnen show reveal explorer finder"),
    (SettingsPage::Storage, 0, "sp.storage.local", "local lokal files dateien folder ordner mp3 flac music musik bibliothek library"),
    (SettingsPage::Audio, 0, "sp.eq.enable", "equalizer eq"),
    (SettingsPage::Audio, 0, "sp.eq.preset", "preset bass treble höhen tiefen"),
    (SettingsPage::Audio, 0, "sp.eq.share", "share teilen export import code datei file"),
    (SettingsPage::Appearance, 0, "settings.language", "language sprache"),
    (SettingsPage::Appearance, 0, "settings.visuals.music_reactivity", "music musik reaction reaktion beat bass"),
    (SettingsPage::Appearance, 0, "settings.visuals.intensity", "background hintergrund glow intensity intensität"),
    (SettingsPage::Appearance, 0, "settings.visuals.ascii_covers", "ascii cover"),
    (SettingsPage::Appearance, 0, "sp.appearance.startup", "startup start animation splash"),
    (SettingsPage::Appearance, 0, "sp.appearance.mini", "mini player miniplayer card karte bar leiste"),
    (SettingsPage::Appearance, 0, "sp.appearance.theme", "theme farbe color colour accent akzent farbschema orange blau blue green grün"),
    (SettingsPage::Appearance, 0, "sp.appearance.ascii_fx", "ascii animation reveal einblenden skeleton laden loading shimmer"),
    (SettingsPage::Appearance, 0, "sp.appearance.throttle", "fps frame rate bildrate background hintergrund akku battery power energie"),
    (SettingsPage::Account, 0, "account.handle", "handle name rename umbenennen"),
    (SettingsPage::Account, 0, "account.library", "sync api keys schlüssel encryption verschlüsselung e2e library bibliothek"),
    (SettingsPage::Account, 0, "account.limit", "device limit gerätelimit"),
    (SettingsPage::Account, 1, "account.pair", "pair koppeln code device gerät add hinzufügen"),
    (SettingsPage::Account, 2, "account.password.change", "password passwort"),
    (SettingsPage::Account, 2, "account.passkey.add", "passkey"),
    (SettingsPage::Account, 2, "account.recovery", "recovery wiederherstellung code"),
    (SettingsPage::Account, 2, "account.delete", "delete löschen account"),
    (SettingsPage::Profile, 0, "profile.avatar", "profile profil picture bild avatar pfp foto photo"),
    (SettingsPage::Profile, 0, "profile.banner", "profile profil banner header titelbild"),
    (SettingsPage::Profile, 1, "profile.public", "privacy privatsphäre public öffentlich private privat profile profil"),
    (SettingsPage::Profile, 1, "profile.show_recent", "privacy privatsphäre recent zuletzt songs history verlauf"),
    (SettingsPage::Server, 0, "settings.server.url", "server url adresse"),
    (SettingsPage::Server, 1, "settings.keys.lastfm", "api key lastfm last.fm"),
    (SettingsPage::Server, 1, "settings.keys.youtube", "api key youtube"),
    (SettingsPage::Server, 1, "settings.keys.spotify_client_id", "api key spotify"),
    (SettingsPage::Server, 1, "settings.keys.jamendo_client_id", "api key jamendo"),
    (SettingsPage::Integrations, 0, "sp.discord", "discord rich presence integration integrations status activity listening"),
    (SettingsPage::Integrations, 0, "sp.discord.id", "discord application id client id app developer"),
    (SettingsPage::Integrations, 0, "sp.mcp", "mcp model context protocol ai claude assistant ki agent"),
    (SettingsPage::Integrations, 0, "sp.mcp.access.playback", "mcp ai assistant permissions berechtigungen rechte scopes tools read write lesen schreiben"),
    (SettingsPage::Plugins, 1, "plugins.catalog", "plugins plugin erweiterungen extensions addons add-ons themes farbschema github topic community"),
    (SettingsPage::Plugins, 0, "plugins.publish", "plugin erstellen create publish veröffentlichen developer entwickler github"),
    (SettingsPage::Storage, 0, "sp.storage.limit", "cache limit size größe"),
    (SettingsPage::Storage, 0, "sp.storage.saver", "data saver datensparmodus mobile traffic daten sparen"),
    (SettingsPage::Storage, 0, "sp.storage.usage", "clear leeren cache downloads covers lyrics speicher"),
    (SettingsPage::Transfer, 0, "sp.transfer.code", "share teilen export code copy kopieren"),
    (SettingsPage::Transfer, 0, "sp.transfer.file", "export file datei save speichern backup"),
    (SettingsPage::Transfer, 0, "sp.transfer.import", "import apply übernehmen paste einfügen"),
    (SettingsPage::Migrate, 0, "migrate.link", "link playlist import einfügen paste spotify soundcloud deezer apple bandcamp"),
    (SettingsPage::Migrate, 0, "migrate.file", "file datei csv m3u xspf json export"),
    (SettingsPage::System, 0, "sp.system.autostart", "autostart start startup boot login anmelden hochfahren pc computer windows autorun"),
    (SettingsPage::System, 0, "sp.system.start_mode", "tray systray infobereich taskleiste hintergrund background minimized minimiert hidden versteckt window fenster"),
    (SettingsPage::About, 0, "update.auto", "auto update automatisch"),
    (SettingsPage::About, 0, "update.channel", "prerelease beta channel kanal"),
    (SettingsPage::About, 1, "sp.support.issue", "bug issue report melden"),
    (SettingsPage::About, 1, "sp.support.diag", "diagnostics diagnose"),
    (SettingsPage::About, 1, "sp.support.data", "data daten folder ordner"),
    (SettingsPage::About, 2, "sp.keys.play", "keyboard shortcuts tastenkürzel tastatur hotkeys keys tasten ändern change rebind belegen"),
    (SettingsPage::About, 2, "sp.keys.zen", "zen cover vollbild fullscreen focus hotkey"),
    (SettingsPage::About, 2, "sp.keys.download", "download herunterladen hotkey"),
    (SettingsPage::About, 2, "sp.keys.wheel_seek", "mouse maus wheel mausrad scroll"),
];

/// Content of the current settings page and sub-tab (shared by the desktop
/// modal and the phone settings in `ui::compact`).
pub fn page_body(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    select_anim_tick(app.open_select);
    PAGE_SCROLL.with(|h| *h.borrow_mut() = Some(app.settings_scroll.clone()));
    // Sections plugins add to this page (and rows they hide on it).
    let added = super::plugin_ui::begin_page(app, cx);
    let body = match app.settings_page {
        SettingsPage::Playback => playback(app, cx),
        SettingsPage::Audio => equalizer(app, cx),
        SettingsPage::Appearance => appearance(app, cx),
        SettingsPage::Server => server(app, cx),
        SettingsPage::Account => crate::ui::account::page(app, cx),
        SettingsPage::Profile => crate::ui::profile::settings_page(app, cx),
        SettingsPage::Storage => storage(app, cx),
        SettingsPage::Transfer => crate::ui::transfer::page(app, cx),
        SettingsPage::Migrate => crate::ui::migrate::page(app, cx),
        SettingsPage::Integrations => integrations(app, cx),
        SettingsPage::Plugins if app.settings_sub >= 2 && app.plugins.detail.is_none() => super::plugin_ui::plugin_page(app, cx),
        SettingsPage::Plugins => plugins_page(app, cx),
        SettingsPage::About if app.settings_sub == 1 => support(app, cx),
        SettingsPage::About if app.settings_sub == 2 => shortcuts(app, cx),
        SettingsPage::About => about(app, cx),
        SettingsPage::System => system(app, cx),
    };
    let rest = super::plugin_ui::end_page();
    body.children(added).children(rest)
}

pub(super) fn modal_inner(app: &mut Glyph, _window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let page = app.settings_page;
    let (title, sub, _) = page_info(page);
    let tab = subtabs(app, page).get(app.settings_sub as usize).map(|k| t(k));
    let body = page_body(app, cx);

    // Outer shell holds the navigation; the page sits in its own inset,
    // bordered card next to it.
    let page_bg = hsla(0.0, 0.0, 0.045, 1.0);
    let content = div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .rounded(px(12.0))
        .overflow_hidden()
        .bg(page_bg)
        .border_1()
        .border_color(white(0.08))
        .child(
            fade_edges(
                div()
                    .id("settings-page")
                    .track_scroll(&app.settings_scroll)
                    .size_full()
                    .overflow_y_scroll().touch_axis()
                    .px(px(28.0))
                    .py(px(22.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .pr(px(36.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .text_size(px(17.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(div().text_color(if tab.is_some() { white(0.5) } else { gpui::white() }).child(title))
                                    .when_some(tab, |d, tab| d.child(icon("chevron-right", 16.0, white(0.4))).child(div().text_color(gpui::white()).child(tab))),
                            )
                            .child(div().text_size(px(13.0)).text_color(white(0.55)).child(sub)),
                    )
                    // Switching page or sub-tab fades the new page in.
                    .child(div().pt(px(10.0)).child(crate::ui::widgets::fade_in(
                        "settings-fade",
                        crate::ui::widgets::fade_key((format!("{page:?}"), app.settings_sub)),
                        false,
                        body,
                    ))),
                page_bg,
                22.0,
                Some(&app.settings_scroll),
            )
            .size_full()
            .child(scrollbar(&app.settings_scroll, true, cx))
            .child(crate::ui::widgets::smooth_wheel(&app.settings_scroll, false, cx)),
        )
        .child(
            div()
                .id("settings-close")
                .absolute()
                .top(px(14.0))
                .right(px(14.0))
                .size(px(30.0))
                .rounded(px(8.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .child(hover_fill("settings-close", white(0.08), 8.0))
                .on_click(cx.listener(|this, _, w, cx| this.close_settings(w, cx)))
                .child(icon("x", 16.0, white(0.7))),
        );

    let lift = (1.0 - app.settings_k()) * 14.0;
    let panel = div()
        .id("settings-panel")
        .occlude()
        .relative()
        .mt(px(lift))
        .w(px(PANEL_W))
        .max_w(gpui::relative(0.94))
        .h(px(PANEL_H))
        .max_h(gpui::relative(0.9))
        .flex()
        .p(px(8.0))
        .gap(px(4.0))
        .rounded(px(16.0))
        .overflow_hidden()
        .bg(hsla(0.0, 0.0, 0.08, 0.985))
        .border_1()
        .border_color(white(0.07))
        .font_family(theme::FONT)
        // Clicks inside never reach the backdrop (which closes the modal).
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(nav(app, cx))
        .child(content);

    let backdrop = crate::ui::widgets::modal_backdrop("settings-backdrop", 0.30)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|this, _, w, cx| {
            cx.stop_propagation();
            this.close_settings(w, cx);
        }))
        .child(panel);
    crate::ui::widgets::modal_layer(0.30, backdrop).into_any_element()
}
