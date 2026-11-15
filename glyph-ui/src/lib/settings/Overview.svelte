<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { onWeb } from "../web";
  import { backend } from "../api/backend";
  import { LANGS, t } from "../i18n.svelte";
  import { allThemes, css } from "../theme";
  import { tip } from "../ui.svelte";
  import Icon from "../components/Icon.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import Tog from "./Tog.svelte";
  import Select from "./Select.svelte";
  import Segmented from "./Segmented.svelte";
  import SectionLabel from "./SectionLabel.svelte";
  import { setTab } from "./settings.svelte";
  import { loadPlugins, pl } from "./plugins-state.svelte";
  import { eqPresetLabel } from "./eq";
  import { isMobile } from "./util";

  let {}: { sub?: number } = $props();

  const web = onWeb();
  const touch = $derived(isMobile());
  const au = $derived(app.config.audio);
  const v = $derived(app.config.visuals);
  const pv = $derived(app.config.providers);
  const st = $derived(app.config.storage);

  let outputs = $state<string[]>([]);
  let radio = $state<{ on: boolean; ok: boolean | null }>({ on: false, ok: null });
  onMount(() => {
    backend().call<typeof radio>("radio_state").then((r) => r && (radio = r));
    if (!web) {
      if (!pl.loaded) loadPlugins();
      backend().call<string[]>("output_devices").then((l) => l && (outputs = l));
    }
  });
  function setRadio(on: boolean) { radio.on = on; backend().call("set_radio", { on }); }

  const themes = $derived((void pl.installed, allThemes()));
  const curTheme = $derived(themes.some((th) => th.id === v.theme) ? v.theme : "glyph");

  const langOptions = LANGS.map(([code, name]) => ({ value: code as string, label: name }));
  const lang = $derived(LANGS.find(([c]) => c === (app.config.language || ""))?.[0] ?? LANGS.find(([c]) => c === document.documentElement.lang)?.[0] ?? "en");

  const scale = $derived(Math.min(1.3, Math.max(0.9, app.config.ui.text_scale ?? 1)));
  const scaleBucket = $derived<"compact" | "standard" | "enlarged">(
    scale < 0.975 ? "compact" : scale <= 1.05 ? "standard" : "enlarged",
  );
  const scaleOptions = $derived([
    { value: "compact", label: t("sp.appearance.text.compact") },
    { value: "standard", label: t("sp.appearance.text.standard") },
    { value: "enlarged", label: t("sp.appearance.text.enlarged") },
  ]);
  const SCALE_VALS = { compact: 0.9, standard: 1.0, enlarged: 1.15 } as const;

  const qOptions = $derived(["auto", "low", "standard", "high"].map((q) => ({ value: q, label: t(`sp.audio.q.${q}`) })));
  const outputOptions = $derived.by(() => {
    const names = [...new Set([...(outputs.length ? outputs : app.outputDevices), ...(au.output_device ? [au.output_device] : [])])];
    return [{ value: null as string | null, label: t("sp.output.default") }, ...names.map((n) => ({ value: n as string | null, label: n }))];
  });
  const shuffleOptions: { value: "smart" | "random"; label: string }[] = $derived([
    { value: "smart", label: t("sp.shuffle.smart") },
    { value: "random", label: t("sp.shuffle.random") },
  ]);

  const embed = $derived(web ? pv.enabled.includes("youtube") : (pv.enabled.includes("youtube") || pv.enabled.includes("soundcloud")));
  function toggleEmbed(on: boolean) {
    app.setConfig((c) => {
      c.providers.enabled = c.providers.enabled.filter((p) => p !== "youtube" && p !== "soundcloud");
      if (on) {
        if (web) c.providers.enabled.push("youtube");
        else c.providers.enabled.push("soundcloud", "youtube");
      }
    });
  }
</script>

