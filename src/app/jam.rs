//! Jam: listening together through the Glyph server (`glyph_protocol::jam`;
//! the decisions themselves are in `glyph_core::jam`).
//!
//! The host's app owns the jam. Its hand-queued songs ("Up next") are the
//! jam's queue: songs friends add land there, votes reorder them. It
//! publishes what plays (and from which source), where, what comes next and
//! what happened, and runs the friends' requests as far as the host allows
//! them ([`Perms`]). A guest's queue shows the host's; its app plays the
//! host's song at the host's moment (clock synced with the server, small
//! gaps closed by playing a little faster or slower) or, when the guest
//! listens at the host, plays nothing and only shows it. When the host's
//! app is gone for a while, a guest takes over.
//!
//! Where the server pushes (`/v1/jam/{code}/live`), changes arrive over a
//! WebSocket; otherwise, and whenever the socket is gone, the apps long
//! poll as before.

use super::*;
use glyph_core::jam::{ClockSync, Step};
use glyph_core::server_client::LiveEvent;
use glyph_protocol::jam::{
    push_feed, Control, Event, Item, Live, Member, Now, Op, OpFrom, OpsResponse, Perms, Reaction, State, What,
    HOST_TIMEOUT_MS, LIVE_CLOSE_AGAIN, LIVE_HOST_EVERY_MS, MAX_AFTER, MAX_MEMBERS, MAX_QUEUE,
};
use std::collections::{BTreeSet, HashMap};

/// Host: publish at least this often while a song plays.
const HEARTBEAT: Duration = Duration::from_secs(15);
/// The same with pushes: every publish costs a push, and pushed guests
/// need no heartbeat to stay current.
const HEARTBEAT_LIVE: Duration = Duration::from_secs(60);
/// After the push socket closed or failed, long poll this long before
/// trying it again.
const LIVE_RETRY: Duration = Duration::from_secs(300);
/// Host: a position this far from the published one was a seek.
const SEEK_JUMP_MS: i64 = 2000;
/// Host: how often to look for something to publish (not every frame).
const PUBLISH_CHECK: Duration = Duration::from_millis(100);
/// Guest: how often the position is checked against the host.
const SYNC_EVERY: Duration = Duration::from_millis(500);
/// Guest: after starting a song or a jump, give the stream this long.
const SETTLE: Duration = Duration::from_secs(3);
/// Clock samples: a few quick ones at the start, then one a minute.
const CLOCK_FIRST: usize = 4;
const CLOCK_QUICK: Duration = Duration::from_millis(700);
const CLOCK_EVERY: Duration = Duration::from_secs(60);
/// Asking again while waiting to be let in.
const KNOCK_EVERY: Duration = Duration::from_secs(3);
/// One reaction per person this often (host side).
const REACT_GAP: Duration = Duration::from_millis(1500);

/// Jam state of the app (Connect panel).
#[derive(Default)]
pub struct JamUi {
    pub jam: Option<Jam>,
    /// Starting or joining right now.
    pub busy: bool,
    /// Waiting for the host to let us in (code).
    pub waiting: Option<String>,
    /// Last problem (bad code, jam ended, …), shown in the panel.
    pub error: Option<String>,
    /// "Enter a code" field, made on first draw.
    pub input: Option<Entity<TextInput>>,
    /// The panel shows the whole queue, not only its start.
    pub queue_all: bool,
    /// Invite QR code: link, modules per side, dark modules row by row.
    pub qr: Option<(String, usize, Arc<Vec<bool>>)>,
    /// A jam link that arrived before the app was ready.
    pending: Option<String>,
    /// Jam shown in Discord: (code, people).
    discord_party: Option<(String, usize)>,
    /// Bumped to stop the running loops.
    gen: u64,
}

pub struct Jam {
    pub code: String,
    pub host: bool,
    /// Own handle.
    pub me: String,
    /// Last state seen (guest) or published (host).
    pub state: State,
    /// Code / link copied (check mark on the button).
    pub copied: Option<Instant>,
    /// Guest: the music plays here (else only at the host's device).
    pub listen_here: bool,
    /// Guest: the host's song has no source for this app (song key).
    pub unavailable: Option<String>,
    /// Host: people waiting to be let in (handle, picture).
    pub knocks: Vec<(String, Option<String>)>,
    clock: ClockSync,
    /// Rough server minus local clock until `clock` has samples (ms).
    offset_ms: i64,
    clock_at: Option<Instant>,
    clock_busy: bool,
    // ---- host
    /// Who added which queued song (song key -> handle).
    by: HashMap<String, String>,
    /// Votes per queued song (song key -> handles).
    votes: HashMap<String, BTreeSet<String>>,
    /// Guests in the order they joined.
    guests: Vec<String>,
    /// Profile pictures by handle.
    avatars: HashMap<String, String>,
    perms: Perms,
    approve: bool,
    feed: Vec<Event>,
    /// Last reaction per person.
    reacted: HashMap<String, Instant>,
    /// After a takeover: continue the song from here (ms) once it is loaded.
    resume_at: Option<u64>,
    /// What was published last: signature, position (ms), when.
    published: Option<(u64, u64, Instant)>,
    publishing: bool,
    /// The push socket is open (fewer heartbeats).
    live: bool,
    /// Last look for something to publish.
    checked: Option<Instant>,
    /// The server takes position-only updates (`/now`).
    light: bool,
    // ---- guest
    outbox: Vec<Op>,
    sending: bool,
    synced_at: Option<Instant>,
    settle_until: Option<Instant>,
    /// Tempo set for catching up (1.0 = normal).
    speed: f32,
    /// The own queue, back when leaving.
    saved_queue: Option<Queue>,
    /// The host has been offline since.
    host_gone: Option<Instant>,
    claiming: bool,
    /// Newest feed entry already shown (server ms).
    feed_seen: u64,
}

impl Jam {
    fn new(code: String, host: bool, me: String, state: State) -> Self {
        let avatars = state.members.iter().filter_map(|m| Some((m.handle.clone(), m.avatar.clone()?))).collect();
        let feed_seen = state.feed.iter().map(|e| e.at_ms).max().unwrap_or(0);
        Self {
            code,
            host,
            me,
            offset_ms: state.server_ms as i64 - unix_ms() as i64,
            perms: state.perms,
            approve: state.approve_joins,
            feed: state.feed.clone(),
            state,
            copied: None,
            listen_here: true,
            unavailable: None,
            knocks: Vec::new(),
            clock: ClockSync::default(),
            clock_at: None,
            clock_busy: false,
            by: HashMap::new(),
            votes: HashMap::new(),
            guests: Vec::new(),
            avatars,
            reacted: HashMap::new(),
            resume_at: None,
            published: None,
            publishing: false,
            live: false,
            checked: None,
            light: true,
            outbox: Vec::new(),
            sending: false,
            synced_at: None,
            settle_until: None,
            speed: 1.0,
            saved_queue: None,
            host_gone: None,
            claiming: false,
            feed_seen,
        }
    }

    /// The server clock now (ms).
    pub fn server_now(&self) -> u64 {
        (unix_ms() as i64 + self.clock.offset().unwrap_or(self.offset_ms)).max(0) as u64
    }

    /// What guests may do.
    pub fn perms(&self) -> Perms {
        if self.host {
            self.perms
        } else {
            self.state.perms
        }
    }

    /// New people wait for the host.
    pub fn approve_joins(&self) -> bool {
        if self.host {
            self.approve
        } else {
            self.state.approve_joins
        }
    }

    /// People in the jam, host included.
    pub fn member_count(&self) -> usize {
        if self.host {
            1 + self.guests.len()
        } else {
            self.state.members.len()
        }
    }

    /// The current member list (host first).
    pub fn members(&self) -> Vec<Member> {
        if !self.host {
            return self.state.members.clone();
        }
        std::iter::once(Member { handle: self.me.clone(), host: true, avatar: self.avatars.get(&self.me).cloned() })
            .chain(self.guests.iter().map(|g| Member { handle: g.clone(), host: false, avatar: self.avatars.get(g).cloned() }))
            .collect()
    }

    /// Profile picture of a member (sha256), if they have one.
    pub fn avatar_of(&self, handle: &str) -> Option<String> {
        if self.host {
            self.avatars.get(handle).cloned()
        } else {
            self.state.members.iter().find(|m| m.handle == handle).and_then(|m| m.avatar.clone())
        }
    }

