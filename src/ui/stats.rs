//! Listening statistics page (opt-in): listening time, plays, top artists and
//! songs, when the user listens, month by month. All devices of the account
//! add up (see `glyph_core::stats`).

use crate::app::Glyph;
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::widgets::*;
use glyph_core::stats::{self, Range};
use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, Hsla, SharedString, Window};

fn white(a: f32) -> Hsla {
    gpui::hsla(0.0, 0.0, 1.0, a)
}

/// "12 h 05 min" / "42 min".
pub fn fmt_listen(ms: u64) -> String {
    let min = ms / 60_000;
    if min >= 60 {
        format!("{} h {:02} min", min / 60, min % 60)
    } else {
        format!("{min} min")
    }
}

fn tile(label: SharedString, value: String, sub: Option<String>) -> AnyElement {
    div()
        .flex_1()
        .min_w(px(150.0))
        .p(px(16.0))
        .rounded(px(12.0))
        .bg(theme::card())
        .border_1()
        .border_color(theme::border_faint())
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(mono().text_size(px(11.0)).text_color(theme::text_faint()).child(format!("[ {} ]", label.to_uppercase())))
        .child(div().text_size(px(26.0)).font_weight(FontWeight::BOLD).text_color(theme::text()).child(value))
        .children(sub.map(|s| div().text_size(theme::text_sm()).text_color(theme::text_dim()).child(s)))
        .into_any_element()
}

/// Vertical bars with labels under them (hours, weekdays, months).
fn bars(id: &'static str, values: &[u64], labels: &[String], highlight: Option<usize>) -> AnyElement {
    let max = values.iter().copied().max().unwrap_or(0).max(1) as f32;
    div()
        .id(id)
        .flex()
        .items_end()
        .gap(px(4.0))
        .h(px(140.0))
        .children(values.iter().enumerate().map(|(i, v)| {
            let k = *v as f32 / max;
            let on = highlight == Some(i);
            div()
                .flex_1()
                .min_w(px(6.0))
                .h_full()
                .flex()
                .flex_col()
                .justify_end()
                .items_center()
                .gap(px(4.0))
                .child(
                    div()
                        .w_full()
                        .h(px((110.0 * k).max(if *v > 0 { 3.0 } else { 1.0 })))
                        .rounded_t(px(4.0))
                        .bg(if on { theme::pink() } else { theme::pink().opacity(0.25 + 0.55 * k) }),
                )
                .child(mono().h(px(14.0)).text_size(px(10.0)).text_color(theme::text_faint()).child(labels.get(i).cloned().unwrap_or_default()))
        }))
        .into_any_element()
}

fn section(title: SharedString, body: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(div().text_size(px(18.0)).font_weight(FontWeight::SEMIBOLD).child(title))
        .child(body)
        .into_any_element()
}

