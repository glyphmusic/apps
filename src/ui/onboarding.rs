//! First start: a short tour of what Glyph does, one idea per page, each with
//! its own small live illustration to play with (sources to switch, hearts to
//! tap, an equalizer to set, a device to pick, a colour for the whole app).
//! It follows the splash on a fresh install, on desktop and phones; Settings
//! › About opens it again. Desktop: arrow keys, Enter, Escape. Phones: swipe,
//! and the back gesture goes one page back.

use crate::app::Glyph;
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::compact;
use crate::ui::widgets::*;
use gpui::{actions, div, prelude::*, px, AnimationExt, AnyElement, Context, Div, ElementId, FocusHandle, FontWeight, Hsla, MouseButton, Transformation, Window};
use std::f32::consts::PI;
use std::time::{Duration, Instant};

actions!(onboarding, [OnbNext, OnbBack, OnbSkip]);

/// Key context of the tour: its keys win over the app's while it is open.
pub const CONTEXT: &str = "Onboarding";
/// Marker in the data folder: the tour was finished or skipped.
const SEEN: &str = "onboarding-seen";
const CLOSE: Duration = Duration::from_millis(240);

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Welcome,
    Search,
    Library,
    Sound,
    Lyrics,
    Devices,
    Theme,
}

const PAGES: [Page; 7] = [Page::Welcome, Page::Search, Page::Library, Page::Sound, Page::Lyrics, Page::Devices, Page::Theme];

impl Page {
    fn key(self) -> &'static str {
        match self {
            Page::Welcome => "welcome",
            Page::Search => "search",
            Page::Library => "library",
            Page::Sound => "sound",
            Page::Lyrics => "lyrics",
            Page::Devices => "devices",
            Page::Theme => "theme",
        }
    }
}

// ---- the pages' toys ---------------------------------------------------------

/// Demo sources: name and the hue of their covers.
const SOURCES: [(&str, f32); 5] = [("Audius", 0.83), ("Archive", 0.58), ("Deezer", 0.75), ("Jamendo", 0.08), ("YouTube", 0.0)];
const QUERY: &str = "night drive";
/// Search results: title, artist, source.
const SONGS: [(&str, &str, usize); 6] = [
    ("Night Drive", "Neon Harbor", 0),
    ("Night Drive (Live)", "Neon Harbor", 1),
    ("Night Drive", "Lumen Bay", 3),
    ("Drive at Night", "Mira Sol", 2),
    ("Night Drive (Remix)", "Polar Echo", 4),
    ("Nightdrive", "Quiet Arcade", 0),
];
/// Library rows: title, artist, cover hue.
const LIB_SONGS: [(&str, &str, f32); 3] = [("Night Drive", "Neon Harbor", 0.83), ("Paper Moons", "Mira Sol", 0.08), ("Low Tide", "Polar Echo", 0.55)];
const MOSAIC: [f32; 4] = [0.83, 0.08, 0.55, 0.3];
const SHARE_CODE: &str = "glyph:playlist:7Kq2mX";
const BANDS: [&str; 10] = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
const BLOCKS: usize = 10;
const PRESETS: [(&str, [f32; 10]); 4] = [
    ("eq.preset.flat", [0.5; 10]),
    ("eq.preset.bass_boost", [0.95, 0.85, 0.72, 0.6, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5]),
    ("eq.preset.vocal", [0.35, 0.4, 0.45, 0.6, 0.75, 0.82, 0.72, 0.6, 0.5, 0.42]),
    ("eq.preset.electronic", [0.85, 0.75, 0.55, 0.42, 0.38, 0.5, 0.62, 0.72, 0.82, 0.88]),
];
/// Lyrics lines (their widths) and how long each one is sung.
const LINES: [f32; 10] = [0.62, 0.84, 0.5, 0.92, 0.7, 0.46, 0.8, 0.66, 0.9, 0.56];
const LINE_S: f32 = 1.7;
const DEVICES: [&str; 4] = ["laptop", "smartphone", "tv", "speaker"];

pub struct Onboarding {
    pub focus: FocusHandle,
    page: usize,
    /// The page before the last move and that move's direction (+1 / -1).
    prev: usize,
    dir: f32,
    /// When the current page came up: its illustration's clock.
    page_at: Instant,
    /// When the tour came up (None while the splash still runs).
    shown_at: Option<Instant>,
    closing: Option<Instant>,
    /// Open Settings › Transfer music once closed.
    then_migrate: bool,
    /// Search: sources switched on (bits of `SOURCES`); changed: results come in again.
    sources: u8,
    results_at: Option<Instant>,
    /// Library: liked rows (bits of `LIB_SONGS`), when each was liked, copy pressed.
    likes: u8,
    liked_at: [Option<Instant>; 3],
    auto_liked: bool,
    copied_at: Option<Instant>,
    /// Sound: band levels 0..1 (target and on screen), the preset picked.
    eq: [f32; 10],
    eq_shown: [f32; 10],
    eq_tick: Instant,
    preset: Option<usize>,
    /// Lyrics: song time = base + time since `lyrics_at`, or where it was paused.
    lyrics_base: f32,
    lyrics_at: Instant,
    lyrics_paused: Option<f32>,
    /// Devices: the one playing, picked when.
    device: usize,
    device_at: Option<Instant>,
    /// Phones: how far the current pan went sideways.
    swipe: Option<f32>,
}

