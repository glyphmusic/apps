//! Settings -> Backup & share: copy the settings as a code, save them as a
//! file, and apply a code or file from someone else (with undo). Format:
//! `glyph_core::settings_share` (no API keys, no server, no device values).

use crate::app::{Glyph, SettingsPage};
use crate::i18n::{t, tf};
use crate::ui::input::TextInput;
use crate::ui::settings::{input_box, row, rows, section_label, white};
use crate::ui::widgets::*;
use glyph_core::config::Config;
use glyph_core::settings_share;
use gpui::{div, prelude::*, px, Context, Div, Entity};

#[derive(Default)]
pub struct TransferUi {
    pub code_input: Option<Entity<TextInput>>,
    /// Settings before the last import ("Undo").
    pub undo: Option<Config>,
}

impl Glyph {
    /// Copies the settings as one shareable line.
    pub fn copy_settings_code(&mut self, cx: &mut Context<Self>) {
        let code = settings_share::encode(&self.config);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(code));
        self.toast(t("sp.transfer.copied").to_string());
        cx.notify();
    }

    /// Saves the settings as a JSON file where the user picks.
    pub fn save_settings_file(&mut self, cx: &mut Context<Self>) {
        let json = settings_share::to_json(&self.config);
        let dir = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(std::path::PathBuf::from)
            .map(|h| if h.join("Downloads").is_dir() { h.join("Downloads") } else { h })
            .unwrap_or_else(glyph_core::paths::data_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(settings_share::FILE_NAME));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let r = cx.background_executor().spawn({
                let path = path.clone();
                async move { std::fs::write(&path, json) }
            });
            let r = r.await;
            this.update(cx, |s, cx| {
                match r {
                    Ok(()) => s.toast_sub(t("sp.transfer.saved"), path.display().to_string()),
                    Err(e) => s.toast_info(tf("sp.transfer.error", &[("e", &e)])),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Picks a settings file and applies it.
    pub fn open_settings_file(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("sp.transfer.file.pick").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let text = cx.background_executor().spawn(async move { std::fs::read_to_string(path) }).await;
            this.update(cx, |s, cx| match text {
                Ok(text) => {
                    s.import_settings_text(&text, cx);
                }
                Err(e) => {
                    s.toast_info(tf("sp.transfer.error", &[("e", &e)]));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Applies the code typed into the import field.
    pub fn import_settings_from_input(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.transfer.code_input.clone() else { return };
        let text = input.read(cx).value.clone();
        if text.trim().is_empty() {
            return;
        }
        if self.import_settings_text(&text, cx) {
            input.update(cx, |i, cx| i.set_value("", cx));
        }
    }

    /// Applies a settings code or file content; the old settings stay for undo.
    pub fn import_settings_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let ok = match settings_share::decode(text) {
            Ok(shared) => {
                let before = self.config.clone();
                let mut cfg = before.clone();
                let r = shared.apply(&mut cfg);
                if r.changed {
                    self.transfer.undo = Some(before);
                    self.adopt_synced_config(cfg, r.providers_changed, cx);
                    self.schedule_sync(cx);
                    self.toast(t("sp.transfer.applied").to_string());
                } else {
                    self.toast_info(t("sp.transfer.same"));
                }
                true
            }
            Err(e) => {
                self.toast_info(tf("sp.transfer.invalid", &[("e", &format!("{e:#}"))]));
                false
            }
        };
        self.open_settings(Some(SettingsPage::Transfer), cx);
        cx.notify();
        ok
    }

    pub fn undo_settings_import(&mut self, cx: &mut Context<Self>) {
        let Some(old) = self.transfer.undo.take() else { return };
        let providers_changed = old.providers != self.config.providers;
        self.adopt_synced_config(old, providers_changed, cx);
        self.schedule_sync(cx);
        self.toast(t("sp.transfer.undone").to_string());
        cx.notify();
    }
}

pub fn page(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    if app.transfer.code_input.is_none() {
        app.transfer.code_input = Some(cx.new(|cx| TextInput::new("sp.transfer.placeholder", cx)));
    }
    let input = app.transfer.code_input.clone().unwrap();
    let share = rows(vec![
        row(
            t("sp.transfer.code"),
            Some(t("sp.transfer.code.desc")),
            button("tr-copy", t("sp.transfer.code.button"))
                .child(icon("copy", 14.0, white(0.6)))
                .on_click(cx.listener(|this, _, _, cx| this.copy_settings_code(cx))),
        ),
        row(
            t("sp.transfer.file"),
            Some(t("sp.transfer.file.desc")),
            button("tr-save", t("sp.transfer.file.button"))
                .child(icon("download", 14.0, white(0.6)))
                .on_click(cx.listener(|this, _, _, cx| this.save_settings_file(cx))),
        ),
    ]);
    let mut apply = vec![
        row(
            t("sp.transfer.import"),
            Some(t("sp.transfer.import.desc")),
            div()
                .flex()
                .gap(px(8.0))
                .child(input_box(input).w(px(200.0)))
                .child(button("tr-apply", t("sp.transfer.import.button")).on_click(cx.listener(|this, _, _, cx| this.import_settings_from_input(cx)))),
        ),
        row(
            t("sp.transfer.open"),
            Some(t("sp.transfer.open.desc")),
            button("tr-open", t("sp.transfer.open.button"))
                .child(icon("folder-open", 14.0, white(0.6)))
                .on_click(cx.listener(|this, _, _, cx| this.open_settings_file(cx))),
        ),
    ];
    if app.transfer.undo.is_some() {
        apply.push(row(
            t("sp.transfer.undo"),
            Some(t("sp.transfer.undo.desc")),
            button("tr-undo", t("sp.transfer.undo.button")).on_click(cx.listener(|this, _, _, cx| this.undo_settings_import(cx))),
        ));
    }
    div()
        .flex()
        .flex_col()
        .child(section_label(t("sp.transfer.share")))
        .child(share)
        .child(section_label(t("sp.transfer.apply")))
        .child(rows(apply))
        .child(div().pt(px(14.0)).text_size(px(12.0)).text_color(white(0.4)).child(t("sp.transfer.note")))
}