<div class="col">
  <div class="cards" class:web>
    <button class="qcard jump" onclick={() => setTab("account")}>
      <div class="qi"><Icon name="user-round" size={16} color={app.account ? "var(--accent)" : "rgba(255,255,255,.72)"} /></div>
      <div class="qt">
        <span class="ql">{t("sp.account")}</span>
        <span class="qs trunc">{app.account ? `@${app.account.handle}` : t("sp.overview.signed_out")}</span>
      </div>
      <Icon name="chevron-right" size={14} color="rgba(255,255,255,.35)" />
    </button>

    {#if !web}
      <div class="qcard">
        <button class="qmain" onclick={() => setTab("audio")}>
          <div class="qi"><Icon name="volume-2" size={16} color={au.eq_enabled ? "var(--accent)" : "rgba(255,255,255,.72)"} /></div>
          <div class="qt">
            <span class="ql">{t("sp.eq.enable")}</span>
            <span class="qs trunc">{au.eq_enabled ? eqPresetLabel(au.eq_preset) : t("common.off")}</span>
          </div>
        </button>
        <Tog on={au.eq_enabled} onchange={(x) => app.setConfig((c) => { c.audio.eq_enabled = x; })} label={t("sp.eq.enable")} />
      </div>

      <div class="qcard">
        <button class="qmain" onclick={() => setTab("storage")}>
          <div class="qi"><Icon name="chart-column" size={16} color={st.data_saver ? "var(--accent)" : "rgba(255,255,255,.72)"} /></div>
          <div class="qt">
            <span class="ql">{t("sp.storage.saver")}</span>
            <span class="qs trunc">{st.data_saver ? t("common.on") : t("common.off")}</span>
          </div>
        </button>
        <Tog on={st.data_saver} onchange={(x) => app.setConfig((c) => { c.storage.data_saver = x; })} label={t("sp.storage.saver")} />
      </div>
    {/if}
  </div>

  <div class="sec">
    <SectionLabel text={t("sp.overview.appearance")} />
    <Rows>
      <Row label={t("sp.appearance.theme")} info={t("sp.appearance.theme.desc")}>
        <div class="sw">
          {#each themes as th (th.id)}
            <button
              class="th"
              class:on={th.id === curTheme}
              use:tip={t(th.name)}
              aria-label={t(th.name)}
              aria-pressed={th.id === curTheme}
              onclick={() => app.setConfig((c) => { c.visuals.theme = th.id; })}
            >
              <span class="ring" style:background={css(th.glow ? th.glow[1] : 0x1d5cff)}>
                <span class="in" style:background={css(th.accent)}></span>
              </span>
            </button>
          {/each}
        </div>
      </Row>
      <Row label={t("sp.appearance.text")} info={t("sp.appearance.text.desc")}>
        <Segmented
          value={scaleBucket}
          options={scaleOptions}
          onpick={(b) => app.setConfig((c) => { c.ui.text_scale = SCALE_VALS[b as keyof typeof SCALE_VALS]; })}
        />
      </Row>
      <Row label={t("settings.language")} info={t("sp.appearance.lang.desc")}>
        <Select value={lang} options={langOptions} onpick={(c) => app.setConfig((cfg) => { cfg.language = c; })} />
      </Row>
    </Rows>
  </div>

  <div class="sec">
    <SectionLabel text={t("sp.overview.playback")} />
    <Rows>
      {#if !web}
        <Row label={t("sp.audio.quality")} info={t("sp.audio.quality.desc")}>
          <Select value={au.quality} options={qOptions} onpick={(v) => app.setConfig((c) => { c.audio.quality = v; })} />
        </Row>
        <Row label={t("sp.output")} info={t("sp.output.desc")}>
          <Select value={au.output_device} options={outputOptions} label={au.output_device ?? t("sp.output.default")} onpick={(v) => app.setConfig((c) => { c.audio.output_device = v; })} />
        </Row>
        <Row label={t("sp.normalize")} info={t("sp.normalize.desc")}>
          <Tog on={au.normalize} onchange={(x) => app.setConfig((c) => { c.audio.normalize = x; })} label={t("sp.normalize")} />
        </Row>
        {#if !touch}
          <Row label={t("sp.extractor")} info={t("sp.extractor.desc")}>
            <Segmented
              value={pv.extractor === "yt_dlp"}
              options={[{ value: false, label: "r-dlp" }, { value: true, label: "yt-dlp" }]}
              onpick={(yt) => app.setConfig((c) => { c.providers.extractor = yt ? "yt_dlp" : "r_dlp"; })}
            />
          </Row>
        {/if}
      {:else}
        <Row label={t("settings.sources.fallback.web")} info={t("settings.sources.fallback.web.hint")}>
          <Tog on={embed} onchange={toggleEmbed} label={t("settings.sources.fallback.web")} />
        </Row>
      {/if}
      <Row label={t("sp.shuffle")} info={t("sp.shuffle.desc")}>
        <Segmented
          value={app.config.playback.shuffle === "random" ? "random" : "smart"}
          options={shuffleOptions}
          onpick={(s) => app.setConfig((c) => { c.playback.shuffle = s; })}
        />
      </Row>
      <Row label={t("sp.radio")} info={radio.ok === false ? t("radio.off") : t("sp.radio.desc")}>
        <Tog on={radio.on} disabled={radio.ok === false} why={t("radio.off")} onchange={setRadio} label={t("sp.radio")} />
      </Row>
      <Row label={t("sp.explicit")} info={t("sp.explicit.desc")}>
        <Tog on={au.hide_explicit} onchange={(x) => app.setConfig((c) => { c.audio.hide_explicit = x; })} label={t("sp.explicit")} />
      </Row>
    </Rows>
  </div>
</div>

<style>
  .col { display: flex; flex-direction: column; gap: 18px; }
  .sec { display: flex; flex-direction: column; gap: 6px; }

  .cards {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 10px;
  }
  .cards.web { grid-template-columns: minmax(0, 1fr); }

  .qcard {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px;
    border-radius: 10px;
    background: rgba(255, 255, 255, .04);
    transition: background-color .15s ease-out;
  }
  .qcard.jump:hover { background: rgba(255, 255, 255, .07); }
  .qmain {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    text-align: left;
    border-radius: 6px;
  }
  .qmain:hover .ql { color: #fff; }
  .qi {
    flex: none;
    width: 32px;
    height: 32px;
    border-radius: 8px;
    background: rgba(255, 255, 255, .05);
    display: grid;
    place-items: center;
  }
  .qt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; text-align: left; }
  .ql { font-size: 13.5px; font-weight: 500; color: rgba(255, 255, 255, .92); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .qs { font-size: 12px; color: rgba(255, 255, 255, .52); }

  .sw { display: flex; flex-wrap: wrap; justify-content: flex-end; align-items: center; gap: 5px; }
  .th { flex: none; width: 26px; height: 26px; border-radius: 50%; display: grid; place-items: center; border: 2px solid transparent; transition: border-color .15s ease-out, transform .15s var(--ease); }
  .th:hover { transform: scale(1.08); }
  .th.on { border-color: #fff; }
  .ring { width: 18px; height: 18px; border-radius: 50%; display: grid; place-items: center; }
  .in { width: 10px; height: 10px; border-radius: 50%; }

  @media (max-width: 680px) {
    .cards { grid-template-columns: 1fr; }
  }
</style>
