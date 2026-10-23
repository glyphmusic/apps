//! Jam page of the Connect panel (its header's Jam button): start or join
//! a jam (or wait to be let in), then what plays at the host, reactions, the invite (code, QR code,
//! link), people waiting, who is in, what guests may do or where a guest
//! listens, the songs coming up with who added them, and what happened
//! (see `crate::app::jam`).

use crate::app::{jam_reaction_label, Glyph};
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::widgets::*;
use glyph_protocol::jam::{Event, Item, Perms, Reaction, What};
use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, SharedString};
use std::sync::Arc;
use std::time::Duration;

/// Side of one QR module (px); whole pixels keep the code sharp.
const QR_CELL: f32 = 4.0;
/// White border around the code (px), so cameras find it.
const QR_QUIET: f32 = 10.0;
/// Feed entries shown.
const FEED_SHOWN: usize = 8;
/// Queued songs shown until "Show more" (each row has a cover).
const QUEUE_SHOWN: usize = 20;

/// `[ LABEL ]` heading, as in the rest of the panel.
fn section(text: SharedString) -> AnyElement {
    mono().px(px(16.0)).pt(px(4.0)).pb(px(8.0)).text_size(px(11.0)).text_color(theme::text_faint()).child(format!("[ {} ]", text.to_uppercase())).into_any_element()
}

/// Small grey note line.
fn note(text: impl Into<SharedString>) -> AnyElement {
    div().text_size(theme::text_xs()).text_color(theme::text_dim()).child(text.into()).into_any_element()
}

/// Round profile picture of a member (the person icon without one).
pub fn person(app: &mut Glyph, handle: &str, sha: Option<String>, size: f32, cx: &mut Context<Glyph>) -> AnyElement {
    let path = sha.and_then(|sha| app.profile_image(handle, &sha, cx));
    crate::ui::profile::avatar(path, size).border_0().into_any_element()
}

/// Who added a song of the queue (the queue panel, in a jam).
pub fn added_by(app: &mut Glyph, track: &glyph_protocol::Track, i: usize, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let (handle, sha) = app.jam_added_by(track)?;
    let pic = person(app, &handle, sha, 20.0, cx);
    Some(with_tip(div().id(("q-by", i)).flex_none().child(pic), Some(tf("jam.added_by", &[("who", &format!("@{handle}"))]))).into_any_element())
}

/// The invite QR code (dark modules on white), cached per link.
fn qr(app: &mut Glyph, link: &str) -> Option<AnyElement> {
    if app.jam.qr.as_ref().is_none_or(|(l, _, _)| l != link) {
        let code = qrcode::QrCode::with_error_correction_level(link.as_bytes(), qrcode::EcLevel::M).ok()?;
        let n = code.width();
        let dark: Vec<bool> = code.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect();
        app.jam.qr = Some((link.to_string(), n, Arc::new(dark)));
    }
    let (_, n, dark) = app.jam.qr.clone()?;
    let side = n as f32 * QR_CELL;
    let modules = gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let (x0, y0) = (f32::from(bounds.origin.x).round(), f32::from(bounds.origin.y).round());
            // One quad per run of dark modules in a row, not one per module.
            for (y, row) in dark.chunks(n).enumerate() {
                let mut x = 0;
                while x < n {
                    if !row[x] {
                        x += 1;
                        continue;
                    }
                    let start = x;
                    while x < n && row[x] {
                        x += 1;
                    }
                    let origin = gpui::point(px(x0 + start as f32 * QR_CELL), px(y0 + y as f32 * QR_CELL));
                    let b = gpui::Bounds { origin, size: gpui::size(px((x - start) as f32 * QR_CELL), px(QR_CELL)) };
                    window.paint_quad(gpui::fill(b, gpui::black()));
                }
            }
        },
    )
    .size(px(side));
    Some(div().flex_none().p(px(QR_QUIET)).rounded(px(8.0)).bg(gpui::white()).child(modules).into_any_element())
}

pub fn section_el(app: &mut Glyph, cx: &mut Context<Glyph>) -> AnyElement {
    if app.jam_active() {
        active(app, cx)
    } else {
        start(app, cx)
    }
}

