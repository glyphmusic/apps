use super::*;

pub(super) const STORAGE_COLORS: [u32; 5] = [0x22c3e6, 0xa970ff, 0xff5fa2, 0xf2b84b, 0x3ddc97];
pub(super) const LIMITS_MB: [u64; 7] = [250, 1024, 2048, 3072, 5120, 10240, 0];

pub(super) fn limit_label(mb: u64) -> SharedString {
    match mb {
        0 => t("sp.storage.nolimit"),
        m if m < 1024 => format!("{m} MB").into(),
        m => format!("{} GB", m / 1024).into(),
    }
}

pub(super) fn storage_label(k: StorageKind) -> SharedString {
    match k {
        StorageKind::Covers => t("sp.storage.covers"),
        StorageKind::Downloads => t("sp.storage.downloads"),
        StorageKind::Lyrics => t("sp.storage.lyrics"),
        StorageKind::Catalog => t("sp.storage.catalog"),
        StorageKind::AudioRanges => t("sp.storage.streams"),
    }
}

/// Ring chart of the storage split, drawn as filled ring segments.
thread_local! {
    /// Storage ring segment under the mouse (index into the parts).
    pub(super) static DONUT_HOVER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

pub(super) fn donut(parts: Vec<(f32, Hsla)>, size: f32, thickness: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            // Which segment the mouse is over: by radius and angle.
            let pm = parts.iter().map(|p| p.0).collect::<Vec<f32>>();
            window.on_mouse_event(move |e: &gpui::MouseMoveEvent, phase, window, _| {
                if phase != gpui::DispatchPhase::Bubble {
                    return;
                }
                let c = bounds.center();
                let (dx, dy) = (f32::from(e.position.x - c.x), f32::from(e.position.y - c.y));
                let r = (dx * dx + dy * dy).sqrt();
                let total: f32 = pm.iter().sum();
                let hit = if r <= size / 2.0 && r >= size / 2.0 - thickness - 4.0 && total > 0.0 {
                    // 0 at the top, clockwise, like the drawing.
                    let mut a = dy.atan2(dx) + std::f32::consts::FRAC_PI_2;
                    if a < 0.0 {
                        a += std::f32::consts::TAU;
                    }
                    let mut acc = 0.0;
                    pm.iter().position(|v| {
                        acc += v / total * std::f32::consts::TAU;
                        *v > 0.0 && a < acc
                    })
                } else {
                    None
                };
                if DONUT_HOVER.with(|h| h.replace(hit)) != hit {
                    window.refresh();
                }
            });
            let hovered = DONUT_HOVER.with(|h| h.get());
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
            for (i, (v, color)) in parts.iter().enumerate() {
                if *v <= 0.0 {
                    continue;
                }
                let sweep = v / total * std::f32::consts::TAU;
                // The hovered part stays bright, the others step back.
                let color = match hovered {
                    Some(h) if h != i => color.opacity(0.35),
                    _ => *color,
                };
                ring(a, a + sweep - 0.0001, color, window);
                a += sweep;
            }
        },
    )
    .size(px(size))
}

/// Filled checkbox: white square with a black check when on.
pub(crate) fn checkbox(on: bool) -> Div {
    div()
        .flex_none()
        .size(px(16.0))
        .rounded(px(4.0))
        .flex()
        .items_center()
        .justify_center()
        .when(on, |d| d.bg(gpui::white()).child(icon("check", 12.0, gpui::black())))
        .when(!on, |d| d.border_1().border_color(white(0.35)))
}

