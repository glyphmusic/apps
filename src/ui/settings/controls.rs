use super::*;

pub(crate) fn rows(items: Vec<AnyElement>) -> Div {
    let n = items.len();
    div().flex().flex_col().children(items.into_iter().enumerate().map(move |(i, r)| {
        div().when(i + 1 < n, |d| d.border_b_1().border_color(white(0.06))).child(r)
    }))
}

pub(crate) fn section_label(text: impl Into<SharedString>) -> Div {
    div()
        .pt(px(18.0))
        .pb(px(2.0))
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(white(0.45))
        .child(text.into().to_uppercase())
}

/// A flat surface for grouped content (storage breakdown, slider).
pub(super) fn surface() -> Div {
    div().rounded(px(10.0)).bg(white(0.035)).p(px(18.0))
}

pub(super) fn link_button(id: &'static str, icon_name: &'static str, label: impl Into<SharedString>, url: &'static str) -> gpui::Stateful<Div> {
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
        .relative()
        .child(hover_fill(id, white(0.05), 8.0))
        .text_size(px(13.0))
        .text_color(gpui::white())
        .child(icon(icon_name, 14.0, white(0.8)))
        .child(label.into())
        .on_click(move |_, _, cx| cx.open_url(url))
}

/// Segmented control: one pill, the selected option filled.
pub(crate) fn segmented(items: Vec<(SharedString, bool, Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>)>, cx: &mut Context<Glyph>, id: &'static str) -> Div {
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
                .when(!on, |d| d.text_color(mix(white(0.6), gpui::white(), hover_k(&format!("{id}-{i}")))).relative().child(hover_fill(format!("{id}-{i}"), white(0.04), 7.0)))
                .on_click(cx.listener(move |this, _, _, cx| f(this, cx)))
                .child(label)
        }))
}

/// Dropdown select: a button with the current value; the open list floats
/// below it (right-aligned) with a check on the selected entry.
/// Dropdown open / close animation (same feel as the context menu).
pub(super) const SEL_IN: f32 = 0.14;
pub(super) const SEL_OUT: f32 = 0.11;

thread_local! {
    /// Open dropdown and since when; one closing and since when.
    pub(super) static SEL_ANIM: std::cell::RefCell<(Option<&'static str>, std::time::Instant, Option<(&'static str, std::time::Instant)>)> =
        std::cell::RefCell::new((None, std::time::Instant::now(), None));
}

/// Notes which dropdown is open this frame (starts the open / close fades).
pub(super) fn select_anim_tick(open: Option<&'static str>) {
    SEL_ANIM.with(|a| {
        let mut a = a.borrow_mut();
        if a.0 != open {
            if let Some(prev) = a.0 {
                a.2 = Some((prev, std::time::Instant::now()));
            }
            a.0 = open;
            a.1 = std::time::Instant::now();
        }
        if a.2.is_some_and(|(_, at)| at.elapsed().as_secs_f32() >= SEL_OUT) {
            a.2 = None;
        }
    });
}

/// A dropdown is still fading in or out (the app keeps rendering).
pub fn select_animating() -> bool {
    SEL_ANIM.with(|a| {
        let a = a.borrow();
        (a.0.is_some() && a.1.elapsed().as_secs_f32() < SEL_IN + 0.02) || a.2.is_some()
    })
}

thread_local! {
    /// Window y of each dropdown button's bottom edge (measured while painting).
    pub(super) static SELECT_BOTTOM: std::cell::RefCell<std::collections::HashMap<&'static str, f32>> = std::cell::RefCell::new(Default::default());
}

pub(super) fn select(
    app: &Glyph,
    id: &'static str,
    value: SharedString,
    options: Vec<(SharedString, bool, Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>)>,
    cx: &mut Context<Glyph>,
) -> Div {
    let open = app.open_select == Some(id);
    // Fade + slide in when opening; a closing list fades out without
    // taking clicks.
    let (alpha, live) = SEL_ANIM.with(|a| {
        let a = a.borrow();
        if open {
            let k = (a.1.elapsed().as_secs_f32() / SEL_IN).min(1.0);
            (1.0 - (1.0 - k).powi(3), true)
        } else if let Some((cid, at)) = a.2.filter(|(c, _)| *c == id) {
            let _ = cid;
            ((1.0 - at.elapsed().as_secs_f32() / SEL_OUT).clamp(0.0, 1.0), false)
        } else {
            (0.0, false)
        }
    });
    let show = open || alpha > 0.0;
    // The list always opens below the button at full size; what does not
    // fit above the window's bottom edge is cut off and scrolls.
    let room = SELECT_BOTTOM.with(|m| m.borrow().get(id).copied()).map(|b| crate::ui::widgets::viewport_h() - b - 14.0).unwrap_or(600.0);
    let list_scroll = crate::ui::widgets::handle_for(&format!("select:{id}"));
    let list = show.then(|| {
        let panel = div()
                .id((id, 999usize))
                .track_scroll(&list_scroll)
                .when(live, |d| d.occlude())
                .opacity(alpha)
                .mt(px(-6.0 * (1.0 - alpha)))
                .min_w(px(240.0))
                .max_h(px(room.max(120.0)))
                .overflow_y_scroll()
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
                        // Full height always: a long list scrolls instead of squeezing.
                        .flex_none()
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
                        .when(!on, |d| {
                            let key = format!("{id}-opt-{i}");
                            d.text_color(mix(white(0.7), gpui::white(), hover_k(&key))).relative().child(hover_fill(key, white(0.05), 7.0))
                        })
                        .when(live, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                this.open_select = None;
                                f(this, cx);
                                cx.notify();
                            }))
                        })
                        .child(label)
                        .when(on, |d| d.child(icon("check", 14.0, gpui::white())))
                }));
        let wheel = crate::ui::widgets::smooth_wheel(&list_scroll, false, cx);
        div()
            .absolute()
            .top(px(40.0))
            .right_0()
            .child(deferred(anchored().anchor(gpui::Anchor::TopRight).child(div().relative().child(panel).child(wheel))).with_priority(2))
    });
    div()
        .relative()
        .child(
            gpui::canvas(
                move |b, _, _| SELECT_BOTTOM.with(|m| m.borrow_mut().insert(id, f32::from(b.origin.y + b.size.height))),
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
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
                .relative()
                .when(!open, |d| d.child(hover_fill(id, white(0.04), 8.0)))
                .text_size(px(13.5))
                .text_color(gpui::white())
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_select(id, cx)))
                .child(div().max_w(px(220.0)).truncate().child(value))
                .child(icon("chevron-down", 14.0, white(0.6))),
        )
        .children(list)
}

pub(crate) fn input_box(input: impl IntoElement) -> Div {
    div()
        // Phones: as wide as the row allows (it wraps under the label).
        .when(crate::ui::compact::TOUCH, |d| d.flex_1().min_w(px(200.0)))
        .when(!crate::ui::compact::TOUCH, |d| d.w(px(300.0)))
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