impl Onboarding {
    fn new(cx: &mut Context<Glyph>) -> Self {
        let now = Instant::now();
        Self {
            focus: cx.focus_handle(),
            page: 0,
            prev: 0,
            dir: 0.0,
            page_at: now,
            shown_at: None,
            closing: None,
            then_migrate: false,
            sources: 0b1_1111,
            results_at: None,
            likes: 0b100,
            liked_at: [None; 3],
            auto_liked: false,
            copied_at: None,
            eq: PRESETS[3].1,
            eq_shown: [0.0; 10],
            eq_tick: now,
            preset: Some(3),
            lyrics_base: LINE_S * 3.0,
            lyrics_at: now,
            lyrics_paused: None,
            device: 0,
            device_at: None,
            swipe: None,
        }
    }

    /// The tour on a fresh install (no config file yet), once.
    /// `GLYPH_ONBOARDING=1` shows it anyway (testing).
    pub fn at_start(cx: &mut Context<Glyph>) -> Option<Self> {
        let forced = std::env::var_os("GLYPH_ONBOARDING").is_some_and(|v| v == "1");
        let seen = glyph_core::paths::data_dir().join(SEEN).exists();
        let fresh = !glyph_core::Config::path().exists();
        (forced || (fresh && !seen)).then(|| Self::new(cx))
    }

    fn song_time(&self) -> f32 {
        self.lyrics_paused.unwrap_or_else(|| self.lyrics_base + self.lyrics_at.elapsed().as_secs_f32())
    }
}

impl Glyph {
    /// Settings › About › Introduction: the tour again.
    pub fn open_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_open {
            self.close_settings(window, cx);
        }
        self.onboarding = Some(Onboarding::new(cx));
        cx.notify();
    }

    fn onb_go(&mut self, to: usize, cx: &mut Context<Self>) {
        let Some(o) = self.onboarding.as_mut() else { return };
        if to == o.page || to >= PAGES.len() || o.closing.is_some() {
            return;
        }
        o.dir = if to > o.page { 1.0 } else { -1.0 };
        o.prev = o.page;
        o.page = to;
        o.page_at = Instant::now();
        // Coming back replays the page's intro.
        match PAGES[to] {
            Page::Search => o.results_at = None,
            Page::Sound => o.eq_shown = [0.0; 10],
            _ => {}
        }
        reveal_restart(&title_key(to));
        cx.notify();
    }

    fn onb_page(&self) -> Option<usize> {
        self.onboarding.as_ref().filter(|o| o.closing.is_none()).map(|o| o.page)
    }

    /// Phones: a sideways swipe turns the page. Touch pans arrive as scroll
    /// steps (taps stay taps, so the buttons keep working); the fling's
    /// momentum after the release is ignored.
    fn onb_swipe(&mut self, e: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        let Some(o) = self.onboarding.as_mut() else { return };
        let dx = f32::from(e.delta.pixel_delta(px(16.0)).x);
        match e.touch_phase {
            gpui::TouchPhase::Started => o.swipe = Some(dx),
            gpui::TouchPhase::Moved => {
                if let Some(s) = o.swipe.as_mut() {
                    *s += dx;
                }
            }
            gpui::TouchPhase::Cancelled => o.swipe = None,
            gpui::TouchPhase::Ended => {
                let Some(s) = o.swipe.take().map(|s| s + dx) else { return };
                // A swipe never finishes the tour: that is the button's job.
                match self.onb_page() {
                    Some(p) if s < -56.0 && p + 1 < PAGES.len() => self.onb_go(p + 1, cx),
                    Some(_) if s > 56.0 => self.onb_back(cx),
                    _ => {}
                }
            }
        }
    }

    fn onb_next(&mut self, cx: &mut Context<Self>) {
        match self.onb_page() {
            Some(p) if p + 1 >= PAGES.len() => self.onb_finish(false, cx),
            Some(p) => self.onb_go(p + 1, cx),
            None => {}
        }
    }

    fn onb_back(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.onb_page().filter(|p| *p > 0) {
            self.onb_go(p - 1, cx);
        }
    }

    /// Escape skips the tour; the phones' back gesture goes one page back.
    fn onb_escape(&mut self, cx: &mut Context<Self>) {
        if compact::TOUCH && self.onb_page().is_some_and(|p| p > 0) {
            self.onb_back(cx);
        } else {
            self.onb_finish(false, cx);
        }
    }

    fn onb_finish(&mut self, migrate: bool, cx: &mut Context<Self>) {
        let Some(o) = self.onboarding.as_mut() else { return };
        if o.closing.is_some() {
            return;
        }
        o.closing = Some(Instant::now());
        o.then_migrate = migrate;
        let _ = glyph_core::paths::write_atomic(&glyph_core::paths::data_dir().join(SEEN), b"1");
        cx.notify();
    }
}

fn title_key(page: usize) -> String {
    format!("onb-title-{page}")
}

fn white(a: f32) -> Hsla {
    gpui::hsla(0.0, 0.0, 1.0, a)
}

fn ink(a: f32) -> Hsla {
    gpui::hsla(0.0, 0.0, 0.0, a)
}

/// Progress 0..1 of a step `dur` long that starts at `start`.
fn k(s: f32, start: f32, dur: f32) -> f32 {
    ((s - start) / dur).clamp(0.0, 1.0)
}

fn ease_out(x: f32) -> f32 {
    1.0 - (1.0 - x.clamp(0.0, 1.0)).powi(3)
}

fn ease_in_out(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if x < 0.5 {
        4.0 * x * x * x
    } else {
        1.0 - (-2.0 * x + 2.0).powi(3) / 2.0
    }
}

