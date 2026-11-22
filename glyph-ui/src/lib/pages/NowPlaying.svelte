<script lang="ts">
  // now_playing.rs now_playing: Liner layout. One centred block (px 40, max
  // 1180, gap 56) of the left column (w 380, gap 16: cover, title / artist,
  // seek, transport, volume row) and the lyrics. The cover grows with tall
  // windows (vh x .36, 280..360; zen: vh - 290, max vw x .6, 240..640),
  // radius 12. Meta: title 16 semibold (zen 20) with the E badge, below it
  // [DZ] badge, artist link md dim and [PREVIEW 0:30]. Seek: times mono xs
  // dim, thin bar (cover - 30). Transport (gap 14): shuffle 17, prev 19, play
  // 44 white circle, next 19, repeat 17; right click opens the control menus.
  // Volume row (gap 10): mute, bar 150, like 16 pink, sleep timer, download
  // (ring / pink check), zen. Lyrics only: a centred column (max 820, the
  // player bar does transport). Zen: only cover + meta, exit button top
  // right. Nothing playing: empty state.
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import ToggleButton from "../components/ToggleButton.svelte";
  import ThinBar from "../components/ThinBar.svelte";
  import Cover from "../components/Cover.svelte";
  import SongTitle from "../components/SongTitle.svelte";
  import Badges from "../components/Badges.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import PlayGlyph from "../components/PlayGlyph.svelte";
  import DownloadBadge from "../components/DownloadBadge.svelte";
  import Lyrics from "./nowplaying/Lyrics.svelte";
  import ModeSwitch from "./nowplaying/ModeSwitch.svelte";
  import LyricsSource from "./nowplaying/LyricsSource.svelte";
  import VideoView from "./nowplaying/VideoView.svelte";
  import { app, type ControlMenu } from "../state.svelte";
  import { saveMenu } from "./lists/actions";
  import { t, tf } from "../i18n.svelte";
  import { fmtTime } from "../fmt";
  import { keyLabel, keyOf } from "../keys";
  import { press, tip } from "../ui.svelte";
  import { rating, trackKey } from "../api/types";

  let vw = $state(1200), vh = $state(800);
  const cur = $derived(app.player.current);
  const zen = $derived(app.zen);
  const cover = $derived(zen ? Math.min(Math.max(Math.min(vh - 290, vw * 0.6), 240), 640) : Math.min(360, Math.max(280, vh * 0.36)));
  const pos = $derived(app.position);
  const dur = $derived(app.player.duration);
  const frac = $derived(dur ? pos / dur : 0);
  const key = $derived(cur ? trackKey(cur) : "");
  const dl = $derived(key in app.downloading ? "busy" : cur && app.isDownloaded(cur) ? "done" : "none");
  const prog = $derived(app.downloading[key] ?? null);
  const dlTip = $derived(
    dl === "busy"
      ? `${prog == null ? t("tip.dl_song_prepare") : prog >= 0.995 ? t("tip.dl_song_finish") : tf("tip.dl_song_busy", { pct: Math.round(prog * 100) })} · ${t("ctx.cancel_download")}`
      : dl === "done" ? t("tip.dl_song_done") : (keyOf("download") ? tf("np.download.key", { key: keyLabel(keyOf("download")) }) : t("tip.download")),
  );
  const zenKey = $derived(keyLabel(keyOf("zen")));
  const zenTip = $derived(zen ? (zenKey ? tf("np.zen.exit_key", { key: zenKey }) : t("np.zen.exit")) : (zenKey ? tf("np.zen.key", { key: zenKey }) : t("np.zen")));

  function menu(m: ControlMenu) {
    return (e: MouseEvent) => { e.preventDefault(); e.stopPropagation(); app.closeMenus(); app.controlMenu = { menu: m, x: e.clientX, y: e.clientY }; };
  }
  function wheelSeek(n: number) { const s = app.config.ui.wheel_seek_s; if (s && dur) app.seekRelative(n * s); }
  function wheelVol(n: number) {
    const s = app.config.ui.wheel_volume_pct / 100;
    if (!s) return;
    app.setVolume(Math.min(1, Math.max(0, Math.round((app.volume + n * s) / s) * s)));
  }
