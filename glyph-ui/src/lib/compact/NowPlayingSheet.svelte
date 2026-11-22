<script lang="ts">
  // compact/now_playing.rs sheet: slides up over the page, follows the
  // finger down to close. Header (chevron, "Playing from", ⋮) turns into a
  // small bar with cover / like / play once scrolled past the controls.
  // Cover carousel (swipe to skip), current lyric line, title 21 bold +
  // artist, like 24, seek, transport (68 px play), device / share / queue,
  // then the lyrics card (accent tile) and the "Next in queue" card.
  import { onDestroy } from "svelte";
  import Cover from "../components/Cover.svelte";
  import SongTitle from "../components/SongTitle.svelte";
  import ThinBar from "../components/ThinBar.svelte";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import ToggleButton from "../components/ToggleButton.svelte";
  import PlayGlyph from "../components/PlayGlyph.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import IconAction from "./IconAction.svelte";
  import VideoPlayer from "../pages/nowplaying/VideoPlayer.svelte";
  import { videoState } from "../pages/nowplaying/video-state.svelte";
  import { app } from "../state.svelte";
  import { saveMenu } from "../pages/lists/actions";
  import { jam } from "../jam.svelte";
  import { thisIcon, thisName } from "../connect";
  import { t } from "../i18n.svelte";
  import { fmtTime } from "../fmt";
  import { accentTile, themeColors } from "../theme";
  import { rating, splitArtists } from "../api/types";
  import { fadeEdges } from "../ui.svelte";
  import { phone, drag, sheetIn, sheetOut, shouldClose, SIDE, SWIPE_ACT } from "./cstate.svelte";

  let vw = $state(390);
  let bodyH = $state(700);
  let safeBottom = $state(0);
  let scroller: HTMLDivElement | undefined = $state();
  let contentEl: HTMLDivElement | undefined = $state();
  let lyBox: HTMLDivElement | undefined = $state();
  let scrolled = $state(0);
  let pull = $state(0);
  let ax = $state(0);
  const cur = $derived(app.player.current!);
  const lines = $derived(app.lyrics.lyrics?.synced ? app.lyrics.lyrics.lines : []);
  const showLyricLine = $derived(app.config.ui.np_lyric_line !== false);
  const active = $derived.by(() => {
    const ms = app.position * 1000 + (app.lyrics.lyrics?.offset_ms ?? 0);
    let a = -1;
    for (let i = 0; i < lines.length; i++) if ((lines[i].time_ms ?? 0) <= ms) a = i; else break;
    return a;
  });
  const lerp = (a: number, b: number, k: number) => a + (b - a) * k;
  const availH = $derived(Math.max(240, bodyH - safeBottom));
  const c = $derived(Math.min(1, Math.max(0, (660 - availH) / 220)));
  const padTop = $derived(Math.round(lerp(12, 6, c)));
  const lineMt = $derived(Math.round(lerp(18, 8, c)));
  const titleGapNoLine = $derived(Math.round(lerp(24, 12, c)));
  const titleGapWithLine = $derived(Math.round(lerp(10, 6, c)));
  const titleMt = $derived(showLyricLine && active >= 0 ? titleGapWithLine : titleGapNoLine);
  const seekMt = $derived(Math.round(lerp(16, 8, c)));
  const trMt = $derived(Math.round(lerp(10, 4, c)));
  const playBtn = $derived(Math.round(lerp(56, 46, c)));
  const toolsMt = $derived(Math.round(lerp(6, 2, c)));
  const hasLyrics = $derived(app.lyrics.loading || lines.length > 0 || (app.lyrics.lyrics?.lines.length ?? 0) > 0);
  const hasNextCard = $derived(hasLyrics || app.upNext.length > 0);
  const cardMt = $derived(Math.round(hasLyrics ? lerp(22, 10, c) : lerp(12, 8, c)));
  const peek = $derived(Math.round(hasNextCard ? lerp(56, 40, c) : lerp(16, 8, c)));
  const lineReserve = $derived(showLyricLine && lines.length > 0 ? lineMt + 22 + titleGapWithLine - titleGapNoLine : 0);
  const overhead = $derived(
    padTop + titleGapNoLine + 52 + lineReserve + seekMt + 34 + trMt + playBtn + toolsMt + 40 + (hasNextCard ? cardMt : 0) + peek
  );
  const art = $derived(Math.max(96, Math.min(vw - SIDE * 2, availH - overhead)));
  const slideStep = $derived(Math.max(art + SIDE, Math.ceil((vw + art) / 2)));
  const sk = $derived(Math.min(1, Math.max(0, (scrolled - art - (180 - Math.round(c * 24))) / 40)));
  const dur = $derived(app.player.duration ?? 0);
  const frac = $derived(dur ? Math.min(1, app.position / dur) : 0);
  const prev = $derived(app.queue.index != null && app.queue.index > 0 ? app.queue.items[app.queue.index - 1] : null);
  const next = $derived(app.upNext[0]?.track ?? (app.player.repeat === "all" && app.queue.items.length > 1 ? app.queue.items[0] : null));
  const remote = $derived(app.devices.find((d) => d.active && d.kind !== "this"));
  const tile = $derived(accentTile(0.24, 0.5, themeColors(app.config.visuals.theme).accent));
  const close = () => {
    if (phone.videoMode) {
      videoState.leave(true);
      phone.videoMode = false;
      phone.videoPickerOpen = false;
    }
    phone.npOpen = false;
  };
  const fromTarget = $derived(app.playingFromTarget());

  function setVideoMode(on: boolean) {
    if (phone.videoMode === on) return;
    if (!on) {
      videoState.leave(true);
      phone.videoPickerOpen = false;
    }
    phone.videoMode = on;
  }

  $effect(() => {
    if (phone.videoMode && cur) {
      void videoState.ensureForTrack(cur);
    }
  });

  onDestroy(() => {
    if (phone.videoMode) {
      videoState.leave(true);
      phone.videoMode = false;
      phone.videoPickerOpen = false;
    }
  });
  $effect(() => {
    void bodyH;
    if (!contentEl) return;
    const pb = parseFloat(getComputedStyle(contentEl).paddingBottom);
    if (Number.isFinite(pb)) safeBottom = Math.max(0, pb - 32);
  });
  $effect(() => {
    phone.npPull = pull;
    return () => { phone.npPull = 0; };
  });
  $effect(() => {
    const i = active;
    if (i < 0 || !lyBox) return;
    const el = lyBox.children[i] as HTMLElement | undefined;
    if (!el) return;
    lyBox.scrollTo({ top: Math.max(0, el.offsetTop - (lyBox.clientHeight - el.offsetHeight) * 0.42), behavior: "smooth" });
  });
  function carouselEnd(dx: number, _dy: number, vx: number) {
    const can = dx < 0 ? !!next : !!prev;
    if (can && (Math.abs(dx) >= SWIPE_ACT || (dx < 0 ? vx < -0.4 && dx < -18 : vx > 0.4 && dx > 18))) {
      if (dx < 0) app.next();
      else if (app.queue.index != null && app.queue.index > 0) app.queueJump(app.queue.index - 1);
      else app.previous();
    }
    ax = 0;
  }