/// Not in a jam: start one, join with a code, or wait to be let in.
fn start(app: &mut Glyph, cx: &mut Context<Glyph>) -> AnyElement {
    if let Some(code) = app.jam.waiting.clone() {
        return div()
            .flex()
            .flex_col()
            .pb(px(12.0))
            .child(
                div()
                    .mx(px(10.0))
                    .p(px(12.0))
                    .rounded(px(10.0))
                    .bg(theme::card())
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(icon("users", 16.0, theme::text_dim()))
                            .child(div().flex_1().text_size(theme::text_sm()).text_color(theme::text()).child(format!("{} {}", app.spinner(), t("jam.waiting"))))
                            .child(mono().text_size(theme::text_xs()).text_color(theme::text_dim()).child(code)),
                    )
                    .child(note(t("jam.waiting.sub")))
                    .child(div().flex().child(button("jam-wait-cancel", t("jam.cancel")).on_click(cx.listener(|this, _, _, cx| this.jam_stop_waiting(cx))))),
            )
            .into_any_element();
    }
    let busy = app.jam.busy;
    let input = app.jam_input(cx);
    let sub: SharedString = if busy { format!("{} {}", app.spinner(), t("jam.working")).into() } else { t("jam.start.sub") };
    let start_row = div()
        .id("jam-start")
        .mx(px(10.0))
        .flex()
        .items_center()
        .gap(px(12.0))
        .h(px(52.0))
        .px(px(6.0))
        .rounded(px(8.0))
        .relative()
        .cursor_pointer()
        .child(hover_fill("jam-start", theme::card(), 8.0))
        .child(
            div()
                .flex_none()
                .size(px(40.0))
                .rounded(px(6.0))
                .bg(theme::card())
                .flex()
                .items_center()
                .justify_center()
                .child(icon("users", 20.0, theme::text_dim())),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().truncate().text_size(theme::text_sm()).font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(t("jam.start")))
                .child(div().truncate().text_size(theme::text_xs()).text_color(theme::text_dim()).child(sub)),
        )
        .on_click(cx.listener(|this, _, _, cx| this.jam_start(cx)));
    let join = div()
        .mx(px(16.0))
        .mt(px(8.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(crate::ui::settings::input_box(input.clone()).flex_1().min_w_0())
        .child(button("jam-join", t("jam.join")).on_click(cx.listener(move |this, _, _, cx| {
            let v = input.read(cx).value.clone();
            this.jam_join(&v, cx);
        })));
    div()
        .flex()
        .flex_col()
        .pb(px(12.0))
        .child(start_row)
        .child(join)
        .when_some(app.jam.error.clone(), |d, e| d.child(div().mx(px(16.0)).mt(px(8.0)).child(note(e))))
        .into_any_element()
}

/// One switch of the host's settings.
fn perm_toggle(id: &'static str, on: bool, label: SharedString, perms: Perms, set: fn(&mut Perms, bool), cx: &mut Context<Glyph>) -> AnyElement {
    toggle(id, on, label)
        .on_click(cx.listener(move |this, _, _, cx| {
            let mut p = perms;
            set(&mut p, !on);
            this.jam_set_perms(p, cx);
        }))
        .into_any_element()
}

/// What an activity entry says.
fn feed_text(e: &Event) -> SharedString {
    let who = format!("@{}", e.who);
    match &e.what {
        What::Joined => tf("jam.f.joined", &[("who", &who)]),
        What::Left => tf("jam.f.left", &[("who", &who)]),
        What::Added { title } => tf("jam.f.added", &[("who", &who), ("title", title)]),
        What::Played { title } => tf("jam.f.played", &[("who", &who), ("title", title)]),
        What::React { reaction } => format!("{who} · {}", jam_reaction_label(*reaction)).into(),
        What::Host => tf("jam.f.host", &[("who", &who)]),
    }
}

/// "now", "3 min", "2 h" since `at_ms` (server time).
fn ago(now_ms: u64, at_ms: u64) -> String {
    let s = now_ms.saturating_sub(at_ms) / 1000;
    if s < 60 {
        t("jam.now").to_string()
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h", s / 3600)
    }
}

/// Guests: what plays at the host, and where it plays.
fn now_card(app: &mut Glyph, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let j = app.jam.jam.as_ref().filter(|j| !j.host)?;
    let now = j.state.now.clone()?;
    let pos = j.host_position().unwrap_or(0);
    let dur = now.track.duration_ms.unwrap_or(0);
    let frac = if dur > 0 { (pos as f32 / dur as f32).clamp(0.0, 1.0) } else { 0.0 };
    let online = j.host_online();
    let unavailable = j.listen_here && j.unavailable.as_deref().is_some_and(|k| k == crate::app::track_key(now.resolved.as_ref().unwrap_or(&now.track)));
    let place = if j.listen_here { t("jam.playing_here") } else { tf("jam.playing_at", &[("who", &format!("@{}", j.state.host))]) };
    let art = cover(app, &now.track, 48.0, 6.0, cx);
    Some(
        div()
            .mx(px(10.0))
            .mb(px(8.0))
            .p(px(10.0))
            .rounded(px(10.0))
            .bg(theme::card())
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(art)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(song_title(now.track.title.clone(), crate::ui::widgets::rating(&now.track)).text_size(theme::text_sm()).text_color(theme::text()))
                            .child(div().truncate().text_size(theme::text_xs()).text_color(theme::text_dim()).child(now.track.artist.clone())),
                    )
                    .child(icon(if now.playing { "audio-lines" } else { "pause" }, 14.0, if now.playing { theme::pink() } else { theme::text_dim() })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(mono().text_size(px(10.0)).text_color(theme::text_faint()).child(fmt_time(Duration::from_millis(pos))))
                    .child(div().flex_1().h(px(3.0)).rounded_full().bg(theme::active()).child(div().h_full().rounded_full().bg(theme::text_dim()).w(gpui::relative(frac))))
                    .child(mono().text_size(px(10.0)).text_color(theme::text_faint()).child(fmt_time(Duration::from_millis(dur)))),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.0))
                    .child(div().text_size(theme::text_xs()).text_color(theme::text_dim()).child(place))
                    .when(!online, |d| d.child(kind_chip(t("jam.host_offline"))))
                    .when(unavailable, |d| d.child(kind_chip(t("jam.unavailable")))),
            )
            .into_any_element(),
    )
}

