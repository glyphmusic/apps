use super::*;

/// What the box in front of a song title says about explicit content.
#[derive(Clone, Copy, PartialEq)]
pub enum Rating {
    Clean,
    Explicit,
    /// No source can tell (YouTube noise): "?".
    Unknown,
}

pub fn rating(t: &Track) -> Rating {
    if t.explicit {
        Rating::Explicit
    } else if t.explicit_unknown() {
        Rating::Unknown
    } else {
        Rating::Clean
    }
}

/// Small "E" box in front of explicit song titles ("?" when unknown).
pub fn explicit_badge(r: Rating) -> Option<Div> {
    let (id, tip, label) = match r {
        Rating::Clean => return None,
        Rating::Explicit => ("explicit-badge", "tip.explicit", "E"),
        Rating::Unknown => ("explicit-unknown-badge", "tip.explicit_unknown", "?"),
    };
    // Hover says what the letter means.
    Some(div().flex_none().child(with_tip(div().id(id), Some(t(tip)))
        .flex_none()
        .size(px(15.0))
        .rounded(px(3.0))
        .bg(gpui::hsla(0.0, 0.0, 1.0, if r == Rating::Unknown { 0.12 } else { 0.22 }))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(9.5))
        .font_weight(FontWeight::BOLD)
        .text_color(if r == Rating::Unknown { gpui::hsla(0.0, 0.0, 1.0, 0.6) } else { gpui::hsla(0.0, 0.0, 0.0, 0.85) })
        .child(label)))
}

/// A song title with the "E" / "?" box in front (truncates).
pub fn song_title(title: impl Into<SharedString>, r: Rating) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .min_w_0()
        .children(explicit_badge(r))
        .child(div().min_w_0().truncate().child(title.into()))
}

/// Opacity for something in progress (downloads): a soft 1.5 s pulse.
pub fn pulse() -> f32 {
    let t = elapsed_t0_secs();
    0.35 + 0.65 * (0.5 + 0.5 * (t * std::f32::consts::TAU / 1.5).sin())
}

// ---- download badge ------------------------------------------------------
//
// While a song downloads a ring fills around the arrow (accent, clockwise
// from 12 o'clock; a short spinning arc while the size is still unknown).
// When it is done the full ring fades and the arrow shrinks away while the
// check pops in.

/// Seconds the "done" transition takes.
pub const DL_DONE_ANIM_S: f32 = 0.65;

/// What a download badge shows.
#[derive(Clone, Copy, PartialEq)]
pub enum DlView {
    None,
    /// Fraction fetched (`None` = size not known yet).
    Busy(Option<f32>),
    /// Downloaded; seconds since it finished while that is still animating.
    Done(Option<f32>),
}

/// Ring of `size` px: a faint track plus the done part. `alpha` fades both.
pub fn progress_ring(size: f32, stroke: f32, fraction: Option<f32>, alpha: f32) -> gpui::Canvas<()> {
    use std::f32::consts::{FRAC_PI_2, TAU};
    let spin = elapsed_t0_secs();
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let c = bounds.center();
        let r = size / 2.0 - stroke / 2.0;
        let mut arc = |from: f32, sweep: f32, color: Hsla| {
            let n = ((sweep.abs() / TAU) * 64.0).ceil().max(2.0) as usize;
            let mut b = gpui::PathBuilder::stroke(px(stroke));
            for i in 0..=n {
                let a = from + sweep * (i as f32 / n as f32);
                let p = gpui::point(c.x + px(r * a.cos()), c.y + px(r * a.sin()));
                if i == 0 {
                    b.move_to(p)
                } else {
                    b.line_to(p)
                }
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, color);
            }
        };
        arc(0.0, TAU, gpui::hsla(0.0, 0.0, 1.0, 0.16 * alpha));
        match fraction {
            Some(f) if f > 0.0 => arc(-FRAC_PI_2, TAU * f.min(1.0), theme::pink().opacity(alpha)),
            Some(_) => {}
            None => arc(spin * TAU * 1.1 - FRAC_PI_2, TAU * 0.28, theme::pink().opacity(alpha)),
        }
    })
    .size(px(size))
}

