use super::*;
use glyph_protocol::playback::{Command, Send, State};

/// Last report older than this: commands waiting on the server may be older
/// than the sender waits for them (25 s; asleep, offline, just started). They
/// are refused instead of, say, pausing out of nowhere half a minute later.
const STALE_POLL: Duration = Duration::from_secs(25);
/// A received transfer is confirmed at the latest after this, even while the
/// song is still being found (the sender gives up after 25 s).
const TRANSFER_ACK_MAX: Duration = Duration::from_secs(18);
/// The controlled device may miss reports this long before it counts as gone.
const OUTPUT_GRACE: Duration = Duration::from_secs(20);
/// Play / pause sent to the controlled device shows this long, until it reports it.
const OUTPUT_OPTIMISTIC: Duration = Duration::from_secs(6);
/// Polls failing in a row before the device list is given up.
const POLL_FAILURES: u32 = 3;

impl Glyph {
    pub(super) fn finish_account_transfer(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        self.account.transfer_ack_at = None;
        if let Some(id) = self.account.transfer_ack.take() {
            self.account.output_seen.insert(id.clone(), error.clone());
            self.ack_account_command(id, error, cx);
        }
    }

    fn ack_account_command(&mut self, id: String, error: Option<String>, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        cx.spawn(async move |_, _| {
            if let Err(e) = b.playback_ack(glyph_protocol::playback::Receipt { id, done: true, error }).await { tracing::info!("playback acknowledgement: {e:#}"); }
        }).detach();
    }

    fn connect_account_live(&mut self, cx: &mut Context<Self>) {
        if self.account.output_live.is_some() || self.account.output_live_connecting || self.account.output_live_checked.is_some_and(|at| at.elapsed() < Duration::from_secs(15)) { return; }
        let Some(b) = self.backend.clone() else { return };
        let owner = b.account();
        self.account.output_live_connecting = true;
        self.account.output_live_checked = Some(Instant::now());
        cx.spawn(async move |this, cx| {
            let result = b.playback_live().await;
            this.update(cx, |s, _| {
                if s.signed_in() != owner { return; }
                s.account.output_live_connecting = false;
                if let Ok(rx) = result {
                    s.account.output_live = Some(rx);
                    // Commands sent while the line was down: fetch them now.
                    s.account.output_checked = None;
                }
            }).ok();
        }).detach();
    }

    /// The controlled device (also while it misses a report or two).
    pub fn account_output(&self) -> Option<&glyph_protocol::playback::Output> {
        let id = self.account.output_target.as_ref()?;
        self.account.outputs.iter().find(|o| &o.device.id == id)
    }

    /// Play / pause just sent to the controlled device, until it reports it.
    pub(super) fn account_playing_hint(&self) -> Option<bool> {
        self.account.output_playing.get().filter(|(_, at)| at.elapsed() < OUTPUT_OPTIMISTIC).map(|(p, _)| p)
    }

    pub(super) fn account_control(&self, command: Command) -> bool {
        let Some(target) = self.account.output_target.clone() else {
            return false;
        };
        // The device reports back only with its next poll: show the wish now,
        // so a second click toggles back instead of sending the same again.
        match command {
            Command::Play => self.account.output_playing.set(Some((true, Instant::now()))),
            Command::Pause | Command::Stop => self.account.output_playing.set(Some((false, Instant::now()))),
            _ => {}
        }
        if let Some(b) = self.backend.as_ref().filter(|_| !self.account.signing_out) {
            let rx = b.playback_send(Send { target, command, request_id: None });
            self.account.output_commands.borrow_mut().push(rx);
        }
        true
    }

    /// Select an account device to control its existing playback.
    pub fn select_account_output(&mut self, id: String, cx: &mut Context<Self>) {
        if !self
            .account
            .outputs
            .iter()
            .any(|o| o.device.id == id && !o.device.current && o.online && o.state.available)
        {
            return;
        }
        self.flush_stats(cx);
        self.with_player(|p| p.pause());
        self.drop_device_output();
        if self.config.mpd.enabled {
            self.config.mpd.enabled = false;
            self.save_config(cx);
            self.sync_mpd(cx);
        }
        self.account.output_target = Some(id);
        self.account.output_error = None;
        self.account.output_error_at = None;
        self.account.output_missing_since = None;
        self.account.output_playing.set(None);
        if let Some(output) = self.account_output() {
            self.current = output.state.track.clone();
        }
        cx.notify();
    }

    /// A click on a Glyph device: what plays here moves there. With nothing
    /// playing here while the device plays, its playback is taken over.
    pub fn connect_account_output(&mut self, id: String, cx: &mut Context<Self>) {
        if self.account.transfer_to.is_some() || self.account.output_target.as_ref() == Some(&id) {
            return;
        }
        let theirs = self.account.outputs.iter().find(|o| o.device.id == id).is_some_and(|o| o.state.playing && o.state.track.is_some());
        if self.current.is_none() || (theirs && !self.is_playing()) {
            self.select_account_output(id, cx);
        } else {
            self.transfer_account_output(id, cx);
        }
    }

