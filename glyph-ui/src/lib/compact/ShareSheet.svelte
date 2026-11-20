<script lang="ts">
  // Phone share menu (app.shareSheet): a bottom sheet like the queue sheet
  // (handle, dim behind, pull down or tap outside to close). On top a card
  // that shows what is shared (song: big cover, title, artist; list: cover,
  // name, first songs) with the "glyph" mark. Under it round colour dots for
  // the card: colours taken from the cover first, then every accent colour
  // (the one in use first) as a dark tile, then black; the picked one shows
  // a check. The row scrolls sideways when they do not fit. Text turns dark on light
  // colours. Then a row of round targets: copy link, WhatsApp, Telegram,
  // SMS, e-mail and More (system share sheet). The link itself stays hidden;
  // only a failure shows a line with "Try again". Artists show their round
  // photo, a ready link (Jam invite) its icon, title and code.
  // Copying says so on its own button only ("Copied" with a check; Android
  // shows its own copy note too): no toast on top.
  import { fade } from "svelte/transition";
  import Cover from "../components/Cover.svelte";
  import ListCover from "../components/ListCover.svelte";
  import SongTitle from "../components/SongTitle.svelte";
  import Icon from "../components/Icon.svelte";
  import ArtistAvatar from "../components/ArtistAvatar.svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import { rating, type Track } from "../api/types";
  import { accentTile, allThemes, themeColors } from "../theme";
  import { paletteFromImage, type Rgb } from "../glow";
  import { drag, sheetIn, sheetOut, shouldClose } from "./cstate.svelte";

  const what = app.shareSheet!;
  let pull = $state(0);
  let link = $state<string | null>(null);
  let failed = $state(false);
  let copied = $state(false);

  const close = () => (app.shareSheet = null);
  const track = what.track ?? (what.songs?.kind === "song" ? what.songs.tracks[0] : undefined);
  const artist = what.artist;
  const ready = what.link;
  const tracks: Track[] = what.track ? [what.track] : what.list ? app.listTracks(what.list) : what.songs?.tracks ?? [];
  const title = what.track ? what.track.title : what.list ? app.listTitle(what.list) : artist ?? ready?.title ?? what.songs?.title ?? "Glyph";
  const kindLabel = what.list?.kind === "album" || what.songs?.kind === "album" ? t("list.kind.album") : t("share.kind.playlist");
  /** "Title · Artist" for a song, the list name otherwise (mail subject,
   *  share sheet title). Messages carry only the link, so chats show it as
   *  a link with the share page's preview (cover, title). */
  const label = track ? `${track.title} · ${track.artist}` : title;
  /** Card is min(264, screen - 64) wide with 16 px padding; the cover fills it. */
  const art = Math.max(120, Math.min(232, innerWidth - 96));
  const tile = $derived(accentTile(0.24, 0.5, themeColors(app.config.visuals.theme).accent));

  // ---- card colours ----
  const hex = (c: Rgb) => "#" + c.map((v) => Math.round(Math.min(1, Math.max(0, v)) * 255).toString(16).padStart(2, "0")).join("");
  const lum = (c: Rgb) => 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
  let coverCols = $state<Rgb[]>([]);
  /** null: not picked yet (the first cover colour, else the accent tile). */
  let picked = $state<string | null>(null);
  const swatches = $derived([
    // From the cover: a deep and a bright version of its main colour, then its second colour.
    ...coverCols.flatMap((c, i) => (i === 0 ? [hex(c.map((v) => v * 0.45) as Rgb), hex(c)] : [hex(c.map((v) => v * 0.6) as Rgb)])),
    // Every accent colour Glyph has (Settings > Appearance), as a dark tile; the one in use first.
    ...[...new Set([tile, ...allThemes().map((th) => accentTile(0.24, 0.5, th.accent))])],
    "#0a0a0a",
  ]);
  const bg = $derived(picked ?? swatches[0]);
  /** Dark text on light cards (a cover colour can be bright). */
  const light = $derived.by(() => {
    const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(bg);
    return !!m && lum([parseInt(m[1], 16) / 255, parseInt(m[2], 16) / 255, parseInt(m[3], 16) / 255]) > 0.55;
  });
  const first = track ?? tracks[0];
  const photo = artist ? app.artistPhoto(artist) : null;
  if (first || photo) {
    (photo ? Promise.resolve(photo) : backend().cover(first!)).then((u) => (u ? paletteFromImage(u) : null)).then((p) => {
      if (!p) return;
      // Palette is sorted dark to bright: main colour first, then the brightest.
      coverCols = [p[1], p[2]];
    }).catch(() => {});
  }

  let pending: Promise<string | null> = make();
  function make(): Promise<string | null> {
    failed = false;
    link = null;
    const req = ready
      ? Promise.resolve(ready.url)
      : artist
        ? backend().call<string>("copy_artist_link", { artist, copy: false })
        : what.songs
          ? backend().call<string>("share_songs", { ...what.songs, kind: what.songs.kind ?? "playlist", copy: false })
          : backend().call<string>("share", { track: what.track, list: what.list, copy: false });
    return req.then(
      (s) => { const v = s?.trim() || null; link = v; failed = !v; return v; },
      () => { failed = true; return null; },
    );
  }
  function retry() { pending = make(); }

  const enc = encodeURIComponent;
  async function go(run: (l: string) => unknown) {
    const l = link ?? (await pending);
    if (!l) return;
    await run(l);
  }
  const copy = () => go(async (l) => {
    try { await backend().call("copy_text", { text: l }); } catch { return; }
    copied = true;
    setTimeout(close, 900);
  });
  const open = (url: (l: string) => string) => go((l) => { backend().call("open_url", { url: url(l) }); close(); });
  const more = () => go(async (l) => {
    try { await backend().call("share_system", { text: l, title: label }); close(); }
    catch { await copy(); }
  });

  const targets = [
    { id: "copy", icon: "link-2", label: () => (copied ? t("share.copied") : t("ctx.copy_link")), run: copy },
    { id: "wa", icon: "whatsapp", label: () => "WhatsApp", run: () => open((l) => `https://wa.me/?text=${enc(l)}`) },
    { id: "tg", icon: "send", label: () => "Telegram", run: () => open((l) => `https://t.me/share/url?url=${enc(l)}`) },
    { id: "sms", icon: "message-square-more", label: () => t("share.sms"), run: () => open((l) => `sms:?body=${enc(l)}`) },
    { id: "mail", icon: "mail", label: () => t("share.mail"), run: () => open((l) => `mailto:?subject=${enc(label)}&body=${enc(l)}`) },
    { id: "more", icon: "ellipsis", label: () => t("share.more"), run: more },
  ];