/// "Your 2026 in Glyph": the year at a glance (only with statistics on).
fn recap(app: &mut Glyph, year: String, cx: &mut Context<Glyph>) -> Vec<AnyElement> {
    let s = app.library.stats.year(&year);
    let mut body: Vec<AnyElement> = Vec::new();
    body.push(
        div()
            .id("recap-back")
            .flex()
            .items_center()
            .gap(px(6.0))
            .cursor_pointer()
            .text_size(theme::text_sm())
            .text_color(theme::text_dim())
            .hover(|d| d.text_color(theme::text()))
            .on_click(cx.listener(|this, _, _, cx| this.close_recap(cx)))
            .child(icon("chevron-left", 14.0, theme::text_dim()))
            .child(t("stats.title"))
            .into_any_element(),
    );
    body.push(
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(mono().text_size(px(11.0)).text_color(theme::text_faint()).child("[ RECAP ]"))
            .child(heading(tf("recap.title", &[("year", &year)])))
            .into_any_element(),
    );
    if s.ms == 0 {
        body.push(ascii_state(ART_EMPTY, t("stats.empty.title"), t("stats.empty.sub")).into_any_element());
        return body;
    }
    let top_artists = stats::top_artists(&s, 5);
    let top_tracks = stats::top_tracks(&s, 50);
    let days = (s.ms / 86_400_000) as u32;
    body.push(
        div()
            .flex()
            .flex_wrap()
            .gap(px(10.0))
            .child(tile(t("stats.time"), fmt_listen(s.ms), (days > 0).then(|| tf("recap.days", &[("n", &days)]).to_string())))
            .child(tile(t("stats.plays"), s.plays.to_string(), None))
            .child(tile(t("stats.artists"), s.artists.len().to_string(), None))
            .child(tile(t("stats.songs"), s.tracks.len().to_string(), None))
            .into_any_element(),
    );
    // Number one: the artist with the most listening time, shown big.
    if let Some((name, a)) = top_artists.first().cloned() {
        let seed = top_tracks.iter().find(|x| glyph_core::dedupe::by_artist(&x.track.artist, &name)).map(|x| x.track.clone());
        let art = seed.map(|tr| cover(app, &tr, 120.0, 10.0, cx));
        let n = name.clone();
        body.push(
            div()
                .id("recap-top-artist")
                .flex()
                .items_center()
                .gap(px(20.0))
                .p(px(16.0))
                .rounded(px(12.0))
                .bg(theme::card())
                .border_1()
                .border_color(theme::border_faint())
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.open_artist(&n, cx)))
                .children(art)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .min_w_0()
                        .child(mono().text_size(px(11.0)).text_color(theme::text_faint()).child(format!("[ {} ]", t("recap.top_artist").to_uppercase())))
                        .child(div().truncate().text_size(px(28.0)).font_weight(FontWeight::BOLD).text_color(theme::text()).child(name))
                        .child(div().text_size(theme::text_sm()).text_color(theme::text_dim()).child(tf("recap.top_artist_line", &[("t", &fmt_listen(a.ms)), ("n", &a.plays)]).to_string())),
                )
                .into_any_element(),
        );
    }
    // Facts: when the year was listened to.
    let peak_h = s.hours.iter().enumerate().filter(|(_, v)| **v > 0).max_by_key(|(_, v)| **v).map(|(i, _)| i);
    let wd = ["stats.mon", "stats.tue", "stats.wed", "stats.thu", "stats.fri", "stats.sat", "stats.sun"];
    let peak_d = s.weekdays.iter().enumerate().filter(|(_, v)| **v > 0).max_by_key(|(_, v)| **v).map(|(i, _)| t(wd[i]).to_string());
    let months: Vec<(String, u64)> = app.library.stats.by_month().into_iter().filter(|(m, _)| m.starts_with(year.as_str())).collect();
    let peak_m = months.iter().max_by_key(|(_, v)| *v).map(|(m, _)| m.clone());
    let mut facts = div().flex().flex_wrap().gap(px(10.0));
    if let Some(h) = peak_h {
        facts = facts.child(tile(t("recap.peak_hour"), format!("{h:02}:00"), None));
    }
    if let Some(d) = peak_d {
        facts = facts.child(tile(t("recap.peak_day"), d, None));
    }
    if let Some(m) = peak_m {
        facts = facts.child(tile(t("recap.peak_month"), m, None));
    }
    body.push(facts.into_any_element());
    // Top 5 artists and top songs.
    let artists = div().flex().flex_col().gap(px(6.0)).children(top_artists.into_iter().enumerate().map(|(i, (name, a))| {
        let n = name.clone();
        div()
            .id(("recap-artist", i))
            .flex()
            .items_center()
            .gap(px(12.0))
            .h(px(34.0))
            .px(px(10.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .relative()
            .child(hover_fill(format!("recap-artist-{i}"), white(0.05), 8.0))
            .on_click(cx.listener(move |this, _, _, cx| this.open_artist(&n, cx)))
            .child(mono().w(px(22.0)).text_size(theme::text_sm()).text_color(theme::text_faint()).child(format!("{:02}", i + 1)))
            .child(div().flex_1().truncate().text_size(theme::text_md()).child(name))
            .child(mono().w(px(90.0)).flex().justify_end().text_size(theme::text_sm()).text_color(theme::text_dim()).child(fmt_listen(a.ms)))
    }));
    body.push(section(t("recap.top_artists"), artists));
    let songs: Vec<glyph_protocol::Track> = top_tracks.iter().map(|x| x.track.clone()).collect();
    let list = std::rc::Rc::new(songs.clone());
    let rows = div().flex().flex_col().children((0..list.len().min(10)).map(|i| crate::ui::lists::song_row(app, list.clone(), i, "recap-row", None, false, cx)));
    body.push(section(t("recap.top_songs"), rows));
    // Keep or send the year: the top songs as a playlist or a link.
    if !songs.is_empty() {
        let (y1, y2, s1, s2) = (year.clone(), year.clone(), songs.clone(), songs);
        body.push(
            div()
                .flex()
                .gap(px(8.0))
                .child(primary_button("recap-save", t("recap.save")).on_click(cx.listener(move |this, _, _, cx| this.save_recap_playlist(&y1, s1.clone(), cx))))
                .child(button("recap-share", t("recap.share")).on_click(cx.listener(move |this, _, _, cx| {
                    let title = tf("recap.title", &[("year", &y2)]).to_string();
                    this.share_songs(title, s2.clone(), "playlist", None, cx)
                })))
                .into_any_element(),
        );
    }
    body
}

/// Entry to the recap on the statistics page: one card per year with data.
fn recap_cards(app: &Glyph, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let years: Vec<String> = app.library.stats.years().into_iter().take(2).collect();
    if years.is_empty() {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_wrap()
            .gap(px(10.0))
            .children(years.into_iter().enumerate().map(|(i, y)| {
                let y2 = y.clone();
                div()
                    .id(("recap-card", i))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(16.0))
                    .h(px(52.0))
                    .rounded(px(12.0))
                    .bg(theme::card())
                    .border_1()
                    .border_color(theme::border_faint())
                    .cursor_pointer()
                    .relative()
                    .child(hover_fill(format!("recap-card-{i}"), white(0.04), 12.0))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_recap(y2.clone(), cx)))
                    .child(icon("sparkles", 16.0, theme::pink()))
                    .child(div().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).child(tf("recap.title", &[("year", &y)])))
                    .child(icon("chevron-right", 14.0, theme::text_dim()))
            }))
            .into_any_element(),
    )
}

