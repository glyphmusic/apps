//! Small building blocks. Everything is flat: no gradients, no glow, no
//! shadows. ASCII only as accents (badges, spinner, empty states).

use crate::app::{track_key, Glyph};
use crate::i18n::t;
use crate::theme;
use crate::visuals::{ascii, Palette};
use glyph_protocol::{ProviderId, Track};
use gpui::{
    div, img, prelude::*, px, svg, AnimationExt, AnyElement, Context, Div, ElementId, FontWeight, Hsla, ObjectFit,
    SharedString, Stateful, Svg, Window,
};
use std::rc::Rc;
use std::time::Duration;

pub type Pick = Rc<dyn Fn(f32, &mut Window, &mut gpui::App)>;

/// Flat Lucide icon.
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/ui/{name}.svg")))
        .flex_none()
        .size(px(size))
        .text_color(color)
}

/// Round/square icon button with a subtle hover.
pub fn icon_button(id: impl Into<ElementId>, name: &str, size: f32, color: Hsla) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(size + 14.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(theme::hover()))
        .active(|s| s.bg(theme::active()))
        .child(icon(name, size, color))
}

// ---- toggle buttons ------------------------------------------------------
//
// Design rule: anything toggleable shows its state in the icon itself. On =
// filled icon (where a filled variant exists) in full white or the accent;
// line-only icons (shuffle, repeat) get a small dot underneath instead.
// Off = outline icon, dimmed. Clicking pops the icon briefly.

thread_local! {
    static LAST_TOGGLE: std::cell::RefCell<Option<(SharedString, std::time::Instant)>> =
        const { std::cell::RefCell::new(None) };
}

/// Call from a toggle's click handler so its icon plays the pop animation.
pub fn mark_toggled(key: impl Into<SharedString>) {
    let key = key.into();
    LAST_TOGGLE.with(|l| *l.borrow_mut() = Some((key, std::time::Instant::now())));
}

fn has_fill(name: &str) -> bool {
    matches!(name, "heart" | "list-music" | "picture-in-picture-2" | "layout-split")
}

/// Toggle icon button. `key` identifies it for the click animation (pass the
/// same key to [`mark_toggled`]); `accent` colours the "on" state.
pub fn toggle_button(id: impl Into<SharedString>, name: &str, size: f32, on: bool, accent: Option<Hsla>) -> Stateful<Div> {
    let id: SharedString = id.into();
    let fill = on && has_fill(name);
    let icon_name = if fill { format!("{name}-fill") } else { name.to_string() };
    let color = if on { accent.unwrap_or_else(theme::text) } else { theme::text_dim() };
    let popping = LAST_TOGGLE.with(|l| {
        l.borrow()
            .as_ref()
            .is_some_and(|(k, at)| *k == id && at.elapsed() < Duration::from_millis(400))
    });
    let glyph = icon(&icon_name, size, color);
    let glyph: AnyElement = if popping {
        // A new element id per state, so the animation runs once per click.
        glyph
            .with_animation(
                ElementId::Name(format!("{id}-pop-{on}").into()),
                gpui::Animation::new(Duration::from_millis(260)).with_easing(gpui::ease_out_quint()),
                move |svg, d| {
                    // 0.7 → 1.18 → 1.0
                    let s = if d < 0.45 { 0.7 + (d / 0.45) * 0.48 } else { 1.18 - ((d - 0.45) / 0.55) * 0.18 };
                    svg.size(px(size * s))
                },
            )
            .into_any_element()
    } else {
        glyph.into_any_element()
    };
    div()
        .id(ElementId::Name(id))
        .flex_none()
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .size(px(size + 14.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(theme::hover()))
        .active(|s| s.bg(theme::active()))
        .child(glyph)
        .when(on && !has_fill(name), |d| {
            d.child(
                div()
                    .absolute()
                    .bottom(px(1.0))
                    .left(px((size + 14.0 - 4.0) / 2.0))
                    .size(px(4.0))
                    .rounded_full()
                    .bg(color),
            )
        })
}

/// Floating flat panel: black 68 %, 1 px white-12 % border.
pub fn panel() -> Div {
    div()
        .bg(theme::panel())
        .border_1()
        .border_color(theme::border())
        .rounded(px(theme::RADIUS))
}

/// Flat dark card (#141414-like), no border.
pub fn card() -> Div {
    // Clip children (covers, icon tiles) to the card's rounded corners.
    div().bg(theme::card()).rounded(px(8.0)).overflow_hidden()
}

pub fn pill() -> Div {
    div()
        .flex()
        .items_center()
        .bg(theme::panel_strong())
        .border_1()
        .border_color(theme::border())
        .rounded_full()
}

pub fn mono() -> Div {
    div().font_family(theme::MONO)
}

/// Page heading; sits on the background, so its ink adapts to it.
pub fn heading(text: impl Into<SharedString>) -> AnyElement {
    let text: SharedString = text.into();
    theme::adaptive(("heading", text.clone()), || {
        div()
            .self_start()
            .text_size(px(24.0))
            .font_weight(FontWeight::BOLD)
            .text_color(theme::text())
            .child(text)
            .into_any_element()
    })
}

/// Section title with a small icon (Liner style), adaptive ink.
pub fn section_title(icon_name: &str, title: impl Into<SharedString>) -> AnyElement {
    let title: SharedString = title.into();
    let icon_name = icon_name.to_string();
    theme::adaptive(("section", title.clone()), || {
        div()
            .flex()
            .self_start()
            .items_center()
            .gap(px(10.0))
            .text_color(theme::text())
            .child(icon(&icon_name, 18.0, theme::text()))
            .child(
                div()
                    .text_size(px(16.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .into_any_element()
    })
}

/// Text button (flat, rounded).
pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.0))
        .px(px(14.0))
        .h(px(32.0))
        .rounded_full()
        .bg(theme::card())
        .border_1()
        .border_color(theme::border())
        .cursor_pointer()
        .text_size(theme::text_sm())
        .text_color(theme::text())
        .hover(|s| s.bg(theme::card_hover()))
        .active(|s| s.bg(theme::active()))
        .child(label.into())
}

/// Primary button: flat white, black text.
pub fn primary_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.0))
        .px(px(16.0))
        .h(px(32.0))
        .rounded_full()
        .bg(theme::text())
        .cursor_pointer()
        .text_size(theme::text_sm())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(gpui::black())
        .hover(|s| s.opacity(0.88))
        .child(label.into())
}

