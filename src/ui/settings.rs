//! Settings as a modal (Liner layout): left navigation with search and
//! grouped pages, right the page with rows (label + description left,
//! control right). Pages: Playback, Audio, Appearance, Server, Cache &
//! Storage, About.

use crate::app::{BackendState, ConnTest, Glyph, SettingsPage};
use crate::i18n::{t, tf, Lang};
use crate::theme;
use crate::ui::widgets::*;
use glyph_core::config::AudioQuality;
use glyph_core::storage::{format_bytes, StorageKind};
use glyph_core::{BackendMode, UpdateStatus};
use glyph_protocol::ProviderId;
use gpui::{
    canvas, deferred, div, hsla, point, prelude::*, px, AnyElement, Context, Div, FontWeight, Hsla,
    MouseButton, Pixels, SharedString, Window,
};
use std::rc::Rc;

pub const REPO_URL: &str = "https://github.com/glyphmusic/glyph";
const DOCS_URL: &str = "https://github.com/glyphmusic/glyph#readme";
const ISSUES_URL: &str = "https://github.com/glyphmusic/glyph/issues/new";
const RELEASES_URL: &str = "https://github.com/glyphmusic/glyph/releases";

const PANEL_W: f32 = 920.0;
const PANEL_H: f32 = 600.0;
const NAV_W: f32 = 232.0;

fn white(a: f32) -> Hsla {
    hsla(0.0, 0.0, 1.0, a)
}

// ------------------------------------------------------------------ pieces

fn switch(id: impl Into<gpui::ElementId>, on: bool) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .cursor_pointer()
        .w(px(36.0))
        .h(px(20.0))
        .rounded_full()
        .p(px(2.0))
        .flex()
        .when(on, |d| d.justify_end())
        .bg(if on { gpui::white() } else { white(0.14) })
        .child(div().size(px(16.0)).rounded_full().bg(if on { gpui::black() } else { white(0.75) }))
}

/// Settings row: label + description left, control right.
fn row(label: impl Into<SharedString>, desc: Option<SharedString>, control: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(24.0))
        .py(px(14.0))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap(px(3.0))
                .child(div().text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(label.into()))
                .when_some(desc, |d, s| d.child(div().text_size(px(12.5)).text_color(white(0.55)).child(s))),
        )
        .child(div().flex_none().flex().items_center().gap(px(8.0)).child(control))
        .into_any_element()
}

/// Rows separated by hairlines.
fn rows(items: Vec<AnyElement>) -> Div {
    let n = items.len();
    div().flex().flex_col().children(items.into_iter().enumerate().map(move |(i, r)| {
        div().when(i + 1 < n, |d| d.border_b_1().border_color(white(0.06))).child(r)
    }))
}

fn section_label(text: impl Into<SharedString>) -> Div {
    div()
        .pt(px(18.0))
        .pb(px(2.0))
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(white(0.45))
        .child(text.into().to_uppercase())
}

/// A flat surface for grouped content (storage breakdown, slider).
fn surface() -> Div {
    div().rounded(px(10.0)).bg(white(0.035)).p(px(18.0))
}

fn link_button(id: &'static str, icon_name: &'static str, label: impl Into<SharedString>, url: &'static str) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.0))
        .h(px(32.0))
        .px(px(12.0))
        .rounded(px(8.0))
        .bg(white(0.07))
        .cursor_pointer()
        .hover(|s| s.bg(white(0.12)))
        .text_size(px(13.0))
        .text_color(gpui::white())
        .child(icon(icon_name, 14.0, white(0.8)))
        .child(label.into())
        .on_click(move |_, _, cx| cx.open_url(url))
}

/// Segmented control: one pill, the selected option filled.
fn segmented(items: Vec<(SharedString, bool, Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>)>, cx: &mut Context<Glyph>, id: &'static str) -> Div {
    div()
        .flex()
        .p(px(3.0))
        .gap(px(2.0))
        .rounded(px(9.0))
        .bg(white(0.05))
        .children(items.into_iter().enumerate().map(|(i, (label, on, f))| {
            div()
                .id((id, i))
                .px(px(12.0))
                .h(px(28.0))
                .flex()
                .items_center()
                .rounded(px(7.0))
                .cursor_pointer()
                .text_size(px(13.0))
                .when(on, |d| d.bg(white(0.12)).text_color(gpui::white()).font_weight(FontWeight::MEDIUM))
                .when(!on, |d| d.text_color(white(0.6)).hover(|s| s.text_color(gpui::white())))
                .on_click(cx.listener(move |this, _, _, cx| f(this, cx)))
                .child(label)
        }))
}

