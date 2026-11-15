<script lang="ts">
  // settings/playback.rs playback: sub-tabs General (sound and output),
  // Content (what plays), Sources, Player. General: streaming quality,
  // download quality, crossfade (thin bar 180 + value w 36, 13 px white 75 %),
  // output device, loudness level, normalize, Play on MPD (input 170 +
  // switch, not on phones), network devices, channels.
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { onWeb } from "../web";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import Tog from "./Tog.svelte";
  import Select from "./Select.svelte";
  import Segmented from "./Segmented.svelte";
  import Field from "./Field.svelte";
  import ThinBar from "../components/ThinBar.svelte";
  import PluginHost from "./PluginHost.svelte";
  import PlayContent from "./PlayContent.svelte";
  import Sources from "./Sources.svelte";
  import PlayerOpts from "./PlayerOpts.svelte";
  import { isMobile } from "./util";
  import type { DeviceDiscovery, MpdStatus } from "./types";
  let { sub = 0 }: { sub?: number } = $props();
  // The web player shows Content and Sources (plus Player on desktop).
  const tab = $derived(onWeb() ? ([1, 2, 3][sub] ?? 1) : sub);

  const QUALITIES = ["auto", "low", "standard", "high"] as const;
  const qOptions = QUALITIES.map((q) => ({ value: q as string, label: t(`sp.audio.q.${q}`) }));
  const mobile = $derived(isMobile());
  const au = $derived(app.config.audio);

  let outputs = $state<string[]>([]);
  let status = $state<{ mpd: MpdStatus; devices: DeviceDiscovery | null }>({ mpd: { connected: null, error: null }, devices: null });
  let mpdAddr = $state(app.config.mpd.address ?? "");

  onMount(() => {
    const load = async () => { const s = await backend().call<typeof status | undefined>("playback_status"); if (s) status = s; };
    backend().call<string[]>("output_devices").then((l) => l && (outputs = l));
    load();
    const id = setInterval(load, 2000);
    return () => clearInterval(id);
  });

  const outputOptions = $derived.by(() => {
    const names = [...new Set([...(outputs.length ? outputs : app.outputDevices), ...(au.output_device ? [au.output_device] : [])])];
    return [{ value: null as string | null, label: t("sp.output.default") }, ...names.map((n) => ({ value: n as string | null, label: n }))];
  });
  const levelOptions = [{ value: "loud", label: t("sp.level.loud") }, { value: "normal", label: t("sp.level.normal") }, { value: "quiet", label: t("sp.level.quiet") }];
  const channelOptions = ["mono", "stereo", "spatial"].map((c) => ({ value: c, label: t(`sp.audio.channels.${c}`) }));
  const fade = (s: number) => (s === 0 ? t("sp.crossfade.off") : `${s} s`);

  const mpdStatus = $derived(
    status.mpd.error ? `${t("sp.mcp.problem")} ${status.mpd.error}`
      : status.mpd.connected ? tf("sp.mpd.connected", { host: status.mpd.connected })
      : app.config.mpd.enabled && !(app.config.mpd.address ?? "").trim() ? t("sp.mpd.need_address") : null,
  );
  const devStatus = $derived.by(() => {
    const d = status.devices;
    if (!d) return null;
    if (d.playing || d.error) return d.error ? `${t("sp.mcp.problem")} ${d.error}` : tf("sp.devices.playing", { name: d.playing! });
    return d.found > 0 ? tf("sp.devices.found", { n: d.found }) : d.searching ? t("sp.devices.searching") : t("sp.devices.none");
  });
</script>

{#if tab === 1}
  <PlayContent />
{:else if tab === 2}
  <Sources />
{:else if tab === 3 && !mobile}
  <PlayerOpts />
{:else}
  <PluginHost page="playback">
    <Rows>
      <Row label={t("sp.audio.quality")} info={t("sp.audio.quality.desc")}>
        <Select value={au.quality} options={qOptions} onpick={(v) => app.setConfig((c) => { c.audio.quality = v; })} />
      </Row>
      <Row label={t("sp.audio.dl_quality")} info={t("sp.audio.dl_quality.desc")}>
        <Select value={au.download_quality} options={qOptions} onpick={(v) => app.setConfig((c) => { c.audio.download_quality = v; })} />
      </Row>
      <Row label={t("sp.crossfade")} info={t("sp.crossfade.desc")}>
        <ThinBar
          width={180}
          value={au.crossfade_s / 20}
          step={1 / 20}
          live={false}
          label={(f) => fade(Math.round(f * 20))}
          onpick={(f) => {
            const s = Math.round(f * 20);
            if (s !== au.crossfade_s) app.setConfig((c) => { c.audio.crossfade_s = s; });
          }}
        />
        <span class="mono val">{fade(au.crossfade_s)}</span>
      </Row>
      <Row label={t("sp.output")} info={t("sp.output.desc")}>
        <Select value={au.output_device} options={outputOptions} label={au.output_device ?? t("sp.output.default")} onpick={(v) => app.setConfig((c) => { c.audio.output_device = v; })} />
      </Row>
      <Row label={t("sp.level")} info={t("sp.level.desc")}>
        <Select value={au.level} options={levelOptions} onpick={(v) => app.setConfig((c) => { c.audio.level = v; })} />
      </Row>
      <Row label={t("sp.normalize")} info={t("sp.normalize.desc")}>
        <Tog on={au.normalize} onchange={(v) => app.setConfig((c) => { c.audio.normalize = v; })} label={t("sp.normalize")} />
      </Row>
      <!-- MPD is a music daemon on a computer; phones have nothing to talk to. -->
      {#if !mobile}
        <Row label={t("sp.mpd")} info={t("sp.mpd.desc")} status={mpdStatus}>
          <Field bind:value={mpdAddr} width={170} placeholder={t("sp.mpd.placeholder")} onenter={(v) => app.setConfig((c) => { c.mpd.address = v.trim(); })} onblur={(v) => v.trim() !== app.config.mpd.address && app.setConfig((c) => { c.mpd.address = v.trim(); })} />
          <Tog on={app.config.mpd.enabled} onchange={(v) => app.setConfig((c) => { c.mpd.enabled = v; })} label={t("sp.mpd")} />
        </Row>
      {/if}
      <Row label={t("sp.devices")} info={t("sp.devices.desc")} status={devStatus}>
        <Tog on={app.config.devices.discovery} onchange={(v) => app.setConfig((c) => { c.devices.discovery = v; })} label={t("sp.devices")} />
      </Row>
      <Row label={t("sp.audio.channels")} info={t("sp.audio.channels.desc")}>
        <Segmented value={au.channels} options={channelOptions} onpick={(v) => app.setConfig((c) => { c.audio.channels = v; })} />
      </Row>
    </Rows>
  </PluginHost>
{/if}

<style>
  .val { width: 36px; text-align: right; font-size: 13px; color: rgba(255, 255, 255, .75); }
</style>