/// Overshoots a little, then settles: things pop in.
fn back_out(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0) - 1.0;
    let c = 1.7;
    1.0 + (c + 1.0) * x * x * x + c * x * x
}

fn cover_tile(size: f32, hue: f32) -> Div {
    div().flex_none().size(px(size)).rounded(px(size * 0.2)).bg(gpui::hsla(hue, 0.45, 0.42, 1.0))
}

fn chip(id: impl Into<ElementId>, on: bool) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(24.0))
        .px(px(10.0))
        .rounded_full()
        .bg(white(if on { 0.11 } else { 0.03 }))
        .cursor_pointer()
        .hover(|st| st.bg(white(0.14)))
}

/// The tour over everything (after the splash), or nothing.
pub fn layer(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    if app.splash.is_some() {
        return None;
    }
    // Closed: the keys go back to the app, then what the last page asked for.
    if app.onboarding.as_ref().is_some_and(|o| o.closing.is_some_and(|c| c.elapsed() >= CLOSE)) {
        let migrate = app.onboarding.take().is_some_and(|o| o.then_migrate);
        let f = app.focus.clone();
        window.focus(&f, cx);
        if migrate {
            app.open_migrate(cx);
        }
        return None;
    }
    let theme_id = app.config.visuals.theme.clone();
    let still = cx.reduce_motion();
    let o = app.onboarding.as_mut()?;
    let now = Instant::now();
    if o.shown_at.is_none() {
        o.shown_at = Some(now);
        o.page_at = now;
        o.lyrics_at = now;
        reveal_restart(&title_key(o.page));
    }
    if o.closing.is_none() && !o.focus.is_focused(window) {
        let f = o.focus.clone();
        window.focus(&f, cx);
    }
    let page = PAGES[o.page];
    let s = if still { 100.0 } else { o.page_at.elapsed().as_secs_f32() };

    // The pages' own motion: the first heart taps itself, the bands ease.
    if page == Page::Library && !o.auto_liked && s > 1.1 {
        o.auto_liked = true;
        if o.likes & 1 == 0 {
            o.likes |= 1;
            o.liked_at[0] = Some(now);
        }
    }
    let dt = o.eq_tick.elapsed().as_secs_f32().min(0.1);
    o.eq_tick = now;
    let a = if still { 1.0 } else { 1.0 - (-dt / 0.09).exp() };
    for (shown, target) in o.eq_shown.iter_mut().zip(o.eq) {
        *shown += (target - *shown) * a;
    }

    let touch = compact::TOUCH;
    let n = PAGES.len();
    let last = o.page + 1 == n;
    let vp = window.viewport_size();
    let (vw, vh) = (f32::from(vp.width), f32::from(vp.height));
    let card_w = if touch { vw } else { (vw - 48.0).min(640.0) };
    let pad = if touch { 20.0 } else { 30.0 };
    let stage_w = card_w - pad * 2.0;
    let stage_h = if touch { (vh * 0.36).clamp(220.0, 330.0) } else { (vh - 400.0).clamp(200.0, 250.0) };
    let open = if still { 1.0 } else { ease_out(o.shown_at.map_or(0.0, |at| at.elapsed().as_secs_f32()) / 0.32) };
    let close = o.closing.map_or(1.0, |c| if still { 0.0 } else { 1.0 - ease_out(c.elapsed().as_secs_f32() / CLOSE.as_secs_f32()) });
    let vis = open.min(close);

    let o: &Onboarding = o;
    let art = match page {
        Page::Welcome => welcome(s, stage_h),
        Page::Search => search(o, s, still, stage_h, cx),
        Page::Library => library(o, s, stage_w, cx),
        Page::Sound => sound(o, s, still, stage_w, stage_h, cx),
        Page::Lyrics => lyrics(o, still, stage_w, stage_h, cx),
        Page::Devices => devices(o, s, still, stage_w, stage_h, cx),
        Page::Theme => themes(s, still, &theme_id, stage_w, cx),
    };
    let stage = div().relative().w(px(stage_w)).h(px(stage_h)).rounded(px(14.0)).bg(white(0.035)).overflow_hidden().child(art);

    let title = t(&format!("onboarding.{}.title", page.key()));
    let body = t(&format!("onboarding.{}.body", page.key()));
    let text = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(div().text_size(px(if touch { 24.0 } else { 22.0 })).font_weight(FontWeight::BOLD).text_color(white(0.96)).child(decode(&title, reveal_k(&title_key(o.page), 0.5))))
        .child(
            div()
                .min_h(px(if touch { 88.0 } else { 63.0 }))
                .text_size(px(if touch { 15.0 } else { 14.0 }))
                .line_height(px(if touch { 22.0 } else { 21.0 }))
                .text_color(white(0.66))
                .child(body),
        );
    // The page slides in from the side it comes from.
    let block = div().relative().flex().flex_col().gap(px(22.0)).child(stage).child(text);
    let block = if still {
        block.into_any_element()
    } else {
        let dir = o.dir;
        block
            .with_animation(
                ElementId::NamedInteger("onb-page".into(), (o.page * 2 + usize::from(dir > 0.0)) as u64),
                gpui::Animation::new(Duration::from_millis(360)),
                move |d, x| {
                    let e = ease_out(x);
                    d.opacity(e).left(px(dir * 36.0 * (1.0 - e)))
                },
            )
            .into_any_element()
    };

    let step = div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(mono().text_size(px(11.5)).text_color(white(0.45)).child(format!("{:02} / {:02}", o.page + 1, n)))
        .when(page != Page::Welcome, |d| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .opacity(ease_out(k(s, 0.9, 0.4)))
                    .child(icon("sparkles", 12.0, theme::pink()))
                    .child(div().text_size(px(12.0)).text_color(white(0.55)).child(t("onboarding.try"))),
            )
        });
    let skip: AnyElement = if last {
        div().h(px(24.0)).into_any_element()
    } else {
        text_button("onb-skip", t("onboarding.skip")).text_size(px(12.5)).on_click(cx.listener(|this, _, _, cx| this.onb_finish(false, cx))).into_any_element()
    };
    let top = div().flex().items_center().justify_between().child(step).child(skip);

    let switch_k = if still { 1.0 } else { ease_out(s / 0.3) };
    let dots = div().flex().items_center().children((0..n).map(|i| {
        let a = if i == o.page {
            switch_k
        } else if i == o.prev {
            1.0 - switch_k
        } else {
            0.0
        };
        div()
            .id(("onb-dot", i))
            .py(px(8.0))
            .px(px(3.0))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.onb_go(i, cx)))
            .child(div().h(px(6.0)).w(px(6.0 + 16.0 * a)).rounded_full().bg(mix(white(0.2), theme::pink(), a)))
    }));
    let next_label = if last { t("onboarding.start") } else { t("onboarding.next") };
    let next = primary_button("onb-next", next_label)
        .child(icon(if last { "check" } else { "arrow-right" }, 14.0, ink(0.9)))
        .on_click(cx.listener(|this, _, _, cx| this.onb_next(cx)));
    let migrate = last.then(|| {
        button("onb-migrate", t("onboarding.migrate"))
            .child(icon("arrow-right-left", 14.0, white(0.7)))
            .on_click(cx.listener(|this, _, _, cx| this.onb_finish(true, cx)))
    });
    let focus = o.focus.clone();

    let content: AnyElement = if touch {
        let back = (o.page > 0).then(|| {
            div()
                .id("onb-back")
                .flex_none()
                .size(px(48.0))
                .rounded_full()
                .bg(white(0.08))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .child(icon("chevron-left", 20.0, white(0.85)))
                .on_click(cx.listener(|this, _, _, cx| this.onb_back(cx)))
        });
        let footer = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(div().flex().justify_center().child(dots))
            .children(migrate.map(|m| m.h(px(44.0)).rounded(px(22.0)).justify_center()))
            .child(div().flex().items_center().gap(px(10.0)).children(back).child(next.flex_1().h(px(48.0)).rounded(px(24.0)).justify_center()));
        let safe = window.insets().safe_area;
        div()
            .id("onb-sheet")
            .absolute()
            .inset_0()
            .occlude()
            // Opaque: Android blends in linear light, where even 4% of the
            // screen behind reads as a clear ghost of it.
            .bg(gpui::hsla(0.0, 0.0, 0.02, vis))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(cx.listener(|this, e: &gpui::ScrollWheelEvent, _, cx| {
                cx.stop_propagation();
                this.onb_swipe(e, cx);
            }))
            .child(
                div()
                    .size_full()
                    .opacity(vis)
                    .flex()
                    .flex_col()
                    .px(px(pad))
                    .pt(safe.top + px(14.0))
                    .pb(safe.bottom + px(16.0))
                    .font_family(theme::FONT)
                    .child(top)
                    .child(div().mt(px(14.0)).child(block))
                    .child(div().flex_1())
                    .child(footer),
            )
            .into_any_element()
    } else {
        let buttons = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .children((o.page > 0).then(|| button("onb-back", t("onboarding.back")).on_click(cx.listener(|this, _, _, cx| this.onb_back(cx)))))
            .children(migrate)
            .child(next);
        let card = div()
            .id("onb-card")
            .relative()
            .top(px(16.0 * (1.0 - vis)))
            .opacity(vis)
            .w(px(card_w))
            .p(px(pad))
            .rounded(px(18.0))
            .bg(gpui::hsla(0.0, 0.0, 0.075, 0.985))
            .flex()
            .flex_col()
            .gap(px(18.0))
            .font_family(theme::FONT)
            .child(top)
            .child(block)
            .child(div().flex().items_center().justify_between().child(dots).child(buttons));
        let backdrop = modal_backdrop("onb-backdrop", 0.78 * vis)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, w, cx| {
                    // Clicks inside keep the keys here (the app would take the focus back).
                    if let Some(f) = this.onboarding.as_ref().map(|o| o.focus.clone()) {
                        w.focus(&f, cx);
                    }
                    cx.stop_propagation();
                }),
            )
            .child(card);
        modal_layer(0.78 * vis, backdrop).into_any_element()
    };

    Some(
        theme::on_panel(|| {
            div()
                .absolute()
                .inset_0()
                .track_focus(&focus)
                .key_context(CONTEXT)
                .on_action(cx.listener(|this, _: &OnbNext, _, cx| this.onb_next(cx)))
                .on_action(cx.listener(|this, _: &OnbBack, _, cx| this.onb_back(cx)))
                .on_action(cx.listener(|this, _: &OnbSkip, _, cx| this.onb_escape(cx)))
                .child(content)
        })
        .into_any_element(),
    )
}

