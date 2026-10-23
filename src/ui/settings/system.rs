use super::*;

/// Settings > System (desktop only): start with the computer, and whether
/// such a start opens the window or stays in the background.
pub(super) fn system(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let autostart = app.config.desktop.autostart;
    let tray = app.config.desktop.start_in_tray;
    let mode = segmented(
        vec![
            (t("sp.system.mode.window"), !tray, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_start_in_tray(false, cx)) as Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>),
            (t("sp.system.mode.tray"), tray, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_start_in_tray(true, cx))),
        ],
        cx,
        "start-mode",
    );
    // macOS has no tray icon here: the hidden app opens from the Dock.
    let mode_desc = if crate::tray::SUPPORTED { t("sp.system.start_mode.desc") } else { t("sp.system.start_mode.desc_dock") };
    rows(vec![
        row(
            t("sp.system.autostart"),
            Some(t("sp.system.autostart.desc")),
            switch("autostart", autostart).on_click(cx.listener(move |this, _, _, cx| this.set_autostart(!autostart, cx))),
        ),
        row(t("sp.system.start_mode"), Some(mode_desc), div().when(!autostart, |d| d.opacity(0.45)).child(mode)),
    ])
}