/// Small text link-style button.
pub fn text_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(8.0))
        .py(px(3.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .text_color(theme::text_dim())
        .hover(|s| s.bg(theme::hover()).text_color(theme::text()))
        .child(label.into())
}

pub fn toggle(id: impl Into<ElementId>, on: bool, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.0))
        .cursor_pointer()
        .text_size(theme::text_sm())
        .child(
            div()
                .flex_none()
                .w(px(30.0))
                .h(px(18.0))
                .rounded_full()
                .p(px(2.0))
                .flex()
                .when(on, |d| d.justify_end())
                .bg(if on { theme::text() } else { theme::active() })
                .child(
                    div()
                        .size(px(14.0))
                        .rounded_full()
                        .bg(if on { gpui::black() } else { theme::text_dim() }),
                ),
        )
        .child(label.into())
}

/// ASCII source badge `[AU]` (mono accent).
pub fn badge(p: ProviderId) -> Div {
    mono()
        .flex_none()
        .text_size(theme::text_xs())
        .text_color(theme::text_faint())
        .child(format!("[{}]", p.badge()))
}

pub fn badges(t: &Track) -> Div {
    div()
        .flex()
        .flex_none()
        .gap(px(2.0))
        .children(t.sources().into_iter().map(badge))
}

/// Flat type chip (`Track`, `Album`, `Artist`) in a solid song colour.
pub fn chip(label: impl Into<SharedString>, on: bool) -> Div {
    div()
        .flex_none()
        .px(px(12.0))
        .h(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .text_size(px(12.0))
        // Fixed line height so the label sits in the vertical centre.
        .line_height(px(14.0))
        // Own surface: fixed colours, never the adaptive background ink.
        .bg(if on { gpui::white() } else { gpui::hsla(0.0, 0.0, 0.08, 0.92) })
        .text_color(if on { gpui::black() } else { gpui::hsla(0.0, 0.0, 1.0, 0.85) })
        .child(label.into())
}

pub fn fmt_time(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn fmt_ms(ms: Option<u64>) -> String {
    ms.map(|m| fmt_time(Duration::from_millis(m)))
        .unwrap_or_else(|| "-:--".into())
}

/// Thin flat bar (white fill on grey track, 4 px), clickable in segments.
pub fn thin_bar(id: &'static str, width: f32, frac: f32, on_pick: Pick) -> Div {
    const N: usize = 80;
    let filled = (frac.clamp(0.0, 1.0) * N as f32).round() as usize;
    let seg = width / N as f32;
    div()
        .flex()
        .flex_none()
        .w(px(width))
        .h(px(14.0))
        .items_center()
        .children((0..N).map(move |i| {
            let pick = on_pick.clone();
            let first = i == 0;
            let last = i + 1 == N;
            div()
                .id((id, i))
                .w(px(seg))
                .h(px(14.0))
                .flex()
                .items_center()
                .cursor_pointer()
                .on_click(move |_, w, cx| pick((i as f32 + 0.5) / N as f32, w, cx))
                .child(
                    div()
                        .w_full()
                        .h(px(4.0))
                        .when(first, |d| d.rounded_l(px(2.0)))
                        .when(last, |d| d.rounded_r(px(2.0)))
                        .bg(if i < filled {
                            theme::text()
                        } else {
                            gpui::hsla(0.0, 0.0, 1.0, 0.18)
                        }),
                )
        }))
}

/// Empty / error state, Liner style: a small icon in a soft circle, title and
/// one hint line, left-aligned on the background (no card, no ASCII art).
/// `art` picks the icon.
pub fn ascii_state(
    art: &'static str,
    title: impl Into<SharedString>,
    sub: impl Into<SharedString>,
) -> AnyElement {
    let (title, sub) = (title.into(), sub.into());
    let icon_name = match art {
        ART_SEARCH => "search",
        ART_OFFLINE => "refresh-cw",
        _ => "music-2",
    };
    theme::adaptive(("empty", title.clone()), move || {
        div()
            .self_start()
            .flex()
            .items_center()
            .gap(px(14.0))
            .py(px(8.0))
            .child(
                div()
                    .flex_none()
                    .size(px(44.0))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::text().opacity(0.08))
                    .child(icon(icon_name, 18.0, theme::text_dim())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(theme::text_md())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(title),
                    )
                    .when(!sub.is_empty(), |d| {
                        d.child(div().text_size(theme::text_sm()).text_color(theme::text_dim()).child(sub))
                    }),
            )
            .into_any_element()
    })
}

pub fn spinner_line(spin: char, label: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .text_size(theme::text_sm())
        .text_color(theme::text_dim())
        .child(mono().text_color(theme::text()).child(spin.to_string()))
        .child(label.into())
}

pub const ART_EMPTY: &str = r" .-------------.
 |  .       .  |
 |      _      |
 |   .-' '-.   |
 '-------------'";

pub const ART_OFFLINE: &str = r"  ____   ____
 |    |-|    |
 |  x | | x  |
 |____|-|____|";

pub const ART_SEARCH: &str = r"    .--.
   / .. \
   \ '' /__
    '--'\  \
         \__\";

/// Cover image (real provider cover via `Backend::cover`), with a flat
/// placeholder or – if `visuals.ascii_covers` – an ASCII pattern.
pub fn cover(app: &mut Glyph, t: &Track, size: f32, radius: f32, cx: &mut Context<Glyph>) -> Div {
    let path = app.cover_path(t, cx);
    let base = div()
        .flex_none()
        .size(px(size))
        .rounded(px(radius))
        .overflow_hidden()
        .bg(theme::card());
    // Mono cell is ~0.6 em wide and 1.05 em tall: size the grid to fill the square.
    let fs = (size / 26.0).clamp(5.0, 11.0);
    let cols = ((size / (fs * 0.6)) as u32).max(6);
    let rows = ((size / (fs * 1.05)) as u32).max(3);
    let grid = |bg: Hsla, fg: Hsla| {
        div()
            .flex_none()
            .size(px(size))
            .rounded(px(radius))
            .overflow_hidden()
            .bg(bg)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .font_family(theme::MONO)
            .text_size(px(fs))
            .line_height(px(fs * 1.05))
            .text_color(fg)
    };
    if let Some(p) = path {
        if app.config.visuals.ascii_covers {
            // The real cover rendered through the ASCII ramp (computed off the
            // UI thread, cached per track and size).
            match app.ascii_cover(t, &p, cols, rows, cx) {
                Some(rows_) if !rows_.is_empty() => {
                    return grid(gpui::black(), theme::text_dim()).children(rows_.into_iter().map(|(line, runs)| {
                        let hl: Vec<_> = runs
                            .into_iter()
                            .map(|(start, len, c)| {
                                (start..start + len, gpui::HighlightStyle { color: Some(c.hsla(1.0)), ..Default::default() })
                            })
                            .collect();
                        div().whitespace_nowrap().child(gpui::StyledText::new(line).with_highlights(hl))
                    }));
                }
                Some(_) => {}
                None => return skeleton(format!("cover-sk-{}-{size}", track_key(t)), size, size, radius),
            }
        }
        return base.child(
            img(p)
                .size(px(size))
                .rounded(px(radius))
                .object_fit(ObjectFit::Cover),
        );
    }
    if app.config.visuals.ascii_covers && !app.cover_loading(t) {
        // No cover anywhere: generated pattern (end of the fallback chain).
        let seed = format!("{}{}", t.artist, t.title);
        let pal = Palette::from_seed(&seed);
        return grid(pal.0[0].scale(0.3).hsla(1.0), pal.0[2].lighten(0.3).hsla(0.9))
            .children(ascii::pattern(&seed, cols, rows).into_iter().map(|(l, _)| div().whitespace_nowrap().child(l)));
    }
    if app.cover_loading(t) {
        return skeleton(format!("cover-sk-{}-{size}", track_key(t)), size, size, radius);
    }
    base.flex()
        .items_center()
        .justify_center()
        .child(icon("music-2", (size * 0.36).clamp(14.0, 64.0), theme::text_faint()))
}

// ---- loading skeletons ----------------------------------------------------

// The shimmer is ASCII: a dim field of ramp characters with a brighter band
// sweeping across it. It is drawn from the clock at render time, so the app
// keeps re-rendering while any skeleton is on screen (see `take_skeleton_seen`).

thread_local! {
    static SKELETON_SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SKELETON_T0: std::time::Instant = std::time::Instant::now();
}

/// True if a skeleton was rendered since the last call (the app then keeps
/// animating).
pub fn take_skeleton_seen() -> bool {
    SKELETON_SEEN.with(|c| c.replace(false))
}

const SHIMMER_RAMP: &[u8] = b" .:-=+*#";

fn shimmer_text(cols: usize, rows: usize, seed: u32) -> Vec<String> {
    SKELETON_SEEN.with(|c| c.set(true));
    let t = SKELETON_T0.with(|t0| t0.elapsed().as_secs_f32());
    // Band position in columns; it crosses ~60 columns per second, with a
    // pause off-screen so the sweep reads as a glint, not a scroll.
    let period = (cols as f32 + 40.0) / 60.0;
    let x0 = ((t % period) / period) * (cols as f32 + 40.0) - 20.0;
    (0..rows)
        .map(|y| {
            (0..cols)
                .map(|x| {
                    let d = (x as f32 + y as f32 * 1.6) - x0;
                    let band = (-(d * d) / 18.0).exp();
                    // A little per-cell grain so the field is not flat.
                    let h = (x as u32).wrapping_mul(2654435761) ^ (y as u32 + seed).wrapping_mul(40503);
                    let grain = (h % 7) as f32 / 7.0 * 0.22;
                    let v = (0.18 + grain + band * 0.8).min(1.0);
                    SHIMMER_RAMP[((v * (SHIMMER_RAMP.len() - 1) as f32).round() as usize).min(SHIMMER_RAMP.len() - 1)] as char
                })
                .collect()
        })
        .collect()
}

fn shimmer_block(cols: usize, rows: usize, fs: f32, seed: u32, radius: f32) -> Div {
    div()
        .size_full()
        .overflow_hidden()
        .rounded(px(radius))
        .bg(gpui::hsla(0.0, 0.0, 1.0, 0.03))
        .flex()
        .flex_col()
        .justify_center()
        .font_family(theme::MONO)
        .text_size(px(fs))
        .line_height(px(fs * 1.05))
        .text_color(gpui::hsla(0.0, 0.0, 1.0, 0.32))
        .children(shimmer_text(cols, rows, seed).into_iter().map(|l| div().whitespace_nowrap().child(l)))
}

fn seed_of(id: &str) -> u32 {
    id.bytes().fold(2166136261u32, |a, b| (a ^ b as u32).wrapping_mul(16777619))
}

/// A placeholder block with the ASCII shimmer while content loads.
pub fn skeleton(id: impl Into<SharedString>, w: f32, h: f32, radius: f32) -> Div {
    let id: SharedString = id.into();
    let fs = if h < 18.0 { (h * 0.95).max(6.0) } else { 9.0 };
    let cols = ((w / (fs * 0.6)).ceil() as usize).max(1);
    let rows = ((h / (fs * 1.05)).floor() as usize).max(1);
    div().flex_none().w(px(w)).h(px(h)).child(shimmer_block(cols, rows, fs, seed_of(&id), radius))
}

/// One shimmering text line that takes its parent's width (clipped).
fn skeleton_fill(id: impl Into<SharedString>, radius: f32, h: f32) -> AnyElement {
    let id: SharedString = id.into();
    let fs = (h * 0.95).max(6.0);
    shimmer_block(140, 1, fs, seed_of(&id), radius).into_any_element()
}

/// `n` skeleton track rows (cover, title, artist, duration).
pub fn skeleton_rows(prefix: &str, n: usize) -> Div {
    div().flex().flex_col().gap(px(2.0)).children((0..n).map(|i| {
        // Vary the bar widths a little so it does not look like a grid.
        let title_w = [0.42, 0.55, 0.36, 0.48, 0.6, 0.4][i % 6];
        let artist_w = [0.24, 0.3, 0.2, 0.27, 0.33, 0.22][i % 6];
        div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .px(px(8.0))
            .py(px(6.0))
            .child(skeleton(format!("{prefix}-sk-c{i}"), 44.0, 44.0, 6.0))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(div().h(px(12.0)).w(gpui::relative(title_w)).child(skeleton_fill(format!("{prefix}-sk-t{i}"), 4.0, 12.0)))
                    .child(div().h(px(10.0)).w(gpui::relative(artist_w)).child(skeleton_fill(format!("{prefix}-sk-a{i}"), 4.0, 10.0))),
            )
            .child(skeleton(format!("{prefix}-sk-d{i}"), 32.0, 10.0, 4.0))
    }))
}