// ---- illustrations ----------------------------------------------------------

/// The note assembles out of its characters and dodges the mouse.
fn welcome(s: f32, h: f32) -> AnyElement {
    let row_h = ((h - 34.0) / 33.0).clamp(3.5, 6.0);
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .child(crate::ui::splash::note_logo_flying(row_h))
        .when(!compact::TOUCH, |d| d.child(mono().text_size(px(11.0)).text_color(white(0.4 * ease_out(k(s, 2.2, 0.6)))).child(t("onboarding.welcome.hint"))))
        .into_any_element()
}

/// A query types itself, the sources search, results from all of them fall
/// into one list. The chips switch sources on and off.
fn search(o: &Onboarding, s: f32, still: bool, h: f32, cx: &mut Context<Glyph>) -> AnyElement {
    let typing_done = 0.3 + QUERY.len() as f32 * 0.075;
    let replay = o.results_at.is_some() || still;
    let typed = if replay { QUERY.len() } else { (((s - 0.3) / 0.075).max(0.0) as usize).min(QUERY.len()) };
    let caret = !replay && s < typing_done + 0.5 && ((s * 2.4) as i32) % 2 == 0;
    let searching = !replay && s > typing_done && s < typing_done + 0.55;
    let field = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .h(px(36.0))
        .px(px(12.0))
        .rounded(px(10.0))
        .bg(white(0.06))
        .child(icon("search", 15.0, white(0.5)))
        .child(
            div()
                .flex()
                .items_center()
                .text_size(px(13.5))
                .text_color(white(0.92))
                .child(QUERY[..typed].to_string())
                .child(div().ml(px(1.0)).w(px(1.5)).h(px(16.0)).bg(theme::pink()).opacity(if caret { 1.0 } else { 0.0 })),
        );
    let chips = div().flex().flex_wrap().gap(px(6.0)).children(SOURCES.iter().enumerate().map(|(i, &(name, hue))| {
        let on = o.sources & (1 << i) != 0;
        let pulse = if searching { 0.35 + 0.65 * ((s - typing_done) * 9.0 + i as f32 * 0.8).sin().abs() } else { 1.0 };
        chip(("onb-src", i), on)
            .opacity(ease_out(k(s, 0.05 * i as f32, 0.35)))
            .child(div().size(px(6.0)).rounded_full().bg(if on { gpui::hsla(hue, 0.75, 0.62, pulse) } else { white(0.18) }))
            .child(div().text_size(px(11.5)).text_color(white(if on { 0.88 } else { 0.35 })).child(name))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(o) = this.onboarding.as_mut() {
                    o.sources ^= 1 << i;
                    o.results_at = Some(Instant::now());
                }
                cx.notify();
            }))
    }));
    let since = match o.results_at {
        Some(at) => at.elapsed().as_secs_f32(),
        None if still => 100.0,
        None => s - typing_done - 0.55,
    };
    let room = ((h - 36.0 - 24.0 - 36.0 - 20.0) / 32.0).floor().max(1.0) as usize;
    let rows: AnyElement = if searching {
        div().flex().flex_col().gap(px(8.0)).pt(px(4.0)).children((0..room.min(3)).map(|i| skeleton(format!("onb-sk-{i}"), 160.0 + 50.0 * (i % 2) as f32, 18.0, 5.0))).into_any_element()
    } else {
        let shown: Vec<_> = SONGS.iter().filter(|x| o.sources & (1 << x.2) != 0).take(room).collect();
        if shown.is_empty() && since > 0.0 {
            mono().pt(px(10.0)).text_size(px(11.0)).text_color(white(0.4)).child(format!("[ {} ]", t("onboarding.search.none"))).into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .children(shown.into_iter().enumerate().map(|(j, &(title, artist, src))| {
                    let e = ease_out(k(since, j as f32 * 0.08, 0.3));
                    div()
                        .relative()
                        .top(px(8.0 * (1.0 - e)))
                        .opacity(e)
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .h(px(28.0))
                        .child(cover_tile(24.0, SOURCES[src].1))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .flex()
                                .items_center()
                                .gap(px(7.0))
                                .child(div().text_size(px(13.0)).text_color(white(0.9)).child(title))
                                .child(div().text_size(px(12.0)).text_color(white(0.42)).child(artist)),
                        )
                        .child(mono().flex_none().text_size(px(10.0)).text_color(white(0.45)).child(format!("[ {} ]", SOURCES[src].0.to_uppercase())))
                }))
                .into_any_element()
        }
    };
    div().size_full().flex().flex_col().gap(px(10.0)).px(px(20.0)).py(px(18.0)).child(field).child(chips).child(rows).into_any_element()
}

