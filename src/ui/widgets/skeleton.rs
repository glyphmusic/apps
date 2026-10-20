use super::*;

/// True if a skeleton was rendered since the last call (the app then keeps
/// animating).
pub fn take_skeleton_seen() -> bool {
    UI_RT.with(|rt| std::mem::replace(&mut rt.borrow_mut().skeleton_seen, false))
}

pub(super) fn elapsed_t0_secs() -> f32 {
    UI_RT.with(|rt| rt.borrow().t0.elapsed().as_secs_f32())
}

pub(super) const SHIMMER_RAMP: &[u8] = b" .:-=+*#";

pub(super) fn shimmer_block(cols: usize, rows: usize, fs: f32, seed: u32, radius: f32) -> Div {
    let _ = cols;
    if !ascii_effects() {
        // ASCII effects off: a plain, still box.
        return div().size_full().rounded(px(radius)).bg(gpui::hsla(0.0, 0.0, 1.0, 0.06));
    }
    div()
        .size_full()
        .overflow_hidden()
        .rounded(px(radius))
        .bg(gpui::hsla(0.0, 0.0, 1.0, 0.03))
        .child(canvas(|_, _, _| (), move |bounds, _, window, cx| paint_shimmer(bounds, rows, fs, seed, radius, window, cx)).size_full())
}

/// Paints the shimmer field into `bounds`, vertically centred.
pub(super) fn paint_shimmer(bounds: gpui::Bounds<gpui::Pixels>, rows: usize, fs: f32, seed: u32, radius: f32, window: &mut Window, cx: &mut gpui::App) {
    let key_fs = (fs * 10.0).round() as u32;
    let (glyphs, t) = UI_RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.skeleton_seen = true;
        let t = rt.t0.elapsed().as_secs_f32();
        let g = &mut rt.shimmer_glyphs;
        if g.len() > 512 {
            g.clear();
        }
        let glyphs: Vec<Option<gpui::ShapedLine>> = SHIMMER_RAMP
            .iter()
            .map(|&b| {
                (b != b' ').then(|| {
                    g.entry((b, key_fs))
                        .or_insert_with(|| {
                            window.text_system().shape_line(
                                SharedString::from((b as char).to_string()),
                                px(fs),
                                &[gpui::TextRun {
                                    len: 1,
                                    font: gpui::font(theme::MONO),
                                    color: gpui::hsla(0.0, 0.0, 1.0, 0.32),
                                    background_color: None,
                                    underline: None,
                                    strikethrough: None,
                                }],
                                None,
                            )
                        })
                        .clone()
                })
            })
            .collect();
        (glyphs, t)
    });
    let cw = glyphs.iter().flatten().next().map(|l| f32::from(l.width)).filter(|w| *w > 0.5).unwrap_or(fs * 0.6);
    let lh = fs * 1.05;
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    let cols = (w / cw).ceil().max(1.0) as usize;
    let rows = rows.min(((h / lh).floor() as usize).max(1));
    let ox = f32::from(bounds.origin.x);
    let r = radius.min(w / 2.0).min(h / 2.0);
    let oy = f32::from(bounds.origin.y) + ((h - rows as f32 * lh) / 2.0).max(0.0);
    let period = (cols as f32 + 40.0) / 60.0;
    let x0 = ((t % period) / period) * (cols as f32 + 40.0) - 20.0;
    // Only the visible part of the window is worth painting.
    let vis = window.content_mask().bounds;
    let (vy0, vy1) = (f32::from(vis.origin.y), f32::from(vis.origin.y + vis.size.height));
    let (vx0, vx1) = (f32::from(vis.origin.x), f32::from(vis.origin.x + vis.size.width));
    for y in 0..rows {
        let py = oy + y as f32 * lh;
        if py + lh < vy0 || py > vy1 {
            continue;
        }
        for x in 0..cols {
            let px_ = ox + x as f32 * cw;
            if px_ + cw < vx0 || px_ > vx1 {
                continue;
            }
            // GPUI clips children rectangularly: drop glyphs outside the rounded
            // corners so round skeletons (artist pictures) stay round.
            if r > 0.5 {
                let (cx_, cy_) = (px_ + cw / 2.0, py + lh / 2.0);
                let qx = (cx_ - ox - w / 2.0).abs() - (w / 2.0 - r);
                let qy = (cy_ - f32::from(bounds.origin.y) - h / 2.0).abs() - (h / 2.0 - r);
                if qx > 0.0 && qy > 0.0 && qx * qx + qy * qy > r * r {
                    continue;
                }
            }
            let d = (x as f32 + y as f32 * 1.6) - x0;
            let band = (-(d * d) / 18.0).exp();
            let hh = (x as u32).wrapping_mul(2654435761) ^ (y as u32 + seed).wrapping_mul(40503);
            let grain = (hh % 7) as f32 / 7.0 * 0.22;
            let v = (0.18 + grain + band * 0.8).min(1.0);
            let i = ((v * (SHIMMER_RAMP.len() - 1) as f32).round() as usize).min(SHIMMER_RAMP.len() - 1);
            if let Some(line) = &glyphs[i] {
                let _ = line.paint(gpui::point(px(px_), px(py)), px(lh), gpui::TextAlign::Left, None, window, cx);
            }
        }
    }
}

pub(super) fn seed_of(id: &str) -> u32 {
    id.bytes().fold(2166136261u32, |a, b| (a ^ b as u32).wrapping_mul(16777619))
}

/// A placeholder block with the ASCII shimmer while content loads.
pub fn skeleton(id: impl Into<SharedString>, w: f32, h: f32, radius: f32) -> Div {
    let id: SharedString = id.into();
    let fs = if h < 18.0 { (h * 0.95).max(6.0) } else { 9.0f32.max((w * h / (0.6 * 1.05 * 1500.0)).sqrt()) };
    let cols = ((w / (fs * 0.6)).ceil() as usize).max(1);
    let rows = ((h / (fs * 1.05)).floor() as usize).max(1);
    div().flex_none().w(px(w)).h(px(h)).child(shimmer_block(cols, rows, fs, seed_of(&id), radius))
}

/// One shimmering text line that takes its parent's width (clipped).
pub(super) fn skeleton_fill(id: impl Into<SharedString>, radius: f32, h: f32) -> AnyElement {
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

/// Window height (logical px) of the frame being drawn.
pub fn viewport_h() -> f32 {
    UI_RT.with(|rt| rt.borrow().cur_win().viewport_h)
}

pub fn set_viewport_h(h: f32) {
    UI_RT.with(|rt| rt.borrow_mut().cur_win_mut().viewport_h = h);
}
