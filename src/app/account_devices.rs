use super::*;
use glyph_protocol::playback::{Command, Send, State};

impl Glyph {
    pub(super) fn finish_account_transfer(&mut self, error: Option<String>, cx: &mut Context<Self>) {
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
        if self.account.output_live.is_some() || self.account.output_live_connecting || self.account.output_live_checked.is_some_and(|at| at.elapsed() < Duration::from_secs(60)) { return; }
        let Some(b) = self.backend.clone() else { return };
        let owner = b.account();
        self.account.output_live_connecting = true;
        self.account.output_live_checked = Some(Instant::now());
        cx.spawn(async move |this, cx| {
            let result = b.playback_live().await;
            this.update(cx, |s, _| {
                if s.signed_in() != owner { return; }
                s.account.output_live_connecting = false;
                if let Ok(rx) = result { s.account.output_live = Some(rx); }
            }).ok();
        }).detach();
    }
    pub fn account_output(&self) -> Option<&glyph_protocol::playback::Output> {
        let id = self.account.output_target.as_ref()?;
        self.account
            .outputs
            .iter()
            .find(|o| &o.device.id == id && o.online)
    }

    pub(super) fn account_control(&self, command: Command) -> bool {
        let Some(target) = self.account.output_target.clone() else {
            return false;
        };
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
        if let Some(output) = self.account_output() {
            self.current = output.state.track.clone();
        }
        cx.notify();
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
        let mut tracks = vec![track];
        tracks.extend(self.upcoming().into_iter().take(99));
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
        cx.spawn(async move |this, cx| {
            let r = b.playback_command(send).await;
            this.update(cx, |s, cx| {
                if s.signed_in() != owner {
                    return;
                }
                match r {
                    Ok(()) => {
                        s.select_account_output(id, cx);
                    }
                    Err(e) => {
                        s.account.output_error = Some(e.to_string());
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn poll_account_outputs(&mut self, cx: &mut Context<Self>) {
        if self.account.transfer_ack.is_some() && !self.resolving && self.account.transfer_resume.is_none()
            && self.player_state.as_ref().is_some_and(|p| p.track.as_ref().map(track_key) == self.current.as_ref().map(track_key) && matches!(p.status, glyph_audio::Status::Playing | glyph_audio::Status::Paused)) {
            self.finish_account_transfer(None, cx);
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
            self.account.output_error = Some(e);
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
        let interval = if self.show_devices || self.account.output_target.is_some() {
            2
        } else if self.is_playing() {
            5
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
        let controlling = self.account.output_target.is_some();
        let state = State {
            acknowledgements: true,
            track: if controlling {
                None
            } else {
                self.current.clone()
            },
            playing: !controlling && self.is_playing(),
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
                        s.account.outputs = poll.devices;
                        s.account.output_received = Some(Instant::now());
                        if let Some(output) = s.account_output() {
                            s.current = output.state.track.clone();
                        } else if s.account.output_target.take().is_some() {
                            s.current = None;
                            s.account.output_error = Some(t("connect.account.offline").to_string());
                        }
                        for command in poll.commands {
                            if !s.jam_active() {
                                s.apply_account_command(command, cx);
                            }
                        }
                        for delivery in poll.deliveries {
                            if s.account.transfer_ack.as_ref() == Some(&delivery.id) { continue; }
                            if let Some(error) = s.account.output_seen.get(&delivery.id).cloned() {
                                s.ack_account_command(delivery.id, error, cx);
                                continue;
                            }
                            let error = (s.jam_active() || s.account.output_target.is_some()).then(|| "Device is busy".to_string());
                            let transfer = matches!(delivery.command, Command::Transfer { .. });
                            if error.is_none() { s.apply_account_command(delivery.command, cx); }
                            if transfer && error.is_none() { s.account.transfer_ack = Some(delivery.id); }
                            else {
                                s.account.output_seen.insert(delivery.id.clone(), error.clone());
                                s.ack_account_command(delivery.id, error, cx);
                            }
                        }
                        while s.account.output_seen.len() > 256 { let key = s.account.output_seen.keys().next().cloned().unwrap(); s.account.output_seen.remove(&key); }
                    }
                    Err(e) => {
                        s.account.output_error = Some(e.to_string());
                        // Never resume locally when a selected device disappears.
                        s.account.outputs.clear();
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
            Command::Play => {
                if !self.is_playing() {
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