/// Dropdown select: a button with the current value; the open list floats
/// below it (right-aligned) with a check on the selected entry.
fn select(
    app: &Glyph,
    id: &'static str,
    value: SharedString,
    options: Vec<(SharedString, bool, Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>)>,
    cx: &mut Context<Glyph>,
) -> Div {
    let open = app.open_select == Some(id);
    let list = open.then(|| {
        deferred(
            div()
                .id((id, 999usize))
                .occlude()
                .absolute()
                .top(px(40.0))
                .right_0()
                .min_w(px(240.0))
                .p(px(4.0))
                .rounded(px(10.0))
                .bg(hsla(0.0, 0.0, 0.09, 0.99))
                .border_1()
                .border_color(white(0.08))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .children(options.into_iter().enumerate().map(|(i, (label, on, f))| {
                    div()
                        .id((id, i))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.0))
                        .h(px(36.0))
                        .px(px(10.0))
                        .rounded(px(7.0))
                        .cursor_pointer()
                        .text_size(px(13.5))
                        .when(on, |d| d.bg(white(0.08)).text_color(gpui::white()))
                        .when(!on, |d| d.text_color(white(0.7)).hover(|s| s.bg(white(0.05)).text_color(gpui::white())))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_select = None;
                            f(this, cx);
                            cx.notify();
                        }))
                        .child(label)
                        .when(on, |d| d.child(icon("check", 14.0, gpui::white())))
                })),
        )
        .with_priority(2)
    });
    div()
        .relative()
        .child(
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(8.0))
                .h(px(34.0))
                .pl(px(12.0))
                .pr(px(10.0))
                .rounded(px(8.0))
                .bg(white(if open { 0.10 } else { 0.06 }))
                .border_1()
                .border_color(white(0.06))
                .cursor_pointer()
                .hover(|s| s.bg(white(0.10)))
                .text_size(px(13.5))
                .text_color(gpui::white())
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_select(id, cx)))
                .child(div().max_w(px(220.0)).truncate().child(value))
                .child(icon("chevron-down", 14.0, white(0.6))),
        )
        .children(list)
}

/// "Advanced" disclosure row; the section below only shows when expanded.
fn advanced_toggle(app: &Glyph, cx: &mut Context<Glyph>) -> AnyElement {
    let open = app.settings_advanced;
    div()
        .id("sp-advanced")
        .flex()
        .items_center()
        .gap(px(6.0))
        .pt(px(18.0))
        .pb(px(4.0))
        .cursor_pointer()
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(white(0.55))
        .hover(|s| s.text_color(gpui::white()))
        .on_click(cx.listener(|this, _, _, cx| {
            this.settings_advanced = !this.settings_advanced;
            cx.notify();
        }))
        .child(icon(if open { "chevron-down" } else { "chevron-right" }, 14.0, white(0.55)))
        .child(t("sp.advanced"))
        .into_any_element()
}

