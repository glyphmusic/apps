<script lang="ts">
  // ui/stats.rs view(): listening statistics (opt-in).
  //  page: gap 28; back pill, head (heading + range chips: this month / last
  //  month / this year / all time), then one of: "statistics off" with a
  //  primary "turn on" button, "nothing counted yet", or
  //   recap cards (one per year, h 52, sparkles in accent) → four tiles →
  //   top artists (h 34 rows with a 6 px share bar, click opens the artist) →
  //   top songs (song rows + "n plays · time") → when you listen: by hour and
  //   by weekday bars (peak bar in accent) → month by month (last 12) → note.
  //  A recap card opens the year view (stats/Recap.svelte) in place.
  import { backend } from "../api/backend";
  import { app } from "../state.svelte";
  import { t, tf } from "../i18n.svelte";
  import { ink } from "../ui.svelte";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import ArtistAvatar from "../components/ArtistAvatar.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import SectionTitle from "../components/SectionTitle.svelte";
  import SongRow from "../components/SongRow.svelte";
  import PhoneSongRow from "../compact/PhoneSongRow.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import SkeletonSwap from "../components/SkeletonSwap.svelte";
  import Segmented from "../settings/Segmented.svelte";
  import BackPill from "./lists/BackPill.svelte";
  import Tile from "./stats/Tile.svelte";
  import Bars from "./stats/Bars.svelte";
  import Recap from "./stats/Recap.svelte";
  import { statsState } from "./stats/stats.svelte";
  import { fmtListen, peakOf, type StatsRange, type StatsSummary } from "./stats/types";

  const RANGES: [StatsRange, string][] = [["month", "stats.range.month"], ["last_month", "stats.range.last_month"], ["year", "stats.range.year"], ["all", "stats.range.all"]];
  const WD = ["stats.mon", "stats.tue", "stats.wed", "stats.thu", "stats.fri", "stats.sat", "stats.sun"];

  let data = $state<StatsSummary | null>(null);
  let loading = $state(true);
  const enabled = $derived(app.config?.stats.enabled ?? false);

  $effect(() => {
    const r = statsState.range;
    if (!enabled) { loading = false; return; }
    let alive = true;
    loading = true;
    backend().call<StatsSummary>("stats_summary", { range: r }).then((s) => { if (alive) { data = s ?? null; loading = false; } });
    return () => { alive = false; };
  });

  const empty = $derived(!!data && data.ms === 0 && data.plays === 0);
  const avgDay = $derived(data ? Math.floor(data.ms / Math.max(1, data.active_days)) : 0);
  const maxA = $derived(Math.max(1, data?.top_artists[0]?.ms ?? 1));
  const songs = $derived((data?.top_tracks ?? []).map((x) => x.track));
  const hourLabels = Array.from({ length: 24 }, (_, h) => (h % 3 === 0 ? String(h).padStart(2, "0") : ""));
  const peakH = $derived(data ? peakOf(data.hours) : null);
  const peakD = $derived(data ? peakOf(data.weekdays) : null);
  const months = $derived((data?.months ?? []).slice(-12));
  const rangeOptions = $derived(RANGES.map(([value, key]) => ({ value, label: t(key) })));
</script>