    /// What happened, newest last.
    pub fn feed(&self) -> &[Event] {
        if self.host {
            &self.feed
        } else {
            &self.state.feed
        }
    }

    /// The host's app is around (always, for the host itself).
    pub fn host_online(&self) -> bool {
        self.host || self.state.host_online
    }

    /// Voted for this queued song.
    pub fn voted(&self, item: &Item) -> bool {
        item.votes.iter().any(|v| *v == self.me)
    }

    /// Link that invites to this jam (the QR code).
    pub fn link(&self) -> String {
        self.state.link.clone().unwrap_or_else(|| glyph_protocol::jam::share_text(&self.code))
    }

    /// Position of the host's song now (ms).
    pub fn host_position(&self) -> Option<u64> {
        self.state.position_at(self.server_now())
    }

    fn event(&mut self, who: &str, what: What) {
        let at_ms = self.server_now();
        push_feed(&mut self.feed, Event { at_ms, who: who.to_string(), what });
    }
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// A server answer with its error code kept (`jam_ended`, `not_in_jam`, …).
async fn call<T>(fut: impl std::future::Future<Output = anyhow::Result<T>>) -> Result<T, (Option<String>, String)> {
    use futures::FutureExt;
    match std::panic::AssertUnwindSafe(fut).catch_unwind().await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) if glyph_core::account::is_signed_out(&e) => Err((Some("signed_out".into()), format!("{e:#}"))),
        Ok(Err(e)) => Err((glyph_core::server_client::server_error(&e).map(|(_, c)| c.to_string()), format!("{e:#}"))),
        Err(_) => Err((None, "not implemented".into())),
    }
}

/// The message for a failed jam call.
fn problem(code: Option<&str>, msg: &str) -> String {
    match code {
        Some("jam_ended") => t("jam.err.ended").to_string(),
        Some("not_in_jam") | Some("jam_removed") => t("jam.err.removed").to_string(),
        Some("jam_denied") => t("jam.err.denied").to_string(),
        Some("jam_full") => t("jam.err.full").to_string(),
        // Older servers do not know `/v1/jam`.
        Some("not_found") => t("jam.err.server").to_string(),
        Some("signed_out") => t("jam.sign_in").to_string(),
        _ => msg.to_string(),
    }
}

/// The call failed because the jam is gone for this app.
fn gone(code: Option<&str>) -> bool {
    matches!(code, Some("jam_ended" | "not_in_jam" | "jam_removed" | "signed_out"))
}

/// How a push session ended.
enum LiveEnd {
    /// The loop is over (jam left, ended, or handed over).
    Stop,
    /// Connect again right away (someone took over as host).
    Again,
    /// Long poll for [`LIVE_RETRY`], then try again.
    Later,
    /// No pushes for this jam: the server has none, or today's are used up.
    Never,
}

impl LiveEnd {
    fn closed(code: Option<u16>) -> Self {
        if code == Some(LIVE_CLOSE_AGAIN) {
            LiveEnd::Again
        } else {
            // Ended or removed: the next long poll tells.
            LiveEnd::Later
        }
    }

    fn refused(code: Option<&str>) -> Self {
        match code {
            Some("jam_push_off" | "jam_push_busy" | "websocket_required" | "not_found") => LiveEnd::Never,
            _ => LiveEnd::Later,
        }
    }
}

/// What came of asking the server.
#[derive(PartialEq)]
enum Asked {
    Done,
    /// Try again after a pause.
    Failed,
    /// The loop is over.
    Over,
}

/// Text of a reaction.
pub fn reaction_label(r: Reaction) -> SharedString {
    t(match r {
        Reaction::Heart => "jam.r.heart",
        Reaction::Fire => "jam.r.fire",
        Reaction::Sparkles => "jam.r.sparkles",
        Reaction::ThumbsUp => "jam.r.thumbs_up",
    })
}

impl Glyph {
    // ------------------------------------------------------------ state

    pub fn jam_active(&self) -> bool {
        self.jam.jam.is_some()
    }

    /// In someone else's jam: playback follows the host.
    pub fn jam_guest(&self) -> bool {
        self.jam.jam.as_ref().is_some_and(|j| !j.host)
    }

    /// A guest listening at the host's device: nothing plays here.
    pub fn jam_listen_remote(&self) -> bool {
        self.jam.jam.as_ref().is_some_and(|j| !j.host && !j.listen_here)
    }

    /// The first `limit` songs of the jam's queue as it is now (the host
    /// builds them from its queue), and how many there are.
    pub fn jam_queue(&self, limit: usize) -> (Vec<Item>, usize) {
        match &self.jam.jam {
            Some(j) if j.host => (self.jam_host_items(j, limit), self.queue.manual_len().min(MAX_QUEUE)),
            Some(j) => (j.state.queue.iter().take(limit).cloned().collect(), j.state.queue.len()),
            None => (Vec::new(), 0),
        }
    }

    /// Who added this song of the jam's queue (handle, profile picture).
    pub fn jam_added_by(&self, track: &Track) -> Option<(String, Option<String>)> {
        let j = self.jam.jam.as_ref()?;
        let key = track_key(track);
        let by = if j.host { j.by.get(&key)?.clone() } else { j.state.queue.iter().find(|i| i.id == key)?.by.clone() };
        let sha = j.avatar_of(&by);
        Some((by, sha))
    }

    /// The "Enter a code" field (made on first use).
    pub fn jam_input(&mut self, cx: &mut Context<Self>) -> Entity<TextInput> {
        if let Some(i) = &self.jam.input {
            return i.clone();
        }
        let i = cx.new(|cx| TextInput::new("jam.code.placeholder", cx).max_chars(64));
        let sub = cx.subscribe(&i, |this, _, ev: &InputEvent, cx| {
            if let InputEvent::Submit(v) = ev {
                let v = v.clone();
                this.jam_join(&v, cx);
            }
        });
        self._subs.push(sub);
        self.jam.input = Some(i.clone());
        i
    }

    // ------------------------------------------------------------ start / join / leave