fn input_box(input: impl IntoElement) -> Div {
    div()
        .w(px(300.0))
        .h(px(34.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(8.0))
        .bg(hsla(0.0, 0.0, 0.0, 0.45))
        .border_1()
        .border_color(white(0.08))
        .text_size(px(13.0))
        .font_family(theme::MONO)
        .child(input)
}

// ------------------------------------------------------------------ modal

pub fn modal(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    theme::on_panel(|| modal_inner(app, window, cx))
}

fn modal_inner(app: &mut Glyph, _window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let page = app.settings_page;
    let (title, sub, body) = match page {
        SettingsPage::Playback => (t("sp.playback"), t("sp.playback.sub"), playback(app, cx)),
        SettingsPage::Audio => (t("sp.playback"), t("sp.playback.sub"), playback(app, cx)),
        SettingsPage::Appearance => (t("sp.appearance"), t("sp.appearance.sub"), appearance(app, cx)),
        SettingsPage::Server => (t("sp.server"), t("sp.server.sub"), server(app, cx)),
        SettingsPage::Storage => (t("sp.storage"), t("sp.storage.sub"), storage(app, cx)),
        SettingsPage::About => (t("sp.about"), t("sp.about.sub"), about(app, cx)),
        SettingsPage::Support => (t("sp.support"), t("sp.support.sub"), support(app, cx)),
    };

    let panel = div()
        .id("settings-panel")
        .occlude()
        .relative()
        .w(px(PANEL_W))
        .max_w(gpui::relative(0.94))
        .h(px(PANEL_H))
        .max_h(gpui::relative(0.9))
        .flex()
        .rounded(px(14.0))
        .overflow_hidden()
        .bg(hsla(0.0, 0.0, 0.055, 0.985))
        .border_1()
        .border_color(white(0.07))
        .font_family(theme::FONT)
        // Clicks inside never reach the backdrop (which closes the modal).
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(nav(app, cx))
        .child(fade_edges(div()
                .id("settings-page")
                .track_scroll(&app.settings_scroll)
                .size_full()
                .overflow_y_scroll()
                .px(px(32.0))
                .py(px(26.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .pr(px(36.0))
                        .child(div().text_size(px(19.0)).font_weight(FontWeight::SEMIBOLD).text_color(gpui::white()).child(title))
                        .child(div().text_size(px(13.0)).text_color(white(0.55)).child(sub)),
                )
                .child(div().pt(px(10.0)).child(body)), hsla(0.0, 0.0, 0.055, 1.0), 26.0, Some(&app.settings_scroll)).flex_1().min_w_0().h_full().child(scrollbar(&app.settings_scroll, true, cx)))
        .child(
            div()
                .id("settings-close")
                .absolute()
                .top(px(18.0))
                .right(px(18.0))
                .size(px(30.0))
                .rounded(px(8.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(white(0.08)))
                .on_click(cx.listener(|this, _, w, cx| this.close_settings(w, cx)))
                .child(icon("x", 16.0, white(0.7))),
        );

    div()
        .id("settings-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(hsla(0.0, 0.0, 0.0, 0.30))
        .on_click(cx.listener(|this, _, w, cx| this.close_settings(w, cx)))
        .child(panel)
        .into_any_element()
}

fn nav(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let q = app.settings_search.read(cx).value.trim().to_lowercase();
    let current = app.settings_page;
    // (page, icon, label, extra search words)
    let groups: [(SharedString, Vec<(SettingsPage, &'static str, SharedString, &'static str)>); 2] = [
        (
            t("sp.group.prefs"),
            vec![
                (SettingsPage::Playback, "play", t("sp.playback"), "sources source quelle youtube soundcloud priority radio quality qualität audio"),
                (SettingsPage::Appearance, "palette", t("sp.appearance"), "language sprache theme ascii cover glow"),
            ],
        ),
        (
            t("sp.group.app"),
            vec![
                (SettingsPage::Server, "server", t("sp.server"), "url api key keys schlüssel"),
                (SettingsPage::Storage, "database", t("sp.storage"), "cache download downloads speicher"),
                (SettingsPage::About, "info", t("sp.about"), "update version"),
            ],
        ),
    ];
    let matches = |label: &SharedString, words: &str| q.is_empty() || label.to_lowercase().contains(&q) || words.contains(&q);
    let item = |page: SettingsPage, icon_name: &'static str, label: SharedString, cx: &mut Context<Glyph>| {
        let on = page == current;
        div()
            .id(("sp-nav", page as usize))
            .flex()
            .items_center()
            .gap(px(10.0))
            .h(px(36.0))
            .px(px(10.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .text_size(px(14.0))
            .when(on, |d| d.bg(white(0.08)).text_color(gpui::white()))
            .when(!on, |d| d.text_color(white(0.75)).hover(|s| s.bg(white(0.04)).text_color(gpui::white())))
            .on_click(cx.listener(move |this, _, _, cx| this.set_settings_page(page, cx)))
            .child(icon(icon_name, 16.0, if on { gpui::white() } else { white(0.7) }))
            .child(label)
    };

    let mut col = div()
        .flex_none()
        .w(px(NAV_W))
        .h_full()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .p(px(12.0))
        .bg(white(0.02))
        .border_r_1()
        .border_color(white(0.05))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .h(px(34.0))
                .px(px(10.0))
                .mb(px(6.0))
                .rounded(px(8.0))
                .bg(white(0.06))
                .text_size(px(13.0))
                .child(icon("search", 14.0, white(0.5)))
                .child(div().flex_1().min_w_0().child(app.settings_search.clone())),
        );
    for (group, items) in groups {
        let visible: Vec<_> = items.into_iter().filter(|(_, _, l, w)| matches(l, w)).collect();
        if visible.is_empty() {
            continue;
        }
        col = col.child(div().px(px(10.0)).pt(px(10.0)).pb(px(4.0)).text_size(px(12.0)).text_color(white(0.45)).child(group));
        for (page, ic, label, _) in visible {
            col = col.child(item(page, ic, label, cx));
        }
    }
    col
}

// ------------------------------------------------------------------ pages

fn playback(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    // Everyday options first; sources and their order live under "Advanced".
    let cur = app.config.audio.quality;
    let options = [AudioQuality::Low, AudioQuality::Standard, AudioQuality::High]
        .into_iter()
        .map(|q| {
            let f: Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> = Rc::new(move |this, cx| this.set_audio_quality(q, cx));
            (quality_label(q), q == cur, f)
        })
        .collect();
    let quality = select(app, "sel-quality", quality_label(cur), options, cx);
    let basic = rows(vec![
        row(t("sp.audio.quality"), Some(t("sp.audio.quality.desc")), quality),
        row(t("sp.crossfade"), Some(t("sp.crossfade.desc")), {
            const STEPS: [u32; 9] = [0, 2, 4, 6, 8, 10, 12, 15, 20];
            let cur = app.config.audio.crossfade_s;
            let label = |s: u32| -> SharedString { if s == 0 { t("sp.crossfade.off") } else { format!("{s} s").into() } };
            let options = STEPS
                .into_iter()
                .map(|s| {
                    let f: Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> = Rc::new(move |this, cx| this.set_crossfade(s, cx));
                    (label(s), s == cur, f)
                })
                .collect();
            select(app, "sel-crossfade", label(cur), options, cx)
        }),
        row(
            t("settings.sources.fallback"),
            Some(t("settings.sources.fallback.hint")),
            switch("embed-fallback", app.embed_fallback()).on_click(cx.listener(|this, _, _, cx| this.toggle_embed_fallback(cx))),
        ),
        row(
            t("sp.radio"),
            Some(t("sp.radio.desc")),
            switch("radio", app.radio).on_click(cx.listener(|this, _, _, cx| {
                this.radio = !this.radio;
                cx.notify();
            })),
        ),
    ]);

    // Artists hidden with "Don't show this artist", each with "Show again".
    let hidden = app.library.hidden_artists.clone();
    let hidden_row = row(
        t("sp.hidden"),
        Some(if hidden.is_empty() { t("sp.hidden.none") } else { t("sp.hidden.desc") }),
        div().flex().flex_wrap().justify_end().gap(px(6.0)).max_w(px(320.0)).children(hidden.into_iter().enumerate().map(|(i, name)| {
            let n = name.clone();
            div()
                .id(("hidden-artist", i))
                .flex()
                .items_center()
                .gap(px(6.0))
                .h(px(28.0))
                .pl(px(10.0))
                .pr(px(6.0))
                .rounded(px(8.0))
                .bg(white(0.06))
                .border_1()
                .border_color(white(0.08))
                .text_size(px(12.5))
                .text_color(white(0.85))
                .child(name)
                .child(
                    div()
                        .id(("hidden-show", i))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(18.0))
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(white(0.12)))
                        .tooltip(|_, cx| cx.new(|_| Tip(t("ctx.show_artist"))).into())
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_hidden_artist(&n, cx)))
                        .child(icon("eye", 12.0, white(0.7))),
                )
        })),
    );
    let mut col = div().flex().flex_col().child(basic).child(rows(vec![hidden_row])).child(advanced_toggle(app, cx));
    if app.settings_advanced {
        let enabled = app.config.providers.enabled.clone();
        let sources: Vec<AnyElement> = [ProviderId::Audius, ProviderId::Jamendo, ProviderId::Archive]
            .into_iter()
            .map(|p| {
                let cooling = app
                    .backend
                    .as_ref()
                    .and_then(|b| crate::guard::guard(|| b.provider_cooling_down(p)))
                    .unwrap_or(false);
                row(
                    p.display_name(),
                    Some(if cooling { t("settings.sources.paused") } else { t(&format!("sp.src.{}", p.badge().to_lowercase())) }),
                    switch(("prov", p as usize), enabled.contains(&p)).on_click(cx.listener(move |this, _, _, cx| this.toggle_provider(p, cx))),
                )
            })
            .collect();
        col = col
            .child(rows(sources))
            .child(section_label(t("settings.priority")))
            .child(div().pb(px(8.0)).text_size(px(12.5)).text_color(white(0.55)).child(t("settings.priority.hint")))
            .child(priority_list(app, cx));
    }
    col
}

