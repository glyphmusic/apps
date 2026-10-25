use super::*;
use glyph_core::config::UiConfig;

type Act = Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>;

/// One dropdown option that changes a device-local player setting.
fn opt(label: SharedString, on: bool, f: impl Fn(&mut UiConfig) + 'static) -> (SharedString, bool, Act) {
    let f = Rc::new(f);
    (label, on, Rc::new(move |this: &mut Glyph, cx: &mut Context<Glyph>| {
        let f = f.clone();
        this.set_ui(move |u| f(u), cx)
    }))
}

/// Settings > Playback > Player: the player bar's mouse wheel, Now
/// Playing's lyrics and download button, and the way to the shortcuts (all
/// on this device only).
pub(super) fn player(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let ui = app.config.ui.clone();
    let mut items = Vec::new();
    if !crate::ui::compact::TOUCH {
        let seek_label = |s: u32| -> SharedString { if s == 0 { t("sp.player.off") } else { format!("{s} s").into() } };
        let seek = [0u32, 1, 5, 10, 15, 30].into_iter().map(|s| opt(seek_label(s), ui.wheel_seek_s == s, move |u| u.wheel_seek_s = s)).collect();
        items.push(row(t("sp.player.wheel_seek"), Some(t("sp.player.wheel_seek.desc")), select(app, "sel-wheel-seek", seek_label(ui.wheel_seek_s), seek, cx)));
        let vol_label = |p: u32| -> SharedString { if p == 0 { t("sp.player.off") } else { format!("{p} %").into() } };
        let vol = [0u32, 1, 2, 5, 10].into_iter().map(|p| opt(vol_label(p), ui.wheel_volume_pct == p, move |u| u.wheel_volume_pct = p)).collect();
        items.push(row(t("sp.player.wheel_volume"), Some(t("sp.player.wheel_volume.desc")), select(app, "sel-wheel-vol", vol_label(ui.wheel_volume_pct), vol, cx)));
    }
    items.push(row(
        t("sp.player.lyrics"),
        Some(t("sp.player.lyrics.desc")),
        switch("np-lyrics", ui.np_lyrics).on_click(cx.listener(|this, _, _, cx| this.set_ui(|u| u.np_lyrics = !u.np_lyrics, cx))),
    ));
    items.push(row(
        t("sp.player.np_download"),
        Some(t("sp.player.np_download.desc")),
        switch("np-download", ui.np_download).on_click(cx.listener(|this, _, _, cx| this.set_ui(|u| u.np_download = !u.np_download, cx))),
    ));
    if !crate::ui::compact::TOUCH {
        // All shortcuts (zen mode, download, ...) are changed in one place.
        items.push(row(
            t("sp.player.keys"),
            Some(t("sp.player.keys.desc")),
            button("player-keys", t("sp.player.keys.open")).on_click(cx.listener(|this, _, _, cx| this.set_settings_tab(SettingsPage::About, 2, cx))),
        ));
    }
    rows(items)
}