/// Songs with hearts to tap, a playlist cover that assembles from its
/// songs' covers and the code that shares it.
fn library(o: &Onboarding, s: f32, w: f32, cx: &mut Context<Glyph>) -> AnyElement {
    let narrow = w < 460.0;
    let rows = if narrow { 2 } else { 3 };
    let songs = div().when(!narrow, |d| d.flex_1().max_w(px(290.0))).min_w_0().flex().flex_col().gap(px(6.0)).children(LIB_SONGS.iter().take(rows).enumerate().map(|(i, &(title, artist, hue))| {
        let appear = ease_out(k(s, 0.08 * i as f32, 0.35));
        let liked = o.likes & (1 << i) != 0;
        let pk = o.liked_at[i].map_or(1.0, |at| (at.elapsed().as_secs_f32() / 0.5).min(1.0));
        let popping = liked && pk < 1.0;
        let scale = if popping { 1.0 + 0.5 * (pk * PI).sin() * (1.0 - pk) } else { 1.0 };
        // A small burst around the heart while it pops.
        let burst = (0..6).filter(|_| popping).map(move |d| {
            let ang = d as f32 * PI / 3.0 - PI / 2.0;
            let r = 7.0 + 13.0 * ease_out(pk);
            div().absolute().left(px(15.0 + r * ang.cos() - 1.75)).top(px(15.0 + r * ang.sin() - 1.75)).size(px(3.5)).rounded_full().bg(theme::pink()).opacity(1.0 - pk)
        });
        let heart = div()
            .id(("onb-like", i))
            .relative()
            .flex_none()
            .size(px(30.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .cursor_pointer()
            .hover(|st| st.bg(white(0.08)))
            .child(icon(if liked { "heart-fill" } else { "heart" }, 16.0, if liked { theme::pink() } else { white(0.45) }).with_transformation(Transformation::scale(gpui::size(scale, scale))))
            .children(burst)
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(o) = this.onboarding.as_mut() {
                    o.auto_liked = true;
                    o.likes ^= 1 << i;
                    o.liked_at[i] = (o.likes & (1 << i) != 0).then(Instant::now);
                }
                cx.notify();
            }));
        div()
            .relative()
            .left(px(-10.0 * (1.0 - appear)))
            .opacity(appear)
            .flex()
            .items_center()
            .gap(px(10.0))
            .h(px(42.0))
            .pl(px(8.0))
            .pr(px(4.0))
            .rounded(px(10.0))
            .bg(white(0.04))
            .child(cover_tile(28.0, hue))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(12.5)).text_color(white(0.9)).child(title))
                    .child(div().text_size(px(11.0)).text_color(white(0.42)).child(artist)),
            )
            .child(heart)
    }));

    let m = if narrow { 64.0 } else { 88.0 };
    let tile = (m - 3.0) / 2.0;
    let mosaic = div().relative().flex_none().size(px(m)).children((0..4).map(|i| {
        let (col, row) = ((i % 2) as f32, (i / 2) as f32);
        let e = ease_out(k(s, 0.35 + 0.1 * i as f32, 0.5));
        let (dx, dy) = ((col * 2.0 - 1.0) * 26.0, (row * 2.0 - 1.0) * 26.0);
        div()
            .absolute()
            .left(px(col * (tile + 3.0) + dx * (1.0 - e)))
            .top(px(row * (tile + 3.0) + dy * (1.0 - e)))
            .size(px(tile))
            .rounded(px(5.0))
            .opacity(e)
            .bg(gpui::hsla(MOSAIC[i], 0.5, 0.45, 1.0))
    }));
    let copied = o.copied_at.is_some_and(|at| at.elapsed() < Duration::from_millis(1600));
    let code = div()
        .id("onb-copy")
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(26.0))
        .px(px(8.0))
        .rounded(px(7.0))
        .bg(white(0.05))
        .cursor_pointer()
        .hover(|st| st.bg(white(0.09)))
        .opacity(ease_out(k(s, 1.0, 0.3)))
        .child(mono().text_size(px(10.5)).text_color(white(0.7)).child(decode(SHARE_CODE, k(s, 1.1, 0.8))))
        .child(icon(if copied { "check" } else { "copy" }, 12.0, if copied { theme::ok() } else { white(0.5) }))
        .on_click(cx.listener(|this, _, _, cx| {
            if let Some(o) = this.onboarding.as_mut() {
                o.copied_at = Some(Instant::now());
            }
            cx.notify();
        }));
    let name = div()
        .flex()
        .flex_col()
        .when(!narrow, |d| d.items_center())
        .opacity(ease_out(k(s, 0.8, 0.3)))
        .child(div().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(white(0.9)).child("Roadtrip"))
        .child(div().text_size(px(11.0)).text_color(white(0.42)).child(tf("common.n_songs", &[("n", &24)])));
    let list: AnyElement = if narrow {
        div().flex().items_center().gap(px(12.0)).child(mosaic).child(div().flex().flex_col().gap(px(8.0)).child(name).child(code)).into_any_element()
    } else {
        div().flex_none().flex().flex_col().items_center().gap(px(8.0)).child(mosaic).child(name).child(code).into_any_element()
    };
    let wrap = div().size_full().flex().px(px(20.0)).py(px(18.0)).gap(px(18.0)).justify_center();
    let wrap = if narrow { wrap.flex_col() } else { wrap.items_center() };
    wrap.child(songs).child(list).into_any_element()
}