/// The dragged priority entry: the whole row follows the cursor.
#[derive(Clone)]
struct PrioDrag {
    from: usize,
    provider: ProviderId,
    width: Pixels,
}

fn prio_row_content(i: usize, p: ProviderId) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(10.0))
        .child(icon("grip-vertical", 15.0, white(0.4)))
        .child(mono().w(px(18.0)).text_size(px(13.0)).text_color(white(0.45)).child(format!("{}", i + 1)))
        .child(div().flex_1().text_size(px(14.0)).text_color(gpui::white()).child(p.display_name()))
        .child(badge(p))
}

impl Render for PrioDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        theme::on_panel(|| {
            prio_row_content(self.from, self.provider)
                .w(self.width)
                .rounded(px(8.0))
                .bg(hsla(0.0, 0.0, 0.17, 0.98))
                .border_1()
                .border_color(white(0.12))
                .font_family(theme::FONT)
                .cursor_grabbing()
        })
    }
}

fn priority_list(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let width = px(PANEL_W - NAV_W - 64.0);
    div()
        .flex()
        .flex_col()
        .rounded(px(10.0))
        .bg(white(0.035))
        .children(app.config.providers.priority.clone().into_iter().enumerate().map(|(i, p)| {
            div()
                .id(("prio", i))
                .rounded(px(8.0))
                .cursor_grab()
                .hover(|s| s.bg(white(0.03)))
                .on_drag(PrioDrag { from: i, provider: p, width }, |d, _, _, cx| cx.new(|_| d.clone()))
                .drag_over::<PrioDrag>(|s, _, _, _| s.bg(white(0.08)))
                .on_drop(cx.listener(move |this, d: &PrioDrag, _, cx| this.reorder_priority(d.from, i, cx)))
                .child(prio_row_content(i, p))
        }))
}

