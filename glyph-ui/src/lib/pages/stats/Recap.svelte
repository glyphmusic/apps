<script lang="ts">
  // stats.rs recap(): "Your 2026 in Glyph", the year at a glance.
  //  back link (chevron 14 + "Your stats", sm dim) → [ RECAP ] (mono 11
  //  faint) + heading → tiles (time, plays, artists, songs) → number one
  //  artist card (cover 120, p 16, radius 12, gap 20, name 28 bold) → facts
  //  (peak hour / day / month) → top 5 artists (rows h 34) → top 10 songs →
  //  save as playlist (primary) + share.
  import { backend } from "../../api/backend";
  import { app } from "../../state.svelte";
  import { t, tf } from "../../i18n.svelte";
  import Icon from "../../components/Icon.svelte";
  import ArtistAvatar from "../../components/ArtistAvatar.svelte";
  import ArtistName from "../lists/ArtistName.svelte";
  import Cover from "../../components/Cover.svelte";
  import SectionTitle from "../../components/SectionTitle.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import SongRow from "../../components/SongRow.svelte";
  import PhoneSongRow from "../../compact/PhoneSongRow.svelte";
  import Button from "../../components/Button.svelte";
  import Skeleton from "../../components/Skeleton.svelte";
  import { ink } from "../../ui.svelte";
  import Tile from "./Tile.svelte";
  import { fmtListen, peakOf, type StatsRecap } from "./types";

  let { year, onback }: { year: string; onback: () => void } = $props();
  let data = $state<StatsRecap | null>(null);
  $effect(() => {
    const y = year;
    data = null;
    let alive = true;
    backend().call<StatsRecap>("stats_recap", { year: y }).then((r) => { if (alive) data = r ?? null; });
    return () => { alive = false; };
  });

  const WD = ["stats.mon", "stats.tue", "stats.wed", "stats.thu", "stats.fri", "stats.sat", "stats.sun"];
  const days = $derived(data ? Math.floor(data.ms / 86_400_000) : 0);
  const top = $derived(data?.top_artists[0]);
  const seed = $derived(top ? (data!.top_tracks.find((x) => x.track.artist.toLowerCase().includes(top.name.toLowerCase()))?.track ?? top.track ?? null) : null);
  const peakH = $derived(data ? peakOf(data.hours) : null);
  const peakD = $derived(data ? peakOf(data.weekdays) : null);
  const peakM = $derived.by(() => { const m = data?.months ?? []; let b: (typeof m)[number] | null = null; for (const x of m) if (!b || x.ms > b.ms) b = x; return b?.ms ? b.month : null; });
  const songs = $derived((data?.top_tracks ?? []).map((x) => x.track));

  function save() {
    app.createPlaylist(tf("recap.playlist", { year }), songs).then(() => app.toast(tf("recap.playlist", { year })));
  }
  function share() {
    app.shareSongs({ title: tf("recap.title", { year }), tracks: songs, kind: "playlist" });
  }
</script>

