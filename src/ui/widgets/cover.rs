use super::*;

/// Cover image (real provider cover via `Backend::cover`), with a flat
/// placeholder or – if `visuals.ascii_covers` – an ASCII pattern.
pub fn cover(app: &mut Glyph, t: &Track, size: f32, radius: f32, cx: &mut Context<Glyph>) -> Div {
    let el = cover_inner(app, t, size, radius, cx);
    // Once the picture is there it "ASCIIs in" (everywhere: rows, home,
    // search), timed from the moment it first shows.
    if app.cover_path(t, cx).is_some() && (!app.config.visuals.ascii_covers || app.ascii_ready(t, size)) {
        let k = reveal_k(&format!("cover:{}:{size}", track_key(t)), 0.9);
        if k < 1.0 {
            return ascii_veil(el, format!("cv-{}-{size}", track_key(t)), size, size, radius, k);
        }
    }
    el
}

pub(super) fn cover_inner(app: &mut Glyph, t: &Track, size: f32, radius: f32, cx: &mut Context<Glyph>) -> Div {
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
            // Rendered at 2x so it stays sharp on high-DPI screens.
            match app.ascii_cover(t, &p, (size * 2.0) as u32, cx) {
                Some(im) => return base.child(img(im).size(px(size)).rounded(px(radius))),
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
    // ASCII mode only ever shows the real cover; without one it is the same
    // flat placeholder as the normal mode.
    if app.cover_loading(t) || app.backend.is_none() {
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

pub fn set_ascii_effects(on: bool) {
    UI_RT.with(|rt| rt.borrow_mut().ascii_fx = on);
}

/// ASCII reveal and shimmer are on.
pub fn ascii_effects() -> bool {
    UI_RT.with(|rt| rt.borrow().ascii_fx)
}

/// Keeps the equalizer flag set (the app skipped a redraw this tick).
pub fn keep_eq_seen() {
    UI_RT.with(|rt| rt.borrow_mut().eq_seen = true);
}

/// True if a playing-song equalizer was drawn since the last call.
pub fn take_eq_seen() -> bool {
    UI_RT.with(|rt| std::mem::replace(&mut rt.borrow_mut().eq_seen, false))
}

/// The moving bars next to the song that plays (like other players show it):
/// they follow the music's spectrum, or a gentle motion when there is none
/// (embedded players); paused they stand low and still.
pub fn playing_bars(app: &Glyph, size: f32, color: Hsla) -> Div {
    let playing = app.is_playing();
    let spec = app.player.as_ref().and_then(|p| crate::guard::guard(|| p.spectrum())).unwrap_or([0.0; glyph_audio::SPECTRUM_BANDS]);
    let has_spec = spec.iter().any(|v| *v > 0.01);
    let t = elapsed_t0_secs();
    const BANDS: [usize; 4] = [2, 6, 12, 21];
    if playing {
        UI_RT.with(|rt| rt.borrow_mut().eq_seen = true);
    }
    div()
        .flex_none()
        .w(px(size))
        .h(px(size))
        .flex()
        .items_end()
        .justify_center()
        .gap(px((size * 0.1).max(1.5)))
        .children(BANDS.iter().enumerate().map(|(i, b)| {
            let k = if !playing {
                0.25
            } else if has_spec {
                (0.18 + spec[*b] * 0.95).clamp(0.15, 1.0)
            } else {
                let p = i as f32 * 1.7;
                0.2 + 0.8 * (0.5 + 0.5 * (t * (5.0 + i as f32 * 1.3) + p).sin()) * (0.6 + 0.4 * (t * 2.3 + p).sin().abs())
            };
            div().w(px((size * 0.16).max(2.0))).h(px(size * k)).rounded_t(px(1.0)).bg(color)
        }))
}