fn quality_label(q: AudioQuality) -> SharedString {
    match q {
        AudioQuality::Low => t("sp.audio.q.low"),
        AudioQuality::Standard => t("sp.audio.q.standard"),
        AudioQuality::High => t("sp.audio.q.high"),
    }
}

fn appearance(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let cur = app.lang;
    let options = Lang::ALL
        .into_iter()
        .map(|l| {
            let f: Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> = Rc::new(move |this, cx| this.set_language(l, cx));
            (SharedString::from(l.native_name()), l == cur, f)
        })
        .collect();
    let language = select(app, "sel-lang", cur.native_name().into(), options, cx);

    let weak = cx.entity().downgrade();
    let set_int: Pick = Rc::new(move |f, _, cx| {
        weak.update(cx, |this, cx| this.set_intensity(f, cx)).ok();
    });
    let intensity = app.config.visuals.intensity;
    rows(vec![
        row(t("settings.language"), Some(t("sp.appearance.lang.desc")), language),
        row(
            t("settings.visuals.intensity"),
            Some(t("sp.appearance.intensity.desc")),
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(thin_bar("intensity", 160.0, intensity, set_int, None, cx))
                .child(mono().w(px(36.0)).flex().justify_end().text_size(px(12.0)).text_color(white(0.6)).child(format!("{:.0}%", intensity * 100.0))),
        ),
        row(
            t("settings.visuals.ascii_covers"),
            Some(t("sp.appearance.ascii.desc")),
            switch("ascii-covers", app.config.visuals.ascii_covers).on_click(cx.listener(|this, _, _, cx| {
                this.config.visuals.ascii_covers = !this.config.visuals.ascii_covers;
                this.save_config(cx);
                cx.notify();
            })),
        ),
        row(
            t("sp.appearance.startup"),
            Some(t("sp.appearance.startup.desc")),
            switch("startup-anim", app.config.visuals.startup_animation).on_click(cx.listener(|this, _, _, cx| {
                this.config.visuals.startup_animation = !this.config.visuals.startup_animation;
                this.save_config(cx);
                cx.notify();
            })),
        ),
    ])
}

