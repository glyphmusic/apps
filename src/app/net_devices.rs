use super::*;
use crate::devices::{Device, Kind};
use crate::mpd::Cmd;

impl Glyph {
    // ------------------------------------------------------------ remote outputs

    /// Songs play somewhere else (MPD or a network device), not here.
    pub fn remote_on(&self) -> bool {
        (self.mpd.is_some() || self.device_out.is_some()) && !self.local_fallback
    }

    /// A remote output is chosen (even while this song plays here because
    /// the device cannot play it).
    pub fn remote_chosen(&self) -> bool {
        self.mpd.is_some() || self.device_out.is_some()
    }

    /// Sends a command to the remote output; `false` when playing here.
    pub(super) fn remote_send(&self, c: Cmd) -> bool {
        if self.local_fallback {
            return false;
        }
        if let Some(m) = &self.mpd {
            m.send(c);
            return true;
        }
        if let Some(d) = &self.device_out {
            d.send(c);
            return true;
        }
        false
    }

    pub(super) fn remote_status(&self) -> Option<crate::mpd::Status> {
        if self.local_fallback {
            return None;
        }
        if let Some(m) = &self.mpd {
            return m.status();
        }
        self.device_out.as_ref().and_then(|d| d.status())
    }

    /// Hands a found song to where it plays: this computer, MPD or a
    /// network device, `at` seconds in.
    pub(super) fn output_load_at(&mut self, tr: Track, src: glyph_protocol::PlaybackSource, start: bool, at: f64) {
        self.last_load = Some((tr.clone(), src.clone()));
        self.device_failed = false;
        self.local_fallback = false;
        if let Some((ok, name)) = self.device_out.as_ref().map(|d| (d.load(&tr, &src, start, at), d.device.name.clone())) {
            if ok {
                self.mpd_was_playing = false;
                // Nothing plays here at the same time.
                self.with_player(|p| p.stop());
                return;
            }
            self.local_fallback = true;
            self.toast_info(tf("toast.device_unsupported", &[("name", &name)]));
        } else if self.mpd.is_some() {
            let url = self.mpd.as_ref().and_then(|m| match &src {
                glyph_protocol::PlaybackSource::Stream { url, .. } | glyph_protocol::PlaybackSource::Preview { url, .. } => m.url_for(url),
                _ => None,
            });
            match url {
                Some(url) => {
                    if let Some(m) = &self.mpd {
                        m.send(Cmd::Play { url, start });
                        if at > 1.0 {
                            m.send(Cmd::Seek(at));
                        }
                    }
                    self.mpd_was_playing = false;
                    self.with_player(|p| p.stop());
                    return;
                }
                None => {
                    self.local_fallback = true;
                    self.toast_info(t("toast.mpd_unsupported"));
                }
            }
        }
        self.with_player(move |p| {
            p.load(tr, src);
            if start {
                p.play();
            }
            if at > 1.0 {
                p.seek(Duration::from_secs_f64(at));
            }
        });
    }

    /// After the output changed: the current song goes on where it is now
    /// played, from the same position.
    fn continue_on_new_output(&mut self, pos: f64, playing: bool) {
        if self.resolving {
            return;
        }
        let (Some(cur), Some((tr, src))) = (self.current.clone(), self.last_load.clone()) else { return };
        if !glyph_core::dedupe::same_song(&cur, &tr) {
            return;
        }
        self.output_load_at(tr, src, playing, pos);
    }

    /// Stops the network device output (if any) without touching MPD.
    pub(super) fn drop_device_output(&mut self) {
        if let Some(old) = self.device_out.take() {
            old.send(Cmd::Stop);
        }
        self.device_failed = false;
    }