<div class="rc">
  <div class="bk"><Button kind="back" icon="arrow-left" onclick={onback}>{t("stats.title")}</Button></div>
  <div class="hd">
    <div class="tag" use:ink>Recap</div>
    <SectionTitle heading title={tf("recap.title", { year })} />
  </div>

  {#if !data}
    <div class="tiles">{#each [0, 1, 2, 3] as i (i)}<div class="sk"><Skeleton w="100%" h={92} radius={12} id="recap-{i}" /></div>{/each}</div>
  {:else if data.ms === 0}
    <EmptyState icon="music-2" title={t("stats.empty.title")} sub={t("stats.empty.sub")} />
  {:else}
    <div class="tiles">
      <Tile label={t("stats.time")} value={fmtListen(data.ms)} sub={days > 0 ? tf("recap.days", { n: days }) : null} />
      <Tile label={t("stats.plays")} value={String(data.plays)} delay={40} />
      <Tile label={t("stats.artists")} value={String(data.artists)} delay={80} />
      <Tile label={t("stats.songs")} value={String(data.songs)} delay={120} />
    </div>

    {#if top}
      <button class="top" onclick={() => app.openArtistPage(top.name)}>
        {#if seed}<Cover track={seed} size={120} radius={10} />{/if}
        <div class="tt">
          <div class="l">{t("recap.top_artist")}</div>
          <div class="nm"><ArtistName name={top.name} badge={20} /></div>
          <div class="ln">{tf("recap.top_artist_line", { t: fmtListen(top.ms), n: top.plays })}</div>
        </div>
      </button>
    {/if}

    <div class="tiles">
      {#if peakH != null}<Tile label={t("recap.peak_hour")} value="{String(peakH).padStart(2, '0')}:00" />{/if}
      {#if peakD != null}<Tile label={t("recap.peak_day")} value={t(WD[peakD])} delay={40} />{/if}
      {#if peakM}<Tile label={t("recap.peak_month")} value={peakM} delay={80} />{/if}
    </div>

    <section class="sec">
      <SectionTitle icon="users" title={t("recap.top_artists")} />
      <div class="card list">
        {#each data.top_artists.slice(0, 5) as a, i (a.name)}
          <button class="arow" onclick={() => app.openArtistPage(a.name)}>
            <span class="n mono">{String(i + 1).padStart(2, "0")}</span>
            <ArtistAvatar name={a.name} size={32} />
            <span class="an"><ArtistName name={a.name} badge={14} /></span>
            <span class="tm mono">{fmtListen(a.ms)}</span>
          </button>
        {/each}
      </div>
    </section>

    <section class="sec">
      <SectionTitle icon="music-2" title={t("recap.top_songs")} />
      <div class="card rows">
        {#each songs.slice(0, 10) as _, i (i)}
          {#if app.compact}
            <PhoneSongRow tracks={songs} index={i} context={tf("recap.title", { year })} number={i + 1} />
          {:else}
            <SongRow tracks={songs} index={i} context={tf("recap.title", { year })} />
          {/if}
        {/each}
      </div>
    </section>

    {#if songs.length}
      <div class="btns">
        <Button kind="primary" onclick={save}>{t("recap.save")}</Button>
        <Button onclick={share}>{t("recap.share")}</Button>
      </div>
    {/if}
  {/if}
</div>

<style>
  .rc { display: flex; flex-direction: column; gap: 24px; }
  .bk { display: flex; align-self: flex-start; }
  .hd { display: flex; flex-direction: column; gap: 4px; }
  .tag { font-size: var(--xs); font-weight: 600; text-transform: uppercase; letter-spacing: .06em; color: var(--faint); }
  .tiles { display: flex; flex-wrap: wrap; gap: 10px; }
  .sk { flex: 1; min-width: 150px; }
  .top { display: flex; align-items: center; gap: 20px; padding: 16px; border-radius: 12px; background: var(--card); text-align: left; transition: background-color .12s ease-out; animation: rise-in .3s var(--ease) both; }
  .top:hover { background: var(--card-hover); }
  .tt { min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .l { font-size: var(--sm); font-weight: 500; color: var(--dim); }
  .nm { font-size: 28px; font-weight: 700; color: var(--text); min-width: 0; }
  .ln { font-size: var(--sm); color: var(--dim); }
  .sec { display: flex; flex-direction: column; gap: 12px; }
  .card { background: var(--card); border-radius: 12px; }
  .list { display: flex; flex-direction: column; gap: 2px; padding: 8px; }
  .arow { display: flex; align-items: center; gap: 12px; height: 46px; padding: 0 10px; border-radius: 8px; transition: background-color .12s ease-out; }
  .arow:hover { background: rgba(255, 255, 255, .05); }
  .n { width: 22px; font-size: var(--sm); color: var(--faint); flex: none; }
  .an { flex: 1; min-width: 0; font-size: var(--md); font-weight: 500; }
  .tm { width: 90px; text-align: right; font-size: var(--sm); color: var(--dim); }
  .rows { display: flex; flex-direction: column; padding: 8px; }
  .btns { display: flex; gap: 8px; }
</style>