pub(super) fn storage(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let Some(stats) = app.storage else {
        return div().child(skeleton("storage-sk", 520.0, 160.0, 10.0));
    };
    let total = stats.total();
    let selected = app.storage_selected;
    let legend = StorageKind::ALL.into_iter().enumerate().map(|(i, k)| {
        let v = stats.get(k);
        let pct = if total > 0 { v as f32 / total as f32 * 100.0 } else { 0.0 };
        let pct = if v > 0 && pct < 1.0 { "<1%".to_string() } else { format!("{pct:.0}%") };
        let on = selected[i];
        let lit = DONUT_HOVER.with(|h| h.get()) == Some(i);
        div()
            .id(("storage-kind", i))
            .when(lit, |d| d.bg(white(0.07)))
            .flex()
            .items_center()
            .gap(px(10.0))
            .h(px(36.0))
            .px(px(10.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .text_size(px(13.5))
            .relative()
            .child(hover_fill(format!("storage-kind-{i}"), white(0.05), 8.0))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.storage_selected[i] = !this.storage_selected[i];
                cx.notify();
            }))
            .child(checkbox(on))
            .child(div().ml(px(4.0)).size(px(8.0)).rounded_full().bg(gpui::rgb(STORAGE_COLORS[i])))
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if on { gpui::white() } else { white(0.55) })
                    .child(storage_label(k)),
            )
            .child(
                mono()
                    .px(px(6.0))
                    .rounded(px(4.0))
                    .bg(white(0.08))
                    .text_size(px(11.0))
                    .text_color(white(0.6))
                    .child(pct),
            )
            .child(div().flex_1())
            .child(
                mono()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if on { white(0.85) } else { white(0.45) })
                    .child(format_bytes(v)),
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
        .p(px(14.0))
        .flex()
        .items_center()
        .gap(px(28.0))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.0)).children(legend))
        .child(
            div()
                .flex_none()
                .pr(px(16.0))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .relative()
                        .size(px(116.0))
                        .child(donut(parts, 116.0, 14.0))
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .child(div().text_size(px(22.0)).font_weight(FontWeight::BOLD).text_color(gpui::white()).child(num))
                                .child(div().text_size(px(10.5)).text_color(white(0.6)).child(unit.to_uppercase())),
                        ),
                )
                .child(div().text_size(px(12.0)).text_color(white(0.6)).child(t("sp.storage.usage"))),
        );

    // Cache limit: a slider track with tick stops, filled up to the current one.
    let cur = app.config.storage.cache_limit_mb;
    let cur_i = LIMITS_MB.iter().position(|m| *m == cur);
    let n = LIMITS_MB.len();
    let stops = LIMITS_MB.into_iter().enumerate().map(|(i, mb)| {
        let on = Some(i) == cur_i;
        let passed = cur_i.map(|c| i <= c).unwrap_or(false);
        let out_filled = cur_i.map(|c| i < c).unwrap_or(false);
        // Each stop draws the half of the track into it and the half out of it.
        let seg = |filled: bool, visible: bool| {
            div().flex_1().h(px(2.0)).when(visible, |d| d.bg(if filled { gpui::white() } else { white(0.18) }))
        };
        let mark = if on {
            div().size(px(14.0)).rounded_full().bg(gpui::white())
        } else {
            div().w(px(1.5)).h(px(8.0)).bg(if passed { gpui::white() } else { white(0.3) })
        };
        div()
            .id(("limit", i))
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.0))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.set_cache_limit(mb, cx)))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(16.0))
                    .flex()
                    .items_center()
                    .child(seg(passed, i > 0))
                    .child(seg(out_filled, i + 1 < n))
                    .child(div().absolute().inset_0().flex().items_center().justify_center().child(mark)),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .when(on, |d| d.font_weight(FontWeight::SEMIBOLD).text_color(gpui::white()))
                    .when(!on, |d| d.text_color(white(0.5)).hover(|s| s.text_color(white(0.8))))
                    .child(limit_label(mb)),
            )
    });
    let limit = surface()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(div().text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(t("sp.storage.limit")))
                .child(info_tip("info-storage-limit", t("sp.storage.limit.desc"))),
        )
        .child(div().pt(px(20.0)).flex().children(stops));

    // Bottom actions: clear the checked kinds, open the cache folder.
    let sel_bytes: u64 = StorageKind::ALL
        .into_iter()
        .enumerate()
        .filter(|(i, _)| selected[*i])
        .map(|(_, k)| stats.get(k))
        .sum();
    let can_clear = sel_bytes > 0;
    let clear = div()
        .id("storage-clear")
        .flex_1()
        .h(px(40.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.0))
        .bg(gpui::white())
        .text_size(px(13.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(gpui::black())
        .when(can_clear, |d| {
            d.cursor_pointer().hover(|s| s.opacity(0.88)).on_click(cx.listener(move |this, _, _, cx| {
                for (i, k) in StorageKind::ALL.into_iter().enumerate() {
                    if this.storage_selected[i] {
                        this.clear_storage(k, cx);
                    }
                }
            }))
        })
        .when(!can_clear, |d| d.opacity(0.4))
        .child(tf("sp.storage.clearsel", &[("size", &format_bytes(sel_bytes))]));
    let data_dir = glyph_core::paths::data_dir();
    let open = div()
        .id("storage-open")
        .flex_none()
        .h(px(40.0))
        .px(px(18.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .rounded(px(10.0))
        .bg(white(0.07))
        .cursor_pointer()
        .relative()
        .child(hover_fill("storage-reveal", white(0.04), 10.0))
        .text_size(px(13.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(gpui::white())
        .on_click(move |_, _, cx| cx.reveal_path(&data_dir))
        .child(icon("folder-open", 15.0, white(0.85)))
        .child(t("sp.storage.openfolder"));

    div()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .pt(px(6.0))
        .child(breakdown)
        .child(limit)
        .child({
            let on = app.config.storage.data_saver;
            surface().py(px(4.0)).child(row(
                t("sp.storage.saver"),
                Some(t("sp.storage.saver.desc")),
                switch("data-saver", on).on_click(cx.listener(move |this, _, _, cx| this.set_data_saver(!on, cx))),
            ))
        })
        .child(div().pt(px(6.0)).flex().gap(px(10.0)).child(clear).child(open))
}