/// Skeleton lyric lines.
pub fn skeleton_lyrics(n: usize) -> Div {
    div().flex().flex_col().gap(px(26.0)).children((0..n).map(|i| {
        let w = [0.62, 0.48, 0.7, 0.4, 0.56, 0.66, 0.44][i % 7];
        div().h(px(24.0)).w(gpui::relative(w)).child(skeleton_fill(format!("lyr-sk-{i}"), 6.0, 24.0))
    }))
}

/// Quiet track row: cover 48 px, title, artist, badges, duration, actions.
pub fn track_row(
    app: &mut Glyph,
    list: Rc<Vec<Track>>,
    i: usize,
    id_prefix: &'static str,
    cx: &mut Context<Glyph>,
) -> Stateful<Div> {
    let t = list[i].clone();
    let playing = app.current.as_ref().map(track_key) == Some(track_key(&t));
    let liked = app.is_liked(&t);
    let group: SharedString = format!("{id_prefix}-row-{i}").into();
    let l2 = list.clone();
    let (t_next, t_q, t_like, t_share) = (t.clone(), t.clone(), t.clone(), t.clone());
    let art = cover(app, &t, 44.0, 6.0, cx);

    div()
        .id((id_prefix, i))
        .group(group.clone())
        .flex()
        .items_center()
        .gap(px(14.0))
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(theme::card()))
        .on_click(cx.listener(move |this, _, _, cx| this.play_list((*l2).clone(), i, cx)))
        .child(art)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.0))
                .child(
                    div()
                        .truncate()
                        .text_size(theme::text_md())
                        .font_weight(if playing { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                        .text_color(theme::text())
                        .child(t.title.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(theme::text_sm())
                        .text_color(theme::text_dim())
                        .child(match &t.album {
                            Some(a) => format!("{} · {a}", t.artist),
                            None => t.artist.clone(),
                        }),
                ),
        )
        .child(badges(&t))
        .child(
            div()
                .flex()
                .flex_none()
                .gap(px(2.0))
                .opacity(0.0)
                .group_hover(group, |s| s.opacity(1.0))
                .child(
                    icon_button((id_prefix, i * 10 + 1), "list-start", 15.0, theme::text_dim())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.queue_add(t_next.clone(), true, cx)
                        })),
                )
                .child(
                    icon_button((id_prefix, i * 10 + 2), "list-plus", 15.0, theme::text_dim())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.queue_add(t_q.clone(), false, cx)
                        })),
                )
                .child(
                    icon_button((id_prefix, i * 10 + 3), "link-2", 15.0, theme::text_dim())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.share(&t_share, cx)
                        })),
                ),
        )
        .child({
            let key: SharedString = format!("{id_prefix}-like-{}", track_key(&t_like)).into();
            let k = key.clone();
            toggle_button(key, "heart", 15.0, liked, Some(theme::pink())).on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                mark_toggled(k.clone());
                this.toggle_like(&t_like, cx)
            }))
        })
        .child(
            mono()
                .flex_none()
                .w(px(44.0))
                .text_size(theme::text_sm())
                .text_color(theme::text_faint())
                .child(fmt_ms(t.duration_ms)),
        )
}

pub fn status_line(text: impl Into<SharedString>) -> Div {
    mono()
        .text_size(theme::text_xs())
        .text_color(theme::text_faint())
        .child(text.into())
}

/// Translate helper kept here so view code reads short.
pub fn tr(key: &str) -> SharedString {
    t(key)
}