    /// Starts a jam hosted here.
    pub fn jam_start(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let Some(acc) = self.signed_in() else {
            self.jam.error = Some(t("jam.sign_in").to_string());
            cx.notify();
            return;
        };
        if self.jam.busy {
            return;
        }
        if self.jam_active() {
            self.jam_leave(cx);
        }
        self.jam.busy = true;
        self.jam.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = call(b.jam_start()).await;
            let _ = this.update(cx, |s, cx| {
                s.jam.busy = false;
                match r {
                    Ok(state) => {
                        let mut j = Jam::new(state.code.clone(), true, acc.account.handle.clone(), state);
                        // Songs already queued by hand are the jam's first ones.
                        for tr in s.queue.upcoming().into_iter().take(s.queue.manual_len()) {
                            j.by.insert(track_key(tr), j.me.clone());
                        }
                        s.jam.jam = Some(j);
                        s.jam.gen += 1;
                        s.jam_host_loop(cx);
                        s.toast(t("jam.started"));
                    }
                    Err((code, msg)) => s.jam.error = Some(problem(code.as_deref(), &msg)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A `glyph://jam/…` link opened the app (or reached the running one).
    pub fn jam_open_link(&mut self, link: &str, cx: &mut Context<Self>) {
        if self.backend.is_none() {
            self.jam.pending = Some(link.to_string());
            return;
        }
        self.jam_join(link, cx);
    }

    /// Joins the jam with this code (or `glyph:jam:` text, or a jam link).
    /// When the host lets people in by hand, this waits (and asks again)
    /// until they do.
    pub fn jam_join(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(code) = glyph_protocol::jam::normalize_code(text) else {
            self.jam.error = Some(t("jam.err.code").to_string());
            cx.notify();
            return;
        };
        if self.jam.jam.as_ref().is_some_and(|j| j.code == code) {
            self.show_devices_panel(cx);
            return;
        }
        let Some(b) = self.backend.clone() else { return };
        let Some(acc) = self.signed_in() else {
            self.jam.error = Some(t("jam.sign_in").to_string());
            self.show_devices_panel(cx);
            return;
        };
        if self.jam.busy {
            return;
        }
        if self.jam_active() {
            self.jam_leave(cx);
        }
        self.jam.busy = true;
        self.jam.error = None;
        self.jam.waiting = None;
        self.jam.gen += 1;
        let gen = self.jam.gen;
        self.show_devices_panel(cx);
        cx.spawn(async move |this, cx| {
            loop {
                let r = call(b.jam_join(&code)).await;
                let again = this
                    .update(cx, |s, cx| {
                        if s.jam.gen != gen {
                            return false;
                        }
                        match r {
                            Ok(res) if res.waiting => {
                                s.jam.busy = false;
                                s.jam.waiting = Some(code.clone());
                                cx.notify();
                                true
                            }
                            Ok(res) => {
                                s.jam.busy = false;
                                s.jam.waiting = None;
                                match res.state {
                                    Some(state) if state.host == acc.account.handle => {
                                        // Own jam (another device of this account): not as a guest.
                                        s.jam.error = Some(t("jam.err.own").to_string());
                                    }
                                    Some(state) => s.jam_enter_guest(code.clone(), acc.account.handle.clone(), state, cx),
                                    None => s.jam.error = Some(t("jam.err.server").to_string()),
                                }
                                cx.notify();
                                false
                            }
                            Err((c, msg)) => {
                                s.jam.busy = false;
                                s.jam.waiting = None;
                                s.jam.error = Some(problem(c.as_deref(), &msg));
                                cx.notify();
                                false
                            }
                        }
                    })
                    .unwrap_or(false);
                if !again {
                    break;
                }
                cx.background_executor().timer(KNOCK_EVERY).await;
            }
        })
        .detach();
    }

    /// Stops waiting to be let in.
    pub fn jam_stop_waiting(&mut self, cx: &mut Context<Self>) {
        let Some(code) = self.jam.waiting.take() else { return };
        self.jam.gen += 1;
        if let Some(b) = self.backend.clone() {
            cx.background_executor()
                .spawn(async move {
                    let _ = call(b.jam_leave(&code)).await;
                })
                .detach();
        }
        cx.notify();
    }

    /// In as a guest: the queue shows the host's (the own one comes back
    /// when leaving), playback follows the host.
    fn jam_enter_guest(&mut self, code: String, me: String, state: State, cx: &mut Context<Self>) {
        let host = state.host.clone();
        let mut j = Jam::new(code, false, me, state);
        j.saved_queue = Some(self.queue.clone());
        self.jam.jam = Some(j);
        self.jam.gen += 1;
        if let Some(i) = self.jam.input.clone() {
            i.update(cx, |i, cx| i.set_value(String::new(), cx));
        }
        self.jam_mirror_queue();
        self.jam_guest_loop(cx);
        self.jam_sync(cx);
        self.toast_sub(t("jam.joined_you"), format!("@{host}"));
    }

    /// Leaves the jam; the host ends it for everyone.
    pub fn jam_leave(&mut self, cx: &mut Context<Self>) {
        let Some(j) = self.jam.jam.take() else { return };
        self.jam.gen += 1;
        self.jam.error = None;
        let (host, code) = (j.host, j.code.clone());
        self.jam_after(j);
        if let Some(b) = self.backend.clone() {
            cx.background_executor()
                .spawn(async move {
                    let _ = call(b.jam_leave(&code)).await;
                })
                .detach();
        }
        self.toast_info(t(if host { "jam.ended_you" } else { "jam.left" }));
        cx.notify();
    }

    /// The jam is gone for this app (ended, removed): say why.
    fn jam_lost(&mut self, why: String, cx: &mut Context<Self>) {
        let Some(j) = self.jam.jam.take() else { return };
        self.jam.gen += 1;
        self.jam_after(j);
        self.toast_info(why.clone());
        self.jam.error = Some(why);
        cx.notify();
    }

    /// Someone else took over this app's jam: back in as a guest.
    fn jam_demote(&mut self, cx: &mut Context<Self>) {
        let Some(j) = self.jam.jam.take() else { return };
        self.jam.gen += 1;
        let code = j.code.clone();
        self.jam_after(j);
        self.toast_info(t("jam.taken_over"));
        self.jam_join(&code, cx);
    }

    /// Back to normal after a jam: tempo, the own queue, nothing shown that
    /// is not loaded here.
    fn jam_after(&mut self, j: Jam) {
        self.jam.queue_all = false;
        if j.host {
            return;
        }
        if (j.speed - 1.0).abs() > f32::EPSILON {
            self.with_player(|p| p.set_speed(1.0));
        }
        if !j.listen_here {
            self.jam_clear_remote();
        }
        if let Some(q) = j.saved_queue {
            self.queue = q;
            self.queue_changed();
        }
    }

    /// The host's song shown while listening at the host is not loaded
    /// here: forget it.
    fn jam_clear_remote(&mut self) {
        self.current = None;
        self.player_state = None;
        self.pos_anchor = None;
    }

    fn show_devices_panel(&mut self, cx: &mut Context<Self>) {
        if !self.show_devices {
            self.toggle_devices_panel(cx);
        }
        self.devices_jam = true;
    }

    /// Copies the invite link (QR code) or the code.
    pub fn jam_copy(&mut self, link: bool, cx: &mut Context<Self>) {
        let Some(j) = self.jam.jam.as_mut() else { return };
        let text = if link { j.link() } else { glyph_protocol::jam::share_text(&j.code) };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        j.copied = Some(Instant::now());
        self.toast(t(if link { "jam.link_copied" } else { "jam.copied" }));
        cx.notify();
    }

    // ------------------------------------------------------------ loops

    /// Host: picks up the friends' requests until the jam ends, pushed
    /// where the server can, else by long polling.
    fn jam_host_loop(&mut self, cx: &mut Context<Self>) {
        let (Some(b), Some(j)) = (self.backend.clone(), self.jam.jam.as_ref()) else { return };
        let (gen, code) = (self.jam.gen, j.code.clone());
        cx.spawn(async move |this, cx| {
            let mut failures = 0u32;
            let mut live_at = Some(Instant::now());
            loop {
                if !this.update(cx, |s, _| s.jam.gen == gen).unwrap_or(false) {
                    break;
                }
                if live_at.is_some_and(|t| t <= Instant::now()) {
                    match Self::jam_host_live(&this, cx, &b, gen, &code).await {
                        LiveEnd::Stop => break,
                        LiveEnd::Again => continue,
                        LiveEnd::Later => live_at = Some(Instant::now() + LIVE_RETRY),
                        LiveEnd::Never => live_at = None,
                    }
                }
                let r = call(b.jam_ops(&code, false)).await;
                match this.update(cx, |s, cx| s.jam_host_ops(gen, r, cx)).unwrap_or(Asked::Over) {
                    Asked::Done => failures = 0,
                    Asked::Over => break,
                    Asked::Failed => {
                        failures += 1;
                        let wait = (1u64 << failures.min(5)).min(30);
                        cx.background_executor().timer(Duration::from_secs(wait)).await;
                    }
                }
            }
        })
        .detach();
    }

    /// Host on push: asks for the requests when the server rings, and every
    /// [`LIVE_HOST_EVERY_MS`] anyway (sign of life, lost pushes).
    async fn jam_host_live(this: &gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp, b: &glyph_core::Backend, gen: u64, code: &str) -> LiveEnd {
        use futures::future::{select, Either};
        let mut rx = match call(b.jam_live(code)).await {
            Ok(rx) => rx,
            Err((c, msg)) => {
                tracing::info!("jam live: {msg}");
                return LiveEnd::refused(c.as_deref());
            }
        };
        Self::jam_set_live(this, cx, gen, true);
        let end = loop {
            let tick = cx.background_executor().timer(Duration::from_millis(LIVE_HOST_EVERY_MS));
            let ev = match select(Box::pin(rx.recv()), Box::pin(tick)).await {
                Either::Left((ev, _)) => ev,
                Either::Right(_) => Some(LiveEvent::Msg(Live::Ops)),
            };
            match ev {
                None => break LiveEnd::Later,
                Some(LiveEvent::Closed(c)) => break LiveEnd::closed(c),
                Some(LiveEvent::Msg(Live::Fallback)) => break LiveEnd::Never,
                Some(LiveEvent::Msg(Live::Ops)) => {
                    let r = call(b.jam_ops(code, true)).await;
                    if this.update(cx, |s, cx| s.jam_host_ops(gen, r, cx)).unwrap_or(Asked::Over) == Asked::Over {
                        break LiveEnd::Stop;
                    }
                }
                Some(LiveEvent::Msg(_)) => {}
            }
        };
        Self::jam_set_live(this, cx, gen, false);
        end
    }

    fn jam_set_live(this: &gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp, gen: u64, on: bool) {
        let _ = this.update(cx, |s, _| {
            let mine = s.jam.gen == gen;
            if let Some(j) = s.jam.jam.as_mut().filter(|_| mine) {
                j.live = on;
            }
        });
    }

    /// The host's answer to asking for requests: runs them.
    fn jam_host_ops(&mut self, gen: u64, r: Result<OpsResponse, (Option<String>, String)>, cx: &mut Context<Self>) -> Asked {
        if self.jam.gen != gen {
            return Asked::Over;
        }
        match r {
            Ok(r) => {
                if !r.ops.is_empty() {
                    self.jam_apply_ops(r.ops, cx);
                }
                Asked::Done
            }
            Err((c, _)) if c.as_deref() == Some("not_jam_host") => {
                self.jam_demote(cx);
                Asked::Over
            }
            Err((c, msg)) if gone(c.as_deref()) => {
                self.jam_lost(problem(c.as_deref(), &msg), cx);
                Asked::Over
            }
            Err((_, msg)) => {
                tracing::info!("jam: {msg}");
                Asked::Failed
            }
        }
    }

    /// Guest: follows the published state until the jam ends, pushed where
    /// the server can, else by long polling.
    fn jam_guest_loop(&mut self, cx: &mut Context<Self>) {
        let (Some(b), Some(j)) = (self.backend.clone(), self.jam.jam.as_ref()) else { return };
        let (gen, code, mut rev) = (self.jam.gen, j.code.clone(), j.state.rev);
        cx.spawn(async move |this, cx| {
            let mut failures = 0u32;
            let mut live_at = Some(Instant::now());
            loop {
                if !this.update(cx, |s, _| s.jam.gen == gen).unwrap_or(false) {
                    break;
                }
                if live_at.is_some_and(|t| t <= Instant::now()) {
                    match Self::jam_guest_live(&this, cx, &b, gen, &code, &mut rev).await {
                        LiveEnd::Stop => break,
                        LiveEnd::Again => continue,
                        LiveEnd::Later => live_at = Some(Instant::now() + LIVE_RETRY),
                        LiveEnd::Never => live_at = None,
                    }
                }
                let r = call(b.jam_state(&code, rev)).await;
                match this.update(cx, |s, cx| s.jam_guest_got(gen, r, &mut rev, cx)).unwrap_or(Asked::Over) {
                    Asked::Done => failures = 0,
                    Asked::Over => break,
                    Asked::Failed => {
                        failures += 1;
                        let wait = (1u64 << failures.min(5)).min(30);
                        cx.background_executor().timer(Duration::from_secs(wait)).await;
                    }
                }
            }
        })
        .detach();
    }

    /// Guest on push. While the host is not on push too (an older app, or
    /// gone), it also long polls: that is how it learns that the host is
    /// offline and when to take over.
    async fn jam_guest_live(this: &gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp, b: &glyph_core::Backend, gen: u64, code: &str, rev: &mut u64) -> LiveEnd {
        use futures::future::{select, Either};
        let mut rx = match call(b.jam_live(code)).await {
            Ok(rx) => rx,
            Err((c, msg)) => {
                tracing::info!("jam live: {msg}");
                return LiveEnd::refused(c.as_deref());
            }
        };
        // What was published before the socket was open.
        let r = call(b.jam_state(code, 0)).await;
        if this.update(cx, |s, cx| s.jam_guest_got(gen, r, rev, cx)).unwrap_or(Asked::Over) == Asked::Over {
            return LiveEnd::Stop;
        }
        let mut host_live = false;
        loop {
            let ev = if host_live {
                rx.recv().await
            } else {
                match select(Box::pin(rx.recv()), Box::pin(call(b.jam_state(code, *rev)))).await {
                    Either::Left((ev, _)) => ev,
                    Either::Right((r, _)) => {
                        match this.update(cx, |s, cx| s.jam_guest_got(gen, r, rev, cx)).unwrap_or(Asked::Over) {
                            Asked::Over => return LiveEnd::Stop,
                            Asked::Failed => cx.background_executor().timer(Duration::from_secs(5)).await,
                            Asked::Done => {}
                        }
                        continue;
                    }
                }
            };
            match ev {
                None => return LiveEnd::Later,
                Some(LiveEvent::Closed(c)) => return LiveEnd::closed(c),
                Some(LiveEvent::Msg(Live::Fallback)) => return LiveEnd::Never,
                Some(LiveEvent::Msg(Live::State { state })) => {
                    if this.update(cx, |s, cx| s.jam_guest_got(gen, Ok(*state), rev, cx)).unwrap_or(Asked::Over) == Asked::Over {
                        return LiveEnd::Stop;
                    }
                }
                Some(LiveEvent::Msg(Live::Host { online })) => {
                    host_live = online;
                    if online {
                        let _ = this.update(cx, |s, cx| {
                            let mine = s.jam.gen == gen;
                            if let Some(j) = s.jam.jam.as_mut().filter(|_| mine) {
                                j.state.host_online = true;
                                j.host_gone = None;
                                cx.notify();
                            }
                        });
                    }
                }
                Some(LiveEvent::Msg(Live::Now { rev: r, now, server_ms })) => {
                    if this.update(cx, |s, cx| s.jam_guest_now(gen, r, now, server_ms, rev, cx)).unwrap_or(Asked::Over) == Asked::Over {
                        return LiveEnd::Stop;
                    }
                }
                Some(LiveEvent::Msg(Live::Ops)) => {}
            }
        }
    }

    /// Pushed: only the host's position moved (revision `r`).
    #[allow(clippy::too_many_arguments)]
    fn jam_guest_now(&mut self, gen: u64, r: u64, now: Now, server_ms: u64, rev: &mut u64, cx: &mut Context<Self>) -> Asked {
        if self.jam.gen != gen {
            return Asked::Over;
        }
        let Some(j) = self.jam.jam.as_mut() else { return Asked::Over };
        if r <= *rev {
            return Asked::Done;
        }
        *rev = r;
        j.offset_ms = server_ms as i64 - unix_ms() as i64;
        if j.state.now.as_ref().map(|n| track_key(&n.track)) != Some(track_key(&now.track)) {
            j.unavailable = None;
        }
        j.state.rev = r;
        j.state.now = Some(now);
        j.state.host_online = true;
        j.host_gone = None;
        j.synced_at = None;
        self.jam_sync(cx);
        cx.notify();
        Asked::Done
    }

    /// A guest's answer about the state: shows it.
    fn jam_guest_got(&mut self, gen: u64, r: Result<State, (Option<String>, String)>, rev: &mut u64, cx: &mut Context<Self>) -> Asked {
        if self.jam.gen != gen {
            return Asked::Over;
        }
        match r {
            Ok(state) => {
                // An older state than the one shown (a slow answer) changes nothing.
                if state.rev >= *rev {
                    *rev = state.rev;
                    self.jam_apply_state(state, cx);
                }
                Asked::Done
            }
            Err((c, msg)) if gone(c.as_deref()) => {
                self.jam_lost(problem(c.as_deref(), &msg), cx);
                Asked::Over
            }
            Err((_, msg)) => {
                tracing::info!("jam: {msg}");
                Asked::Failed
            }
        }
    }

    /// Each frame: a waiting jam link is opened, the clock is synced, the
    /// host publishes changes, a guest sends its requests, stays in step
    /// and takes over from a host that is gone. Returns whether to redraw.
    pub(super) fn poll_jam(&mut self, cx: &mut Context<Self>) -> bool {
        if self.jam.pending.is_some() && self.backend.is_some() && !self.jam.busy {
            if let Some(link) = self.jam.pending.take() {
                self.jam_join(&link, cx);
            }
        }
        self.jam_discord();
        let Some(j) = self.jam.jam.as_ref() else { return false };
        let clock_due = !j.clock_busy
            && j.clock_at.is_none_or(|t| t.elapsed() >= if j.clock.len() < CLOCK_FIRST { CLOCK_QUICK } else { CLOCK_EVERY });
        if clock_due {
            self.jam_clock_sample(cx);
        }
        let Some(j) = self.jam.jam.as_ref() else { return false };
        if j.host {
            self.jam_resume();
            self.jam_maybe_publish(cx);
            return false;
        }
        let flush = !j.sending && !j.outbox.is_empty();
        let sync = j.synced_at.is_none_or(|t| t.elapsed() >= SYNC_EVERY);
        let take_over = !j.claiming
            && j.host_gone.is_some_and(|since| {
                let offline = HOST_TIMEOUT_MS + since.elapsed().as_millis() as u64;
                glyph_core::jam::should_take_over(&j.state.members, &j.me, offline)
            });
        if flush {
            self.jam_flush(cx);
        }
        if sync {
            self.jam_sync(cx);
        }
        if take_over {
            self.jam_take_over(cx);
        }
        false
    }

    /// One timed request for the clock sync.
    fn jam_clock_sample(&mut self, cx: &mut Context<Self>) {
        let (Some(b), Some(j)) = (self.backend.clone(), self.jam.jam.as_mut()) else { return };
        j.clock_busy = true;
        j.clock_at = Some(Instant::now());
        let (gen, code) = (self.jam.gen, j.code.clone());
        cx.spawn(async move |this, cx| {
            let r = call(b.jam_time(&code)).await;
            let _ = this.update(cx, |s, _| {
                if s.jam.gen != gen {
                    return;
                }
                if let Some(j) = s.jam.jam.as_mut() {
                    j.clock_busy = false;
                    if let Ok((sent, server, recv)) = r {
                        j.clock.add(sent, server, recv);
                    }
                }
            });
        })
        .detach();
    }

    /// Shows the jam in Discord ("Listening … (3 of 20)"). Runs every frame,
    /// so it only compares until something changed.
    fn jam_discord(&mut self) {
        let want = self.jam.jam.as_ref().map(|j| (j.code.as_str(), j.member_count()));
        if want == self.jam.discord_party.as_ref().map(|(c, n)| (c.as_str(), *n)) {
            return;
        }
        let want = want.map(|(c, n)| (c.to_string(), n));
        if let Some(d) = &self.discord {
            d.set_party(want.as_ref().map(|(code, n)| {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                code.hash(&mut h);
                (format!("jam-{:x}", h.finish()), *n, MAX_MEMBERS)
            }));
        }
        self.jam.discord_party = want;
    }

    // ------------------------------------------------------------ host

    fn jam_host_items(&self, j: &Jam, limit: usize) -> Vec<Item> {
        let n = self.queue.manual_len();
        self.queue
            .upcoming()
            .into_iter()
            .take(n.min(MAX_QUEUE).min(limit))
            .map(|tr| {
                let id = track_key(tr);
                Item {
                    by: j.by.get(&id).cloned().unwrap_or_else(|| j.me.clone()),
                    votes: j.votes.get(&id).map(|v| v.iter().cloned().collect()).unwrap_or_default(),
                    id,
                    track: tr.clone(),
                }
            })
            .collect()
    }

    /// The state to publish now.
    /// What plays now, as published.
    fn jam_host_now(&self, j: &Jam) -> Option<Now> {
        let resolved = match (&self.current, &self.last_load) {
            (Some(c), Some((tr, _))) if glyph_core::dedupe::same_song(c, tr) => Some(tr.clone()),
            _ => None,
        };
        self.current.clone().map(|track| Now {
            track,
            position_ms: self.position().as_millis() as u64,
            playing: self.is_playing() || (self.resolving && !self.hold_play),
            // The server fills it in while the clock is not synced yet.
            at_ms: if j.clock.is_empty() { 0 } else { j.server_now() },
            resolved,
        })
    }

    fn jam_host_state(&self) -> Option<State> {
        let j = self.jam.jam.as_ref()?;
        let now = self.jam_host_now(j);
        Some(State {
            code: j.code.clone(),
            rev: j.state.rev,
            host: j.me.clone(),
            members: j.members(),
            perms: j.perms,
            approve_joins: j.approve,
            now,
            queue: self.jam_host_items(j, MAX_QUEUE),
            after: self.queue.upcoming_context().into_iter().take(MAX_AFTER).cloned().collect(),
            feed: j.feed.clone(),
            host_online: true,
            server_ms: 0,
            link: j.state.link.clone(),
        })
    }

    /// After a takeover: the song continues where the old host was.
    fn jam_resume(&mut self) {
        let Some(at) = self.jam.jam.as_ref().and_then(|j| j.resume_at) else { return };
        if self.resolving || self.current.is_none() || self.player_state.as_ref().is_none_or(|s| s.status == Status::Loading) {
            return;
        }
        if let Some(j) = self.jam.jam.as_mut() {
            j.resume_at = None;
        }
        self.seek_to(Duration::from_millis(at));
    }

    /// Fingerprint of what [`Self::jam_host_state`] would publish (without
    /// the position), made without building it: what plays and from where,
    /// the queue with its votes, what comes after, who is in, the feed and
    /// the settings.
    fn jam_host_sig(&self, j: &Jam) -> u64 {
        use std::hash::{BuildHasher, Hash, Hasher};
        fn song(t: &Track, h: &mut impl Hasher) {
            t.source.hash(h);
            t.id.hash(h);
        }
        // Order-free sum over a map's entries.
        fn map_sum<K: Hash, V: Hash>(m: &HashMap<K, V>) -> u64 {
            let b = std::hash::BuildHasherDefault::<std::collections::hash_map::DefaultHasher>::default();
            m.iter().map(|e| b.hash_one(e)).fold(0u64, u64::wrapping_add)
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        if let Some(c) = &self.current {
            song(c, &mut h);
            (self.is_playing() || (self.resolving && !self.hold_play)).hash(&mut h);
            if let Some((tr, _)) = self.last_load.as_ref().filter(|(tr, _)| glyph_core::dedupe::same_song(c, tr)) {
                song(tr, &mut h);
            }
        }
        let n = self.queue.manual_len().min(MAX_QUEUE);
        for t in self.queue.upcoming().into_iter().take(n) {
            song(t, &mut h);
        }
        map_sum(&j.votes).hash(&mut h);
        for t in self.queue.upcoming_context().into_iter().take(MAX_AFTER) {
            song(t, &mut h);
        }
        j.guests.hash(&mut h);
        map_sum(&j.avatars).hash(&mut h);
        j.feed.last().map(|e| e.at_ms).hash(&mut h);
        j.feed.len().hash(&mut h);
        (j.perms.add, j.perms.play_now, j.perms.skip, j.perms.seek, j.perms.pause, j.approve).hash(&mut h);
        h.finish()
    }

    /// Publishes when something changed, after a seek, or as a heartbeat.
    /// Looks every [`PUBLISH_CHECK`]; the state itself is only built when
    /// it goes out.
    fn jam_maybe_publish(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let Some(j) = self.jam.jam.as_ref() else { return };
        if j.publishing || j.checked.is_some_and(|t| t.elapsed() < PUBLISH_CHECK) {
            return;
        }
        let sig = self.jam_host_sig(j);
        let playing = self.current.is_some() && (self.is_playing() || (self.resolving && !self.hold_play));
        let pos = if self.current.is_some() { self.position().as_millis() as u64 } else { 0 };
        let Some(j) = self.jam.jam.as_mut() else { return };
        j.checked = Some(Instant::now());
        let due = match j.published {
            None => true,
            Some((s, _, _)) if s != sig => true,
            Some((_, p, at)) => {
                let expected = p as i64 + if playing { at.elapsed().as_millis() as i64 } else { 0 };
                (pos as i64 - expected).abs() > SEEK_JUMP_MS || at.elapsed() >= if j.live { HEARTBEAT_LIVE } else { HEARTBEAT }
            }
        };
        if !due {
            return;
        }
        // Only the position moved (a seek, or the heartbeat): send just that.
        let light = j.light && j.published.is_some_and(|(s, _, _)| s == sig);
        let gen = self.jam.gen;
        if light {
            let Some(j) = self.jam.jam.as_ref() else { return };
            let now = self.jam_host_now(j);
            let Some(j) = self.jam.jam.as_mut() else { return };
            j.published = Some((sig, pos, Instant::now()));
            // Nothing plays: nothing moves.
            let Some(now) = now else { return };
            j.publishing = true;
            j.state.now = Some(now.clone());
            let code = j.code.clone();
            cx.spawn(async move |this, cx| {
                let r = call(b.jam_now(&code, now)).await;
                let _ = this.update(cx, |s, cx| s.jam_published(gen, r, cx));
            })
            .detach();
            return;
        }
        let Some(state) = self.jam_host_state() else { return };
        let Some(j) = self.jam.jam.as_mut() else { return };
        j.publishing = true;
        j.published = Some((sig, pos, Instant::now()));
        j.state = state.clone();
        cx.spawn(async move |this, cx| {
            let r = call(b.jam_publish(state)).await;
            let _ = this.update(cx, |s, cx| s.jam_published(gen, r, cx));
        })
        .detach();
    }

    /// A publish came back.
    fn jam_published(&mut self, gen: u64, r: Result<(), (Option<String>, String)>, cx: &mut Context<Self>) {
        if self.jam.gen != gen {
            return;
        }
        if let Some(j) = self.jam.jam.as_mut() {
            j.publishing = false;
            if let Err((c, _)) = &r {
                // An older server without `/now`: whole states from now on.
                if matches!(c.as_deref(), Some("not_found" | "method_not_allowed")) {
                    j.light = false;
                }
                // Try again on a later check.
                j.published = None;
            }
        }
        match r {
            Err((c, _)) if c.as_deref() == Some("not_jam_host") => self.jam_demote(cx),
            Err((c, msg)) if gone(c.as_deref()) => self.jam_lost(problem(c.as_deref(), &msg), cx),
            Err((_, msg)) => tracing::info!("jam publish: {msg}"),
            Ok(()) => {}
        }
    }

    /// Runs the friends' requests, as far as the host allows them.
    fn jam_apply_ops(&mut self, ops: Vec<OpFrom>, cx: &mut Context<Self>) {
        for OpFrom { from, op, avatar } in ops {
            let Some(j) = self.jam.jam.as_mut() else { return };
            let member = j.guests.contains(&from);
            let from_server = matches!(op, Op::Join | Op::Leave | Op::Knock);
            if (!member && !from_server) || !j.perms.allows_op(&op) {
                continue;
            }
            match op {
                Op::Join => {
                    if let Some(a) = avatar {
                        j.avatars.insert(from.clone(), a);
                    }
                    j.knocks.retain(|(h, _)| *h != from);
                    if !member && j.guests.len() + 1 < MAX_MEMBERS && from != j.me {
                        j.guests.push(from.clone());
                        j.event(&from, What::Joined);
                        self.toast_sub(t("jam.joined"), format!("@{from}"));
                    }
                }
                Op::Leave => {
                    if member {
                        j.guests.retain(|g| *g != from);
                        j.event(&from, What::Left);
                        self.toast_sub(t("jam.left_them"), format!("@{from}"));
                    }
                }
                Op::Knock => {
                    if !j.knocks.iter().any(|(h, _)| *h == from) {
                        j.knocks.push((from.clone(), avatar));
                        self.toast_sub(t("jam.knock"), format!("@{from}"));
                    }
                }
                Op::Add { track } => self.jam_host_add(track, &from),
                Op::PlayNow { track } => self.jam_host_play_now(track, &from, cx),
                Op::Vote { item, up } => self.jam_host_vote(&item, &from, up),
                Op::Remove { item } => {
                    if j.by.get(&item) == Some(&from) {
                        self.jam_host_remove(&item, cx);
                    }
                }
                Op::Control { control } => self.jam_run_control(control, cx),
                Op::React { reaction } => {
                    if j.reacted.get(&from).is_none_or(|at| at.elapsed() >= REACT_GAP) {
                        j.reacted.insert(from.clone(), Instant::now());
                        j.event(&from, What::React { reaction });
                        self.toast_sub(format!("@{from}"), reaction_label(reaction).to_string());
                    }
                }
            }
        }
        cx.notify();
    }

    /// A friend's song goes to the end of the jam queue (a song already
    /// queued gets their vote instead).
    fn jam_host_add(&mut self, track: Track, from: &str) {
        if !self.allowed(&track) {
            return;
        }
        let id = track_key(&track);
        let queued: Vec<String> = self.queue.upcoming().into_iter().take(self.queue.manual_len()).map(track_key).collect();
        if queued.contains(&id) {
            self.jam_host_vote(&id, from, true);
            return;
        }
        if queued.len() >= MAX_QUEUE {
            return;
        }
        let title = track.title.clone();
        if guard(|| self.queue.push(track)).is_none() {
            return;
        }
        self.queue_changed();
        if let Some(j) = self.jam.jam.as_mut() {
            j.by.insert(id, from.to_string());
            j.event(from, What::Added { title: title.clone() });
        }
        self.toast_sub(tf("jam.added", &[("who", &format!("@{from}"))]), title);
    }

    /// A friend's song plays right away.
    fn jam_host_play_now(&mut self, track: Track, from: &str, cx: &mut Context<Self>) {
        if !self.allowed(&track) {
            return;
        }
        let title = track.title.clone();
        if let Some(j) = self.jam.jam.as_mut() {
            j.by.insert(track_key(&track), from.to_string());
            j.event(from, What::Played { title: title.clone() });
        }
        if guard(|| self.queue.play_next(track)).is_none() {
            return;
        }
        self.queue_changed();
        self.next(false, cx);
        self.toast_sub(tf("jam.played", &[("who", &format!("@{from}"))]), title);
    }

    /// A vote moves the song up among the jam's songs.
    fn jam_host_vote(&mut self, item: &str, from: &str, up: bool) {
        let Some(j) = self.jam.jam.as_mut() else { return };
        let v = j.votes.entry(item.to_string()).or_default();
        if up {
            v.insert(from.to_string());
        } else {
            v.remove(from);
        }
        let votes: HashMap<String, usize> = j.votes.iter().map(|(k, v)| (k.clone(), v.len())).collect();
        let keys: Vec<String> = self.queue.upcoming().into_iter().take(self.queue.manual_len()).map(track_key).collect();
        let want = glyph_core::jam::vote_order(&keys, &votes);
        if want == keys {
            return;
        }
        let mut cur = keys;
        for (i, k) in want.iter().enumerate() {
            if let Some(from_i) = cur.iter().position(|c| c == k) {
                if from_i != i {
                    guard(|| self.queue.move_item(from_i, i));
                    let moved = cur.remove(from_i);
                    cur.insert(i, moved);
                }
            }
        }
        self.queue_changed();
    }

    fn jam_host_remove(&mut self, item: &str, cx: &mut Context<Self>) {
        let n = self.queue.manual_len();
        if let Some(i) = self.queue.upcoming().into_iter().take(n).position(|tr| track_key(tr) == item) {
            guard(|| self.queue.remove(i));
            self.queue_changed();
            cx.notify();
        }
        if let Some(j) = self.jam.jam.as_mut() {
            j.by.remove(item);
            j.votes.remove(item);
        }
    }

    fn jam_run_control(&mut self, c: Control, cx: &mut Context<Self>) {
        match c {
            Control::Play if !self.is_playing() => self.toggle_play(cx),
            Control::Pause if self.is_playing() => self.toggle_play(cx),
            Control::Play | Control::Pause => {}
            Control::Next => self.next(false, cx),
            Control::Previous => self.previous(cx),
            Control::Seek { ms } => self.seek_to(Duration::from_millis(ms)),
        }
    }

    /// Host: what guests may do.
    pub fn jam_set_perms(&mut self, perms: Perms, cx: &mut Context<Self>) {
        if let Some(j) = self.jam.jam.as_mut().filter(|j| j.host) {
            j.perms = perms;
            cx.notify();
        }
    }

    /// Host: new people wait until the host lets them in.
    pub fn jam_set_approve(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Some(j) = self.jam.jam.as_mut().filter(|j| j.host) {
            j.approve = on;
            cx.notify();
        }
    }

    /// Host: lets someone in who knocked, or not.
    pub fn jam_admit(&mut self, handle: &str, allow: bool, cx: &mut Context<Self>) {
        let (Some(b), Some(j)) = (self.backend.clone(), self.jam.jam.as_mut().filter(|j| j.host)) else { return };
        j.knocks.retain(|(h, _)| h != handle);
        let (code, handle) = (j.code.clone(), handle.to_string());
        // Let in: the server sends the usual `Join`.
        cx.background_executor()
            .spawn(async move {
                if let Err((_, e)) = call(b.jam_admit(&code, &handle, allow)).await {
                    tracing::info!("jam admit: {e}");
                }
            })
            .detach();
        cx.notify();
    }

    /// Host: removes someone from the jam (they cannot join again).
    pub fn jam_kick(&mut self, handle: &str, cx: &mut Context<Self>) {
        let (Some(b), Some(j)) = (self.backend.clone(), self.jam.jam.as_mut().filter(|j| j.host)) else { return };
        j.guests.retain(|g| g != handle);
        let (code, handle) = (j.code.clone(), handle.to_string());
        cx.background_executor()
            .spawn(async move {
                if let Err((_, e)) = call(b.jam_kick(&code, &handle)).await {
                    tracing::info!("jam kick: {e}");
                }
            })
            .detach();
        cx.notify();
    }

    // ------------------------------------------------------------ both

    /// Votes for (or takes the vote back from) a queued song.
    pub fn jam_vote(&mut self, item: &Item, cx: &mut Context<Self>) {
        let Some(j) = self.jam.jam.as_mut() else { return };
        let up = !j.voted(item);
        if j.host {
            let me = j.me.clone();
            self.jam_host_vote(&item.id, &me, up);
        } else if j.state.perms.add {
            // Show it at once; the host's next state confirms it.
            let me = j.me.clone();
            if let Some(i) = j.state.queue.iter_mut().find(|i| i.id == item.id) {
                if up {
                    i.votes.push(me);
                } else {
                    i.votes.retain(|v| *v != me);
                }
            }
            j.outbox.push(Op::Vote { item: item.id.clone(), up });
        } else {
            self.toast_info(t("jam.not_allowed"));
        }
        cx.notify();
    }

    /// Takes a song out of the jam queue (host: any, guest: own).
    pub fn jam_remove(&mut self, item: &Item, cx: &mut Context<Self>) {
        let Some(j) = self.jam.jam.as_mut() else { return };
        if j.host {
            self.jam_host_remove(&item.id, cx);
        } else if item.by == j.me {
            j.state.queue.retain(|i| i.id != item.id);
            j.outbox.push(Op::Remove { item: item.id.clone() });
            self.jam_mirror_queue();
        } else {
            self.toast_info(t("jam.not_allowed"));
        }
        cx.notify();
    }

    /// A reaction to the song playing.
    pub fn jam_react(&mut self, reaction: Reaction, cx: &mut Context<Self>) {
        let Some(j) = self.jam.jam.as_mut() else { return };
        if j.reacted.get(&j.me).is_some_and(|at| at.elapsed() < REACT_GAP) {
            return;
        }
        j.reacted.insert(j.me.clone(), Instant::now());
        if j.host {
            let me = j.me.clone();
            j.event(&me, What::React { reaction });
        } else {
            j.outbox.push(Op::React { reaction });
        }
        cx.notify();
    }

    // ------------------------------------------------------------ guest

    /// Intercepts a playback control of a guest: it goes to the host when
    /// allowed, never plays here. `true` = handled.
    pub(super) fn jam_control(&mut self, c: Control) -> bool {
        let Some(j) = self.jam.jam.as_mut().filter(|j| !j.host) else { return false };
        if j.state.perms.allows(&c) {
            j.outbox.push(Op::Control { control: c });
        } else {
            self.toast_info(t("jam.not_allowed"));
        }
        true
    }

    /// A guest picked (`play`) or queued a song: the host gets it for the
    /// jam, to play right away or to queue, as far as allowed. `true` = handled.
    pub(super) fn jam_request(&mut self, track: &Track, play: bool) -> bool {
        let Some(j) = self.jam.jam.as_mut().filter(|j| !j.host) else { return false };
        // The host's song itself: nothing to ask for.
        if j.state.now.as_ref().is_some_and(|n| glyph_core::dedupe::same_song(&n.track, track)) {
            return true;
        }
        let perms = j.state.perms;
        let key = if play && perms.play_now {
            j.outbox.push(Op::PlayNow { track: track.clone() });
            "jam.play_requested"
        } else if perms.add {
            j.outbox.push(Op::Add { track: track.clone() });
            "jam.requested"
        } else {
            self.toast_info(t("jam.not_allowed"));
            return true;
        };
        let title = track.title.clone();
        self.toast_sub(t(key), title);
        true
    }

    /// A guest clicked a song of the (host's) queue. `true` = handled.
    pub(super) fn jam_queue_jump(&mut self, i: usize) -> bool {
        if !self.jam_guest() {
            return false;
        }
        if let Some(t) = self.queue.upcoming().get(i).map(|t| (*t).clone()) {
            self.jam_request(&t, true);
        }
        true
    }

    /// A guest edits the (host's) queue: only removing own songs, which
    /// goes to the host. `remove`: the row; `None`: moving. `true` = handled.
    pub(super) fn jam_queue_edit(&mut self, remove: Option<usize>, cx: &mut Context<Self>) -> bool {
        let Some(j) = self.jam.jam.as_ref().filter(|j| !j.host) else { return false };
        match remove.and_then(|i| j.state.queue.get(i).cloned()) {
            Some(item) => self.jam_remove(&item, cx),
            None => self.toast_info(t("jam.not_allowed")),
        }
        true
    }

    /// The host's song has no source here (from `start_track`).
    pub(super) fn jam_no_source(&mut self, key: &str) {
        if let Some(j) = self.jam.jam.as_mut().filter(|j| !j.host) {
            j.unavailable = Some(key.to_string());
        }
    }

    /// Guest: the music plays here, or only at the host's device.
    pub fn jam_set_listen_here(&mut self, here: bool, cx: &mut Context<Self>) {
        let Some(j) = self.jam.jam.as_mut().filter(|j| !j.host && j.listen_here != here) else { return };
        j.listen_here = here;
        j.synced_at = None;
        j.settle_until = None;
        j.speed = 1.0;
        if here {
            // `jam_sync` starts the host's song here.
            self.jam_clear_remote();
        } else {
            if !self.remote_send(crate::mpd::Cmd::Pause(true)) {
                self.with_player(|p| {
                    p.set_speed(1.0);
                    p.stop();
                });
            }
            self.preloaded = None;
        }
        self.jam_sync(cx);
        cx.notify();
    }

    /// The guest's queue shows the host's: the jam's songs, then the host's
    /// own list.
    fn jam_mirror_queue(&mut self) {
        let Some(j) = self.jam.jam.as_ref().filter(|j| !j.host) else { return };
        let manual: Vec<Track> = j.state.queue.iter().map(|i| i.track.clone()).collect();
        let after = j.state.after.clone();
        let same = {
            let have: Vec<String> = self.queue.upcoming().into_iter().map(track_key).collect();
            let want: Vec<String> = manual.iter().chain(after.iter()).map(track_key).collect();
            have == want && self.queue.manual_len() == manual.len()
        };
        if same {
            return;
        }
        guard(|| self.queue.mirror(manual, after));
        self.queue_changed();
    }

    /// The host is gone: carry on as the host.
    fn jam_take_over(&mut self, cx: &mut Context<Self>) {
        let (Some(b), Some(j)) = (self.backend.clone(), self.jam.jam.as_mut()) else { return };
        j.claiming = true;
        let (gen, code) = (self.jam.gen, j.code.clone());
        cx.spawn(async move |this, cx| {
            let r = call(b.jam_claim(&code)).await;
            let _ = this.update(cx, |s, cx| {
                if s.jam.gen != gen {
                    return;
                }
                match r {
                    Ok(state) => s.jam_become_host(state, cx),
                    Err((c, msg)) if gone(c.as_deref()) => s.jam_lost(problem(c.as_deref(), &msg), cx),
                    Err((_, msg)) => {
                        // The host is back, or someone else was faster.
                        tracing::info!("jam take over: {msg}");
                        if let Some(j) = s.jam.jam.as_mut() {
                            j.claiming = false;
                            j.host_gone = None;
                        }
                    }
                }
            });
        })
        .detach();
    }

    /// Took over: the jam's queue (already this app's queue) and the song
    /// go on from here.
    fn jam_become_host(&mut self, state: State, cx: &mut Context<Self>) {
        let Some(old) = self.jam.jam.take() else { return };
        let remote = !old.listen_here;
        let mut j = Jam::new(state.code.clone(), true, old.me.clone(), state.clone());
        j.clock = old.clock;
        j.guests = state.members.iter().filter(|m| !m.host && m.handle != j.me).map(|m| m.handle.clone()).collect();
        for item in &state.queue {
            j.by.insert(item.id.clone(), item.by.clone());
            if !item.votes.is_empty() {
                j.votes.insert(item.id.clone(), item.votes.iter().cloned().collect());
            }
        }
        let me = j.me.clone();
        j.event(&me, What::Host);
        if (old.speed - 1.0).abs() > f32::EPSILON {
            self.with_player(|p| p.set_speed(1.0));
        }
        // Not listening here so far: start the song where the old host was.
        if remote {
            if let Some(n) = state.now.clone() {
                j.resume_at = state.position_at(j.server_now());
                self.jam_clear_remote();
                self.start_track(n.resolved.unwrap_or(n.track), None, cx);
                self.hold_play = !n.playing;
            }
        }
        self.jam.jam = Some(j);
        self.jam.gen += 1;
        self.jam_host_loop(cx);
        self.toast(t("jam.you_host"));
        cx.notify();
    }

    fn jam_flush(&mut self, cx: &mut Context<Self>) {
        let (Some(b), Some(j)) = (self.backend.clone(), self.jam.jam.as_mut()) else { return };
        let ops = std::mem::take(&mut j.outbox);
        j.sending = true;
        let (gen, code) = (self.jam.gen, j.code.clone());
        cx.spawn(async move |this, cx| {
            let mut err = None;
            for op in ops {
                if let Err(e) = call(b.jam_op(&code, op)).await {
                    err = Some(e);
                    break;
                }
            }
            let _ = this.update(cx, |s, cx| {
                if s.jam.gen != gen {
                    return;
                }
                if let Some(j) = s.jam.jam.as_mut() {
                    j.sending = false;
                }
                match err {
                    Some((c, msg)) if gone(c.as_deref()) => s.jam_lost(problem(c.as_deref(), &msg), cx),
                    Some((_, msg)) => s.toast_info(msg),
                    None => {}
                }
            });
        })
        .detach();
    }

    fn jam_apply_state(&mut self, state: State, cx: &mut Context<Self>) {
        let Some(j) = self.jam.jam.as_mut() else { return };
        j.offset_ms = state.server_ms as i64 - unix_ms() as i64;
        // Reactions of others since the last state pop up.
        let fresh: Vec<(String, Reaction)> = state
            .feed
            .iter()
            .filter(|e| e.at_ms > j.feed_seen && e.who != j.me)
            .filter_map(|e| match e.what {
                What::React { reaction } => Some((e.who.clone(), reaction)),
                _ => None,
            })
            .collect();
        j.feed_seen = state.feed.iter().map(|e| e.at_ms).max().unwrap_or(j.feed_seen);
        if state.host_online {
            j.host_gone = None;
        } else if j.host_gone.is_none() {
            j.host_gone = Some(Instant::now());
        }
        let new_song = j.state.now.as_ref().map(|n| track_key(&n.track)) != state.now.as_ref().map(|n| track_key(&n.track));
        if new_song {
            j.unavailable = None;
        }
        j.state = state;
        j.synced_at = None;
        for (who, r) in fresh {
            self.toast_sub(format!("@{who}"), reaction_label(r).to_string());
        }
        self.jam_mirror_queue();
        self.jam_sync(cx);
        cx.notify();
    }

    /// Listening at the host: the player shows the host's song and where it
    /// is, nothing plays here. Called every frame instead of reading the
    /// player; returns whether to redraw.
    pub(super) fn jam_remote_state(&mut self) -> bool {
        let Some(j) = self.jam.jam.as_ref().filter(|j| !j.host && !j.listen_here) else { return false };
        let Some(now) = j.state.now.as_ref() else {
            let had = self.player_state.is_some();
            self.player_state = None;
            return had;
        };
        // Every frame: only the song itself is copied.
        let (track, source, playing) = (now.track.clone(), now.resolved.as_ref().unwrap_or(&now.track).source, now.playing);
        let pos = Duration::from_millis(j.host_position().unwrap_or(0));
        if !self.current.as_ref().is_some_and(|c| glyph_core::dedupe::same_song(c, &track)) {
            self.current = Some(track.clone());
            self.current_source = Some(source);
            self.current_preview = false;
            self.resolving = false;
        }
        let state = PlayerState {
            status: if playing { Status::Playing } else { Status::Paused },
            duration: track.duration_ms.map(Duration::from_millis),
            track: Some(track),
            provider: self.current_source,
            backend: Some(glyph_audio::BackendKind::Native),
            is_preview: false,
            position: pos,
            volume: self.volume(),
        };
        let step = |p: Duration| p.as_millis() / 250;
        let changed = match &self.player_state {
            // Field by field: no copy of the old state every frame.
            Some(old) => {
                old.status != state.status
                    || old.track != state.track
                    || old.provider != state.provider
                    || old.backend != state.backend
                    || old.is_preview != state.is_preview
                    || old.duration != state.duration
                    || old.volume != state.volume
                    || step(old.position) != step(state.position)
            }
            None => true,
        };
        if self.pos_anchor.map(|a| a.0) != Some(state.position) {
            self.pos_anchor = Some((state.position, Instant::now()));
        }
        self.player_state = Some(state);
        changed
    }

    /// Plays the host's song here (from the host's source when it can), at
    /// the host's moment, playing or paused like the host.
    fn jam_sync(&mut self, cx: &mut Context<Self>) {
        let Some(j) = self.jam.jam.as_mut().filter(|j| !j.host) else { return };
        j.synced_at = Some(Instant::now());
        if !j.listen_here {
            return;
        }
        let Some(now) = j.state.now.clone() else { return };
        let settling = j.settle_until.is_some_and(|t| Instant::now() < t);
        let here = self.current.as_ref().is_some_and(|c| glyph_core::dedupe::same_song(c, &now.track));
        if !here {
            // The host's song is about to end and this app already went on
            // to the next one: the host follows in a moment.
            let ending = now.track.duration_ms.is_some_and(|d| j.host_position().unwrap_or(0) + 4000 >= d);
            // Also when something else was started here meanwhile.
            if !settling && !ending {
                j.settle_until = Some(Instant::now() + SETTLE);
                j.speed = 1.0;
                self.start_track(now.resolved.clone().unwrap_or_else(|| now.track.clone()), None, cx);
                // Load it paused when the host is paused.
                self.hold_play = !now.playing;
            }
            return;
        }
        if self.resolving || settling {
            return;
        }
        let want = j.host_position().unwrap_or(0);
        let speed = j.speed;
        if now.playing != self.is_playing() {
            if !self.remote_send(crate::mpd::Cmd::Pause(!now.playing)) {
                self.with_player(|p| if now.playing { p.play() } else { p.pause() });
            }
        }
        if !now.playing {
            return;
        }
        let have = self.position().as_millis() as u64;
        let can_nudge = self.device_out.is_none() && self.mpd.is_none() && !self.embed_playing();
        let (speed_to, seek) = match glyph_core::jam::step(have, want, can_nudge) {
            Step::Keep => (1.0, None),
            Step::Nudge(r) => (r, None),
            Step::Seek(ms) => (1.0, Some(ms)),
        };
        if (speed_to - speed).abs() > 0.002 && can_nudge {
            self.with_player(|p| p.set_speed(speed_to));
        }
        if let Some(j) = self.jam.jam.as_mut() {
            j.speed = speed_to;
            if seek.is_some() {
                j.settle_until = Some(Instant::now() + SETTLE);
            }
        }
        if let Some(ms) = seek {
            let to = Duration::from_millis(ms);
            if !self.remote_send(crate::mpd::Cmd::Seek(to.as_secs_f64())) {
                self.with_player(|p| p.seek(to));
            }
        }
    }
}