/// A 10-band equalizer that moves with the "music": click a block to set a
/// band, or a preset.
fn sound(o: &Onboarding, s: f32, still: bool, w: f32, h: f32, cx: &mut Context<Glyph>) -> AnyElement {
    let inner_w = (w - 48.0).min(430.0);
    let gap = 4.0;
    let bw = ((inner_w - gap * 9.0) / 10.0).min(30.0);
    let pitch = ((h - 36.0 - 24.0 - 16.0 - 26.0) / BLOCKS as f32).clamp(8.0, 14.0);
    let presets = div().flex().flex_wrap().justify_center().gap(px(6.0)).children(PRESETS.iter().enumerate().map(|(i, &(key, levels))| {
        chip(("onb-preset", i), o.preset == Some(i))
            .child(div().text_size(px(11.5)).text_color(white(if o.preset == Some(i) { 0.9 } else { 0.5 })).child(t(key)))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(o) = this.onboarding.as_mut() {
                    o.eq = levels;
                    o.preset = Some(i);
                }
                cx.notify();
            }))
    }));
    let bands = div().flex().items_end().gap(px(gap)).children((0..10).map(|b| {
        let wobble = if still { 0.0 } else { 0.07 * (s * 5.1 + b as f32 * 1.7).sin() * (s * 2.3 + b as f32 * 0.6).sin() };
        let lit = ((o.eq_shown[b] + wobble) * BLOCKS as f32).round().clamp(0.0, BLOCKS as f32) as usize;
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(5.0))
            .child(div().flex().flex_col().gap(px(3.0)).children((0..BLOCKS).rev().map(|j| {
                let c = if j < lit { mix(white(0.8), theme::pink(), j as f32 / (BLOCKS - 1) as f32) } else { white(0.06) };
                div()
                    .id(("onb-eq", b * BLOCKS + j))
                    .w(px(bw))
                    .h(px(pitch - 3.0))
                    .rounded(px(2.5))
                    .bg(c)
                    .cursor_pointer()
                    .hover(|st| st.opacity(0.7))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(o) = this.onboarding.as_mut() {
                            o.eq[b] = (j + 1) as f32 / BLOCKS as f32;
                            o.preset = None;
                        }
                        cx.notify();
                    }))
            })))
            .child(mono().text_size(px(9.5)).text_color(white(0.35)).child(BANDS[b]))
    }));
    div().size_full().flex().flex_col().items_center().justify_center().gap(px(14.0)).child(presets).child(bands).into_any_element()
}