</script>

<svelte:window bind:innerWidth={vw} bind:innerHeight={vh} />

{#snippet meta()}
  {#if cur}
    <div class="meta">
      <div class="title" class:zen><SongTitle rating={rating(cur)}>{cur.title}</SongTitle></div>
      <div class="by">
        <Badges track={cur} />
        <button class="artist trunc" onclick={() => app.openArtistPage(cur.artist)}>{cur.artist}</button>
        {#if app.player.preview}<span class="pv mono">[PREVIEW 0:30]</span>{/if}
      </div>
    </div>
  {/if}
{/snippet}

{#if !cur}
  <div class="empty"><EmptyState title={t("np.empty.title")} sub={t("np.empty.sub")} /></div>
{:else if zen}
  <div class="zen">
    <Cover track={cur} size={cover} radius={12} />
    {@render meta()}
    <div class="exit"><IconButton name="minimize-2" size={16} label={zenTip} onclick={() => (app.zen = false)} /></div>
  </div>
{:else}
  <div class="page">
    {#if !app.compact}
      <div class="top">
        <ModeSwitch />
        {#if app.showLyrics && !app.videoMode}
          <LyricsSource />
        {/if}
      </div>
    {/if}
    {#if app.videoMode}
      <VideoView />
    {:else if app.showLyrics && app.lyricsOnly}
      <div class="root only"><div class="col"><Lyrics compact={app.compact} /></div></div>
    {:else}
      <div class="root">
        <div class="wrap">
          <div class="left">
        <Cover track={cur} size={cover} radius={12} />
        {@render meta()}
        <div class="seek">
          <span class="tm mono">{fmtTime(pos)}</span>
          <ThinBar width={cover - 30} value={frac} live={false} onpick={(f) => dur && app.seek(f * dur)} label={dur ? (f) => fmtTime(f * dur) : undefined} onwheelstep={wheelSeek} />
          <span class="tm mono">{dur != null ? fmtTime(dur) : "-:--"}</span>
        </div>
        <div class="tr">
          <ToggleButton name="shuffle" size={17} on={app.player.shuffle || app.controlMenu?.menu === "shuffle"} accent="var(--accent)" onclick={() => app.toggleShuffle()} oncontextmenu={menu("shuffle")} />
          <span use:press><IconButton name="skip-back" size={19} color="var(--text)" onclick={() => app.previous()} oncontextmenu={menu("back")} /></span>
          <button class="play" use:press aria-label={app.playing ? t("common.pause") : t("common.play")} onclick={() => app.togglePlay()}>
            <PlayGlyph loading={app.loading} playing={app.playing} size={18} button={44} />
          </button>
          <span use:press><IconButton name="skip-forward" size={19} color="var(--text)" onclick={() => app.next()} oncontextmenu={menu("forward")} /></span>
          <ToggleButton name={app.player.repeat === "one" ? "repeat-1" : "repeat"} size={17} on={app.player.repeat !== "off" || app.controlMenu?.menu === "repeat"} accent="var(--accent)" onclick={() => app.cycleRepeat()} oncontextmenu={menu("repeat")} />
        </div>
        <div class="vol">
          <IconButton name={app.volume <= 0.001 ? "volume-x" : "volume-2"} size={16} color={app.controlMenu?.menu === "volume" ? "var(--accent)" : undefined} onclick={() => app.toggleMute()} oncontextmenu={menu("volume")} />
          <ThinBar width={150} value={app.volume} onpick={(f) => app.setVolume(f)} label={(f) => `${Math.round(f * 100)}%`} onwheelstep={wheelVol} />
          <ToggleButton name="heart" size={16} on={app.isLiked(cur)} accent="var(--accent)" onclick={() => app.toggleLike(cur)} oncontextmenu={saveMenu(cur)} />
          <ToggleButton name="timer" size={16} on={app.sleep != null || app.controlMenu?.menu === "sleep"} accent="var(--accent)" onclick={menu("sleep")} oncontextmenu={menu("sleep")} />
          <IconButton name="tv" size={16} label={t("np.mode.video")} onclick={() => { app.lyricsOnly = false; app.videoMode = true; }} />
          <IconButton name="arrow-right-left" size={15} label={t("tip.wrong_song")} onclick={() => app.openWrongSong(cur)} />
          {#if app.config.ui.np_download}
            {#if dl === "busy"}
              <button type="button" class="dl dl-cancel" use:tip={dlTip} aria-label={t("ctx.cancel_download")} onclick={() => app.cancelDownloads([cur])}><DownloadBadge state="busy" progress={prog} size={16} arrow="arrow-down" cancellable /></button>
            {:else if dl === "done"}
              <span class="dl" use:tip={dlTip}><DownloadBadge state="done" size={16} /></span>
            {:else}
              <IconButton name="download" size={16} label={dlTip} onclick={() => app.download([cur])} />
            {/if}
          {/if}
          <IconButton name="maximize-2" size={15} label={zenTip} onclick={() => (app.zen = true)} />
        </div>
      </div>
      {#if app.showLyrics}<Lyrics compact={app.compact} />{/if}
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .empty { width: 100%; height: 100%; display: flex; align-items: center; justify-content: center; }
  .empty :global(.es) { align-self: center; }
  .page { width: 100%; height: 100%; display: flex; flex-direction: column; min-height: 0; }
  .top { position: relative; z-index: 20; flex: none; display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 16px var(--page-x) 4px; }
  .root { flex: 1; min-height: 0; width: 100%; display: flex; justify-content: center; padding: 0 40px; }
  .wrap { width: 100%; max-width: 1180px; height: 100%; display: flex; align-items: center; gap: 56px; }
  .only .col { width: 100%; max-width: 820px; height: 100%; display: flex; }
  .left { flex: none; width: 380px; display: flex; flex-direction: column; align-items: center; gap: 16px; }
  .meta { display: flex; flex-direction: column; align-items: center; gap: 4px; max-width: 100%; }
  .title { font-size: 16px; font-weight: 600; max-width: 100%; display: flex; justify-content: center; }
  .title.zen { font-size: 20px; }
  .by { display: flex; align-items: center; gap: 6px; min-width: 0; max-width: 100%; font-size: var(--md); color: var(--dim); }
  .artist:hover { color: var(--text); text-decoration: underline; }
  .pv { flex: none; font-size: var(--xs); color: var(--warn); }
  .seek { display: flex; align-items: center; gap: 10px; }
  .tm { font-size: var(--xs); color: var(--dim); }
  .tr { display: flex; align-items: center; gap: 14px; }
  .play { flex: none; width: 44px; height: 44px; border-radius: 50%; background: var(--text); display: grid; place-items: center; transition: opacity .12s; }
  .play:hover { opacity: .88; }
  .play:global(.pressing) { animation: press .25s var(--ease); }
  .vol { display: flex; align-items: center; gap: 10px; }
  .dl { flex: none; width: 30px; height: 30px; display: grid; place-items: center; border-radius: 8px; transition: background-color .12s; }
  .dl:hover { background: var(--hover); }
  .zen { position: relative; width: 100%; height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 20px; }
  .exit { position: absolute; top: 8px; right: 16px; }
  /* phones: the cover column stacks over the lyrics */
  @media (max-width: 760px) {
    .root { padding: 0 16px; }
    .wrap { flex-direction: column; justify-content: center; gap: 24px; }
    .left { width: 100%; }
  }
</style>
