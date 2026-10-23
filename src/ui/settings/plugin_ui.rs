//! Settings rows that plugins add (`settings` in glyph-plugin.json): the
//! plugin's own page under Settings › Plugins, sections on Glyph's pages
//! (after a given row or at the end), and Glyph rows plugins hide. Every
//! plugin section names its plugin, so nobody mistakes it for Glyph's.

use super::*;
use glyph_core::plugins::{self, Installed, SettingItem, SettingsSection};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

thread_local! {
    /// While a Glyph page renders: labels of rows plugins hide.
    static HIDDEN: std::cell::RefCell<HashSet<String>> = std::cell::RefCell::new(HashSet::new());
    /// While a Glyph page renders: plugin sections to show under the row
    /// with this label.
    static AFTER: std::cell::RefCell<HashMap<String, Vec<AnyElement>>> = std::cell::RefCell::new(HashMap::new());
    /// Stable `&'static str` ids for plugin controls (dropdowns and bars
    /// keep their state by such an id).
    static IDS: std::cell::RefCell<HashMap<String, &'static str>> = std::cell::RefCell::new(HashMap::new());
}

/// Settings › Plugins tabs after Installed and Browse: one per plugin page
/// (label keys, see `Glyph::apply_plugin_extras`).
static PLUGIN_TABS: std::sync::RwLock<&'static [&'static str]> = std::sync::RwLock::new(&["plugins.tab.installed", "plugins.tab.browse"]);

/// Sets the plugin pages' tab keys (their names come from the plugin texts).
pub fn set_plugin_tabs(keys: Vec<String>) {
    let mut all: Vec<&'static str> = vec!["plugins.tab.installed", "plugins.tab.browse"];
    // Leaked once per distinct key: bounded by the plugins ever installed.
    all.extend(keys.into_iter().map(static_str));
    if let Ok(mut t) = PLUGIN_TABS.write() {
        *t = Box::leak(all.into_boxed_slice());
    }
}

pub(super) fn plugin_tabs() -> &'static [&'static str] {
    PLUGIN_TABS.read().map(|t| *t).unwrap_or(&["plugins.tab.installed", "plugins.tab.browse"])
}

fn static_str(s: String) -> &'static str {
    IDS.with(|ids| *ids.borrow_mut().entry(s.clone()).or_insert_with(|| Box::leak(s.into_boxed_str())))
}

/// The plugin page name (`glyph_core::plugins::PAGES`) of a settings page and tab.
fn page_name(page: SettingsPage, sub: u8) -> Option<&'static str> {
    Some(match (page, sub) {
        (SettingsPage::Playback, 0) => "playback",
        (SettingsPage::Playback, 1) => "playback.content",
        (SettingsPage::Playback, 2) => "playback.sources",
        (SettingsPage::Audio, _) => "audio",
        (SettingsPage::Appearance, _) => "appearance",
        (SettingsPage::Integrations, _) => "integrations",
        (SettingsPage::Storage, _) => "storage",
        (SettingsPage::System, _) => "system",
        (SettingsPage::About, 0) => "about",
        _ => return None,
    })
}

/// Plugins whose rows may show: enabled, code running.
fn active(app: &Glyph) -> Vec<Installed> {
    app.plugins.installed.iter().filter(|p| app.plugins.rt.running.contains_key(&p.manifest.id)).cloned().collect()
}

/// Before a Glyph page renders: what plugins hide on it and the sections
/// they add after a row. Returns the sections that go at the end.
pub(super) fn begin_page(app: &mut Glyph, cx: &mut Context<Glyph>) -> Vec<AnyElement> {
    let hidden: HashSet<String> = app.plugins.hidden.iter().map(|k| t(k).to_string()).collect();
    HIDDEN.with(|h| *h.borrow_mut() = hidden);
    AFTER.with(|a| a.borrow_mut().clear());
    let Some(name) = page_name(app.settings_page, app.settings_sub) else { return Vec::new() };
    let mut end = Vec::new();
    for p in active(app) {
        for s in p.manifest.sections_on(name).cloned().collect::<Vec<_>>() {
            let el = section(app, &p, &s, true, cx);
            if s.after.is_empty() {
                end.push(el);
            } else {
                let label = t(&s.after).to_string();
                AFTER.with(|a| a.borrow_mut().entry(label).or_default().push(el));
            }
        }
    }
    end
}

/// After the page rendered: sections whose row was not on it go to the end.
pub(super) fn end_page() -> Vec<AnyElement> {
    HIDDEN.with(|h| h.borrow_mut().clear());
    AFTER.with(|a| a.borrow_mut().drain().flat_map(|(_, v)| v).collect())
}

