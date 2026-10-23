//! Settings -> Plugins: the catalog from GitHub (checked index + `glyph-plugin`
//! topic, see `glyph_core::plugins`), install, update and remove. Theme
//! plugins show up next to the built-in themes; texts and hidden rows of
//! extensions apply at once; their code runs through `plugin_api`.

use super::*;
use glyph_core::plugins::{self, Catalog, Installed, Listing};
use std::collections::HashMap;

pub use super::plugin_api::PluginRuntime;

#[derive(Default)]
pub struct PluginsUi {
    pub installed: Vec<Installed>,
    pub catalog: Option<Catalog>,
    pub loading: bool,
    pub error: Option<String>,
    /// Plugin ids being installed or removed.
    pub busy: HashSet<String>,
    /// Last failed install per plugin id.
    pub failed: HashMap<String, String>,
    /// Plugin shown in the detail view (`glyph://plugin/<id>`, or a click
    /// on a plugin's name).
    pub detail: Option<String>,
    /// First install waiting for the user to accept the warning that
    /// plugins come from other people (shown in the detail view).
    pub confirm: Option<Listing>,
    /// "Don't show again" ticked on that warning.
    pub confirm_skip: bool,
    /// Running code and the state of the settings rows plugins add.
    pub rt: PluginRuntime,
    /// Plugins with their own settings page, in the order of their tabs
    /// under Settings › Plugins (after Installed and Browse).
    pub pages: Vec<String>,
    /// Settings rows hidden by plugins (label keys).
    pub hidden: Vec<String>,
}

impl Glyph {
    /// Reads the installed plugins from disk and publishes their themes,
    /// texts, hidden rows and settings pages (code starts with
    /// [`Glyph::sync_plugin_code`]).
    pub(super) fn load_plugins(&mut self) {
        self.plugins.installed = plugins::installed();
        if self.plugins.catalog.is_none() {
            self.plugins.catalog = plugins::cached_catalog();
        }
        self.apply_plugin_extras();
        let themes = self
            .plugins
            .installed
            .iter()
            .filter_map(|p| {
                Some(crate::theme::PluginTheme {
                    id: plugins::theme_id(&p.manifest.id),
                    name: p.manifest.name.clone(),
                    colors: p.manifest.theme_colors()?,
                })
            })
            .collect();
        crate::theme::set_plugin_themes(themes);
    }

    /// Texts, hidden rows and settings pages of the enabled plugins.
    pub(super) fn apply_plugin_extras(&mut self) {
        let active: Vec<&plugins::Manifest> = self.plugins.installed.iter().filter(|p| self.plugin_enabled(&p.manifest.id)).map(|p| &p.manifest).collect();
        let mut texts: HashMap<String, HashMap<String, String>> = HashMap::new();
        for m in &active {
            for (lang, map) in &m.strings {
                let t = texts.entry(lang.clone()).or_default();
                for (k, v) in map {
                    t.insert(k.clone(), v.clone());
                }
            }
        }
        // Own settings pages: one tab per plugin, named like it; only for
        // plugins whose code runs (nobody else would answer the page).
        let mut pages = Vec::new();
        let mut tabs = Vec::new();
        for p in self.plugins.installed.iter().filter(|p| p.manifest.main.is_some() && self.plugin_enabled(&p.manifest.id) && (p.origin.checked || p.origin.is_local())) {
            if p.manifest.sections_on("own").next().is_none() {
                continue;
            }
            let key = format!("plugin-page.{}", p.manifest.id);
            texts.entry("en".into()).or_default().insert(key.clone(), p.manifest.name.clone());
            tabs.push(key);
            pages.push(p.manifest.id.clone());
        }
        self.plugins.hidden = plugins::hidden_rows(&active);
        self.plugins.pages = pages;
        crate::i18n::set_overrides(texts);
        crate::ui::settings::set_plugin_tabs(tabs);
    }

