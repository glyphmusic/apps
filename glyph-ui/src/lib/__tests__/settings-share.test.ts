import { describe, expect, it } from "vitest";
import { webConfig } from "../api/web";
import {
  SETTINGS_PREFIX,
  applySharedSettings,
  decodeSettings,
  encodeSettings,
  isSettingsCode,
  settingsToJson,
} from "../api/settings-share";

describe("settings-share (web)", () => {
  it("round-trips a settings code without leaking device-bound values", async () => {
    const src = webConfig();
    src.language = "de";
    src.visuals.theme = "ocean";
    src.visuals.ascii_covers = true;
    src.stats.enabled = true;
    src.audio.hide_explicit = true;
    src.audio.eq_user_presets = [{ name: "Warm", gains: [3, 2, 1, 0, 0, -1, -1, 0, 1, 2] }];
    src.server_url = "https://private.example";
    src.player.volume = 0.25;

    const code = await encodeSettings(src, "0.12.16");
    expect(isSettingsCode(code)).toBe(true);
    expect(code.startsWith(SETTINGS_PREFIX)).toBe(true);

    const sections = await decodeSettings(code);
    const dst = webConfig();
    const changed = applySharedSettings(dst, sections);

    expect(changed).toBe(true);
    expect(dst.language).toBe("de");
    expect(dst.visuals.theme).toBe("ocean");
    expect(dst.visuals.ascii_covers).toBe(true);
    expect(dst.stats.enabled).toBe(true);
    expect(dst.audio.hide_explicit).toBe(true);
    expect(dst.audio.eq_user_presets).toEqual([{ name: "Warm", gains: [3, 2, 1, 0, 0, -1, -1, 0, 1, 2] }]);
    // Device-bound fields stay untouched.
    expect(dst.server_url).toBe("");
    expect(dst.player.volume).toBe(0.8);

    // Re-applying identical settings reports no change.
    expect(applySharedSettings(dst, sections)).toBe(false);
  });

  it("round-trips a JSON settings export file", async () => {
    const src = webConfig();
    src.visuals.theme = "forest";
    src.playback.shuffle = "random";

    const json = settingsToJson(src, "0.12.16");
    const sections = await decodeSettings(json);
    const dst = webConfig();

    expect(applySharedSettings(dst, sections)).toBe(true);
    expect(dst.visuals.theme).toBe("forest");
    expect(dst.playback.shuffle).toBe("random");
  });

  it("rejects broken or unsupported settings inputs", async () => {
    await expect(decodeSettings("not-json")).rejects.toThrow(/not Glyph settings/);
    await expect(decodeSettings(`${SETTINGS_PREFIX}!!!invalid!!!`)).rejects.toThrow(/broken settings code/);
    await expect(decodeSettings(JSON.stringify({ glyph_settings: 99, sections: {} }))).rejects.toThrow(/newer Glyph/);
  });
});
