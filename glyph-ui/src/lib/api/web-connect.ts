// Account-scoped remote playback for the browser player (/web): mirrors
// glyph-app::account remote playback over /v1/devices/playback/* so users on
// web can see their signed-in PC/phone devices, transfer playback to/from
// them, and control them from the Connect sidebar.
import type { BackendEvents, PlayerState, Track } from "./types";
import type { DeviceX } from "../connect";
import { t } from "../i18n.svelte";
import {
  server,
  wireTrack,
  type PlaybackCommandWire,
  type PlaybackOutputWire,
  type PlaybackStateWire,
} from "./web-server";

const OUTPUT_GRACE_MS = 20_000;
const OPTIMISTIC_MS = 6_000;
const POSITION_RUN_ON_MS = 40_000;
const POLL_FAILURES = 3;

export interface WebConnectHost {
  isSignedIn: () => boolean;
  withToken: <T>(fn: (token: string) => Promise<T>) => Promise<T>;
  getLocalPlayer: () => PlayerState;
  isLocalPlaying: () => boolean;
  getUpcoming: (limit: number) => Track[];
  emit: <K extends keyof BackendEvents>(ev: K, p: BackendEvents[K]) => void;
  onApplyCommand: (cmd: PlaybackCommandWire) => Promise<void>;
  onResumeLocal: (track: Track, positionSec: number, playing: boolean) => void;
  pauseLocal: () => void;
}

export class WebConnectManager {
  private outputs: PlaybackOutputWire[] = [];
  private targetId: string | null = null;
  private transferToId: string | null = null;
  private error: string | null = null;
  private errorAt: string | null = null;
  private playingHint: { playing: boolean; at: number } | null = null;
  private optimisticPos: { ms: number; at: number } | null = null;
  private optimisticVol: { volume: number; at: number } | null = null;
  private stateAt = 0;
  private missingSince: number | null = null;
  private failures = 0;
  private panelOpen = false;
  private seenAck = new Map<string, string | null>();
  private pollTimer: ReturnType<typeof setTimeout> | null = null;
  private polling = false;
  private pollQueued = false;

  constructor(private host: WebConnectHost) {}

  isControlling(): boolean {
    return this.targetId !== null;
  }

  devicesJson(): DeviceX[] {
    return this.outputs
      .filter((o) => !o.device.current)
      .map((o) => ({
        id: o.device.id,
        name: o.device.name,
        kind: "account" as const,
        active: this.targetId === o.device.id,
        offline: !o.online,
        platform: o.device.platform.toLowerCase(),
        playing: o.online && o.state.playing,
        track: o.state.track ? `${o.state.track.artist} · ${o.state.track.title}` : null,
        busy: !o.state.available,
        connecting: this.transferToId === o.device.id,
        error: this.errorAt === o.device.id ? this.error : null,
      }));
  }

  sync() {
    if (!this.host.isSignedIn()) {
      this.clearTimer();
      const had = this.outputs.length > 0 || this.targetId !== null;
      this.outputs = [];
      this.targetId = null;
      this.transferToId = null;
      this.error = null;
      this.errorAt = null;
      this.playingHint = null;
      this.optimisticPos = null;
      this.optimisticVol = null;
      this.missingSince = null;
      this.failures = 0;
      if (had) {
        this.host.emit("devices", []);
        this.host.emit("player", { ...this.host.getLocalPlayer() });
      }
      return;
    }
    this.schedulePoll(100);
  }

  setPanelOpen(open: boolean) {
    this.panelOpen = open;
    if (open && this.host.isSignedIn()) {
      this.schedulePoll(0);
    }
  }

  async rescan(): Promise<DeviceX[]> {
    if (this.host.isSignedIn()) {
      await this.pollOnce();
    }
    return this.devicesJson();
  }

  async connect(id: string): Promise<DeviceX[]> {
    if (!this.host.isSignedIn()) return this.devicesJson();
    if (this.transferToId || this.targetId === id) return this.devicesJson();
    const o = this.outputs.find((x) => x.device.id === id);
    if (!o || o.device.current || !o.online || !o.state.available) return this.devicesJson();

    const theirs = o.state.playing && o.state.track != null;
    const localCur = this.host.getLocalPlayer().current;
    const localPlaying = this.host.isLocalPlaying();
    if ((!this.targetId && !localCur) || (theirs && !localPlaying)) {
      this.selectOutput(id);
    } else {
      await this.transferOutput(id);
    }
    return this.devicesJson();
  }