/// Icon of a round play button (`button` px): pause / play, or a spinning
/// arc just inside the button's edge while a song loads.
pub fn play_glyph(loading: bool, playing: bool, size: f32, button: f32) -> AnyElement {
    if !loading {
        return icon(if playing { "pause" } else { "play" }, size, gpui::black()).into_any_element();
    }
    use std::f32::consts::{FRAC_PI_2, TAU};
    let spin = elapsed_t0_secs();
    let ring = button * 0.56;
    let stroke = (button / 16.0).max(2.0);
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let c = bounds.center();
        let r = ring / 2.0 - stroke / 2.0;
        // A ~100° arc that turns once every 0.8 s.
        let (from, sweep) = (spin * TAU / 0.8 - FRAC_PI_2, TAU * 0.28);
        let mut b = gpui::PathBuilder::stroke(px(stroke));
        for i in 0..=24 {
            let a = from + sweep * (i as f32 / 24.0);
            let p = gpui::point(c.x + px(r * a.cos()), c.y + px(r * a.sin()));
            if i == 0 {
                b.move_to(p)
            } else {
                b.line_to(p)
            }
        }
        if let Ok(path) = b.build() {
            window.paint_path(path, gpui::black());
        }
    })
    .size(px(ring))
    .into_any_element()
}

/// The download state as a `size` px badge: ring + `arrow` icon while busy,
/// the pink check when done. `None` for songs that are not downloaded.
pub fn download_badge(view: DlView, size: f32, arrow: &str) -> Option<Div> {
    let stroke = (size / 9.0).max(1.5);
    let inner = size * 0.58;
    let frame = || div().flex_none().relative().size(px(size)).flex().items_center().justify_center();
    let ring = |f: Option<f32>, alpha: f32| div().absolute().inset_0().child(progress_ring(size, stroke, f, alpha));
    match view {
        DlView::None => None,
        DlView::Busy(f) => Some(frame().child(ring(f, 1.0)).child(icon(arrow, inner, theme::text()))),
        DlView::Done(None) => Some(frame().child(icon("circle-check", size, theme::pink()))),
        DlView::Done(Some(age)) => {
            // Ring + arrow out over the first 0.25 s ...
            let out = 1.0 - (age / 0.25).min(1.0);
            // ... check in from 0.15 s: 0.4 → 1.15 → 1.0.
            let p = ((age - 0.15) / (DL_DONE_ANIM_S - 0.15)).clamp(0.0, 1.0);
            let scale = if p < 0.6 { 0.4 + p / 0.6 * 0.75 } else { 1.15 - (p - 0.6) / 0.4 * 0.15 };
            let mut d = frame();
            if out > 0.0 {
                d = d.child(ring(Some(1.0), out)).child(icon(arrow, (inner * out).max(0.1), theme::text().opacity(out)));
            }
            if p > 0.0 {
                d = d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity((p * 3.0).min(1.0))
                        .child(icon("circle-check", size * scale, theme::pink())),
                );
            }
            Some(d)
        }
    }
}

/// Skeleton rows for artists in search results: round picture, name, chip.
pub fn skeleton_artist_rows(prefix: &str, n: usize) -> Div {
    div().flex().flex_col().gap(px(2.0)).children((0..n).map(|i| {
        let name_w = [0.3, 0.22, 0.26][i % 3];
        div()
            .flex()
            .items_center()
            .gap(px(14.0))
            .px(px(10.0))
            .py(px(6.0))
            .child(skeleton(format!("{prefix}-ska-c{i}"), 48.0, 48.0, 24.0))
            .child(div().flex_1().child(div().h(px(13.0)).w(gpui::relative(name_w)).child(skeleton_fill(format!("{prefix}-ska-n{i}"), 4.0, 13.0))))
            .child(skeleton(format!("{prefix}-ska-k{i}"), 46.0, 18.0, 4.0))
    }))
}

// ---- ASCII reveal --------------------------------------------------------
//
// New content "decodes" in: text starts as random ramp characters that settle
// into the real letters from left to right, and pictures start under an ASCII
// shimmer that fades out. Keys remember when something was first shown.