{#if statsState.recap && enabled}
  <Recap year={statsState.recap} onback={() => (statsState.recap = null)} />
{:else}
  <div class="pg">
    <BackPill />

    <div class="head">
      {#if !app.compact}<SectionTitle heading title={t("stats.title")} />{/if}
      <div class="seg-wrap">
        <Segmented value={statsState.range} options={rangeOptions} onpick={(v) => (statsState.range = v)} fill={app.compact} />
      </div>
    </div>

    {#if !enabled}
      <div class="off">
        <EmptyState icon="music-2" title={t("stats.off.title")} sub={t("stats.off.sub")} />
        <Button kind="primary" onclick={() => app.setConfig((c) => (c.stats.enabled = true))}>{t("stats.enable")}</Button>
      </div>
    {:else}
      <SkeletonSwap loading={loading && !data}>
        {#snippet skeleton()}
          <div class="tiles">{#each [0, 1, 2, 3] as i (i)}<div class="sk"><Skeleton w="100%" h={92} radius={12} id="stats-{i}" /></div>{/each}</div>
        {/snippet}
        {#if empty}
          <EmptyState icon="music-2" title={t("stats.empty.title")} sub={t("stats.empty.sub")} />
        {:else if data}
          <div class="stats-body">
      {#if data.years.length}
        <div class="cards">
          {#each data.years.slice(0, 2) as y, i (y)}
            <button class="rcard" style:animation-delay="{i * 40}ms" onclick={() => (statsState.recap = y)}>
              <Icon name="sparkles" size={16} color="var(--accent)" />
              <span class="rt">{tf("recap.title", { year: y })}</span>
              <Icon name="chevron-right" size={14} color="var(--dim)" />
            </button>
          {/each}
        </div>
      {/if}

      <div class="tiles">
        <Tile label={t("stats.time")} value={fmtListen(data.ms)} sub={tf("stats.per_day", { t: fmtListen(avgDay) })} />
        <Tile label={t("stats.plays")} value={String(data.plays)} sub={tf("stats.skips", { n: data.skips })} delay={40} />
        <Tile label={t("stats.artists")} value={String(data.artists)} sub={data.top_artists[0] ? tf("stats.top_is", { name: data.top_artists[0].name }) : null} delay={80} />
        <Tile label={t("stats.songs")} value={String(data.songs)} sub={data.top_tracks[0] ? tf("stats.top_is", { name: data.top_tracks[0].track.title }) : null} delay={120} />
      </div>

      <section class="sec">
        <SectionTitle icon="users" title={t("stats.top_artists")} />
        <div class="card list">
          {#each data.top_artists.slice(0, 10) as a, i (a.name)}
            <button class="arow" onclick={() => app.openArtistPage(a.name)}>
              <span class="n mono">{String(i + 1).padStart(2, "0")}</span>
              <ArtistAvatar name={a.name} size={32} />
              <span class="an trunc">{a.name}</span>
              {#if !app.compact}<span class="track"><span class="fill" style:width="{(a.ms / maxA) * 100}%" style:animation-delay="{i * 30}ms"></span></span>{/if}
              <span class="tm mono">{fmtListen(a.ms)}</span>
            </button>
          {/each}
        </div>
      </section>

      <section class="sec">
        <SectionTitle icon="music-2" title={t("stats.top_songs")} />
        <div class="card rows">
          {#each data.top_tracks.slice(0, 10) as x, i (i)}
            {#if app.compact}
              <PhoneSongRow tracks={songs} index={i} context={t("stats.top_songs")} number={i + 1} />
            {:else}
              <div class="srow">
                <div class="sr"><SongRow tracks={songs} index={i} context={t("stats.top_songs")} album={false} /></div>
                <span class="ex mono">{tf("stats.track_line", { n: x.plays, t: fmtListen(x.ms) })}</span>
              </div>
            {/if}
          {/each}
        </div>
      </section>

      <div class="when">
        <div class="card cbox wh">
          <div class="ch"><SectionTitle icon="clock" title={peakH != null ? tf("stats.by_hour_peak", { h: `${String(peakH).padStart(2, "0")}:00` }) : t("stats.by_hour")} /></div>
          <Bars values={data.hours} labels={hourLabels} highlight={peakH} />
        </div>
        <div class="card cbox wd">
          <div class="ch"><SectionTitle icon="calendar" title={t("stats.by_weekday")} /></div>
          <Bars values={data.weekdays} labels={WD.map((k) => t(k))} highlight={peakD} />
        </div>
      </div>

      {#if months.length > 1}
        <div class="card cbox">
          <div class="ch"><SectionTitle icon="chart-column" title={t("stats.history")} /></div>
          <Bars values={months.map((m) => m.ms)} labels={months.map((m) => m.month.slice(5, 7))} highlight={months.length - 1} />
        </div>
      {/if}

          <div class="note" use:ink>{t(app.account ? "stats.note.synced" : "stats.note.local")}</div>
          </div>
        {/if}
      </SkeletonSwap>
    {/if}
  </div>
{/if}

<style>
  .pg, .stats-body { display: flex; flex-direction: column; gap: 24px; }
  .head { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 12px; }
  .seg-wrap { max-width: 100%; }
  :global(html.compact) .seg-wrap { width: 100%; }
  .off { display: flex; flex-direction: column; align-items: flex-start; gap: 14px; }
  .tiles { display: flex; flex-wrap: wrap; gap: 10px; }
  .sk { flex: 1; min-width: 150px; }
  .cards { display: flex; flex-wrap: wrap; gap: 10px; }
  .rcard { display: flex; align-items: center; gap: 10px; height: 46px; padding: 0 14px; border-radius: 10px; background: var(--card); transition: background-color .12s ease-out; animation: rise-in .3s var(--ease) both; }
  .rcard:hover { background: var(--card-hover); }
  .rt { font-size: var(--md); font-weight: 500; }
  .sec { display: flex; flex-direction: column; gap: 12px; }
  .card { background: var(--card); border-radius: 12px; }
  .list { display: flex; flex-direction: column; gap: 2px; padding: 8px; }
  .arow { display: flex; align-items: center; gap: 12px; height: 46px; padding: 0 10px; border-radius: 8px; transition: background-color .12s ease-out; }
  .arow:hover { background: rgba(255, 255, 255, .05); }
  .n { width: 22px; font-size: var(--sm); color: var(--faint); flex: none; }
  .an { width: 180px; font-size: var(--md); font-weight: 500; flex: none; }
  :global(html.compact) .an { flex: 1; width: auto; }
  .track { flex: 1; height: 4px; border-radius: 2px; background: rgba(255, 255, 255, .07); overflow: hidden; }
  .fill { display: block; height: 100%; border-radius: 2px; background: var(--accent); transform-origin: left; animation: fillin .35s var(--ease) both; }
  @keyframes fillin { from { transform: scaleX(0); } to { transform: scaleX(1); } }
  .tm { width: 84px; flex: none; text-align: right; font-size: var(--sm); color: var(--dim); }
  .rows { display: flex; flex-direction: column; padding: 8px; }
  .srow { display: flex; align-items: center; gap: 8px; }
  .sr { flex: 1; min-width: 0; }
  .ex { width: 140px; flex: none; padding-right: 10px; text-align: right; font-size: var(--xs); color: var(--faint); }
  .when { display: flex; flex-wrap: wrap; gap: 12px; }
  .cbox { padding: 16px 18px; display: flex; flex-direction: column; gap: 14px; }
  .ch :global(.sec) { font-size: var(--md); }
  .wh { flex: 1; min-width: 320px; }
  .wd { flex: 1; min-width: 260px; }
  :global(html.compact) .wh, :global(html.compact) .wd { min-width: 0; flex-basis: 100%; }
  .note { font-size: var(--xs); color: var(--faint); }
</style>