  useLocal(): DeviceX[] {
    if (!this.targetId) return this.devicesJson();
    const id = this.targetId;
    const o = this.outputs.find((x) => x.device.id === id) ?? null;
    const track = o?.state.track ?? null;
    const posSec = o ? this.outputPositionMs(o) / 1000 : 0;
    const playing = o ? this.effectivePlaying(o) : false;

    void this.fireCommand(id, { action: "stop" });
    this.targetId = null;
    this.missingSince = null;
    this.playingHint = null;
    this.optimisticPos = null;
    this.optimisticVol = null;
    this.error = null;
    this.errorAt = null;
    this.emitDevices();

    if (track) {
      this.host.onResumeLocal(track, posSec, playing);
    } else {
      this.host.emit("player", { ...this.host.getLocalPlayer() });
    }
    this.schedulePoll(300);
    return this.devicesJson();
  }

  sendCommand(cmd: PlaybackCommandWire): boolean {
    if (!this.targetId) return false;
    const target = this.targetId;
    const now = Date.now();
    if (cmd.action === "play") this.playingHint = { playing: true, at: now };
    else if (cmd.action === "pause" || cmd.action === "stop") this.playingHint = { playing: false, at: now };
    else if (cmd.action === "seek") this.optimisticPos = { ms: cmd.position_ms, at: now };
    else if (cmd.action === "volume") this.optimisticVol = { volume: cmd.volume, at: now };

    this.emitRemotePlayer();
    void this.fireCommand(target, cmd).then(() => {
      this.schedulePoll(350);
    });
    return true;
  }

  transferTracks(tracks: Track[], index: number): boolean {
    if (!this.targetId) return false;
    const target = this.targetId;
    const list = tracks.slice(index, index + 100);
    if (!list.length) return false;
    void this.fireCommand(target, {
      action: "transfer",
      tracks: list.map(wireTrack),
      index: 0,
      position_ms: 0,
      playing: true,
    }).then(() => {
      this.schedulePoll(350);
    });
    return true;
  }

  remoteVolume(): number | null {
    if (!this.targetId) return null;
    if (this.optimisticVol && Date.now() - this.optimisticVol.at < OPTIMISTIC_MS) {
      return this.optimisticVol.volume;
    }
    const o = this.outputs.find((x) => x.device.id === this.targetId);
    return o ? o.state.volume : null;
  }

  private clearTimer() {
    if (this.pollTimer != null) {
      clearTimeout(this.pollTimer);
      this.pollTimer = null;
    }
  }

  private nextIntervalMs(): number {
    if (this.transferToId) return 2_000;
    if (this.panelOpen || this.targetId) return 2_000;
    const hasPeers = this.outputs.some((o) => !o.device.current);
    if (!hasPeers && this.stateAt > 0) return 30_000;
    if (this.host.isLocalPlaying()) return 5_000;
    return 15_000;
  }

  private schedulePoll(delayMs = this.nextIntervalMs()) {
    if (!this.host.isSignedIn()) return;
    this.clearTimer();
    this.pollTimer = setTimeout(() => {
      this.pollTimer = null;
      void this.pollOnce();
    }, delayMs);
  }

  private ownState(): PlaybackStateWire {
    const controlling = this.targetId !== null;
    const p = this.host.getLocalPlayer();
    const playing = !controlling && (p.status === "playing" || p.status === "loading");
    return {
      acknowledgements: true,
      track: controlling ? null : (p.current ? wireTrack(p.current) : null),
      playing,
      position_ms: controlling ? 0 : Math.max(0, Math.round((p.position || 0) * 1000)),
      duration_ms: controlling ? null : (p.duration != null ? Math.round(p.duration * 1000) : null),
      volume: Math.max(0, Math.min(1, p.volume ?? 0.8)),
      available: !controlling,
    };
  }