</script>

<div class="sheet" data-part="now-playing" style:transform="translateY({pull}px)" style:border-radius={pull > 0 ? `${Math.min(22, 10 + pull * 0.3).toFixed(1)}px ${Math.min(22, 10 + pull * 0.3).toFixed(1)}px 0 0` : undefined} class:settle={pull === 0}
  in:sheetIn|global={{ duration: 300 }} out:sheetOut|global={{ duration: 230, radius: true }}
  onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")}>
  <div class="hdr" style:background="rgba(15,15,15,{0.94 * sk})"
    use:drag={{ axis: "y", move: (_, dy) => (pull = Math.max(0, dy)), end: (_, dy, _vx, vy) => { if (shouldClose(dy, vy)) { close(); return; } pull = 0; } }}>
    {#if sk < 0.5}
      <div class="normal" style:opacity={1 - sk * 2}>
        <IconAction name="chevron-down" onclick={close} />
        {#if app.queue.context && fromTarget}
          <!-- "Playing from": a tap opens that playlist, album or artist. -->
          <button class="from link" onclick={() => { if (app.openPlayingFrom()) close(); }}>
            <span class="pf">{t("np.playing_from")}</span>
            <span class="pn-row"><span class="pn trunc">{app.queue.context}</span><Icon name="chevron-right" size={13} color="var(--dim)" /></span>
          </button>
        {:else}
          <div class="from">
            {#if app.queue.context}<span class="pf">{t("np.playing_from")}</span><span class="pn trunc">{app.queue.context}</span>
            {:else}<span class="pn">{t("nav.now_playing")}</span>{/if}
          </div>
        {/if}
        <IconAction name="ellipsis-vertical" onclick={(e) => app.openCtx({ x: e.clientX, y: e.clientY, track: cur, nowPlaying: true })} />
      </div>
    {:else}
      <button class="mb" style:opacity={sk * 2 - 1} onclick={() => scroller?.scrollTo({ top: 0, behavior: "smooth" })}>
        <Cover track={cur} size={40} radius={4} />
        <span class="mt"><span class="t"><SongTitle rating={rating(cur)}>{cur.title}</SongTitle></span>
          {#if remote}<span class="dev"><Icon name="cast" size={12} color="var(--accent)" />{remote.name}</span>{:else}<span class="a trunc">{cur.artist}</span>{/if}</span>
        <ToggleButton name="heart" size={20} on={app.isLiked(cur)} accent="var(--accent)" onclick={() => app.toggleLike(cur)} oncontextmenu={saveMenu(cur)} />
        <span class="p34" onclick={(e) => { e.stopPropagation(); app.togglePlay(); }} onkeydown={() => {}} role="button" tabindex="-1"><PlayGlyph loading={app.loading} playing={app.playing} size={13} button={34} /></span>
        <span class="prog"><i style:width="{frac * 100}%"></i></span>
      </button>
    {/if}
  </div>

  <div class="scroll body" bind:this={scroller} bind:clientWidth={vw} bind:clientHeight={bodyH} use:fadeEdges={{ top: 18, bottom: 18 }} onscroll={() => (scrolled = scroller?.scrollTop ?? 0)}
    use:drag={{ axis: "y", onlyAtTop: () => (scroller?.scrollTop ?? 0) <= 0, move: (_, dy) => (pull = Math.max(0, dy)), end: (_, dy, _vx, vy) => { if (shouldClose(dy, vy)) { close(); return; } pull = 0; } }}>
    <div class="content" bind:this={contentEl} style:padding-top="{padTop}px">
      {#if phone.videoMode}
        <div class="vstage">
          <VideoPlayer compact={true} onOpenPicker={() => (phone.videoPickerOpen = true)} />
        </div>
      {:else}
        <div class="car" style:width="{art}px" style:height="{art}px" use:drag={{ axis: "x", move: (dx) => (ax = (dx > 0 && !prev) || (dx < 0 && !next) ? dx * 0.35 : dx), end: carouselEnd }}>
          <div class="strip" class:settle={ax === 0} style:transform="translateX({ax}px)">
            {#if prev}<span class="c" style:left="{-slideStep}px"><Cover track={prev} size={art} radius={8} /></span>{/if}
            <span class="c" style:left="0"><Cover track={cur} size={art} radius={8} /></span>
            {#if next}<span class="c" style:left="{slideStep}px"><Cover track={next} size={art} radius={8} /></span>{/if}
          </div>
        </div>
      {/if}
      {#if !phone.videoMode && showLyricLine && active >= 0}<button class="line trunc" style:margin-top="{lineMt}px" onclick={() => (phone.lyricsOpen = true)}>{lines[active].text}</button>{/if}
      <div class="title" style:margin-top="{phone.videoMode ? 16 : titleMt}px">
        <span class="tc">
          <span class="tt"><SongTitle rating={rating(cur)}>{cur.title}</SongTitle></span>
          <button class="ar trunc" onclick={() => {
            const list = splitArtists(cur.artist);
            if (list.length > 1) app.openCtx({ x: 0, y: 0, track: cur, artists: list });
            else { app.openArtistPage(cur.artist); close(); }
          }}>{cur.artist}</button>
        </span>
        <ToggleButton name="heart" size={24} on={app.isLiked(cur)} accent="var(--accent)" onclick={() => app.toggleLike(cur)} oncontextmenu={saveMenu(cur)} />
      </div>
      {#if phone.videoMode}
        <div class="vactions">
          <button type="button" class="vact" onclick={() => (phone.videoPickerOpen = true)}>
            <Icon name="search" size={14} />{t("video.switch")}
          </button>
          <button type="button" class="vact ghost" onclick={() => setVideoMode(false)}>
            <Icon name="music-2" size={14} />{t("video.back_to_song")}
          </button>
        </div>
      {:else}
        <div class="seek" style:margin-top="{seekMt}px">
          <ThinBar width="100%" value={frac} live={false} onpick={(f) => dur && app.seek(f * dur)} />
          <div class="times mono"><span>{fmtTime(app.position)}</span><span>{dur ? fmtTime(dur) : ""}</span></div>
        </div>
        <div class="transport" style:margin-top="{trMt}px">
          <button class="side" onclick={() => app.toggleShuffle()} aria-label={t("tip.shuffle")}><Icon name="shuffle" size={19} color={app.player.shuffle ? "var(--accent)" : "var(--dim)"} /></button>
          <IconButton name="skip-back" size={22} box={46} color="var(--text)" notip onclick={() => app.previous()} />
          <button class="big" style:width="{playBtn}px" style:height="{playBtn}px" onclick={() => app.togglePlay()} aria-label={t("common.play")}><PlayGlyph loading={app.loading} playing={app.playing} size={Math.round(playBtn * 0.39)} button={playBtn} /></button>
          <IconButton name="skip-forward" size={22} box={46} color="var(--text)" notip onclick={() => app.next()} />
          <button class="side" onclick={() => app.cycleRepeat()} aria-label={t("tip.repeat")}><Icon name={app.player.repeat === "one" ? "repeat-1" : "repeat"} size={19} color={app.player.repeat !== "off" ? "var(--accent)" : "var(--dim)"} /></button>
        </div>
      {/if}
      <div class="tools" style:margin-top="{toolsMt}px">
        <button class="dv" class:on={!!remote || app.showDevices || app.jamOpen || !!jam.state} onclick={() => (app.showDevices = true)}><Icon name={jam.state ? "users" : remote ? "cast" : thisIcon()} size={16} /><span class="trunc">{remote?.name ?? (app.config.audio.output_device ?? thisName())}</span></button>
        <IconButton name="share-2" size={20} box={40} notip onclick={() => app.share({ track: cur })} />
        <IconButton name="list-music" size={20} box={40} color={app.showQueue ? "var(--accent)" : undefined} notip onclick={() => (app.showQueue = true)} />
      </div>
      {#if !phone.videoMode && hasLyrics}
        <div class="card ly" style:margin-top="{cardMt}px" style:background={tile} onclick={() => (phone.lyricsOpen = true)} onkeydown={() => {}} role="button" tabindex="-1">
          <div class="ch"><b>{t("np.mode.lyrics")}</b><span class="ex"><Icon name="maximize-2" size={15} color="#fff" /></span></div>
          <div class="ly-scroll scroll" bind:this={lyBox} use:fadeEdges={{ top: 20, bottom: 20, minOverflow: 8 }}>
            {#if app.lyrics.loading}
              {#each [0.62, 0.48, 0.7, 0.4] as w}<Skeleton w="{w * 100}%" h={24} radius={6} />{/each}
            {:else if lines.length}
              {#each lines as l, i}
                <div class="ll" class:on={i === active} class:past={i < active}>{l.text.trim() || "♪"}</div>
              {/each}
            {:else}
              <div class="plain">{app.lyrics.lyrics?.lines.map((l) => l.text).join("\n")}</div>
            {/if}
          </div>
        </div>
      {/if}
      {#if app.upNext.length}
        <div class="card q" style:margin-top={!phone.videoMode && hasLyrics ? undefined : `${cardMt}px`}>
          <div class="ch"><b>{t("queue.next")}</b><button class="oq" onclick={() => (app.showQueue = true)}>{t("np.open_queue")}<Icon name="chevron-right" size={14} /></button></div>
          {#each app.upNext.slice(0, 4) as u}
            <button class="up" onclick={() => app.queueJump(u.index)}>
              <Cover track={u.track} size={44} radius={4} />
              <span class="ut"><span class="trunc">{u.track.title}</span><span class="trunc ua">{u.track.artist}</span></span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .sheet { position: fixed; inset: 0; z-index: 40; display: flex; flex-direction: column; overflow: hidden; background: #080808; }
  .sheet.settle { transition: transform .24s var(--ease), border-radius .22s var(--ease); }
  .hdr { position: relative; flex: none; height: calc(52px + var(--safe-top, env(safe-area-inset-top))); padding-top: var(--safe-top, env(safe-area-inset-top)); touch-action: none; }
  .normal { position: absolute; inset: var(--safe-top, env(safe-area-inset-top)) 0 0; padding: 0 4px; display: flex; align-items: center; gap: 4px; }
  .from { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: center; }
  .pf { font-size: 11.5px; color: var(--dim); }
  .pn { max-width: 100%; font-size: 13.5px; font-weight: 700; color: var(--text); }
  .from.link { padding: 2px 8px; border-radius: 8px; transition: background-color .12s ease-out, transform .14s var(--ease); }
  .from.link:active { background: rgba(255, 255, 255, .08); transform: scale(.97); }
  .pn-row { max-width: 100%; min-width: 0; display: flex; align-items: center; gap: 2px; }
  .mb { position: absolute; inset: var(--safe-top, env(safe-area-inset-top)) 0 0; padding: 0 8px 0 16px; display: flex; align-items: center; gap: 10px; text-align: left; }
  .mt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .mt .t { font-size: var(--md); font-weight: 500; min-width: 0; }
  .a { font-size: var(--sm); color: var(--dim); }
  .dev { display: flex; align-items: center; gap: 5px; font-size: 12px; color: var(--accent); }
  .p34 { flex: none; width: 34px; height: 34px; border-radius: 50%; background: #fff; display: grid; place-items: center; }
  .prog { position: absolute; left: 16px; right: 16px; bottom: 0; height: 2px; border-radius: 1px; background: rgba(255, 255, 255, .12); }
  .prog i { display: block; height: 100%; background: #fff; border-radius: inherit; }
  .body { flex: 1; min-height: 0; overscroll-behavior-y: contain; }
  .content { padding: 12px 24px calc(var(--safe-bottom, env(safe-area-inset-bottom)) + 32px); display: flex; flex-direction: column; }
  .vstage { margin: 2px -8px 0; }
  .vactions {
    margin-top: 12px;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .vact {
    flex: 1;
    height: 36px;
    padding: 0 12px;
    border-radius: 9px;
    background: #1b1b20;
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    transition: background-color .14s ease-out;
  }
  .vact:active { background: #26262c; }
  .vact.ghost { background: #141417; color: var(--dim); }
  .car { position: relative; flex: none; align-self: center; overflow: visible; touch-action: pan-y; user-select: none; -webkit-user-select: none; }
  .strip { position: absolute; inset: 0; }
  .strip.settle { transition: transform .24s var(--ease); }
  .c { position: absolute; top: 0; }
  .line { margin-top: 18px; height: 22px; font-size: 15px; font-weight: 700; color: #fff; text-align: left; }
  .title { display: flex; align-items: center; gap: 12px; }
  .tc { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .tt { font-size: 21px; font-weight: 700; min-width: 0; }
  .ar { align-self: flex-start; max-width: 100%; font-size: 15px; color: var(--dim); text-align: left; }
  .ar:active { color: var(--text); }
  .seek { margin-top: 16px; display: flex; flex-direction: column; gap: 4px; }
  .times { display: flex; justify-content: space-between; font-size: 11.5px; color: var(--faint); }
  .transport { margin-top: 10px; display: flex; align-items: center; justify-content: space-between; }
  .side { width: 40px; height: 40px; display: grid; place-items: center; }
  .big { width: 56px; height: 56px; border-radius: 50%; background: #fff; display: grid; place-items: center; }
  .tools { margin-top: 6px; display: flex; align-items: center; gap: 4px; }
  .tools :global(.ib:last-child) { margin-right: -8px; }
  .dv { flex: 1; min-width: 0; height: 40px; display: flex; align-items: center; gap: 8px; font-size: 12.5px; font-weight: 500; color: var(--dim); }
  .dv.on { color: var(--accent); }
  .card { margin-top: 12px; padding: 16px; border-radius: 12px; background: rgba(255, 255, 255, .07); display: flex; flex-direction: column; gap: 10px; }
  .card.ly { margin-top: 22px; height: 252px; gap: 6px; overflow: hidden; }
  .card.q { gap: 4px; }
  .ch { flex: none; display: flex; align-items: center; justify-content: space-between; }
  .ch b { font-size: 16px; color: #fff; }
  .ex { width: 32px; height: 32px; border-radius: 50%; background: rgba(0, 0, 0, .25); display: grid; place-items: center; }
  .ly-scroll { position: relative; flex: 1; min-height: 0; padding: 12px 0 36px; display: flex; flex-direction: column; gap: 10px; scrollbar-width: none; }
  .ly-scroll::-webkit-scrollbar { display: none; }
  .ll { font-size: 19px; line-height: 1.28; font-weight: 700; color: rgba(255, 255, 255, .4); transition: color .24s; }
  .ll.past { color: rgba(255, 255, 255, .62); }
  .ll.on { color: #fff; }
  .plain { white-space: pre-line; font-size: 17px; font-weight: 500; color: rgba(255, 255, 255, .8); }
  .oq { height: 32px; padding-left: 10px; display: flex; align-items: center; gap: 2px; font-size: 12.5px; color: rgba(255, 255, 255, .6); }
  .up { margin: 0 -8px; padding: 8px; border-radius: 8px; display: flex; align-items: center; gap: 12px; text-align: left; }
  .up:active { background: rgba(255, 255, 255, .06); }
  .ut { flex: 1; min-width: 0; display: flex; flex-direction: column; font-size: 14px; color: #fff; }
  .ua { font-size: 12.5px; color: rgba(255, 255, 255, .6); }
</style>