</script>

<div class="dim" transition:fade|global={{ duration: 200 }} onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")} onpointerdown={close} role="presentation">
  <div class="panel" style:transform="translateY({pull}px)" class:settle={pull === 0} in:sheetIn|global={{ duration: 260 }} out:sheetOut|global={{ duration: 210 }}
    onpointerdown={(e) => e.stopPropagation()} role="dialog" aria-label={t("share.title")} tabindex="-1"
    use:drag={{ axis: "y", move: (_, dy) => (pull = Math.max(0, dy)), end: (_, dy, _vx, vy) => { if (shouldClose(dy, vy)) { close(); return; } pull = 0; } }}>
    <span class="hdl"></span>

    <div class="card" class:light style:background={bg}>
      {#if artist}
        <div class="art"><ArtistAvatar name={artist} size={art} iconSize={Math.round(art * 0.4)} /></div>
        <div class="meta">
          <b class="trunc">{artist}</b>
          <span class="trunc">{t("share.kind.artist")}</span>
        </div>
      {:else if ready}
        <div class="lhead">
          <span class="ricon"><Icon name={ready.icon ?? "link-2"} size={32} /></span>
          <div class="meta">
            <b class="trunc">{ready.title}</b>
            {#if ready.sub}<span class="trunc mono">{ready.sub}</span>{/if}
          </div>
        </div>
      {:else if track}
        <div class="art"><Cover {track} size={art} radius={8} /></div>
        <div class="meta">
          <b class="trunc"><SongTitle rating={rating(track)}>{track.title}</SongTitle></b>
          <span class="trunc">{track.artist}</span>
        </div>
      {:else}
        <div class="lhead">
          {#if what.list}<ListCover list={what.list} size={72} />{:else}<Cover track={tracks[0]} size={72} radius={6} />{/if}
          <div class="meta">
            <b class="trunc">{title}</b>
            <span class="trunc">{kindLabel} · {tf("share.songs", { n: tracks.length })}</span>
          </div>
        </div>
        <div class="rows">
          {#each tracks.slice(0, 4) as tr}
            <div class="row">
              <Cover track={tr} size={36} radius={4} />
              <span class="rt"><span class="trunc">{tr.title}</span><span class="trunc ra">{tr.artist}</span></span>
            </div>
          {/each}
        </div>
      {/if}
      <span class="mark mono">glyph</span>
    </div>

    <div class="dots">
      {#each swatches as c (c)}
        <button class="dot" class:on={c === bg} style:background={c} onclick={() => (picked = c)} aria-label={t("share.colour")}>
          {#if c === bg}<span class="chk"><Icon name="check" size={16} /></span>{/if}
        </button>
      {/each}
    </div>

    {#if failed}
      <div class="fail"><span>{t("share.failed")}</span><button class="again" onclick={retry}>{t("common.retry")}</button></div>
    {/if}

    <div class="targets scroll">
      {#each targets as x (x.id)}
        <button class="tg" class:wait={!link} class:done={x.id === "copy" && copied} onclick={x.run} disabled={failed}>
          <span class="ic"><Icon name={x.id === "copy" && copied ? "check" : x.icon} size={24} /></span>
          <span class="tl">{x.label()}</span>
        </button>
      {/each}
    </div>
  </div>
</div>

<style>
  .dim { position: fixed; inset: 0; z-index: 60; background: rgba(0, 0, 0, .55); }
  .panel { position: absolute; left: 0; right: 0; bottom: 0; max-height: calc(100vh - var(--safe-top, env(safe-area-inset-top)) - 12px); border-radius: 16px 16px 0 0; background: hsl(0 0% 8%); padding-bottom: calc(14px + var(--safe-bottom, env(safe-area-inset-bottom))); display: flex; flex-direction: column; align-items: center; overflow: hidden; }
  .panel.settle { transition: transform .24s var(--ease); }
  .hdl { flex: none; display: block; width: 36px; height: 4px; margin: 10px auto 14px; border-radius: 2px; background: rgba(255, 255, 255, .25); }

  .card { flex: none; width: min(264px, max(152px, calc(100vw - 64px))); padding: 16px; border-radius: 16px; display: flex; flex-direction: column; gap: 12px; color: #fff; }
  .art { display: grid; place-items: center; }
  .meta { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .meta b { font-size: 17px; font-weight: 700; min-width: 0; }
  .meta span { font-size: 13.5px; color: rgba(255, 255, 255, .7); }
  .lhead { display: flex; align-items: center; gap: 12px; }
  .lhead .meta { flex: 1; }
  .ricon { flex: none; width: 72px; height: 72px; border-radius: 12px; display: grid; place-items: center; background: rgba(255, 255, 255, .12); color: inherit; }
  .rows { display: flex; flex-direction: column; gap: 8px; }
  .row { display: flex; align-items: center; gap: 10px; }
  .rt { flex: 1; min-width: 0; display: flex; flex-direction: column; font-size: 13.5px; }
  .ra { font-size: 12px; color: rgba(255, 255, 255, .62); }
  .mark { align-self: flex-start; font-size: 13px; font-weight: 700; color: #fff; opacity: .9; }
  .card { transition: background-color .25s ease-out, color .25s ease-out; }
  .card.light { color: #111; }
  .card.light .meta span, .card.light .ra { color: rgba(0, 0, 0, .62); }
  .card.light .mark { color: #111; }

  .dots { flex: none; align-self: stretch; margin-top: 14px; padding: 0 16px; display: flex; gap: 10px; overflow-x: auto; scrollbar-width: none; touch-action: pan-x; }
  /* Centred while they fit, scrollable from the first dot once they do not. */
  .dots > :first-child { margin-left: auto; }
  .dots > :last-child { margin-right: auto; }
  .dots::-webkit-scrollbar { display: none; }
  .dot { flex: none; width: 30px; height: 30px; border-radius: 50%; display: grid; place-items: center; transition: transform .14s var(--ease); }
  .dot:active { transform: scale(.9); }
  .chk { display: grid; place-items: center; width: 22px; height: 22px; border-radius: 50%; background: #fff; color: #000; animation: pop .26s var(--ease); }
  .fail { flex: none; height: 32px; margin-top: 8px; display: flex; align-items: center; gap: 10px; font-size: 12.5px; color: var(--dim); }
  .again { height: 28px; padding: 0 12px; border-radius: 999px; background: rgba(255, 255, 255, .1); font-size: 12.5px; font-weight: 600; color: #fff; }

  .targets { flex: none; align-self: stretch; margin-top: 14px; padding: 8px 16px 4px; display: flex; gap: 6px; overflow-x: auto; scrollbar-width: none; touch-action: pan-x; }
  .targets::-webkit-scrollbar { display: none; }
  .tg { flex: none; width: 72px; display: flex; flex-direction: column; align-items: center; gap: 8px; transition: opacity .2s; }
  .tg.wait { opacity: .5; }
  .tg:disabled { opacity: .3; }
  .ic { width: 56px; height: 56px; border-radius: 50%; background: rgba(255, 255, 255, .08); display: grid; place-items: center; color: #fff; transition: background-color .12s ease-out, transform .14s var(--ease); }
  .tg:active .ic { background: rgba(255, 255, 255, .16); transform: scale(.94); }
  .tg.done .ic { background: var(--accent); color: #000; }
  .tl { max-width: 100%; font-size: 12.5px; color: rgba(255, 255, 255, .85); text-align: center; line-height: 1.25; }
</style>