/// Synced lines glide up while the song plays; the cover pauses it.
fn lyrics(o: &Onboarding, still: bool, w: f32, h: f32, cx: &mut Context<Glyph>) -> AnyElement {
    let lt = o.song_time();
    let pos = lt / LINE_S;
    let line = pos.floor();
    let frac = pos - line;
    let scroll = if still { line } else { line + ease_in_out(k(frac, 0.8, 0.2)) };
    let narrow = w < 460.0;
    let c = if narrow { 76.0 } else { 96.0 };
    let view_h = h - 36.0;
    let lines_w = w - 48.0 - c - 24.0;
    let row_pitch = 30.0;
    let lines = div().relative().flex_1().h(px(view_h)).overflow_hidden().children(((line as i32 - 4)..=(line as i32 + 5)).map(|j| {
        let d = j as f32 - scroll;
        let focus = (1.0 - d.abs()).clamp(0.0, 1.0);
        let fade = (1.0 - d.abs() / 4.0).clamp(0.0, 1.0);
        let bw = lines_w * LINES[j.rem_euclid(LINES.len() as i32) as usize];
        let bh = 9.0 + 3.0 * focus;
        let sung = match (j as f32).total_cmp(&line) {
            std::cmp::Ordering::Less => 1.0,
            std::cmp::Ordering::Equal => (frac / 0.8).min(1.0),
            std::cmp::Ordering::Greater => 0.0,
        };
        div()
            .absolute()
            .left_0()
            .top(px(view_h / 2.0 + d * row_pitch - bh / 2.0))
            .w(px(bw))
            .h(px(bh))
            .rounded_full()
            .overflow_hidden()
            .bg(white((0.07 + 0.1 * focus) * fade))
            .child(div().h_full().w(px(bw * sung)).bg(white((0.22 + 0.73 * focus) * fade)))
    }));
    let playing = o.lyrics_paused.is_none();
    let secs = (lt + 52.0) as u32;
    let cover = div()
        .id("onb-ly-cover")
        .flex_none()
        .size(px(c))
        .rounded(px(12.0))
        .bg(theme::accent_tile(0.32, 0.55))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .child(div().size(px(40.0)).rounded_full().bg(white(0.92)).flex().items_center().justify_center().child(icon(if playing { "pause" } else { "play" }, 18.0, ink(0.9))))
        .on_click(cx.listener(|this, _, _, cx| {
            if let Some(o) = this.onboarding.as_mut() {
                match o.lyrics_paused.take() {
                    Some(at) => {
                        o.lyrics_base = at;
                        o.lyrics_at = Instant::now();
                    }
                    None => o.lyrics_paused = Some(o.song_time()),
                }
            }
            cx.notify();
        }));
    let progress = div()
        .w(px(c))
        .h(px(3.0))
        .rounded_full()
        .bg(white(0.1))
        .child(div().h_full().w(px(c * ((lt + 52.0) / 214.0).fract())).rounded_full().bg(theme::pink()));
    let side = div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(cover)
        .child(progress)
        .child(mono().text_size(px(10.5)).text_color(white(0.45)).child(format!("{}:{:02}", secs / 60, secs % 60)));
    div().size_full().flex().items_center().gap(px(24.0)).px(px(24.0)).child(side).child(lines).into_any_element()
}