    /// Loads the catalog (cached for an hour unless `force`).
    pub fn refresh_plugin_catalog(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.plugins.loading {
            return;
        }
        let Some(b) = self.backend.clone() else {
            self.plugins.error = Some(t("status.backend_offline").to_string());
            cx.notify();
            return;
        };
        self.plugins.loading = true;
        self.plugins.error = None;
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.plugin_catalog(force)).await;
            this.update(cx, |s, cx| {
                s.plugins.loading = false;
                match r {
                    Ok(c) => {
                        // A plugin page opened from the web that the cached
                        // catalog does not know yet: ask GitHub once more.
                        let missing = s.plugins.detail.as_ref().is_some_and(|id| c.find(id).is_none());
                        s.plugins.catalog = Some(c);
                        if s.config.plugins.auto_update {
                            for l in s.silent_plugin_updates() {
                                s.install_plugin(l, cx);
                            }
                        }
                        if missing && !force {
                            s.refresh_plugin_catalog(true, cx);
                        }
                    }
                    Err(e) => s.plugins.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Installs or updates `listing`. A first install shows the warning
    /// first (plugins are made by other people, installed at your own
    /// risk), unless the user turned it off; plugins with code always show
    /// it, with what they may do. Updates of an installed plugin go
    /// straight through unless they ask for more permissions.
    pub fn install_plugin(&mut self, listing: Listing, cx: &mut Context<Self>) {
        let old = self.plugins.installed.iter().find(|p| p.manifest.id == listing.manifest.id);
        let fresh = old.is_none();
        let asks_more = match old {
            Some(o) => !listing.manifest.new_permissions(&o.manifest).is_empty(),
            None => !listing.manifest.data_only(),
        };
        if asks_more || (fresh && !self.config.plugins.skip_install_warning) {
            self.plugins.detail = Some(listing.manifest.id.clone());
            self.plugins.confirm = Some(listing);
            self.plugins.confirm_skip = false;
            self.settings_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
            cx.notify();
            return;
        }
        self.install_plugin_now(listing, cx);
    }

    /// "Install anyway" on the warning.
    pub fn accept_plugin_warning(&mut self, cx: &mut Context<Self>) {
        let Some(listing) = self.plugins.confirm.take() else { return };
        if self.plugins.confirm_skip && listing.manifest.data_only() {
            self.config.plugins.skip_install_warning = true;
            self.save_config(cx);
        }
        self.install_plugin_now(listing, cx);
    }

    pub fn cancel_plugin_warning(&mut self, cx: &mut Context<Self>) {
        self.plugins.confirm = None;
        cx.notify();
    }

    fn install_plugin_now(&mut self, listing: Listing, cx: &mut Context<Self>) {
        let id = listing.manifest.id.clone();
        if !self.plugins.busy.insert(id.clone()) {
            return;
        }
        self.plugins.failed.remove(&id);
        let fresh = !self.plugins.installed.iter().any(|p| p.manifest.id == id);
        let name = listing.manifest.name.clone();
        let is_theme = listing.manifest.kind == "theme";
        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            // Code comes from GitHub first (the same commit as the manifest).
            let code = match (&listing.manifest.main, backend) {
                (None, _) => Ok(None),
                (Some(_), Some(b)) => guard_async(b.plugin_code(listing.clone())).await,
                (Some(_), None) => Err(t("status.backend_offline").to_string()),
            };
            let r = match code {
                Ok(code) => {
                    cx.background_executor()
                        .spawn(async move { plugins::install(&listing, code.as_deref()).map_err(|e| format!("{e:#}")) })
                        .await
                }
                Err(e) => Err(e),
            };
            this.update(cx, |s, cx| {
                s.plugins.busy.remove(&id);
                match r {
                    Ok(_) => {
                        s.load_plugins();
                        s.sync_plugin_code(cx);
                        if s.plugins.rt.missing_credentials.contains_key(&id) {
                            s.open_plugin_page(&id, cx);
                        }
                        let theme = plugins::theme_id(&id);
                        if is_theme && (fresh || s.config.visuals.theme == theme) {
                            // Re-applied on update too, so new colours show at once.
                            s.set_theme(&theme, cx);
                        }
                        let key = if fresh { "plugins.installed_toast" } else { "plugins.updated_toast" };
                        s.toast(tf(key, &[("name", &name)]).to_string());
                    }
                    Err(e) => {
                        s.plugins.failed.insert(id, e);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Removes a plugin; its theme, if in use, goes back to the default.
    pub fn uninstall_plugin(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.plugins.busy.insert(id.clone()) {
            return;
        }
        if self.config.visuals.theme == plugins::theme_id(&id) {
            self.set_theme(crate::theme::THEMES[0].id, cx);
        }
        // Stopped before its folder goes.
        self.plugins.rt.running.remove(&id);
        let name = self.plugins.installed.iter().find(|p| p.manifest.id == id).map(|p| p.manifest.name.clone()).unwrap_or_else(|| id.clone());
        cx.spawn(async move |this, cx| {
            let gone = id.clone();
            let r = cx.background_executor().spawn(async move { plugins::uninstall(&gone).map_err(|e| format!("{e:#}")) }).await;
            this.update(cx, |s, cx| {
                s.plugins.busy.remove(&id);
                match r {
                    Ok(()) => {
                        s.load_plugins();
                        s.sync_plugin_code(cx);
                        s.toast(tf("plugins.removed_toast", &[("name", &name)]).to_string());
                    }
                    Err(e) => s.toast_info(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Opens Settings -> Plugins on one plugin, ready to install.
    pub fn open_plugin(&mut self, id: String, cx: &mut Context<Self>) {
        self.open_settings(Some(SettingsPage::Plugins), cx);
        // Browse tab: back from the detail view lands in the catalog.
        self.set_settings_tab(SettingsPage::Plugins, 1, cx);
        self.plugins.detail = Some(id);
        self.refresh_plugin_catalog(false, cx);
    }

    /// Opens the detail view of `id` from inside the Plugins page.
    pub fn show_plugin(&mut self, id: String, cx: &mut Context<Self>) {
        if self.plugins.confirm.as_ref().is_some_and(|l| l.manifest.id != id) {
            self.plugins.confirm = None;
        }
        self.plugins.detail = Some(id);
        self.settings_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        cx.notify();
    }

    pub fn close_plugin_detail(&mut self, cx: &mut Context<Self>) {
        self.plugins.detail = None;
        self.plugins.confirm = None;
        cx.notify();
    }

    pub fn set_plugin_auto_update(&mut self, on: bool, cx: &mut Context<Self>) {
        self.config.plugins.auto_update = on;
        self.save_config(cx);
        if on {
            for l in self.silent_plugin_updates() {
                self.install_plugin(l, cx);
            }
        }
        cx.notify();
    }

    /// "Reload" for plugin makers: reads the plugins folder again, so an
    /// edited glyph-plugin.json or a new build of the code shows without
    /// restarting Glyph.
    pub fn reload_plugins(&mut self, cx: &mut Context<Self>) {
        self.stop_local_plugins();
        self.load_plugins();
        self.sync_plugin_code(cx);
        let theme = self.config.visuals.theme.clone();
        if theme.starts_with("plugin:") {
            // Re-applies the (maybe changed) colours, glow included.
            self.set_theme(&theme, cx);
        }
        self.toast(tf("plugins.reloaded", &[("n", &self.plugins.installed.len().to_string())]).to_string());
        cx.notify();
    }

    pub fn set_plugin_sort(&mut self, sort: plugins::Sort, cx: &mut Context<Self>) {
        self.config.plugins.sort = sort;
        self.save_config(cx);
        cx.notify();
    }

    /// Installed plugins with a newer version in the catalog.
    pub fn plugin_updates(&self) -> Vec<Listing> {
        let Some(c) = &self.plugins.catalog else { return Vec::new() };
        self.plugins.installed.iter().filter_map(|p| p.update_in(c).cloned()).collect()
    }

    /// Updates that may install by themselves: no new permissions.
    fn silent_plugin_updates(&self) -> Vec<Listing> {
        self.plugin_updates()
            .into_iter()
            .filter(|l| self.plugins.installed.iter().any(|p| p.manifest.id == l.manifest.id && l.manifest.new_permissions(&p.manifest).is_empty()))
            .collect()
    }
}
