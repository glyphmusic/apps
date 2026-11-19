<script lang="ts">
  // settings/sources.rs content: Playback > Content. Shuffle style, skip
  // unplayable, patient search, statistics, explicit, radio (off + tip when
  // the server has no Last.fm key) and, under a hairline, the artists hidden
  // with "Don't show this artist" as chips (h 28, pl 10, pr 6, radius 8,
  // white 6 %, border white 8 %, 12.5 px) with an eye to show them again.
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import Icon from "../components/Icon.svelte";
  import { tip } from "../ui.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import Tog from "./Tog.svelte";
  import ShuffleVisual from "./ShuffleVisual.svelte";
  import PluginHost from "./PluginHost.svelte";
  import AppOnly from "./AppOnly.svelte";
  import { onWeb } from "../web";

  let radio = $state<{ on: boolean; ok: boolean | null }>({ on: false, ok: null });
  let hidden = $state<string[]>([]);
  onMount(() => {
    backend().call<typeof radio>("radio_state").then((r) => r && (radio = r));
    loadHidden();
  });
  async function loadHidden() { hidden = (await backend().call<string[]>("hidden_artists")) ?? []; }
  async function show(name: string) {
    await backend().call("toggle_hidden_artist", { name });
    app.toast(tf("toast.artist_shown", { name }));
    loadHidden();
  }
  function setRadio(on: boolean) { radio.on = on; backend().call("set_radio", { on }); }
</script>

<PluginHost page="playback.content">
  <div class="col">
    <ShuffleVisual />
    <Rows>
      <Row label={t("sp.skip_unplayable")} info={t("sp.skip_unplayable.desc")}>
        <Tog on={app.config.playback.skip_unplayable} onchange={(v) => app.setConfig((c) => { c.playback.skip_unplayable = v; })} label={t("sp.skip_unplayable")} />
      </Row>
      {#if !onWeb()}
      <Row label={t("sp.patient_search")} info={t("sp.patient_search.desc")}>
        <Tog on={app.config.playback.patient_search} onchange={(v) => app.setConfig((c) => { c.playback.patient_search = v; })} label={t("sp.patient_search")} />
      </Row>
      {/if}
      {#if !onWeb() || !!app.account}
      <Row label={t("sp.stats")} info={t("sp.stats.desc")}>
        <Tog on={app.config.stats.enabled} onchange={(v) => app.setConfig((c) => { c.stats.enabled = v; })} label={t("sp.stats")} />
      </Row>
      {/if}
      <Row label={t("sp.explicit")} info={t("sp.explicit.desc")}>
        <Tog on={app.config.audio.hide_explicit} onchange={(v) => app.setConfig((c) => { c.audio.hide_explicit = v; })} label={t("sp.explicit")} />
      </Row>
      <Row label={t("sp.radio")} info={radio.ok === false ? t("radio.off") : t("sp.radio.desc")}>
        <Tog on={radio.on} disabled={radio.ok === false} why={t("radio.off")} onchange={setRadio} label={t("sp.radio")} />
      </Row>
      <Row label={t("sp.recommendations")} info={t("sp.recommendations.desc")}>
        <Tog on={app.config.ui.recommendations !== false} onchange={(v) => app.setConfig((c) => { c.ui.recommendations = v; })} label={t("sp.recommendations")} />
      </Row>
    </Rows>
    <div class="hid">
      <Row label={t("sp.hidden")} info={hidden.length ? t("sp.hidden.desc") : t("sp.hidden.none")}>
        <div class="chips">
          {#each hidden as name (name)}
            <span class="chip">
              <span>{name}</span>
              <button class="eye" onclick={() => show(name)} use:tip={t("ctx.show_artist")} aria-label={t("ctx.show_artist")}><Icon name="eye" size={12} color="rgba(255,255,255,.7)" /></button>
            </span>
          {/each}
        </div>
      </Row>
    </div>
    {#if onWeb()}<AppOnly />{/if}
  </div>
</PluginHost>

<style>
  .col { display: flex; flex-direction: column; gap: 10px; }
  .hid { border-top: 1px solid rgba(255, 255, 255, .06); }
  .chips { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 6px; max-width: 320px; }
  .chip { height: 28px; padding: 0 6px 0 10px; display: flex; align-items: center; gap: 6px; border-radius: 8px; background: rgba(255, 255, 255, .07); font-size: 12.5px; color: rgba(255, 255, 255, .85); }
  .eye { width: 18px; height: 18px; border-radius: 4px; display: grid; place-items: center; transition: background-color .12s; }
  .eye:hover { background: rgba(255, 255, 255, .12); }
</style>
