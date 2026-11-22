<script lang="ts">
  // views.rs section_view: a Home section on its own page ("Show all").
  // Column gap 16: floating Back pill (only with history), then a round
  // white play button (44, left of the heading like Spotify) + the heading,
  // then the song list (track rows). While one of these songs is on, the
  // button pauses / resumes it instead of restarting.
  import Button from "../components/Button.svelte";
  import SectionTitle from "../components/SectionTitle.svelte";
  import PlayGlyph from "../components/PlayGlyph.svelte";
  import VirtualRows from "../components/VirtualRows.svelte";
  import TrackRow from "./search/TrackRow.svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { trackKey } from "../api/types";
  import { press, tip } from "../ui.svelte";
  import { tipFor } from "../i18n.svelte";

  const section = $derived(app.section ?? { title: "", icon: "music-2", tracks: [] });
  const tracks = $derived(section.tracks);
  const here = $derived.by(() => {
    const c = app.currentKey;
    return !!c && tracks.some((x) => trackKey(x) === c);
  });
  const playing = $derived(here && app.playing);
  const loading = $derived(here && app.loading);
  const tipText = $derived(tipFor(playing ? "pause" : "play"));
  const go = () => (here ? app.togglePlay() : app.play(tracks, 0, section.title));
  async function saveAsPlaylist() {
    if (!tracks.length) return;
    await app.createPlaylist(section.title, tracks);
    app.toast(t("taste.mix.saved_playlist"), "ok");
  }
</script>

<div class="sv">
  {#if app.history.length}
    <div class="back"><Button kind="back" icon="arrow-left" onclick={() => app.back()}>{t("common.back")}</Button></div>
  {/if}
  <div class="head">
    {#if tracks.length}
      <button class="play" class:nudge={!playing && !loading} use:press use:tip={tipText} aria-label={tipText} onclick={go}>
        <PlayGlyph {loading} {playing} size={44 * 0.42} button={44} />
      </button>
    {/if}
    <div class="htx">
      <SectionTitle heading title={section.title} />
      {#if section.subtitle}
        <div class="sub">{section.subtitle}</div>
      {/if}
      {#if section.tags?.length}
        <div class="tags">
          {#each section.tags as tag (tag)}
            <span class="tag">{tag}</span>
          {/each}
        </div>
      {/if}
    </div>
    {#if section.canSavePlaylist && tracks.length}
      <div class="acts">
        <Button icon="plus" onclick={saveAsPlaylist}>{t("taste.mix.save_playlist")}</Button>
      </div>
    {/if}
  </div>
  <div class="list">
    <VirtualRows count={tracks.length} pitch={58}>
      {#snippet row(i)}<TrackRow {tracks} index={i} kind="section" context={section.title} />{/snippet}
    </VirtualRows>
  </div>
</div>

<style>
  .sv { display: flex; flex-direction: column; gap: 16px; min-width: 0; }
  .back { align-self: flex-start; }
  .head { display: flex; align-items: center; gap: 14px; flex-wrap: wrap; }
  .htx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .sub { font-size: var(--sm); color: var(--dim); }
  .tags { display: flex; flex-wrap: wrap; gap: 6px; padding-top: 2px; }
  .tag { height: 22px; padding: 0 9px; border-radius: 999px; background: rgba(255, 255, 255, .07); font-size: 11.5px; color: var(--dim); display: inline-flex; align-items: center; }
  .acts { flex: none; display: flex; align-items: center; gap: 8px; }
  /* round_play: 44 px white circle, hover black 10 % */
  .play { position: relative; flex: none; width: 44px; height: 44px; border-radius: 50%; background: #fff; display: grid; place-items: center; transition: transform .12s; }
  .play::after { content: ""; position: absolute; inset: 0; border-radius: 50%; background: rgba(0, 0, 0, .1); opacity: 0; transition: opacity .12s; }
  .play:hover::after { opacity: 1; }
  .play.nudge :global(.ic) { margin-left: 2px; }
  .play:global(.pressing) { animation: press .25s var(--ease); }
  .list { display: flex; flex-direction: column; gap: 2px; }
</style>