/// In a jam.
fn active(app: &mut Glyph, cx: &mut Context<Glyph>) -> AnyElement {
    let Some(j) = app.jam.jam.as_ref() else { return div().into_any_element() };
    let host = j.host;
    let me = j.me.clone();
    let code = j.code.clone();
    let link = j.link();
    let members = j.members();
    let perms = j.perms();
    let approve = j.approve_joins();
    let listen_here = j.listen_here;
    let knocks = j.knocks.clone();
    let copied = j.copied.is_some_and(|c| c.elapsed() < Duration::from_secs(2));
    let host_name = members.iter().find(|m| m.host).map(|m| m.handle.clone()).unwrap_or_default();
    let server_now = j.server_now();
    let feed: Vec<Event> = j.feed().iter().rev().take(FEED_SHOWN).cloned().collect();
    let feed_pics: Vec<Option<String>> = feed.iter().map(|e| j.avatar_of(&e.who)).collect();
    let (queue, queued) = app.jam_queue(if app.jam.queue_all { usize::MAX } else { QUEUE_SHOWN });
    let more = queued > queue.len();
    let votes_of: Vec<bool> = queue.iter().map(|i| j.voted(i)).collect();
    let avatars: Vec<Option<String>> = queue.iter().map(|i| j.avatar_of(&i.by)).collect();

    let now_el = now_card(app, cx);

    // Reactions to the song playing.
    let reactions = div().flex().items_center().gap(px(2.0)).children(Reaction::ALL.iter().map(|r| {
        let r = *r;
        with_tip(icon_button_plain(format!("jam-r-{}", r.icon()), r.icon(), 15.0, theme::text_dim()), Some(jam_reaction_label(r)))
            .on_click(cx.listener(move |this, _, _, cx| this.jam_react(r, cx)))
            .into_any_element()
    }));

    // Invite: QR code (scanned with a phone it opens the invite page, which
    // opens Glyph), the code, the link.
    let qr_el = qr(app, &link);
    let card = div()
        .mx(px(10.0))
        .mb(px(4.0))
        .p(px(12.0))
        .rounded(px(10.0))
        .bg(theme::card())
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(icon("users", 16.0, theme::pink()))
                .child(div().flex_1().text_size(theme::text_md()).font_weight(FontWeight::SEMIBOLD).text_color(theme::pink()).child(t("jam.title")))
                .child(reactions)
                .child(
                    with_tip(icon_button_plain("jam-copy", if copied { "check" } else { "copy" }, 15.0, theme::text_dim()), Some(t("jam.copy")))
                        .on_click(cx.listener(|this, _, _, cx| this.jam_copy(false, cx))),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .children(qr_el)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(mono().text_size(px(20.0)).text_color(theme::text()).child(code))
                        .child(note(if host { t("jam.hosting") } else { tf("jam.hosted_by", &[("who", &format!("@{host_name}"))]) }))
                        .child(div().text_size(theme::text_xs()).text_color(theme::text_faint()).child(t("jam.scan")))
                        .child(
                            div().pt(px(2.0)).flex().child(
                                text_button("jam-link", t("jam.copy_link")).text_size(theme::text_xs()).on_click(cx.listener(|this, _, _, cx| this.jam_copy(true, cx))),
                            ),
                        ),
                ),
        );

    // Host: people waiting to be let in.
    let mut waiting: Vec<AnyElement> = Vec::new();
    for (i, (handle, sha)) in knocks.iter().enumerate() {
        let pic = person(app, handle, sha.clone(), 28.0, cx);
        let (yes, no) = (handle.clone(), handle.clone());
        waiting.push(
            div()
                .id(("jam-k", i))
                .flex()
                .items_center()
                .gap(px(10.0))
                .h(px(40.0))
                .px(px(6.0))
                .child(pic)
                .child(div().flex_1().min_w_0().truncate().text_size(theme::text_sm()).text_color(theme::text()).child(format!("@{handle}")))
                .child(primary_button(("jam-k-yes", i), t("jam.allow")).h(px(26.0)).px(px(10.0)).on_click(cx.listener(move |this, _, _, cx| this.jam_admit(&yes, true, cx))))
                .child(
                    with_tip(icon_button_plain(("jam-k-no", i), "x", 13.0, theme::text_faint()), Some(t("jam.deny")))
                        .on_click(cx.listener(move |this, _, _, cx| this.jam_admit(&no, false, cx))),
                )
                .into_any_element(),
        );
    }

    // People.
    let mut people: Vec<AnyElement> = Vec::new();
    for (i, m) in members.iter().enumerate() {
        let pic = person(app, &m.handle, m.avatar.clone(), 28.0, cx);
        let handle = m.handle.clone();
        people.push(
            div()
                .id(gpui::ElementId::Name(format!("jam-m-{i}").into()))
                .flex()
                .items_center()
                .gap(px(10.0))
                .h(px(36.0))
                .px(px(6.0))
                .rounded(px(8.0))
                .relative()
                .child(pic)
                .child(div().flex_1().min_w_0().truncate().text_size(theme::text_sm()).text_color(theme::text()).child(format!("@{}", m.handle)))
                .when(m.host, |d| d.child(kind_chip(t("jam.host"))))
                .when(m.handle == me && !m.host, |d| d.child(kind_chip(t("jam.you"))))
                .when(host && !m.host, |d| {
                    d.child(
                        with_tip(icon_button_plain(format!("jam-kick-{i}"), "x", 13.0, theme::text_faint()), Some(t("jam.kick")))
                            .on_click(cx.listener(move |this, _, _, cx| this.jam_kick(&handle, cx))),
                    )
                })
                .into_any_element(),
        );
    }

    // Host: what guests may do. Guest: where the music plays.
    let settings: AnyElement = if host {
        div()
            .flex()
            .flex_col()
            .child(section(t("jam.perms")))
            .child(
                div()
                    .mx(px(16.0))
                    .mb(px(6.0))
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(perm_toggle("jam-p-add", perms.add, t("jam.p.add"), perms, |p, v| p.add = v, cx))
                    .child(perm_toggle("jam-p-now", perms.play_now, t("jam.p.play_now"), perms, |p, v| p.play_now = v, cx))
                    .child(perm_toggle("jam-p-skip", perms.skip, t("jam.p.skip"), perms, |p, v| p.skip = v, cx))
                    .child(perm_toggle("jam-p-seek", perms.seek, t("jam.p.seek"), perms, |p, v| p.seek = v, cx))
                    .child(perm_toggle("jam-p-pause", perms.pause, t("jam.p.pause"), perms, |p, v| p.pause = v, cx))
                    .child(toggle("jam-approve", approve, t("jam.approve")).on_click(cx.listener(move |this, _, _, cx| this.jam_set_approve(!approve, cx)))),
            )
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .child(section(t("jam.listen")))
            .child(
                div()
                    .mx(px(16.0))
                    .mb(px(6.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(toggle("jam-here", listen_here, t("jam.listen_here")).on_click(cx.listener(move |this, _, _, cx| this.jam_set_listen_here(!listen_here, cx))))
                    .child(note(if listen_here { t("jam.listen_here.sub") } else { tf("jam.listen_host.sub", &[("who", &format!("@{host_name}"))]) }))
                    .when(!perms.any_control(), |d| d.child(note(t("jam.host_controls")))),
            )
            .into_any_element()
    };

    // Songs coming up.
    let rows: Vec<AnyElement> = queue
        .iter()
        .zip(votes_of)
        .zip(avatars)
        .enumerate()
        .map(|(i, ((item, voted), sha))| song_row(app, i, item, voted, sha, host || item.by == me, perms.add || host, cx))
        .collect();
    let empty = rows.is_empty();

    // What happened.
    let mut events: Vec<AnyElement> = Vec::new();
    for (i, (e, sha)) in feed.iter().zip(feed_pics).enumerate() {
        let pic = person(app, &e.who, sha, 18.0, cx);
        events.push(
            div()
                .id(("jam-f", i))
                .flex()
                .items_center()
                .gap(px(8.0))
                .min_h(px(26.0))
                .px(px(6.0))
                .child(pic)
                .child(div().flex_1().min_w_0().truncate().text_size(theme::text_xs()).text_color(theme::text_dim()).child(feed_text(e)))
                .child(mono().flex_none().text_size(px(10.0)).text_color(theme::text_faint()).child(ago(server_now, e.at_ms)))
                .into_any_element(),
        );
    }

    div()
        .flex()
        .flex_col()
        .pb(px(12.0))
        .children(now_el)
        .child(card)
        .when(!waiting.is_empty(), |d| d.child(section(t("jam.knocks"))).child(div().px(px(10.0)).flex().flex_col().children(waiting)))
        .child(settings)
        .child(section(tf("jam.people", &[("n", &members.len().to_string())])))
        .child(div().px(px(10.0)).flex().flex_col().children(people))
        .child(div().pt(px(8.0)).child(section(t("jam.up_next"))))
        .when(empty, |d| d.child(div().px(px(16.0)).pb(px(6.0)).text_size(theme::text_sm()).text_color(theme::text_dim()).child(t("jam.empty"))))
        .child(div().px(px(10.0)).flex().flex_col().children(rows))
        .when(more, |d| {
            d.child(div().mx(px(16.0)).mt(px(6.0)).flex().child(
                button("jam-queue-more", t("artist.more")).on_click(cx.listener(|this, _, _, cx| {
                    this.jam.queue_all = true;
                    cx.notify();
                })),
            ))
        })
        .when(!events.is_empty(), |d| d.child(div().pt(px(8.0)).child(section(t("jam.activity")))).child(div().px(px(10.0)).flex().flex_col().children(events)))
        .child(
            div().mx(px(16.0)).mt(px(10.0)).flex().child(
                button("jam-leave", t(if host { "jam.end" } else { "jam.leave" })).on_click(cx.listener(|this, _, _, cx| this.jam_leave(cx))),
            ),
        )
        .into_any_element()
}

/// One queued song: cover, title, who added it (picture), votes, remove.
#[allow(clippy::too_many_arguments)]
fn song_row(app: &mut Glyph, i: usize, item: &Item, voted: bool, sha: Option<String>, removable: bool, can_vote: bool, cx: &mut Context<Glyph>) -> AnyElement {
    let n = item.votes.len();
    let vote_key: SharedString = format!("jam-vote-{i}").into();
    let (vote_item, remove_item) = (item.clone(), item.clone());
    let pic = person(app, &item.by, sha, 20.0, cx);
    div()
        .id(gpui::ElementId::Name(format!("jam-q-{i}").into()))
        .flex()
        .items_center()
        .gap(px(10.0))
        .h(px(52.0))
        .px(px(6.0))
        .rounded(px(8.0))
        .relative()
        .child(cover(app, &item.track, 40.0, 6.0, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(song_title(item.track.title.clone(), crate::ui::widgets::rating(&item.track)).text_size(theme::text_sm()).text_color(theme::text()))
                .child(div().truncate().text_size(theme::text_xs()).text_color(theme::text_dim()).child(item.track.artist.clone())),
        )
        .child(with_tip(div().id(("jam-by", i)).flex_none().child(pic), Some(tf("jam.added_by", &[("who", &format!("@{}", item.by))]))))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(2.0))
                .when(can_vote, |d| {
                    d.child(
                        with_tip(toggle_button(vote_key.clone(), "thumbs-up", 14.0, voted, Some(theme::pink())), Some(t("jam.vote")))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                mark_toggled(vote_key.clone());
                                this.jam_vote(&vote_item, cx)
                            })),
                    )
                })
                .when(n > 0, |d| d.child(mono().w(px(14.0)).text_size(theme::text_xs()).text_color(theme::text_dim()).child(n.to_string()))),
        )
        .when(removable, |d| {
            d.child(
                with_tip(icon_button_plain(format!("jam-rm-{i}"), "x", 13.0, theme::text_faint()), Some(t("jam.remove")))
                    .on_click(cx.listener(move |this, _, _, cx| this.jam_remove(&remove_item, cx))),
            )
        })
        .into_any_element()
}