/// One account in the middle, devices around it, data flowing both ways.
/// Click a device to play there.
fn devices(o: &Onboarding, s: f32, still: bool, w: f32, h: f32, cx: &mut Context<Glyph>) -> AnyElement {
    let (cx0, cy0) = (w / 2.0, h / 2.0 - 8.0);
    let rx = (w * 0.33).min(160.0);
    let ry = (h * 0.3).min(68.0);
    let spots = [(-rx, -ry), (rx, -ry), (-rx, ry), (rx, ry)];
    let mut d = div().relative().size_full();
    for (i, &(dx, dy)) in spots.iter().enumerate() {
        let len = (dx * dx + dy * dy).sqrt();
        let (ux, uy) = (dx / len, dy / len);
        let (from, to) = (36.0, len - 28.0);
        let drawn = ease_out(k(s, 0.25 + 0.1 * i as f32, 0.5));
        for m in 0..10 {
            let f = m as f32 / 9.0;
            if f > drawn {
                break;
            }
            let r = from + (to - from) * f;
            d = d.child(div().absolute().left(px(cx0 + ux * r - 1.5)).top(px(cy0 + uy * r - 1.5)).size(px(3.0)).rounded_full().bg(white(0.16)));
        }
        // Sync goes both ways; the playing device gets the music faster.
        if !still && drawn >= 1.0 {
            let speed = if o.device == i { 0.9 } else { 0.35 };
            let p = (s * speed + i as f32 * 0.27).fract();
            let p = if i % 2 == 0 { p } else { 1.0 - p };
            let r = from + (to - from) * p;
            d = d.child(div().absolute().left(px(cx0 + ux * r - 2.5)).top(px(cy0 + uy * r - 2.5)).size(px(5.0)).rounded_full().bg(theme::pink()).opacity((p * PI).sin()));
        }
    }
    let ak = back_out(k(s, 0.0, 0.45));
    let av = 56.0 * ak;
    d = d
        .child(
            div()
                .absolute()
                .left(px(cx0 - av / 2.0))
                .top(px(cy0 - av / 2.0))
                .size(px(av))
                .rounded_full()
                .bg(white(0.09))
                .flex()
                .items_center()
                .justify_center()
                .child(icon("user-round", 26.0 * ak, white(0.85))),
        )
        .child(
            div()
                .absolute()
                .left(px(cx0 - 60.0))
                .w(px(120.0))
                .top(px(cy0 + 34.0))
                .flex()
                .flex_col()
                .items_center()
                .opacity(ease_out(k(s, 0.4, 0.4)))
                .child(mono().text_size(px(11.0)).text_color(white(0.75)).child("@handle"))
                .child(mono().text_size(px(9.5)).text_color(white(0.4)).child("[ E2E ]")),
        );
    for (i, &(dx, dy)) in spots.iter().enumerate() {
        let nk = back_out(k(s, 0.2 + 0.09 * i as f32, 0.45));
        if nk <= 0.01 {
            continue;
        }
        let on = o.device == i;
        let size = 44.0 * nk;
        let (x, y) = (cx0 + dx, cy0 + dy);
        let ripple = o.device_at.filter(|_| on).map(|at| at.elapsed().as_secs_f32() / 0.6).filter(|e| *e < 1.0);
        if let Some(e) = ripple {
            let r = 44.0 + 40.0 * ease_out(e);
            d = d.child(div().absolute().left(px(x - r / 2.0)).top(px(y - r / 2.0)).size(px(r)).rounded_full().border_2().border_color(white(0.5 * (1.0 - e))));
        }
        d = d.child(
            div()
                .id(("onb-dev", i))
                .absolute()
                .left(px(x - size / 2.0))
                .top(px(y - size / 2.0))
                .size(px(size))
                .rounded_full()
                .bg(if on { theme::pink() } else { white(0.07) })
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|st| st.opacity(0.85))
                .child(icon(DEVICES[i], 20.0 * nk, if on { ink(0.85) } else { white(0.8) }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(o) = this.onboarding.as_mut() {
                        o.device = i;
                        o.device_at = Some(Instant::now());
                    }
                    cx.notify();
                })),
        );
    }
    d.into_any_element()
}

/// A small player in the current colours; the swatches recolour the whole
/// app (and the glow behind the tour) right away.
fn themes(s: f32, still: bool, current: &str, w: f32, cx: &mut Context<Glyph>) -> AnyElement {
    let accent = theme::pink();
    let pw = (w - 48.0).min(380.0);
    let prog = if still { 0.4 } else { (0.18 + s * 0.035).fract() };
    let preview = div()
        .w(px(pw))
        .h(px(64.0))
        .px(px(10.0))
        .rounded(px(12.0))
        .bg(white(0.05))
        .flex()
        .items_center()
        .gap(px(12.0))
        .child(div().flex_none().size(px(44.0)).rounded(px(8.0)).bg(theme::accent_tile(0.38, 0.6)).flex().items_center().justify_center().child(icon("music-2", 18.0, white(0.85))))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(div().w(px(110.0)).h(px(8.0)).rounded_full().bg(white(0.75)))
                .child(div().w(px(70.0)).h(px(6.0)).rounded_full().bg(white(0.3)))
                .child(div().w_full().h(px(3.0)).rounded_full().bg(white(0.1)).child(div().h_full().w(gpui::relative(prog)).rounded_full().bg(accent))),
        )
        .child(div().flex_none().size(px(36.0)).rounded_full().bg(accent).flex().items_center().justify_center().child(icon("pause", 16.0, ink(0.9))));
    let swatches = div().flex().items_center().gap(px(6.0)).children(theme::THEMES.iter().enumerate().map(|(i, th)| {
        let on = th.id == current;
        let e = back_out(k(s, 0.15 + 0.05 * i as f32, 0.4));
        let id = th.id;
        div()
            .id(("onb-theme", i))
            .size(px(38.0))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .border_2()
            .border_color(white(if on { 0.9 } else { 0.0 }))
            .child(div().size(px(26.0 * e)).rounded_full().bg(gpui::rgb(th.accent)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_theme(id, cx);
                reveal_restart("onb-theme-name");
            }))
    }));
    let name = theme::THEMES.iter().find(|th| th.id == current).map(|th| t(th.name).to_uppercase()).unwrap_or_default();
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(18.0))
        .child(preview)
        .child(swatches)
        .child(mono().h(px(14.0)).text_size(px(11.0)).text_color(white(0.55)).when(!name.is_empty(), |d| d.child(format!("[ {} ]", decode(&name, reveal_peek("onb-theme-name", 0.4))))))
        .into_any_element()
}