  private effectivePlaying(o: PlaybackOutputWire): boolean {
    if (this.playingHint && Date.now() - this.playingHint.at < OPTIMISTIC_MS) {
      return this.playingHint.playing;
    }
    return o.state.playing;
  }

  private outputPositionMs(o: PlaybackOutputWire): number {
    if (this.optimisticPos && Date.now() - this.optimisticPos.at < OPTIMISTIC_MS) {
      return this.optimisticPos.ms;
    }
    const playing = this.effectivePlaying(o);
    const elapsed = playing && this.stateAt > 0 ? Math.min(POSITION_RUN_ON_MS, Math.max(0, Date.now() - this.stateAt)) : 0;
    const raw = (o.state.position_ms ?? 0) + elapsed;
    return o.state.duration_ms != null ? Math.min(raw, o.state.duration_ms) : raw;
  }

  remotePlayerDto(): Partial<PlayerState> | null {
    if (!this.targetId) return null;
    const o = this.outputs.find((x) => x.device.id === this.targetId);
    if (!o) return null;
    const playing = this.effectivePlaying(o);
    const vol = this.remoteVolume() ?? o.state.volume;
    return {
      status: playing ? "playing" : "paused",
      current: o.state.track,
      position: this.outputPositionMs(o) / 1000,
      duration: o.state.duration_ms != null ? o.state.duration_ms / 1000 : null,
      volume: vol,
      muted: vol <= 0.001,
      source: o.state.track?.source ?? null,
      preview: false,
      error: this.error,
    };
  }

  private emitDevices() {
    this.host.emit("devices", this.devicesJson());
  }

  private emitRemotePlayer() {
    const dto = this.remotePlayerDto();
    if (dto) this.host.emit("player", dto);
  }

  private selectOutput(id: string) {
    const o = this.outputs.find((x) => x.device.id === id);
    if (!o || o.device.current || !o.online || !o.state.available) return;
    this.targetId = id;
    this.error = null;
    this.errorAt = null;
    this.missingSince = null;
    this.playingHint = null;
    this.optimisticPos = null;
    this.optimisticVol = null;
    this.host.pauseLocal();
    this.emitRemotePlayer();
    this.emitDevices();
    this.schedulePoll(200);
  }

  private async transferOutput(id: string) {
    const o = this.outputs.find((x) => x.device.id === id);
    if (!o) return;
    const previous = this.targetId;
    let tracks: Track[] = [];
    let positionMs = 0;
    let playing = false;

    if (previous) {
      const prevOut = this.outputs.find((x) => x.device.id === previous);
      if (prevOut?.state.track) {
        tracks = [prevOut.state.track];
        positionMs = Math.round(this.outputPositionMs(prevOut));
        playing = this.effectivePlaying(prevOut);
      }
    } else {
      const lp = this.host.getLocalPlayer();
      if (lp.current) {
        tracks = [lp.current, ...this.host.getUpcoming(99)];
        positionMs = Math.max(0, Math.round((lp.position || 0) * 1000));
        playing = this.host.isLocalPlaying();
      }
    }

    if (!tracks.length) {
      this.selectOutput(id);
      return;
    }

    this.transferToId = id;
    this.error = null;
    this.errorAt = null;
    this.emitDevices();

    try {
      const confirm = Boolean(o.state.acknowledgements);
      await this.host.withToken(async (token) => {
        const res = await server.playbackCommand(
          token,
          {
            target: id,
            command: {
              action: "transfer",
              tracks: tracks.map(wireTrack),
              index: 0,
              position_ms: positionMs,
              playing,
            },
          },
          confirm,
        );
        if (confirm && res?.id) {
          const deadline = Date.now() + 18_000;
          while (Date.now() < deadline) {
            await new Promise((r) => setTimeout(r, 350));
            const rc = await server.playbackReceipt(token, res.id).catch(() => null);
            if (rc?.done) {
              if (rc.error) throw new Error(rc.error);
              return;
            }
          }
          throw new Error("Transfer confirmation timed out");
        }
      });

      this.transferToId = null;
      if (previous && previous !== id) {
        void this.fireCommand(previous, { action: "stop" });
      }
      // Ensure target shows online/available in local cache before selecting
      o.online = true;
      o.state.available = true;
      o.state.track = tracks[0];
      o.state.playing = playing;
      o.state.position_ms = positionMs;
      this.stateAt = Date.now();
      this.selectOutput(id);
    } catch (e) {
      this.transferToId = null;
      this.error = String((e as Error)?.message ?? e);
      this.errorAt = id;
      this.emitDevices();
    }
  }

