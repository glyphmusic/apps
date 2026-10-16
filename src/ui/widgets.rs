//! Small building blocks: panels, pills, ASCII buttons, badges, states.

use crate::app::{Glyph, track_key};
use crate::theme;
use crate::visuals::{ascii, Palette, Rgb};
use glyph_protocol::{ProviderId, Track};
use gpui::{
    div, prelude::*, px, ClickEvent, Context, Div, ElementId, FontWeight, Hsla, SharedString,
    Stateful, Window,
};
use std::rc::Rc;
use std::time::Duration;

pub type Listener = Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static>;

/// Floating panel: black 68 %, 1 px white-12 % border, no shadow.
pub fn panel() -> Div {
    div()
        .bg(theme::panel())
        .border_1()
        .border_color(theme::border())
        .rounded(px(theme::RADIUS))
}

/// Dark translucent pill.
pub fn pill() -> Div {
    div()
        .flex()
        .items_center()
        .bg(theme::panel_strong())
        .border_1()
        .border_color(theme::border())
        .rounded_full()
}

/// `/section` tag in the Liner-terminal style.
pub fn tag(label: &str) -> Div {
    div()
        .text_size(theme::text_xs())
        .text_color(theme::text_faint())
        .child(format!("/{label}"))
}

pub fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(26.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::text())
        .child(text.into())
}

/// Plain ASCII text button, e.g. `[ ▶ PLAY ]`.
pub fn ascii_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(6.0))
        .py(px(2.0))
        .rounded(px(5.0))
        .cursor_pointer()
        .text_color(theme::text_dim())
        .hover(|s| s.bg(theme::hover()).text_color(theme::text()))
        .active(|s| s.bg(theme::active()))
        .child(label.into())
}

/// Bordered button for primary actions.
pub fn outline_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(12.0))
        .py(px(5.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme::border_strong())
        .bg(theme::panel_light())
        .cursor_pointer()
        .text_color(theme::text())
        .hover(|s| s.bg(theme::hover()).border_color(theme::text_faint()))
        .active(|s| s.bg(theme::active()))
        .child(label.into())
}

pub fn badge(p: ProviderId) -> Div {
    div()
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

/// Coloured type chip (`track`, album, artist).
pub fn chip(label: impl Into<SharedString>, color: Rgb, strong: bool) -> Div {
    div()
        .flex_none()
        .px(px(6.0))
        .py(px(1.0))
        .rounded(px(4.0))
        .text_size(theme::text_xs())
        .bg(color.hsla(if strong { 0.28 } else { 0.14 }))
        .text_color(color.lighten(0.55).hsla(0.95))
        .child(label.into())
}

pub fn fmt_time(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn fmt_ms(ms: Option<u64>) -> String {
    ms.map(|m| fmt_time(Duration::from_millis(m))).unwrap_or_else(|| "-:--".into())
}

/// `░▒▓█` progress as `cells` clickable characters.
pub fn ascii_bar(
    id: &'static str,
    cells: usize,
    frac: f32,
    color: Hsla,
    on_pick: Rc<dyn Fn(f32, &mut Window, &mut gpui::App)>,
) -> Div {
    let filled = (frac.clamp(0.0, 1.0) * cells as f32).floor() as usize;
    div().flex().flex_none().children((0..cells).map(move |i| {
        let (ch, col) = if i < filled {
            ("█", color)
        } else if i == filled {
            ("▓", color.opacity(0.75))
        } else if i == filled + 1 {
            ("▒", theme::text_ghost())
        } else {
            ("░", theme::text_ghost())
        };
        let pick = on_pick.clone();
        div()
            .id((id, i))
            .cursor_pointer()
            .text_color(col)
            .hover(|s| s.text_color(theme::text()))
            .on_click(move |_, w, cx| pick((i as f32 + 0.5) / cells as f32, w, cx))
            .child(ch)
    }))
}

/// Big ASCII empty / error state.
pub fn ascii_state(art: &'static str, title: impl Into<SharedString>, sub: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.0))
        .py(px(48.0))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .text_size(theme::text_sm())
                .text_color(theme::text_faint())
                .children(art.lines().map(|l| div().whitespace_nowrap().child(l.to_string()))),
        )
        .child(div().text_color(theme::text()).child(title.into()))
        .child(div().text_size(theme::text_sm()).text_color(theme::text_faint()).child(sub.into()))
}

pub fn spinner_line(spin: char, label: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .gap(px(8.0))
        .text_color(theme::text_dim())
        .child(div().text_color(theme::text()).child(spin.to_string()))
        .child(label.into())
}

pub const ART_EMPTY: &str = "\
 .-------------.
 |  .       .  |
 |      _      |
 |   .-' '-.   |
 '-------------'";

pub const ART_OFFLINE: &str = "\
  ____   ____
 |    |-|    |
 |  x |  | x  |
 |____|-|____|";

pub const ART_SEARCH: &str = "\
    .--.
   / .. \\
   \\ '' /__
    '--'\\  \\
         \\__\\";

/// Small ASCII cover tile (cover column in lists).
pub fn mini_cover(t: &Track, size: f32) -> Div {
    let seed = format!("{}{}", t.artist, t.title);
    let pal = Palette::from_seed(&seed);
    let rows = ascii::pattern(&seed, 8, 4);
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(6.0))
        .overflow_hidden()
        .bg(pal.0[0].scale(0.35).hsla(0.9))
        .border_1()
        .border_color(theme::border_faint())
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .text_size(px(8.0))
        .line_height(px(9.0))
        .text_color(pal.0[2].lighten(0.3).hsla(0.85))
        .children(rows.into_iter().map(|(l, _)| div().whitespace_nowrap().child(l)))
}

