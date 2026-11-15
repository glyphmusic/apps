<script lang="ts">
  // settings/storage.rs storage: Cache & storage. A flat surface (p 14) with
  // the legend (rows h 36: checkbox, colour dot 8, name medium, percent chip
  // mono 11, size mono 12.5; unchecked ones step back to white 55 %) and a
  // ring chart 116 / 14 (total in 22 bold, unit 10.5 caps, caption 12);
  // hovering a segment lights its legend row and dims the other segments.
  // Cache limit: a track with tick stops (14 px knob on the current one,
  // filled up to it), the wheel steps it. Data saver; folders (download
  // folder, reveal, music folders; not on phones); Clear selected (white,
  // h 40) and Open folder.
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import Icon from "../components/Icon.svelte";
  import Button from "../components/Button.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import Surface from "./Surface.svelte";
  import VisualCard from "./VisualCard.svelte";
  import Tog from "./Tog.svelte";
  import Check from "./Check.svelte";
  import PluginHost from "./PluginHost.svelte";
  import { formatBytes, isMobile } from "./util";
  import type { StorageStats } from "./types";

  const COLORS = [0x22c3e6, 0xa970ff, 0xff5fa2, 0xf2b84b, 0x3ddc97].map((n) => `#${n.toString(16).padStart(6, "0")}`);
  const LIMITS = [250, 1024, 2048, 3072, 5120, 10240, 0];
  const KINDS = ["covers", "downloads", "lyrics", "catalog", "streams"] as const;
  const LABELS: Record<(typeof KINDS)[number], string> = { covers: "sp.storage.covers", downloads: "sp.storage.downloads", lyrics: "sp.storage.lyrics", catalog: "sp.storage.catalog", streams: "sp.storage.streams" };
  let {}: { sub?: number } = $props();
  const touch = $derived(isMobile());

  let stats = $state<StorageStats | null>(null);
  let selected = $state([true, true, true, true, true]);
  let hover = $state<number | null>(null);
  const SIZE = 116, TH = 14, R = (SIZE - TH) / 2, CIRC = 2 * Math.PI * R;

  async function refresh() { stats = (await backend().call<StorageStats>("storage_stats")) ?? stats; }
  onMount(refresh);

  const vals = $derived(stats ? KINDS.map((k) => stats![k]) : []);
  const total = $derived(vals.reduce((a, b) => a + b, 0));
  const pct = (v: number) => { const p = total > 0 ? (v / total) * 100 : 0; return v > 0 && p < 1 ? "<1%" : `${p.toFixed(0)}%`; };
  const totalText = $derived(formatBytes(total));
  const num = $derived(totalText.split(" ")[0]);
  const unit = $derived((totalText.split(" ")[1] ?? "B").toUpperCase());
  const segs = $derived.by(() => {
    let acc = 0;
    return vals.map((v) => { const len = total > 0 ? (v / total) * CIRC : 0; const s = { len, off: -acc }; acc += len; return s; });
  });
  const selBytes = $derived(vals.reduce((a, v, i) => a + (selected[i] ? v : 0), 0));

  const cur = $derived(app.config.storage.cache_limit_mb);
  const curI = $derived(LIMITS.indexOf(cur));
  const limitLabel = (mb: number) => (mb === 0 ? t("sp.storage.nolimit") : mb < 1024 ? `${mb} MB` : `${mb / 1024} GB`);
  const setLimit = (mb: number) => app.setConfig((c) => { c.storage.cache_limit_mb = mb; });
  let rest = 0;
  function wheel(e: WheelEvent) {
    e.preventDefault();
    let n = 0;
    if (e.deltaMode === 1 || Math.abs(e.deltaY) >= 50) n = -Math.sign(e.deltaY);
    else { rest += -e.deltaY; n = Math.trunc(rest / 40); rest -= n * 40; }
    if (!n) return;
    const at = Math.max(0, curI);
    setLimit(LIMITS[Math.min(LIMITS.length - 1, Math.max(0, at + n))]);
  }

  async function clear() {
    for (let i = 0; i < KINDS.length; i++) if (selected[i]) {
      try { await backend().call("storage_clear", { kind: KINDS[i] }); } catch (e) { app.toastInfo(String((e as Error)?.message ?? e)); continue; }
      // app.rs clear_storage: the artist pages kept in memory go with the catalog cache.
      if (KINDS[i] === "catalog") { const open = app.view === "artist" ? app.openArtist?.toLowerCase() : undefined; for (const k of Object.keys(app.artists)) if (k !== open) delete app.artists[k]; }
    }
    refresh();
  }
  async function pickDir() { await backend().call("storage_set_download_dir", { pick: true }); refresh(); }
  async function resetDir() { await backend().call("storage_set_download_dir", { dir: null }); refresh(); }
  const dlDir = $derived(app.config.storage.download_dir ?? stats?.default_download_dir ?? "");
  const nFolders = $derived(app.config.local.folders.length);
  const localStatus = $derived(nFolders === 0 ? t("sp.storage.local.none") : tf("local.info", { n: stats?.local_songs ?? 0, folders: nFolders }));
  function openLocal() { app.settingsOpen = false; app.navigate("local"); }