    /// Move this queue and position to a signed-in app. Stop here after acceptance.
    pub fn transfer_account_output(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else {
            return;
        };
        let owner = b.account();
        let Some(track) = self.current.clone() else {
            self.select_account_output(id, cx);
            return;
        };
        // Controlling another device: its song moves on (this app's own
        // queue is not what plays there), and that device stops afterwards.
        let previous = self.account.output_target.clone();
        let mut tracks = vec![track];
        if previous.is_none() {
            tracks.extend(self.upcoming().into_iter().take(99));
        }
        let send = Send {
            request_id: None,
            target: id.clone(),
            command: Command::Transfer {
                tracks,
                index: 0,
                position_ms: self.position().as_millis() as u64,
                playing: self.is_playing(),
            },
        };
        self.account.transfer_to = Some(id.clone());
        self.account.output_error = None;
        self.account.output_error_at = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = b.playback_command(send).await;
            this.update(cx, |s, cx| {
                if s.signed_in() != owner {
                    return;
                }
                s.account.transfer_to = None;
                match r {
                    Ok(()) => {
                        if let Some(old) = previous.filter(|old| *old != id) {
                            if let Some(b) = s.backend.as_ref() {
                                let rx = b.playback_send(Send { target: old, command: Command::Stop, request_id: None });
                                s.account.output_commands.borrow_mut().push(rx);
                            }
                        }
                        s.select_account_output(id, cx);
                    }
                    Err(e) => {
                        s.account.output_error = Some(e.to_string());
                        s.account.output_error_at = Some(id);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn poll_account_outputs(&mut self, cx: &mut Context<Self>) {
        if let Some(at) = self.account.transfer_ack_at {
            // Confirmed once the moved song runs here (the player holds the
            // resolved track, which may be named differently), or after a
            // while anyway: it was accepted and is still being found.
            let started = !self.resolving
                && self.account.transfer_resume.is_none()
                && self.player_state.as_ref().is_some_and(|p| {
                    matches!(p.status, Status::Playing | Status::Paused)
                        && p.track.as_ref().is_some_and(|t| {
                            self.current.as_ref().is_some_and(|c| track_key(c) == track_key(t) || glyph_core::dedupe::same_song(c, t))
                                || self.last_load.as_ref().is_some_and(|(l, _)| track_key(l) == track_key(t))
                        })
                });
            if started || at.elapsed() > TRANSFER_ACK_MAX {
                let error = self.current.is_none().then(|| "Could not start playback on target device".to_string());
                self.finish_account_transfer(error, cx);
            }
        }
        let errors: Vec<_> = {
            let mut pending = self.account.output_commands.borrow_mut();
            let mut errors = Vec::new();
            pending.retain(|rx| match rx.try_recv() {
                Ok(Err(e)) => {
                    errors.push(e.to_string());
                    false
                }
                Ok(Ok(())) | Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
                Err(std::sync::mpsc::TryRecvError::Empty) => true,
            });
            errors
        };
        for e in errors {
            // Not carried out: the device's own report counts again.
            self.account.output_playing.set(None);
            self.account.output_error = Some(e);
            self.account.output_error_at = self.account.output_target.clone();
            cx.notify();
        }
        let Some(b) = self.backend.clone() else {
            return;
        };
        if b.account().is_none() || self.account.signing_out || self.account.merge_pending {
            self.account.outputs.clear();
            self.account.output_target = None;
            self.account.output_live = None;
            return;
        }
        self.connect_account_live(cx);
        if let Some(rx) = self.account.output_live.as_mut() {
            let mut closed = false;
            loop { match rx.try_recv() {
                Ok(()) => self.account.output_checked = None,
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => { closed = true; break; }
            }}
            if closed { self.account.output_live = None; }
        }
        let controlling = self.account.output_target.is_some();
        // Without the live line nothing wakes this app for a command: ask
        // more often, so a click elsewhere does not take a quarter minute.
        let interval = if self.show_devices || controlling || self.account.transfer_to.is_some() {
            2
        } else if self.is_playing() {
            5
        } else if self.account.output_live.is_none() {
            6
        } else {
            15
        };
        if self.account.output_busy
            || self
                .account
                .output_checked
                .is_some_and(|at| at.elapsed() < Duration::from_secs(interval))
        {
            return;
        }
        self.account.output_checked = Some(Instant::now());
        self.account.output_busy = true;
        let owner = b.account();
        let state = State {
            acknowledgements: true,
            track: if controlling {
                None
            } else {
                self.current.clone()
            },
            // A song being found counts as playing unless it was paused,
            // so the controller's play button does not flip back and forth.
            playing: !controlling && (self.is_playing() || (self.resolving && !self.hold_play)),
            position_ms: if controlling {
                0
            } else {
                self.position().as_millis() as u64
            },
            duration_ms: self.duration().map(|d| d.as_millis() as u64),
            volume: self.player_state.as_ref().map(|s| s.volume).unwrap_or(0.8),
            available: !self.jam_active() && !controlling,
        };
        cx.spawn(async move |this, cx| {
            let result = b.playback_poll(state).await;
            this.update(cx, |s, cx| {
                s.account.output_busy = false;
                if s.signed_in() != owner || s.account.signing_out {
                    return;
                }
                match result {
                    Ok(poll) => {
                        let stale = s.account.output_received.is_none_or(|at| at.elapsed() > STALE_POLL);
                        if s.account.output_failures >= POLL_FAILURES {
                            s.account.output_error = None;
                        }
                        s.account.output_failures = 0;
                        let mut devices = poll.devices;
                        // The controlled device missed a report: keep what it
                        // said last instead of showing it stopped.
                        if let Some(id) = s.account.output_target.as_ref() {
                            if let (Some(new), Some(old)) = (devices.iter_mut().find(|o| &o.device.id == id && !o.online), s.account.outputs.iter().find(|o| &o.device.id == id)) {
                                new.state = old.state.clone();
                            }
                        }
                        s.account.outputs = devices;
                        s.account.output_received = Some(Instant::now());
                        if let Some(id) = s.account.output_target.clone() {
                            let found = s.account.outputs.iter().find(|o| o.device.id == id).map(|o| (o.online, o.state.playing, o.state.track.clone()));
                            match found {
                                Some((true, playing, track)) => {
                                    s.account.output_missing_since = None;
                                    s.current = track;
                                    if s.account.output_playing.get().is_some_and(|(p, _)| p == playing) {
                                        s.account.output_playing.set(None);
                                    }
                                }
                                found => {
                                    let since = *s.account.output_missing_since.get_or_insert_with(Instant::now);
                                    if found.is_none() || since.elapsed() > OUTPUT_GRACE {
                                        s.account.output_target = None;
                                        s.account.output_missing_since = None;
                                        s.account.output_playing.set(None);
                                        s.current = None;
                                        s.account.output_error = Some(t("connect.account.offline").to_string());
                                        s.account.output_error_at = None;
                                    }
                                }
                            }
                        }
                        for command in poll.commands {
                            if !s.jam_active() && !stale {
                                s.apply_account_command(command, cx);
                            }
                        }
                        for delivery in poll.deliveries {
                            if s.account.transfer_ack.as_ref() == Some(&delivery.id) { continue; }
                            if let Some(error) = s.account.output_seen.get(&delivery.id).cloned() {
                                s.ack_account_command(delivery.id, error, cx);
                                continue;
                            }
                            let error = if stale {
                                Some("Command expired".to_string())
                            } else if s.jam_active() || s.account.output_target.is_some() {
                                Some("Device is busy".to_string())
                            } else {
                                None
                            };
                            let transfer = matches!(delivery.command, Command::Transfer { .. });
                            if error.is_none() { s.apply_account_command(delivery.command, cx); }
                            if transfer && error.is_none() {
                                // A newer transfer replaces one still starting: that one is done.
                                if let Some(old) = s.account.transfer_ack.replace(delivery.id) {
                                    s.account.output_seen.insert(old.clone(), None);
                                    s.ack_account_command(old, None, cx);
                                }
                                s.account.transfer_ack_at = Some(Instant::now());
                            } else {
                                s.account.output_seen.insert(delivery.id.clone(), error.clone());
                                s.ack_account_command(delivery.id, error, cx);
                            }
                        }
                        while s.account.output_seen.len() > 256 { let key = s.account.output_seen.keys().next().cloned().unwrap(); s.account.output_seen.remove(&key); }
                    }
                    Err(e) => {
                        // Short outages keep the list (and the controlled
                        // device); only repeated failures clear it. Never
                        // resume locally when a selected device disappears.
                        s.account.output_failures += 1;
                        if s.account.output_failures >= POLL_FAILURES {
                            s.account.output_error = Some(e.to_string());
                            s.account.output_error_at = None;
                            s.account.outputs.clear();
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply_account_command(&mut self, command: Command, cx: &mut Context<Self>) {
        self.account.output_target = None;
        match command {
            Command::Transfer {
                tracks,
                index,
                position_ms,
                playing,
            } => {
                self.use_local_output(cx);
                self.play_list(tracks, index, cx);
                self.account.transfer_resume = Some((position_ms, playing));
            }
            // While a song is being found the player still holds the old
            // one: set the wish directly (toggling would flip it).
            Command::Play => {
                if self.resolving {
                    self.hold_play = false;
                } else if !self.is_playing() {
                    self.toggle_play(cx);
                }
            }
            Command::Pause => {
                if self.resolving {
                    self.hold_play = true;
                } else if self.is_playing() {
                    self.toggle_play(cx);
                }
            }
            Command::Stop => {
                if !self.remote_send(crate::mpd::Cmd::Stop) {
                    self.with_player(|p| p.stop());
                }
                self.current = None;
            }
            Command::Next => self.next(false, cx),
            Command::Previous => self.previous(cx),
            Command::Seek { position_ms } => self.seek_to(Duration::from_millis(position_ms)),
            Command::Volume { volume } => self.set_volume(volume),
        }
    }
}