    /// Plays on a found device; MPD servers go to the MPD output.
    pub fn select_device(&mut self, dev: Device, cx: &mut Context<Self>) {
        if !dev.kind.playable() || self.device_out.as_ref().is_some_and(|d| d.device.id == dev.id) {
            return;
        }
        if dev.kind == Kind::Mpd && self.mpd.as_ref().is_some_and(|m| crate::mpd::Address::parse(&dev.mpd_address()).as_ref() == Some(&m.address)) {
            return;
        }
        let (pos, playing) = (self.position().as_secs_f64(), self.is_playing());
        if dev.kind == Kind::Mpd {
            self.drop_device_output();
            let addr = dev.mpd_address();
            self.config.mpd.address = addr.clone();
            self.mpd_input.update(cx, |i, cx| i.set_value(addr, cx));
            self.local_fallback = false;
            self.set_mpd_enabled(true, cx);
        } else {
            if self.config.mpd.enabled {
                self.config.mpd.enabled = false;
                self.save_config(cx);
                self.sync_mpd(cx);
            }
            self.drop_device_output();
            tracing::info!(device = %dev.name, kind = ?dev.kind, "network output on");
            self.with_player(|p| p.pause());
            self.local_fallback = false;
            self.mpd_was_playing = false;
            self.device_out = Some(crate::devices::Output::start(dev));
        }
        self.continue_on_new_output(pos, playing);
        cx.notify();
    }

    /// Back to this computer (the user picked a local sound device).
    pub fn use_local_output(&mut self, cx: &mut Context<Self>) {
        if !self.remote_chosen() {
            return;
        }
        let (pos, playing) = (self.position().as_secs_f64(), self.is_playing());
        let was_remote = self.remote_on();
        if self.config.mpd.enabled {
            self.config.mpd.enabled = false;
            self.save_config(cx);
        }
        self.sync_mpd(cx);
        self.drop_device_output();
        self.local_fallback = false;
        self.mpd_was_playing = false;
        if was_remote {
            self.continue_on_new_output(pos, playing);
        }
        cx.notify();
    }

    // ------------------------------------------------------------ discovery

    /// Starts / stops the device search as the settings say.
    pub fn sync_discovery(&mut self, cx: &mut Context<Self>) {
        let want = self.config.devices.discovery;
        if want == self.discovery.is_some() {
            return;
        }
        self.discovery = want.then(crate::devices::Discovery::start);
        cx.notify();
    }

    /// Settings: look for devices on the network.
    pub fn set_discovery(&mut self, on: bool, cx: &mut Context<Self>) {
        self.config.devices.discovery = on;
        self.save_config(cx);
        self.sync_discovery(cx);
    }

    /// Opens or closes the Connect panel (the queue's slot); opening looks
    /// for devices again.
    pub fn toggle_devices_panel(&mut self, cx: &mut Context<Self>) {
        self.show_devices = !self.show_devices;
        if self.show_devices {
            self.show_queue = false;
            if let Some(a) = self.add_songs.as_mut() {
                a.open = false;
            }
            self.rescan_devices();
        }
        cx.notify();
    }

    /// The panel's refresh button: search now.
    pub fn rescan_devices_now(&self) {
        if let Some(d) = &self.discovery {
            d.rescan_now();
        }
    }

    /// The output menu opened: look again.
    pub(super) fn rescan_devices(&self) {
        if let Some(d) = &self.discovery {
            d.rescan();
        }
    }

    /// Found devices or the searching state changed (redraw).
    pub(super) fn poll_discovery(&mut self) -> bool {
        let g = self.discovery.as_ref().map_or(0, |d| d.generation());
        if g == self.discovery_gen {
            return false;
        }
        self.discovery_gen = g;
        true
    }

    /// Found devices (empty when the search is off).
    pub fn network_devices(&self) -> Vec<Device> {
        self.discovery.as_ref().map(|d| d.devices()).unwrap_or_default()
    }

    pub fn devices_searching(&self) -> bool {
        self.discovery.as_ref().is_some_and(|d| d.searching())
    }

    /// The network device playing now (menu check, settings status).
    pub fn device_selected(&self) -> Option<&Device> {
        self.device_out.as_ref().map(|d| &d.device)
    }

    /// The device could not play the song it got: say so once.
    pub(super) fn note_device_failure(&mut self, state: &str) {
        if state != "error" || self.device_failed || self.resolving {
            return;
        }
        self.device_failed = true;
        let name = match (&self.device_out, &self.mpd) {
            (Some(d), _) => d.device.name.clone(),
            (None, Some(m)) => m.address.label(),
            _ => return,
        };
        self.toast_info(tf("toast.device_failed", &[("name", &name)]));
    }
}