pub(super) const DECODE_RAMP: &[char] = &['#', '%', '&', '*', '+', '=', '-', ':', '.', '@', '$', '0', 'X'];

/// Progress 0..1 of the reveal for `key`, `dur` seconds long (starts the
/// first time it is asked for).
pub fn reveal_k(key: &str, dur: f32) -> f32 {
    if !ascii_effects() {
        return 1.0;
    }
    UI_RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        let m = &mut rt.reveals;
        if m.len() > 4000 {
            m.retain(|_, at| at.elapsed().as_secs() < 120);
        }
        let at = *m.entry(key.to_string()).or_insert_with(std::time::Instant::now);
        (at.elapsed().as_secs_f32() / dur.max(0.01)).min(1.0)
    })
}

/// Something is still revealing (the app keeps re-rendering).
pub fn reveal_active() -> bool {
    UI_RT.with(|rt| rt.borrow().reveals.values().any(|at| at.elapsed().as_secs_f32() < 1.0))
}

/// Starts the reveal under `key` over (a label that just changed).
pub fn reveal_restart(key: &str) {
    UI_RT.with(|rt| {
        rt.borrow_mut().reveals.insert(key.to_string(), std::time::Instant::now());
    });
}

/// Like [`reveal_k`], but only a reveal started with [`reveal_restart`]
/// runs; anything else is shown finished.
pub fn reveal_peek(key: &str, dur: f32) -> f32 {
    if !ascii_effects() {
        return 1.0;
    }
    UI_RT.with(|rt| rt.borrow().reveals.get(key).map_or(1.0, |at| (at.elapsed().as_secs_f32() / dur.max(0.01)).min(1.0)))
}

/// Forget reveals under `prefix` (e.g. a new search starts over).
pub fn reset_reveals(prefix: &str) {
    UI_RT.with(|rt| rt.borrow_mut().reveals.retain(|k, _| !k.starts_with(prefix)));
}

/// `text` part-way decoded: the first `k` share is real, the rest flickers
/// through ASCII characters (spaces stay spaces).
pub fn decode(text: &str, k: f32) -> String {
    if k >= 1.0 {
        return text.to_string();
    }
    let n = text.chars().count();
    let done = (k.max(0.0) * n as f32) as usize;
    let tick = (elapsed_t0_secs() * 12.0) as u32;
    text.chars()
        .enumerate()
        .map(|(i, c)| {
            if i < done || c.is_whitespace() {
                c
            } else {
                let h = (i as u32).wrapping_mul(2654435761) ^ tick.wrapping_mul(40503);
                DECODE_RAMP[(h >> 7) as usize % DECODE_RAMP.len()]
            }
        })
        .collect()
}

/// `el` (a picture) under an ASCII shimmer that fades out as `k` goes to 1.
pub fn ascii_veil(el: impl IntoElement, id: impl Into<SharedString>, w: f32, h: f32, r: f32, k: f32) -> Div {
    let e = 1.0 - (1.0 - k).powi(2);
    div()
        .relative()
        .flex_none()
        .child(div().opacity(0.15 + 0.85 * e).child(el))
        .when(k < 1.0, |d| d.child(div().absolute().inset_0().opacity(1.0 - e).child(skeleton(id, w, h, r))))
}

/// Skeleton lyric lines.
pub fn skeleton_lyrics(n: usize) -> Div {
    div().flex().flex_col().gap(px(26.0)).children((0..n).map(|i| {
        let w = [0.62, 0.48, 0.7, 0.4, 0.56, 0.66, 0.44][i % 7];
        div().h(px(24.0)).w(gpui::relative(w)).child(skeleton_fill(format!("lyr-sk-{i}"), 6.0, 24.0))
    }))
}

/// Small type label ("Song", "Artist") in mixed search results.
pub fn kind_chip(label: SharedString) -> Div {
    div()
        .flex_none()
        .px(px(6.0))
        .h(px(18.0))
        .flex()
        .items_center()
        .rounded(px(4.0))
        .bg(gpui::hsla(0.0, 0.0, 1.0, 0.08))
        .text_size(px(11.0))
        .text_color(gpui::hsla(0.0, 0.0, 1.0, 0.6))
        .child(label)
}
