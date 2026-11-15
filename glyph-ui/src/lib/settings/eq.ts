// Equalizer presets and helpers (settings.rs EQ_PRESETS, eq_preset_label,
// eq_match; app/settings.rs set_eq, set_eq_extras, add_eq_preset,
// delete_eq_preset).
import { app } from "../state.svelte";
import { backend } from "../api/backend";
import { t } from "../i18n.svelte";
import type { EqPreset } from "../api/types";

/** Equalizer presets: name key and gains (dB) for 32 Hz .. 16 kHz. */
export const EQ_PRESETS: [string, number[]][] = [
  ["flat", [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
  ["bass_boost", [6, 5, 4, 2.5, 1, 0, 0, 0, 0, 0]],
  ["bass_reducer", [-6, -5, -4, -2.5, -1, 0, 0, 0, 0, 0]],
  ["treble_boost", [0, 0, 0, 0, 0, 1, 2.5, 4, 5, 6]],
  ["treble_reducer", [0, 0, 0, 0, 0, -1, -2.5, -4, -5, -6]],
  ["vocal", [-2, -2, -1, 1, 3, 4, 3.5, 2, 0, -1]],
  ["rock", [4, 3, 2, 0.5, -1, -1, 0.5, 2, 3, 4]],
  ["pop", [-1, 1, 2.5, 3.5, 2, 0, -0.5, -0.5, 1, 1.5]],
  ["hiphop", [5, 4.5, 2, 3, -1, -1, 1, -0.5, 1.5, 2.5]],
  ["electronic", [4.5, 4, 1.5, 0, -2, 2, 1, 1.5, 4, 5]],
  ["acoustic", [4, 4, 3, 1, 2, 1.5, 3, 3.5, 3, 2]],
  ["classical", [4.5, 3.5, 3, 2.5, -1.5, -1.5, 0, 2, 3, 3.5]],
  ["jazz", [3.5, 2.5, 1.5, 2, -1.5, -1.5, 0, 1.5, 2.5, 3]],
  ["small_speakers", [5, 4, 3.5, 2.5, 1, 0, -1, -2, -2.5, -3]],
];

/** Display name of a preset key: built-in (translated), own (`user:<name>`) or "custom". */
export function eqPresetLabel(key: string): string {
  return key.startsWith("user:") ? key.slice(5) : t(`eq.preset.${key}`);
}

/** Always ten bands. */
export function eqGains(g: number[]): number[] {
  return Array.from({ length: 10 }, (_, i) => g[i] ?? 0);
}

const same = (a: number[], b: number[]) => a.length === b.length && a.every((v, i) => Math.abs(v - b[i]) < 0.01);

/** Preset key for a curve: the current preset while it still matches, else
 *  the first built-in or own preset with exactly these gains, else "custom". */
export function eqMatch(current: string, gains: number[], user: EqPreset[]): string {
  const all: [string, number[]][] = [...EQ_PRESETS, ...user.map((p): [string, number[]] => [`user:${p.name}`, eqGains(p.gains)])];
  if (all.some(([k, g]) => k === current && same(g, gains))) return current;
  return all.find(([, g]) => same(g, gains))?.[0] ?? "custom";
}

/** Live change while a band is dragged: the audio engine hears it at once, the config is written on release. */
export function setEqLive(enabled: boolean, preset: string, gains: number[]) {
  const a = app.config.audio;
  a.eq_enabled = enabled; a.eq_preset = preset; a.eq_gains = gains;
  backend().call("set_eq_live", { enabled, gains });
}

export function setEq(enabled: boolean, preset: string, gains: number[]) {
  app.setConfig((c) => { c.audio.eq_enabled = enabled; c.audio.eq_preset = preset; c.audio.eq_gains = [...gains]; });
}

/** Bass shelf and headroom (dB), rounded to half dB. */
export function setEqExtras(bass: number, headroom: number) {
  const r = (v: number) => Math.round(v * 2) / 2;
  app.setConfig((c) => { c.audio.eq_bass_db = r(Math.min(12, Math.max(-12, bass))); c.audio.eq_headroom_db = r(Math.min(12, Math.max(0, headroom))); });
}

/** Adds an own preset (one with the same name is replaced) and selects it. */
export function addEqPreset(p: EqPreset) {
  const gains = eqGains(p.gains);
  app.setConfig((c) => {
    const list = c.audio.eq_user_presets;
    const i = list.findIndex((e) => e.name.toLowerCase() === p.name.toLowerCase());
    if (i >= 0) list[i] = { name: p.name, gains }; else list.push({ name: p.name, gains });
    c.audio.eq_enabled = true; c.audio.eq_preset = `user:${p.name}`; c.audio.eq_gains = [...gains];
  });
}

/** Removes an own preset; the curve stays as it is. */
export function deleteEqPreset(name: string) {
  app.setConfig((c) => {
    c.audio.eq_user_presets = c.audio.eq_user_presets.filter((p) => p.name !== name);
    if (c.audio.eq_preset === `user:${name}`) c.audio.eq_preset = "custom";
  });
  app.toast(`${t("toast.eq_deleted")} · ${name}`);
}

/** The curve as it is now, named after its preset (for sharing). */
export function currentEqPreset(): EqPreset {
  return { name: eqPresetLabel(app.config.audio.eq_preset), gains: eqGains(app.config.audio.eq_gains) };
}

export function eqToast(key: string, sub: string) { app.toast(`${t(key)} · ${sub}`); }
export const dbLabel = (db: number) => (Math.abs(db) < 0.05 ? "0 dB" : `${db > 0 ? "+" : ""}${db.toFixed(1)} dB`);