/// A plugin hides the row labelled `label`.
pub(super) fn row_hidden(label: &str) -> bool {
    HIDDEN.with(|h| h.borrow().contains(label))
}

/// Plugin sections that go under the row labelled `label`.
pub(super) fn sections_after(label: &str) -> Vec<AnyElement> {
    AFTER.with(|a| a.borrow_mut().remove(label).unwrap_or_default())
}

/// One plugin section: its title (and, on Glyph's pages, the plugin's
/// name), then its rows.
fn section(app: &mut Glyph, p: &Installed, s: &SettingsSection, named: bool, cx: &mut Context<Glyph>) -> AnyElement {
    let lang = crate::i18n::lang().code();
    let title = s.title.get(lang).to_string();
    let head: SharedString = match (named, title.is_empty()) {
        (true, true) => p.manifest.name.clone().into(),
        (true, false) => format!("{title} · {}", p.manifest.name).into(),
        (false, true) => SharedString::default(),
        (false, false) => title.into(),
    };
    let items: Vec<AnyElement> = s.items.iter().filter_map(|i| item_row(app, &p.manifest.id, i, cx)).collect();
    div()
        .flex()
        .flex_col()
        .when(!head.is_empty(), |d| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(section_label(head))
                    .when(named, |d| d.child(div().pt(px(16.0)).child(icon("puzzle", 12.0, white(0.4))))),
            )
        })
        .child(rows(items))
        .into_any_element()
}

/// A plugin's own page (Settings › Plugins › <name>; tab 2 is the first).
pub(super) fn plugin_page(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let index = (app.settings_sub as usize).saturating_sub(2);
    let Some(p) = app.plugins.pages.get(index).and_then(|id| app.plugins.installed.iter().find(|p| &p.manifest.id == id)).cloned() else {
        return div();
    };
    let id = p.manifest.id.clone();
    let by = if p.manifest.author.trim().is_empty() { t("plugins.warn.someone").to_string() } else { p.manifest.author.clone() };
    let mut col = div().flex().flex_col().child(
        div()
            .flex()
            .items_start()
            .gap(px(8.0))
            .pt(px(4.0))
            .child(div().pt(px(1.0)).child(icon("puzzle", 14.0, white(0.45))))
            .child(div().flex_1().min_w_0().text_size(px(12.5)).line_height(px(18.0)).text_color(white(0.5)).child(tf("plugins.page.by", &[("author", &by)])))
            .child(
                div()
                    .id("plugin-page-about")
                    .flex_none()
                    .text_size(px(12.5))
                    .text_color(white(0.6))
                    .cursor_pointer()
                    .hover(|d| d.text_color(gpui::white()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_settings_tab(SettingsPage::Plugins, 0, cx);
                        this.show_plugin(id.clone(), cx);
                    }))
                    .child(t("plugins.page.about")),
            ),
    );
    if app.plugins.rt.missing_credentials.contains_key(&p.manifest.id) {
        col = col.child(div().pt(px(10.0)).text_size(px(12.5)).text_color(theme::warn()).child(t("plugins.setup.missing")));
    }
    if let Some(why) = app.plugins.rt.stopped.get(&p.manifest.id) {
        col = col.child(div().pt(px(10.0)).text_size(px(12.5)).text_color(theme::warn()).child(tf("plugins.stopped", &[("e", why)])));
    }
    for s in p.manifest.sections_on("own").cloned().collect::<Vec<_>>() {
        col = col.child(section(app, &p, &s, false, cx));
    }
    col
}