/// Liner-style quiet track row: cover 48 px, title, artist, chips, badges,
/// duration and ASCII actions. No table lines, only spacing.
pub fn track_row(
    app: &Glyph,
    list: Rc<Vec<Track>>,
    i: usize,
    id_prefix: &'static str,
    cx: &mut Context<Glyph>,
) -> Stateful<Div> {
    let t = list[i].clone();
    let playing = app.current.as_ref().map(track_key) == Some(track_key(&t));
    let accent = app.palette.map(|p| p.accent()).unwrap_or(Rgb::hex(0x46dcff));
    let liked = app.is_liked(&t);
    let group: SharedString = format!("{id_prefix}-row-{i}").into();
    let l2 = list.clone();
    let t_next = t.clone();
    let t_q = t.clone();
    let t_like = t.clone();
    let t_share = t.clone();

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
        .hover(|s| s.bg(theme::hover()))
        .on_click(cx.listener(move |this, _, _, cx| this.play_list((*l2).clone(), i, cx)))
        .child(mini_cover(&t, 44.0))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(3.0))
                .child(
                    div()
                        .truncate()
                        .text_color(if playing { accent.lighten(0.3).hsla(1.0) } else { theme::text() })
                        .child(format!("{}{}", if playing { "▶ " } else { "" }, t.title)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .min_w_0()
                        .text_size(theme::text_sm())
                        .text_color(theme::text_dim())
                        .child(div().truncate().child(t.artist.clone()))
                        .when_some(t.album.clone(), |d, a| {
                            d.child(div().text_color(theme::text_faint()).truncate().child(format!("· {a}")))
                        }),
                ),
        )
        .child(badges(&t))
        .child(
            div()
                .flex()
                .flex_none()
                .gap(px(2.0))
                .text_size(theme::text_sm())
                .opacity(0.0)
                .group_hover(group, |s| s.opacity(1.0))
                .child(
                    ascii_button((id_prefix, i * 10 + 1), "next")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.queue_add(t_next.clone(), true, cx)
                        })),
                )
                .child(
                    ascii_button((id_prefix, i * 10 + 2), "+q")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.queue_add(t_q.clone(), false, cx)
                        })),
                )
                .child(
                    ascii_button((id_prefix, i * 10 + 3), "link")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.share(&t_share, cx)
                        })),
                ),
        )
        .child(
            ascii_button((id_prefix, i * 10 + 4), if liked { "♥" } else { "♡" })
                .text_color(if liked { accent.lighten(0.2).hsla(1.0) } else { theme::text_faint() })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_like(&t_like, cx)
                })),
        )
        .child(
            div()
                .flex_none()
                .w(px(44.0))
                .text_size(theme::text_sm())
                .text_color(theme::text_faint())
                .child(fmt_ms(t.duration_ms)),
        )
}

/// Thin flat bar (white fill on grey track, 4 px), clickable in 60 segments.
pub fn thin_seek(
    id: &'static str,
    width: f32,
    frac: f32,
    on_pick: Rc<dyn Fn(f32, &mut Window, &mut gpui::App)>,
) -> Div {
    const N: usize = 60;
    let filled = (frac.clamp(0.0, 1.0) * N as f32).round() as usize;
    let seg = width / N as f32;
    div().flex().flex_none().w(px(width)).h(px(12.0)).items_center().children((0..N).map(move |i| {
        let pick = on_pick.clone();
        div()
            .id((id, i))
            .w(px(seg))
            .h(px(12.0))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_click(move |_, w, cx| pick((i as f32 + 0.5) / N as f32, w, cx))
            .child(div().w_full().h(px(4.0)).bg(if i < filled { theme::text() } else { theme::text_ghost() }))
    }))
}