  private async fireCommand(target: string, cmd: PlaybackCommandWire) {
    try {
      await this.host.withToken((token) =>
        server.playbackCommand(token, { target, command: cmd }, false),
      );
    } catch (e) {
      if (this.targetId === target) {
        this.playingHint = null;
        this.error = String((e as Error)?.message ?? e);
        this.errorAt = target;
        this.emitRemotePlayer();
        this.emitDevices();
      }
    }
  }

  private async ack(id: string, error: string | null) {
    this.seenAck.set(id, error);
    if (this.seenAck.size > 64) {
      const first = this.seenAck.keys().next().value;
      if (first !== undefined) this.seenAck.delete(first);
    }
    try {
      await this.host.withToken((token) =>
        server.playbackAck(token, { id, done: true, error }),
      );
    } catch { /* best-effort ack */ }
  }

  private async pollOnce() {
    if (!this.host.isSignedIn()) return;
    if (this.polling) {
      this.pollQueued = true;
      return;
    }
    this.polling = true;
    try {
      const poll = await this.host.withToken((token) =>
        server.playbackPoll(token, this.ownState()),
      );
      if (!this.host.isSignedIn()) return;

      if (this.failures >= POLL_FAILURES) {
        this.error = null;
      }
      this.failures = 0;

      const devices = poll.devices ?? [];
      if (this.targetId) {
        const nextOut = devices.find((o) => o.device.id === this.targetId);
        const oldOut = this.outputs.find((o) => o.device.id === this.targetId);
        if (nextOut && !nextOut.online && oldOut) {
          nextOut.state = oldOut.state;
        }
      }
      this.outputs = devices;
      this.stateAt = Date.now();

      if (this.targetId) {
        const cur = devices.find((o) => o.device.id === this.targetId);
        if (cur && cur.online) {
          this.missingSince = null;
          if (this.playingHint && this.playingHint.playing === cur.state.playing) {
            this.playingHint = null;
          }
          if (this.optimisticPos && Math.abs(this.optimisticPos.ms - cur.state.position_ms) < 2500) {
            this.optimisticPos = null;
          }
          if (this.optimisticVol && Math.abs(this.optimisticVol.volume - cur.state.volume) < 0.02) {
            this.optimisticVol = null;
          }
          this.emitRemotePlayer();
        } else {
          this.missingSince ??= Date.now();
          if (!cur || Date.now() - this.missingSince > OUTPUT_GRACE_MS) {
            this.targetId = null;
            this.missingSince = null;
            this.playingHint = null;
            this.optimisticPos = null;
            this.optimisticVol = null;
            this.error = t("connect.account.offline");
            this.errorAt = null;
            this.host.emit("player", { ...this.host.getLocalPlayer(), error: this.error });
          }
        }
      }

      this.emitDevices();

      const commands = poll.commands ?? [];
      for (const cmd of commands) {
        if (!this.targetId) {
          await this.host.onApplyCommand(cmd);
        }
      }
      const deliveries = poll.deliveries ?? [];
      for (const d of deliveries) {
        if (this.seenAck.has(d.id)) {
          await this.ack(d.id, this.seenAck.get(d.id) ?? null);
          continue;
        }
        if (this.targetId) {
          await this.ack(d.id, "Currently controlling another device");
          continue;
        }
        let err: string | null = null;
        try {
          await this.host.onApplyCommand(d.command);
        } catch (e) {
          err = String((e as Error)?.message ?? e);
        }
        await this.ack(d.id, err);
      }
    } catch (e) {
      this.failures++;
      if (this.failures >= POLL_FAILURES) {
        this.error = String((e as Error)?.message ?? e);
        this.errorAt = null;
        this.outputs = [];
        this.emitDevices();
      }
    } finally {
      this.polling = false;
      if (this.pollQueued) {
        this.pollQueued = false;
        this.schedulePoll(100);
      } else {
        this.schedulePoll();
      }
    }
  }
}