/// One settings row of a plugin; `None` while the plugin hides it.
fn item_row(app: &mut Glyph, pid: &str, item: &SettingItem, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let lang = crate::i18n::lang().code();
    let patch = app.plugins.rt.patches.get(pid).and_then(|m| m.get(&item.id)).cloned().unwrap_or_default();
    if patch.get("hidden").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let text = |k: &str, base: &plugins::Text| -> String { patch.get(k).and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| base.get(lang).to_string()) };
    let label = text("label", &item.label);
    // Buttons and links: the caption on the control (default: the label).
    let caption = if item.caption.is_empty() { label.clone() } else { item.caption.get(lang).to_string() };
    let desc = if let Some(c) = &item.credential {
        format!("{} · https://{}{} · {}: {}", text("desc", &item.desc), c.host, c.path, c.location, c.name)
    } else { text("desc", &item.desc) };
    let desc = (!desc.trim().is_empty()).then(|| SharedString::from(desc));
    let status = if item.secret {
        Some(if app.plugins.rt.configured_credentials.contains(&(pid.to_string(), item.id.clone())) { t("plugins.setup.configured") } else { t("plugins.setup.enter") })
    } else { patch.get("status").and_then(Value::as_str).filter(|s| !s.is_empty()).map(|s| SharedString::from(s.to_string())) };
    let disabled = patch.get("disabled").and_then(Value::as_bool).unwrap_or(false);
    let key = format!("pl-{pid}-{}", item.id);
    let (pid_s, iid) = (pid.to_string(), item.id.clone());
    let value = app.plugin_value(pid, item);
    let control: AnyElement = match item.kind.as_str() {
        "progress" => {
            let value = patch.get("value").and_then(Value::as_f64).unwrap_or(0.0).clamp(0.0, 1.0);
            div().w(px(160.0)).h(px(6.0)).rounded(px(3.0)).bg(white(0.08))
                .child(div().w(px(160.0 * value as f32)).h(px(6.0)).rounded(px(3.0)).bg(theme::pink()))
                .into_any_element()
        }
        "toggle" => {
            let on = value.as_bool().unwrap_or(false);
            switch(SharedString::from(key), on)
                .when(!disabled, |d| d.on_click(cx.listener(move |this, _, _, cx| this.set_plugin_value(&pid_s, &iid, Value::Bool(!on), cx))))
                .into_any_element()
        }
        "slider" => {
            let (min, max) = (item.min.unwrap_or(0.0), item.max.unwrap_or(100.0));
            let x = value.as_f64().unwrap_or(min);
            let frac = ((x - min) / (max - min)).clamp(0.0, 1.0) as f32;
            let weak = cx.entity().downgrade();
            let pick: crate::ui::widgets::Pick = Rc::new(move |f, _, cx| {
                let v = min + (max - min) * f as f64;
                weak.update(cx, |this, cx| this.set_plugin_value(&pid_s, &iid, serde_json::json!(v), cx)).ok();
            });
            let shown = if (x - x.round()).abs() < 1e-9 { format!("{x:.0}{}", item.unit) } else { format!("{x:.1}{}", item.unit) };
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .when(disabled, |d| d.opacity(0.5))
                .child(if disabled { div().w(px(160.0)).into_any_element() } else { thin_bar(static_str(key), 160.0, frac, pick, None, cx).into_any_element() })
                .child(mono().w(px(48.0)).flex().justify_end().text_size(px(12.0)).text_color(white(0.6)).child(shown))
                .into_any_element()
        }
        "select" => {
            let options: Vec<(String, String)> = match patch.get("options").and_then(Value::as_array) {
                Some(o) => o.iter().filter_map(|o| Some((o["value"].as_str()?.to_string(), o["label"].as_str()?.to_string()))).collect(),
                None => item.options.iter().map(|o| (o.value.clone(), o.label.get(lang).to_string())).collect(),
            };
            let cur = value.as_str().unwrap_or("").to_string();
            let cur_label = options.iter().find(|(v, _)| *v == cur).map(|(_, l)| l.clone()).unwrap_or_default();
            let opts = options
                .into_iter()
                .map(|(v, l)| {
                    let (pid, iid) = (pid_s.clone(), iid.clone());
                    let on = v == cur;
                    let f: Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> = Rc::new(move |this, cx| this.set_plugin_value(&pid, &iid, Value::String(v.clone()), cx));
                    (SharedString::from(l), on, f)
                })
                .collect();
            select(app, static_str(key), cur_label.into(), opts, cx).into_any_element()
        }
        "text" => {
            let input = app.plugin_input(pid, item, cx);
            input_box(input).into_any_element()
        }
        "button" => button(SharedString::from(key), SharedString::from(caption))
            .when(disabled, |d| d.opacity(0.5))
            .when(!disabled, |d| d.on_click(cx.listener(move |this, _, _, _| this.plugin_action(&pid_s, &iid))))
            .into_any_element(),
        "link" => {
            let url = item.url.clone();
            button(SharedString::from(key), SharedString::from(caption))
                .child(icon("external-link", 13.0, white(0.7)))
                .on_click(move |_, _, cx| cx.open_url(&url))
                .into_any_element()
        }
        // A note: text only, no control.
        _ => {
            let body = desc.clone().unwrap_or_default();
            return Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .py(px(14.0))
                    .when(!label.is_empty(), |d| d.child(div().text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(label.clone())))
                    .child(div().text_size(px(12.5)).line_height(px(18.0)).text_color(white(0.6)).child(body))
                    .when_some(status, |d, s| d.child(div().text_size(px(12.5)).text_color(white(0.55)).child(s)))
                    .into_any_element(),
            );
        }
    };
    Some(row_full(label, desc, status, control))
}
