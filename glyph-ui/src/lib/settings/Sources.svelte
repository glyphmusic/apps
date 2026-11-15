<script lang="ts">
  // settings/sources.rs sources + ui/ytdlp.rs: Playback > Sources. Search
  // mode; YouTube (where it plays, which program finds it, the yt-dlp row,
  // embed fallback, YouTube only); the free sources (Audius, Jamendo,
  // Internet Archive, paused while cooling down) and their priority, which is
  // changed by dragging the row (grip 15, number mono 13, name 14, [badge]).
  // The program choice and yt-dlp row are not on phones (r-dlp is built in).
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { onWeb } from "../web";
  import { t, tf } from "../i18n.svelte";
  import { PROVIDER_BADGE, PROVIDER_NAME, type ProviderId } from "../api/types";
  import Icon from "../components/Icon.svelte";
  import Button from "../components/Button.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import Tog from "./Tog.svelte";
  import Segmented from "./Segmented.svelte";
  import SectionLabel from "./SectionLabel.svelte";
  import PluginHost from "./PluginHost.svelte";
  import SourceRouteVisual from "./SourceRouteVisual.svelte";
  import SourcesFlowVisual from "./SourcesFlowVisual.svelte";
  import { activeSources } from "./sources";
  import { errText, isMobile } from "./util";
  import type { YtDlpStatus } from "./types";

  const web = onWeb();
  const touch = $derived(isMobile());
  const pv = $derived(app.config.providers);
  const embed = $derived(web ? pv.enabled.includes("youtube") : (pv.enabled.includes("youtube") || pv.enabled.includes("soundcloud")));
  const FREE: ProviderId[] = ["audius", "jamendo", "archive", "deezer"];
  const activePriority = $derived(activeSources(pv, web));

  let cooling = $state<string[]>([]);
  let st = $state<YtDlpStatus | null>(null);
  let checking = $state(false);
  let installing = $state(false);
  let error = $state<string | null>(null);

  onMount(() => {
    if (web) return;
    backend().call<string[]>("providers_cooling").then((c) => c && (cooling = c));
    if (!touch && pv.extractor === "yt_dlp") check(false);
  });
  async function check(force: boolean) {
    if (checking || installing) return;
    checking = true;
    if (force) error = null;
    try { st = (await backend().call<YtDlpStatus>("ytdlp_status", { force })) ?? null; }
    catch (e) { st = { path: null, version: null, js_runtime: null, can_install: false, managed: false, update_available: null, latest_version: null, update_error: null }; error = errText(e); }
    checking = false;
  }
  async function install() {
    if (installing) return;
    installing = true; error = null;
    try {
      st = (await backend().call<YtDlpStatus>("ytdlp_install")) ?? st;
      app.toastInfo(tf("ytdlp.installed", { v: st?.version ?? "" }));
    } catch (e) { error = errText(e); app.toastInfo(tf("ytdlp.install_error", { e: errText(e) })); }
    installing = false;
  }

  /** ui/ytdlp.rs description: found / version / JS runtime hint. */
  const desc = $derived.by(() => {
    if (installing) return t("ytdlp.installing");
    if (!st) return t("ytdlp.checking");
    let s = !st.path ? t("ytdlp.missing") : !st.version ? t("ytdlp.broken") : tf("ytdlp.found", { v: st.version, js: st.js_runtime ?? "-" });
    if (st.path && !st.js_runtime) s += ` · ${t("ytdlp.no_js")}`;
    if (st.version) {
      s += " · ";
      s += checking ? t("ytdlp.checking_update") : st.update_available === true ? tf("ytdlp.update_available", { v: st.latest_version ?? "-" }) : st.update_available === false ? t("ytdlp.up_to_date") : t("ytdlp.update_check_failed");
    }
    if (error) s += ` · ${error}`;
    return s;
  });
  // A yt-dlp from PATH or the config belongs to the user: no button.
  const showInstall = $derived(!!st && st.can_install && (!st.path || (st.managed && (installing || !st.version || st.update_available === true))));
  const installLabel = $derived(installing ? t("ytdlp.installing_short") : st?.managed ? t("ytdlp.update") : t("ytdlp.install"));

  function toggleEmbed(on: boolean) {
    app.setConfig((c) => {
      c.providers.enabled = c.providers.enabled.filter((p) => p !== "youtube" && p !== "soundcloud");
      if (on) {
        if (web) c.providers.enabled.push("youtube");
        else c.providers.enabled.push("soundcloud", "youtube");
      }
    });
  }
  function toggleProvider(p: ProviderId, on: boolean) {
    app.setConfig((c) => { c.providers.enabled = on ? [...c.providers.enabled.filter((x) => x !== p), p] : c.providers.enabled.filter((x) => x !== p); });
  }
  function reorder(from: number, to: number) {
    const fromId = activePriority[from], toId = activePriority[to];
    if (!fromId || !toId) return;
    app.setConfig((c) => {
      for (const p of activePriority) {
        if (!c.providers.priority.includes(p)) c.providers.priority.push(p);
      }
      const fi = c.providers.priority.indexOf(fromId);
      const ti = c.providers.priority.indexOf(toId);
      if (fi < 0 || ti < 0) return;
      const [p] = c.providers.priority.splice(fi, 1);
      c.providers.priority.splice(ti, 0, p);
    });
  }
