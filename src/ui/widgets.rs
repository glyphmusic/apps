//! Small building blocks. Everything is flat: no gradients, no glow, no
//! shadows. ASCII only as accents (badges, spinner, empty states).

use crate::app::{track_key, Glyph};
use crate::i18n::t;
use crate::theme;
use crate::visuals::{ascii, Palette};
use glyph_protocol::{ProviderId, Track};
use gpui::{
    div, img, prelude::*, px, svg, AnyElement, Context, Div, ElementId, FontWeight, Hsla, ObjectFit,
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
    div().bg(theme::card()).rounded(px(8.0))
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
    if let Some(p) = path {
        return base.child(
            img(p)
                .size(px(size))
                .rounded(px(radius))
                .object_fit(ObjectFit::Cover),
        );
    }
    if app.config.visuals.ascii_covers {
        let seed = format!("{}{}", t.artist, t.title);
        let pal = Palette::from_seed(&seed);
        let cols = ((size / 6.5) as u32).max(6);
        let rows = ((size / 9.0) as u32).max(3);
        let fs = (size / cols as f32 * 1.55).clamp(5.0, 11.0);
        return base
            .bg(pal.0[0].scale(0.3).hsla(1.0))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .font_family(theme::MONO)
            .text_size(px(fs))
            .line_height(px(fs * 1.05))
            .text_color(pal.0[2].lighten(0.3).hsla(0.9))
            .children(
                ascii::pattern(&seed, cols, rows)
                    .into_iter()
                    .map(|(l, _)| div().whitespace_nowrap().child(l)),
            );
    }
    base.flex()
        .items_center()
        .justify_center()
        .child(icon("music-2", (size * 0.36).clamp(14.0, 64.0), theme::text_faint()))
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
        .child(
            icon_button(
                (id_prefix, i * 10 + 4),
                "heart",
                15.0,
                if liked { theme::pink() } else { theme::text_faint() },
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_like(&t_like, cx)
            })),
        )
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