fn server(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let mode: String = match &app.backend_state {
        BackendState::Connecting => format!("{} {}", app.spinner(), t("status.connecting")),
        BackendState::Ready(BackendMode::Server { url, server_version }) => {
            // The public server is just "Glyph server"; your own shows its host.
            let name = if url.trim_end_matches('/') == glyph_core::config::DEFAULT_SERVER_URL {
                t("settings.server.public").to_string()
            } else {
                url.split("://").nth(1).unwrap_or(url).trim_end_matches('/').to_string()
            };
            tf("settings.server.connected", &[("name", &name), ("v", server_version)]).to_string()
        }
        BackendState::Ready(BackendMode::Direct) => t("status.direct").to_string(),
        BackendState::Ready(BackendMode::Incompatible { protocol_version, .. }) => tf("status.incompatible", &[("v", protocol_version)]).to_string(),
        BackendState::Offline(e) => format!("{} {e}", t("status.backend_offline")),
    };
    let conn = match &app.conn_test {
        None => None,
        Some(ConnTest::Running) => Some((format!("{} {}", app.spinner(), t("settings.server.testing")), white(0.6))),
        Some(ConnTest::Ok(s)) => Some((s.clone(), theme::ok())),
        Some(ConnTest::Failed(s)) => Some((s.clone(), theme::warn())),
    };
    let mut items = vec![
        row(t("settings.server.url"), Some(t("sp.server.url.desc")), input_box(app.server_input.clone())),
        row(
            mode,
            conn.as_ref().map(|(s, _)| SharedString::from(s.clone())),
            div()
                .flex()
                .gap(px(8.0))
                .child(button("conn-test", t("settings.server.test")).on_click(cx.listener(|this, _, _, cx| this.test_connection(cx))))
                .child(button("conn-apply", t("settings.server.apply")).on_click(cx.listener(|this, _, _, cx| this.apply_server(cx)))),
        ),
    ];
    let keys: Vec<AnyElement> = app
        .key_inputs
        .iter()
        .map(|(name, input)| row(t(&format!("settings.keys.{name}")), None, input_box(input.clone())))
        .collect();
    let mut col = div().flex().flex_col().child(rows(items)).child(advanced_toggle(app, cx));
    if app.settings_advanced {
        col = col
            .child(section_label(t("settings.keys")))
            .child(div().pb(px(4.0)).text_size(px(12.5)).text_color(white(0.55)).child(t("settings.keys.hint")))
            .child(rows(keys));
    }
    col
}

const STORAGE_COLORS: [u32; 4] = [0x22c3e6, 0xa970ff, 0xff5fa2, 0xf2b84b];
const LIMITS_MB: [u64; 7] = [250, 1024, 2048, 3072, 5120, 10240, 0];

fn limit_label(mb: u64) -> SharedString {
    match mb {
        0 => t("sp.storage.nolimit"),
        m if m < 1024 => format!("{m} MB").into(),
        m => format!("{} GB", m / 1024).into(),
    }
}

fn storage_label(k: StorageKind) -> SharedString {
    match k {
        StorageKind::Covers => t("sp.storage.covers"),
        StorageKind::Downloads => t("sp.storage.downloads"),
        StorageKind::Lyrics => t("sp.storage.lyrics"),
        StorageKind::Catalog => t("sp.storage.catalog"),
    }
}