</script>

<PluginHost page="playback.sources">
  <div class="col">
    {#if !web}
      <SourceRouteVisual kind="search" />
    {/if}

    <SectionLabel text="YouTube" />
    <div class="yt">
      {#if !web}
        <SourceRouteVisual kind="youtube" />
      {/if}
      <Rows>
        {#if !web && !touch}
          <Row label={t("sp.extractor")} info={t("sp.extractor.desc")}>
            <Segmented value={pv.extractor === "yt_dlp"} options={[{ value: false, label: "r-dlp" }, { value: true, label: "yt-dlp" }]}
              onpick={(v) => {
                app.setConfig((c) => { c.providers.extractor = v ? "yt_dlp" : "r_dlp"; });
                if (v && !st) check(false);
              }} />
          </Row>
          {#if pv.extractor === "yt_dlp"}
            <Row label={t("ytdlp.title")} status={desc}>
              <span class="dot" class:ok={!!st?.version}></span>
              {#if st && (st.update_error || error) && !checking && !installing}<Button onclick={() => check(true)}>{t("ytdlp.recheck")}</Button>{/if}
              {#if showInstall}
                <button class="inst" class:busy={installing} onclick={install}>
                  <Icon name="download" size={14} color="rgba(255,255,255,.8)" /><span>{installLabel}</span>
                </button>
              {/if}
            </Row>
          {/if}
        {/if}
        <Row label={web ? t("settings.sources.fallback.web") : t("settings.sources.fallback")} info={web ? t("settings.sources.fallback.web.hint") : t("settings.sources.fallback.hint")}>
          <Tog on={embed} onchange={toggleEmbed} label={web ? t("settings.sources.fallback.web") : t("settings.sources.fallback")} />
        </Row>
        <Row label={t("ytdlp.only")} info={t("ytdlp.only.hint")}>
          <Tog on={pv.youtube_only} onchange={(v) => app.setConfig((c) => { c.providers.youtube_only = v; if (v && !c.providers.enabled.includes("youtube")) c.providers.enabled.push("youtube"); })} label={t("ytdlp.only")} />
        </Row>
      </Rows>
    </div>

    <SectionLabel text={t("settings.sources")} />
    <Rows>
      {#each FREE as p (p)}
        <Row label={PROVIDER_NAME[p]} info={cooling.includes(p) ? t("settings.sources.paused") : t(`sp.src.${PROVIDER_BADGE[p].toLowerCase()}`)}>
          <Tog on={pv.enabled.includes(p)} onchange={(v) => toggleProvider(p, v)} label={PROVIDER_NAME[p]} />
        </Row>
      {/each}
    </Rows>

    <div class="flow">
      <SourcesFlowVisual {touch} onmove={reorder} />
    </div>
  </div>
</PluginHost>

<style>
  .col { display: flex; flex-direction: column; }
  .yt { display: flex; flex-direction: column; gap: 10px; }
  .flow { padding-top: 14px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: rgba(255, 255, 255, .25); transition: background-color .2s; }
  .dot.ok { background: #fff; }
  .inst { height: 32px; padding: 0 12px; display: flex; align-items: center; gap: 8px; border-radius: 8px; background: rgba(255, 255, 255, .07); font-size: 13px; color: #fff; transition: background-color .12s; }
  .inst:hover { background: rgba(255, 255, 255, .12); }
  .inst.busy { color: rgba(255, 255, 255, .5); pointer-events: none; }
</style>
