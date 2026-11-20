<script lang="ts">
  // settings/eq.rs equalizer: on / off, preset (built-in and own), bass,
  // headroom (thin bars 180 px + mono value 12 px, white 60 %, w 56; 45 %
  // while off), share (copy / paste code, export / import file), then the
  // curve, the hint (12 px, white 45 %) and Reset / Delete (own presets) /
  // Save (primary). Gap 18.
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t } from "../i18n.svelte";
  import Button from "../components/Button.svelte";
  import ThinBar from "../components/ThinBar.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import Tog from "./Tog.svelte";
  import Select from "./Select.svelte";
  import EqCurve from "./EqCurve.svelte";
  import PluginHost from "./PluginHost.svelte";
  import { EQ_PRESETS, addEqPreset, currentEqPreset, dbLabel, deleteEqPreset, eqGains, eqPresetLabel, eqToast, setEq, setEqExtras } from "./eq";
  import { copyText, errText, pickFiles, readClipboard, saveText } from "./util";
  import type { EqPreset } from "../api/types";
  let {}: { sub?: number } = $props();

  const a = $derived(app.config.audio);
  const on = $derived(a.eq_enabled);
  const preset = $derived(a.eq_preset);
  const own = $derived(preset.startsWith("user:") ? preset.slice(5) : null);
  const presetOptions = $derived([
    ...EQ_PRESETS.map(([k]) => ({ value: k, label: eqPresetLabel(k) })),
    ...a.eq_user_presets.map((p) => ({ value: `user:${p.name}`, label: p.name })),
    ...(preset === "custom" ? [{ value: "custom", label: eqPresetLabel("custom") }] : []),
  ]);
  const share = $derived([
    { value: "copy_code", label: t("sp.eq.copy_code") }, { value: "paste_code", label: t("sp.eq.paste_code") },
    { value: "export_file", label: t("sp.eq.export_file") }, { value: "import_file", label: t("sp.eq.import_file") },
  ]);

  function pick(key: string) {
    if (key === "custom") return;
    const builtin = EQ_PRESETS.find(([k]) => k === key);
    const g = builtin ? builtin[1] : a.eq_user_presets.find((p) => `user:${p.name}` === key)?.gains;
    if (g) setEq(true, key, eqGains(g));
  }

  async function importText(text: string): Promise<boolean> {
    if (!text.trim()) return false;
    try {
      const p = await backend().call<EqPreset | null>("eq_import", { text });
      if (!p) return false;
      addEqPreset(p);
      eqToast("toast.eq_imported", p.name);
      return true;
    } catch (e) { app.toastInfo(errText(e)); return true; }
  }

  // An equalizer code pasted into the search field is imported here.
  $effect(() => {
    const text = app.pendingEqCode;
    if (text) { app.pendingEqCode = null; void importText(text); }
  });

  async function doShare(what: string) {
    const p = currentEqPreset();
    if (what === "copy_code") {
      const code = await backend().call<string>("eq_code", { name: p.name, gains: p.gains });
      if (code && (await copyText(code))) app.copied(`${t("toast.eq_copied")} · ${p.name}`);
    } else if (what === "paste_code") {
      if (!(await importText(await readClipboard()))) app.toastInfo(t("toast.eq_no_code"));
    } else if (what === "export_file") {
      const file = await backend().call<{ name: string; text: string }>("eq_file", { name: p.name, gains: p.gains });
      if (file) { saveText(file.name, file.text); eqToast("toast.eq_exported", p.name); }
    } else {
      const [f] = await pickFiles(".glyph-eq,.json,.txt");
      if (f && !(await importText(f.text))) app.toastInfo(t("toast.eq_no_code"));
    }
  }

  function reset() { setEq(on, "flat", Array(10).fill(0)); setEqExtras(0, 0); }
  function save() { app.nameDialog = { tracks: [], eqPreset: true, initial: own ?? "" }; }
</script>

<PluginHost page="audio">
  <div class="col">
    <Rows>
      <Row label={t("sp.eq.enable")} info={t("sp.eq.enable.desc")}>
        <Tog on={on} onchange={(v) => setEq(v, preset, eqGains(a.eq_gains))} label={t("sp.eq.enable")} />
      </Row>
      <Row label={t("sp.eq.preset")} info={t("sp.eq.preset.desc")}>
        <Select value={preset} options={presetOptions} label={eqPresetLabel(preset)} onpick={pick} />
      </Row>
      <Row label={t("sp.eq.bass")} info={t("sp.eq.bass.desc")}>
        <div class="bar" class:dim={!on}>
          <ThinBar width={180} value={(a.eq_bass_db + 12) / 24} step={0.5 / 24} live={false} label={(f) => dbLabel(Math.round((f * 24 - 12) * 2) / 2)}
            onpick={(f) => setEqExtras(f * 24 - 12, a.eq_headroom_db)} />
          <span class="mono val">{dbLabel(a.eq_bass_db)}</span>
        </div>
      </Row>
      <Row label={t("sp.eq.headroom")} info={t("sp.eq.headroom.desc")}>
        <div class="bar" class:dim={!on}>
          <ThinBar width={180} value={a.eq_headroom_db / 12} step={0.5 / 12} live={false} label={(f) => dbLabel(-Math.round(f * 12 * 2) / 2)}
            onpick={(f) => setEqExtras(a.eq_bass_db, f * 12)} />
          <span class="mono val">{dbLabel(-a.eq_headroom_db)}</span>
        </div>
      </Row>
      <Row label={t("sp.eq.share")} info={t("sp.eq.share.desc")}>
        <Select value="" options={share} label={t("sp.eq.share.button")} onpick={doShare} />
      </Row>
    </Rows>
    <EqCurve {on} />
    <div class="foot">
      <div class="hint">{t("sp.eq.hint")}</div>
      <div class="btns">
        <Button onclick={reset}>{t("sp.eq.reset")}</Button>
        {#if own}<Button onclick={() => deleteEqPreset(own)}>{t("sp.eq.delete")}</Button>{/if}
        <Button kind="primary" onclick={save}>{t("sp.eq.save")}</Button>
      </div>
    </div>
  </div>
</PluginHost>

<style>
  .col { display: flex; flex-direction: column; gap: 18px; }
  .bar { display: flex; align-items: center; gap: 12px; transition: opacity .2s ease-out; }
  .bar.dim { opacity: .45; }
  .val { width: 56px; display: flex; justify-content: flex-end; font-size: 12px; color: rgba(255, 255, 255, .6); }
  .foot { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px; }
  .hint { font-size: 12px; color: rgba(255, 255, 255, .45); }
  .btns { display: flex; gap: 8px; }
</style>