/// Ring chart of the storage split, drawn as filled ring segments.
fn donut(parts: Vec<(f32, Hsla)>, size: f32, thickness: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let c = bounds.center();
            let r_out = size / 2.0;
            let r_in = r_out - thickness;
            let total: f32 = parts.iter().map(|p| p.0).sum();
            let ring = |a0: f32, a1: f32, color: Hsla, window: &mut Window| {
                let steps = (((a1 - a0) / std::f32::consts::TAU) * 96.0).ceil().max(2.0) as usize;
                let at = |r: f32, a: f32| point(c.x + px(r * a.cos()), c.y + px(r * a.sin()));
                let mut b = gpui::PathBuilder::fill();
                b.move_to(at(r_out, a0));
                for i in 1..=steps {
                    b.line_to(at(r_out, a0 + (a1 - a0) * i as f32 / steps as f32));
                }
                for i in (0..=steps).rev() {
                    b.line_to(at(r_in, a0 + (a1 - a0) * i as f32 / steps as f32));
                }
                b.close();
                if let Ok(path) = b.build() {
                    window.paint_path(path, color);
                }
            };
            let start = -std::f32::consts::FRAC_PI_2;
            if total <= 0.0 {
                ring(start, start + std::f32::consts::TAU - 0.0001, white(0.08), window);
                return;
            }
            let mut a = start;
            for (v, color) in &parts {
                if *v <= 0.0 {
                    continue;
                }
                let sweep = v / total * std::f32::consts::TAU;
                ring(a, a + sweep - 0.0001, *color, window);
                a += sweep;
            }
        },
    )
    .size(px(size))
}