pub fn view(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    if let Some(year) = app.recap_year.clone().filter(|_| app.config.stats.enabled) {
        let body = recap(app, year, cx);
        return div()
            .id("stats-scroll")
            .track_scroll(&app.main_scroll)
            .size_full()
            .overflow_y_scroll()
            .px(px(theme::PAGE_X))
            .py(px(20.0))
            .child(div().flex().flex_col().gap(px(28.0)).children(body))
            .into_any_element();
    }
    let back = crate::ui::lists::back_button(app, cx);
    let range = app.stats_range;
    let chips = div().flex().gap(px(8.0)).children(
        [(Range::ThisMonth, t("stats.range.month")), (Range::LastMonth, t("stats.range.last_month")), (Range::Year, t("stats.range.year")), (Range::All, t("stats.range.all"))]
            .into_iter()
            .enumerate()
            .map(|(i, (r, label))| {
                div()
                    .id(("stats-range", i))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.stats_range = r;
                        cx.notify();
                    }))
                    .child(chip(label, r == range))
            }),
    );
    let head = div().flex().items_center().justify_between().flex_wrap().gap(px(12.0)).child(heading(t("stats.title"))).child(chips);
    let mut body: Vec<AnyElement> = Vec::new();
    if !app.config.stats.enabled {
        body.push(
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(14.0))
                .child(ascii_state(ART_EMPTY, t("stats.off.title"), t("stats.off.sub")))
                .child(primary_button("stats-enable", t("stats.enable")).on_click(cx.listener(|this, _, _, cx| this.set_stats_enabled(true, cx))))
                .into_any_element(),
        );
    } else {
        let now = stats::When::now();
        let s = app.library.stats.summary(range, now);
        if s.ms == 0 && s.plays == 0 {
            body.push(ascii_state(ART_EMPTY, t("stats.empty.title"), t("stats.empty.sub")).into_any_element());
        } else {
            body.extend(recap_cards(app, cx));
            let top_artists = stats::top_artists(&s, 10);
            let top_tracks = stats::top_tracks(&s, 10);
            let avg_day = s.ms / s.days.iter().filter(|d| **d > 0).count().max(1) as u64;
            body.push(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(10.0))
                    .child(tile(t("stats.time"), fmt_listen(s.ms), Some(tf("stats.per_day", &[("t", &fmt_listen(avg_day))]).to_string())))
                    .child(tile(t("stats.plays"), s.plays.to_string(), Some(tf("stats.skips", &[("n", &s.skips)]).to_string())))
                    .child(tile(t("stats.artists"), s.artists.len().to_string(), top_artists.first().map(|a| tf("stats.top_is", &[("name", &a.0)]).to_string())))
                    .child(tile(t("stats.songs"), s.tracks.len().to_string(), top_tracks.first().map(|x| tf("stats.top_is", &[("name", &x.track.title)]).to_string())))
                    .into_any_element(),
            );
            // Top artists: bar per artist, click opens the artist.
            let max_a = top_artists.first().map_or(1, |a| a.1.ms.max(1)) as f32;
            let artists = div().flex().flex_col().gap(px(6.0)).children(top_artists.into_iter().enumerate().map(|(i, (name, a))| {
                let n = name.clone();
                div()
                    .id(("stats-artist", i))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .h(px(34.0))
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .relative()
                    .child(hover_fill(format!("stats-artist-{i}"), white(0.05), 8.0))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_artist(&n, cx)))
                    .child(mono().w(px(22.0)).text_size(theme::text_sm()).text_color(theme::text_faint()).child(format!("{:02}", i + 1)))
                    .child(div().w(px(200.0)).truncate().text_size(theme::text_md()).child(name))
                    .child(div().flex_1().h(px(6.0)).rounded_full().bg(white(0.06)).child(div().h_full().rounded_full().w(gpui::relative(a.ms as f32 / max_a)).bg(theme::pink())))
                    .child(mono().w(px(90.0)).flex().justify_end().text_size(theme::text_sm()).text_color(theme::text_dim()).child(fmt_listen(a.ms)))
            }));
            body.push(section(t("stats.top_artists"), artists));
            // Top songs: normal song rows (play from here).
            let songs: Vec<glyph_protocol::Track> = top_tracks.iter().map(|x| x.track.clone()).collect();
            let list = std::rc::Rc::new(songs);
            let rows = div().flex().flex_col().children((0..list.len()).map(|i| {
                let extra = top_tracks.get(i).map(|x| tf("stats.track_line", &[("n", &x.plays), ("t", &fmt_listen(x.ms))]).to_string()).unwrap_or_default();
                div()
                    .flex()
                    .items_center()
                    .child(div().flex_1().min_w_0().child(crate::ui::lists::song_row(app, list.clone(), i, "stats-row", None, false, cx)))
                    .child(mono().w(px(150.0)).flex().justify_end().text_size(theme::text_xs()).text_color(theme::text_faint()).child(extra))
            }));
            body.push(section(t("stats.top_songs"), rows));
            // When: by hour and weekday.
            let hours: Vec<String> = (0..24).map(|h| if h % 3 == 0 { format!("{h:02}") } else { String::new() }).collect();
            let wd: Vec<String> = ["stats.mon", "stats.tue", "stats.wed", "stats.thu", "stats.fri", "stats.sat", "stats.sun"].iter().map(|k| t(k).to_string()).collect();
            let peak_h = s.hours.iter().enumerate().max_by_key(|(_, v)| **v).map(|(i, _)| i);
            let peak_d = s.weekdays.iter().enumerate().max_by_key(|(_, v)| **v).map(|(i, _)| i);
            body.push(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(28.0))
                    .child(div().flex_1().min_w(px(320.0)).child(section(
                        peak_h.map(|h| tf("stats.by_hour_peak", &[("h", &format!("{h:02}:00"))])).unwrap_or_else(|| t("stats.by_hour")),
                        bars("stats-hours", &s.hours, &hours, peak_h),
                    )))
                    .child(div().flex_1().min_w(px(260.0)).child(section(t("stats.by_weekday"), bars("stats-days", &s.weekdays, &wd, peak_d))))
                    .into_any_element(),
            );
            // History: the last twelve months.
            let months = app.library.stats.by_month();
            let months: Vec<(String, u64)> = months.into_iter().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect();
            if months.len() > 1 {
                let labels: Vec<String> = months.iter().map(|(m, _)| m.get(5..7).unwrap_or("").to_string()).collect();
                let values: Vec<u64> = months.iter().map(|(_, v)| *v).collect();
                body.push(section(t("stats.history"), bars("stats-months", &values, &labels, Some(values.len() - 1))));
            }
            body.push(div().text_size(theme::text_xs()).text_color(theme::text_faint()).child(if app.signed_in().is_some() { t("stats.note.synced") } else { t("stats.note.local") }).into_any_element());
        }
    }
    div()
        .id("stats-scroll")
        .track_scroll(&app.main_scroll)
        .size_full()
        .overflow_y_scroll()
        .px(px(theme::PAGE_X))
        .py(px(20.0))
        .child(div().flex().flex_col().gap(px(28.0)).children(back).child(head).children(body))
        .into_any_element()
}