</script>

<PluginHost page="storage">
  {#if !stats}
    <div class="sk"><Skeleton w="100%" h={160} radius={10} id="storage-sk" /></div>
  {:else}
    <div class="col">
      <Surface pad={14}>
        <div class="bd">
          <div class="legend">
            {#each KINDS as k, i (k)}
              <button class="lg" class:lit={hover === i} onclick={() => (selected[i] = !selected[i])} onpointerenter={() => (hover = i)} onpointerleave={() => (hover = null)}>
                <Check on={selected[i]} />
                <span class="dot" style:background={COLORS[i]}></span>
                <span class="nm" class:off={!selected[i]}>{t(LABELS[k])}</span>
                <span class="mono pc">{pct(vals[i])}</span>
                <span class="fl"></span>
                <span class="mono sz" class:off={!selected[i]}>{formatBytes(vals[i])}</span>
              </button>
            {/each}
          </div>
          <div class="ring">
            <div class="don" style:width="{SIZE}px" style:height="{SIZE}px">
              <svg width={SIZE} height={SIZE} viewBox="0 0 {SIZE} {SIZE}" aria-hidden="true">
                {#if total <= 0}
                  <circle cx={SIZE / 2} cy={SIZE / 2} r={R} fill="none" stroke="rgba(255,255,255,.08)" stroke-width={TH} />
                {:else}
                  {#each segs as s, i}
                    {#if vals[i] > 0}
                      <circle class="seg" cx={SIZE / 2} cy={SIZE / 2} r={R} fill="none" stroke={COLORS[i]} stroke-width={TH}
                        stroke-dasharray="{Math.max(0, s.len - 0.5)} {CIRC}" stroke-dashoffset={s.off} transform="rotate(-90 {SIZE / 2} {SIZE / 2})"
                        opacity={hover != null && hover !== i ? 0.35 : 1} role="presentation" onpointerenter={() => (hover = i)} onpointerleave={() => (hover = null)} />
                    {/if}
                  {/each}
                {/if}
              </svg>
              <div class="mid"><div class="num">{num}</div><div class="un">{unit}</div></div>
            </div>
            <div class="cap">{t("sp.storage.usage")}</div>
          </div>
        </div>
      </Surface>

      <div class="act">
        <button class="clr" disabled={selBytes <= 0} onclick={clear}>{tf("sp.storage.clearsel", { size: formatBytes(selBytes) })}</button>
        {#if !touch}
          <button class="opn" onclick={() => backend().call("reveal_data_dir")}>
            <Icon name="folder-open" size={15} color="rgba(255,255,255,.85)" /><span>{t("sp.storage.openfolder")}</span>
          </button>
        {/if}
      </div>

      <VisualCard label={t("sp.storage.limit")} info={t("sp.storage.limit.desc")}>
        {#snippet right()}
          <span class="mono badge">{limitLabel(cur)}</span>
        {/snippet}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="stops" onwheel={wheel}>
          {#each LIMITS as mb, i (mb)}
            {@const on = i === curI}
            <button class="stop" class:on onclick={() => setLimit(mb)}>
              <span class="trk">
                <span class="seg2" class:vis={i > 0}></span>
                <span class="seg2" class:vis={i + 1 < LIMITS.length}></span>
                <span class="sdot"></span>
              </span>
              <span class="sl" class:on>{limitLabel(mb)}</span>
            </button>
          {/each}
        </div>
      </VisualCard>

      <Rows>
        <Row label={t("sp.storage.saver")} info={t("sp.storage.saver.desc")}>
          <Tog on={app.config.storage.data_saver} onchange={(v) => app.setConfig((c) => { c.storage.data_saver = v; })} label={t("sp.storage.saver")} />
        </Row>
        {#if !touch}
          <Row label={t("sp.storage.dl_dir")} info={t("sp.storage.dl_dir.desc")} status={dlDir}>
            {#if app.config.storage.download_dir}<Button onclick={resetDir}>{t("sp.storage.dl_dir.reset")}</Button>{/if}
            <Button onclick={pickDir}>{t("sp.storage.dl_dir.change")}</Button>
          </Row>
          <Row label={t("sp.storage.reveal")} info={t("sp.storage.reveal.desc")}>
            <Tog on={app.config.ui.reveal_downloads} onchange={(v) => app.setConfig((c) => { c.ui.reveal_downloads = v; })} label={t("sp.storage.reveal")} />
          </Row>
          <Row label={t("sp.storage.local")} info={t("sp.storage.local.desc")} status={localStatus}>
            <Button onclick={openLocal}>{t("sp.storage.local.open")}</Button>
          </Row>
        {/if}
      </Rows>
    </div>
  {/if}
</PluginHost>

<style>
  .col { display: flex; flex-direction: column; gap: 14px; padding-top: 6px; }
  .sk { padding-top: 6px; max-width: 520px; }
  .bd { display: flex; flex-wrap: wrap; align-items: center; gap: 16px 28px; }
  .legend { flex: 1 1 280px; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .lg { height: 36px; padding: 0 10px; display: flex; align-items: center; gap: 10px; border-radius: 8px; font-size: 13.5px; transition: background-color .12s; }
  .lg:hover, .lg.lit { background: rgba(255, 255, 255, .06); }
  .dot { width: 8px; height: 8px; border-radius: 50%; margin-left: 4px; flex: none; }
  .nm { font-weight: 500; color: #fff; transition: color .15s; }
  .nm.off { color: rgba(255, 255, 255, .55); }
  .pc { padding: 0 6px; border-radius: 4px; background: rgba(255, 255, 255, .08); font-size: 11px; color: rgba(255, 255, 255, .6); }
  .fl { flex: 1; }
  .sz { font-size: 12.5px; font-weight: 500; color: rgba(255, 255, 255, .85); }
  .sz.off { color: rgba(255, 255, 255, .45); }
  .ring { flex: none; padding-right: 16px; display: flex; flex-direction: column; align-items: center; gap: 8px; margin: 0 auto; }
  .don { position: relative; }
  .seg { transition: opacity .15s ease-out; cursor: default; pointer-events: stroke; }
  .mid { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; pointer-events: none; }
  .num { font-size: 22px; font-weight: 700; color: #fff; }
  .un { font-size: 10.5px; color: rgba(255, 255, 255, .6); }
  .cap { font-size: 12px; color: rgba(255, 255, 255, .6); }
  .badge { padding: 2px 8px; border-radius: 6px; background: rgba(255, 255, 255, .07); font-size: 12px; color: rgba(255, 255, 255, .75); }
  .stops { display: flex; padding: 4px 0 2px; }
  .stop { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: center; gap: 10px; }
  .trk { position: relative; width: 100%; height: 18px; display: flex; align-items: center; }
  .seg2 { flex: 1; height: 1px; }
  .seg2.vis { background: rgba(255, 255, 255, .18); }
  .sdot {
    position: absolute;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: #141414;
    border: 1.5px solid rgba(255, 255, 255, .36);
    transition: border-color .15s, border-width .15s, transform .15s var(--ease);
  }
  .stop:hover .sdot { border-color: rgba(255, 255, 255, .8); transform: translate(-50%, -50%) scale(1.08); }
  .stop.on .sdot { width: 15px; height: 15px; border: 4px solid var(--accent); background: #0e0e0e; }
  .sl { font-size: 11.5px; color: rgba(255, 255, 255, .52); white-space: nowrap; transition: color .15s; }
  .stop:hover .sl { color: rgba(255, 255, 255, .85); }
  .sl.on { font-weight: 600; color: #fff; }
  .act { padding-top: 2px; display: flex; flex-wrap: wrap; gap: 10px; }
  .clr { flex: 1 1 200px; height: 40px; display: flex; align-items: center; justify-content: center; border-radius: 10px; background: #fff; font-size: 13.5px; font-weight: 500; color: #000; transition: opacity .15s; }
  .clr:hover:not(:disabled) { opacity: .88; }
  .clr:disabled { opacity: .4; cursor: default; }
  .opn { flex: none; height: 40px; padding: 0 18px; display: flex; align-items: center; gap: 8px; border-radius: 10px; background: rgba(255, 255, 255, .07); font-size: 13.5px; font-weight: 500; color: #fff; transition: background-color .12s; }
  .opn:hover { background: rgba(255, 255, 255, .12); }
</style>
