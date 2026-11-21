<script lang="ts">
  // Bottom "Recommended" songs section for own playlists (same suggestion engine
  // as AddSongsPanel). Extracted from ListView.svelte to keep list layout modular.
  import IconButton from "../../components/IconButton.svelte";
  import Button from "../../components/Button.svelte";
  import Cover from "../../components/Cover.svelte";
  import SongTitle from "../../components/SongTitle.svelte";
  import Skeleton from "../../components/Skeleton.svelte";
  import SkeletonSwap from "../../components/SkeletonSwap.svelte";
  import { app } from "../../state.svelte";
  import { backend } from "../../api/backend";
  import { t } from "../../i18n.svelte";
  import { rating, trackKey, type Track } from "../../api/types";
  import type { PlaylistX } from "./actions";

  let {
    own,
    showAlbumCol = true,
    adding = false,
    onaddsongs,
  }: {
    own: PlaylistX;
    showAlbumCol?: boolean;
    adding?: boolean;
    onaddsongs: () => void;
  } = $props();

  const SK_TW = [0.42, 0.55, 0.36, 0.48, 0.6, 0.4];
  const SK_AW = [0.24, 0.3, 0.2, 0.27, 0.33, 0.22];
  const SK_ALW = [0.62, 0.5, 0.72, 0.44, 0.58, 0.66];

  let rawRecs = $state<Track[]>([]);
  let recsLoading = $state(false);
  let recEpoch = $state(0);

  const ownId = $derived(app.config.ui.recommendations !== false ? own.id : "");
  const haveKeys = $derived(new Set((own.tracks ?? []).map(trackKey)));

  $effect(() => {
    const pid = ownId;
    void recEpoch;
    rawRecs = [];
    if (!pid) { recsLoading = false; return; }
    let alive = true;
    recsLoading = true;
    backend().call<Track[]>("add_suggestions", { id: pid }).then((v) => {
      if (!alive) return;
      rawRecs = (v ?? []).filter((x) => app.allowed(x));
      recsLoading = false;
    }).catch(() => { if (alive) recsLoading = false; });
    return () => { alive = false; };
  });

  const recTracks = $derived(rawRecs.filter((x) => !haveKeys.has(trackKey(x))).slice(0, 6));
  function addRec(tr: Track) { app.addToPlaylist(own.id, [tr]); }
  function refreshRecs() { recEpoch++; }
</script>

<section class="recs" aria-label={t("playlist.recs.title")}>
  <div class="recs-head">
    <div class="recs-ht">
      <h2 class="recs-title">{t("playlist.recs.title")}</h2>
      <p class="recs-sub">{t("playlist.recs.sub")}</p>
    </div>
    <div class="recs-acts">
      <Button kind={adding ? "primary" : "default"} icon="circle-plus" onclick={onaddsongs}>{t("add.title")}</Button>
      <IconButton name="refresh-cw" size={15} box={32} label={t("plugins.refresh")} onclick={refreshRecs} />
    </div>
  </div>
  <SkeletonSwap loading={recsLoading && !recTracks.length}>
    {#snippet skeleton()}
      <div class="recs-sk" aria-busy="true">
        {#each Array(4) as _, i}
          <div class="rec-row sk-rec">
            <Skeleton w={40} h={40} radius={6} id="recs-sk-c{i}" />
            <span class="rec-tx sk-tx">
              <span style:width="{SK_TW[i % 6] * 100}%"><Skeleton w="100%" h={12} radius={4} id="recs-sk-t{i}" /></span>
              <span style:width="{SK_AW[i % 6] * 100}%"><Skeleton w="100%" h={10} radius={4} id="recs-sk-a{i}" /></span>
            </span>
            {#if showAlbumCol}
              <span class="rec-al"><span style:width="{SK_ALW[i % 6] * 100}%"><Skeleton w="100%" h={10} radius={4} id="recs-sk-al{i}" /></span></span>
            {/if}
            <Skeleton w={56} h={28} radius={99} id="recs-sk-b{i}" />
          </div>
        {/each}
      </div>
    {/snippet}
    {#if recTracks.length}
      <div class="recs-list">
        {#each recTracks as tr, i (`${trackKey(tr)}:${i}`)}
          <div class="rec-row" role="row" tabindex="-1" onclick={() => app.play(recTracks, i)} onkeydown={(e) => e.key === "Enter" && app.play(recTracks, i)}>
            <Cover track={tr} size={40} radius={6} />
            <span class="rec-tx">
              <span class="rec-tl"><SongTitle rating={rating(tr)}>{tr.title}</SongTitle></span>
              <span class="rec-ar">
                <button class="link trunc" onclick={(e) => { e.stopPropagation(); app.openArtistPage(tr.artist); }}>{tr.artist}</button>
              </span>
            </span>
            {#if showAlbumCol}
              <span class="rec-al trunc">
                {#if tr.album}<button class="link trunc" onclick={(e) => { e.stopPropagation(); app.openAlbum(tr.album!, tr.artist, [], tr); }}>{tr.album}</button>{/if}
              </span>
            {/if}
            <button type="button" class="rec-add" onclick={(e) => { e.stopPropagation(); addRec(tr); }}>{t("playlist.recs.add")}</button>
          </div>
        {/each}
      </div>
    {/if}
  </SkeletonSwap>
</section>

<style>
  .recs { margin-top: 28px; padding-top: 20px; border-top: 1px solid rgba(255, 255, 255, .08); display: flex; flex-direction: column; gap: 12px; }
  .recs-head { display: flex; align-items: flex-end; justify-content: space-between; flex-wrap: wrap; gap: 12px; padding: 0 10px; }
  .recs-ht { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .recs-title { margin: 0; font-size: calc(18px * var(--k)); font-weight: 700; color: var(--text); }
  .recs-sub { margin: 0; font-size: var(--sm); color: var(--dim); }
  .recs-acts { display: flex; align-items: center; gap: 8px; flex: none; }
  .recs-sk { display: flex; flex-direction: column; }
  .sk-rec { cursor: default; pointer-events: none; }
  .sk-rec:hover { background: none; }
  .sk-tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 7px; }
  .recs-list { display: flex; flex-direction: column; }
  .rec-row { display: flex; align-items: center; gap: 14px; height: 56px; padding: 0 10px; border-radius: 8px; cursor: pointer; transition: background-color .12s ease-out; min-width: 0; }
  .rec-row:hover { background: rgba(255, 255, 255, .05); }
  .rec-tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .rec-tl { font-size: var(--md); color: var(--text); min-width: 0; }
  .rec-ar { display: flex; align-items: center; min-width: 0; font-size: var(--sm); color: var(--dim); }
  .rec-al { flex: 0 1 200px; width: 200px; min-width: 0; display: flex; font-size: var(--sm); color: var(--faint); }
  .link { max-width: 100%; font-weight: 500; color: var(--text); }
  .link:hover { text-decoration: underline; }
  .rec-add { flex: none; height: 30px; padding: 0 14px; border-radius: 99px; background: #222226; font-size: var(--xs); font-weight: 600; color: var(--text); transition: background-color .12s ease-out, transform .12s ease-out; }
  .rec-add:hover { background: #303036; }
  .rec-add:active { transform: scale(.96); }
</style>