fn storage(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let Some(stats) = app.storage else {
        return div().child(skeleton("storage-sk", 520.0, 160.0, 10.0));
    };
    let total = stats.total();
    let legend = StorageKind::ALL.into_iter().enumerate().map(|(i, k)| {
        let v = stats.get(k);
        let pct = if total > 0 { v as f32 / total as f32 * 100.0 } else { 0.0 };
        let dim = v == 0;
        div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .h(px(32.0))
            .text_size(px(13.5))
            .when(dim, |d| d.opacity(0.45))
            .child(div().size(px(8.0)).rounded_full().bg(gpui::rgb(STORAGE_COLORS[i])))
            .child(div().w(px(140.0)).text_color(gpui::white()).child(storage_label(k)))
            .child(
                mono()
                    .px(px(6.0))
                    .rounded(px(4.0))
                    .bg(white(0.08))
                    .text_size(px(11.0))
                    .text_color(white(0.7))
                    .child(format!("{pct:.0}%")),
            )
            .child(div().flex_1())
            .child(mono().w(px(72.0)).flex().justify_end().text_size(px(12.5)).text_color(white(0.8)).child(format_bytes(v)))
            .child(
                div()
                    .id(("clear", i))
                    .ml(px(8.0))
                    .px(px(8.0))
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(white(0.6))
                    .cursor_pointer()
                    .hover(|s| s.bg(white(0.08)).text_color(gpui::white()))
                    .when(dim, |d| d.invisible())
                    .on_click(cx.listener(move |this, _, _, cx| this.clear_storage(k, cx)))
                    .child(t("sp.storage.clear")),
            )
    });
    let parts: Vec<(f32, Hsla)> = StorageKind::ALL
        .into_iter()
        .enumerate()
        .map(|(i, k)| (stats.get(k) as f32, gpui::rgb(STORAGE_COLORS[i]).into()))
        .collect();
    let (num, unit) = {
        let s = format_bytes(total);
        let mut it = s.splitn(2, ' ');
        (it.next().unwrap_or("0").to_string(), it.next().unwrap_or("B").to_string())
    };
    let breakdown = surface()
        .flex()
        .items_center()
        .gap(px(28.0))
        .child(div().flex_1().min_w_0().flex().flex_col().children(legend))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .relative()
                        .size(px(128.0))
                        .child(donut(parts, 128.0, 16.0))
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .child(div().text_size(px(22.0)).font_weight(FontWeight::BOLD).text_color(gpui::white()).child(num))
                                .child(div().text_size(px(11.0)).text_color(white(0.6)).child(unit)),
                        ),
                )
                .child(div().text_size(px(12.0)).text_color(white(0.6)).child(t("sp.storage.usage"))),
        );

    // Cache limit: stops on a line, the current one highlighted.
    let cur = app.config.storage.cache_limit_mb;
    let stops = LIMITS_MB.into_iter().enumerate().map(|(i, mb)| {
        let on = mb == cur;
        let passed = LIMITS_MB.iter().position(|m| *m == cur).map(|c| i <= c).unwrap_or(false);
        div()
            .id(("limit", i))
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(10.0))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.set_cache_limit(mb, cx)))
            .child(
                div()
                    .h(px(14.0))
                    .flex()
                    .items_center()
                    .child(div().size(px(if on { 14.0 } else { 6.0 })).rounded_full().bg(if passed { gpui::white() } else { white(0.3) })),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .when(on, |d| d.font_weight(FontWeight::SEMIBOLD).text_color(gpui::white()))
                    .when(!on, |d| d.text_color(white(0.55)))
                    .child(limit_label(mb)),
            )
    });
    let limit = surface()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(div().text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(t("sp.storage.limit")))
        .child(div().text_size(px(12.5)).text_color(white(0.55)).child(t("sp.storage.limit.desc")))
        .child(
            div()
                .relative()
                .pt(px(18.0))
                // The track line behind the stops.
                .child(div().absolute().top(px(24.0)).left(px(40.0)).right(px(40.0)).h(px(2.0)).bg(white(0.18)))
                .child(div().flex().children(stops)),
        );

    let data_dir = glyph_core::paths::data_dir();
    let downloads_dir = glyph_core::storage::downloads_dir();
    div()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .pt(px(6.0))
        .child(breakdown)
        .child(limit)
        .child(rows(vec![
            row(
                t("sp.storage.downloads"),
                Some(t("sp.storage.downloads.desc")),
                button("open-downloads", t("sp.storage.open")).on_click(move |_, _, cx| {
                    let _ = std::fs::create_dir_all(&downloads_dir);
                    cx.reveal_path(&downloads_dir)
                }),
            ),
            row(
                t("sp.support.data"),
                Some(t("sp.support.data.desc")),
                button("open-data", t("sp.storage.open")).on_click(move |_, _, cx| cx.reveal_path(&data_dir)),
            ),
        ]))
}

fn about(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let u = &app.update;
    let status: (String, Hsla) = if u.checking {
        (format!("{} {}", app.spinner(), t("update.checking")), white(0.6))
    } else if let Some((done, total)) = u.installing {
        let pct = if total > 0 { done * 100 / total } else { 0 };
        (tf("update.downloading", &[("p", &pct)]).to_string(), gpui::white())
    } else if u.installed {
        (t("update.installed").to_string(), theme::ok())
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
        .filter(|i| i.can_self_install && !app.update.installed && app.update.installing.is_none());
    let prerelease = app.config.updates.channel == "prerelease";

    let card = surface()
        .mt(px(10.0))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(10.0))
        .py(px(28.0))
        .child(gpui::img("icons/app/icon-256.png").size(px(64.0)).rounded(px(14.0)))
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
            mono()
                .text_size(px(11.5))
                .text_color(white(0.5))
                .child(tf("sp.about.client", &[("v", &env!("CARGO_PKG_VERSION")), ("p", &glyph_protocol::PROTOCOL_VERSION), ("target", &glyph_core::updater::current_target())])),
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
                        .hover(|s| s.bg(white(0.13)))
                        .text_size(px(13.5))
                        .text_color(gpui::white())
                        .on_click(cx.listener(|this, _, _, cx| this.check_updates(true, cx)))
                        .child(icon("refresh-cw", 14.0, white(0.85)))
                        .child(t("update.check")),
                )
                .when_some(installable, |d, info| {
                    d.child(primary_button("upd-install", t("update.install")).on_click(cx.listener(move |this, _, _, cx| this.install_update(info.clone(), cx))))
                })
                .when(app.update.installed, |d| {
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

fn support(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
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
