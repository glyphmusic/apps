<script lang="ts">
  // lists/artist.rs artist_view (desktop). Column gap 22:
  // Hero: full bleed (mx -24, mt -12), h 420, rounded top 14; the photo (or
  // the top song's cover once the info is in) fills it, fading out at the
  // bottom (ascii.rs photo_fade: 80 % brightness, alpha falls from 60 % of
  // the height), rising out of an ASCII shimmer over 0.8 s. Back pill at
  // 20 / 32. Bottom-left (left 32, bottom 40): `[ARTIST]` mono 11 / 14 white
  // 55 % → name 46 / 52 bold white + verified 24 (pt 2, gap 12) → `[ 05
  // SONGS · 12K FANS ]` mono 11.5 white 60 % (pt 6) → actions (pt 18, gap
  // 10): round play 48 (mr 6), shuffle, radio, Follow, more. About card
  // bottom-right (w 340, p 14, radius 12, black 60 %, 1 px white 10 %).
  // Then popular songs (5, "Show more") beside "Liked songs" (w 300, gap
  // 32), releases (one sideways row, "Show all"), similar artists.
  import Icon from "../components/Icon.svelte";
  import SongRow from "../components/SongRow.svelte";
  import PhoneSongRow from "../compact/PhoneSongRow.svelte";
  import ArtistAvatar from "../components/ArtistAvatar.svelte";
  import SectionTitle from "../components/SectionTitle.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import Button from "../components/Button.svelte";
  import Decode from "../components/Decode.svelte";
  import BackPill from "./lists/BackPill.svelte";
  import VerifiedBadge from "./lists/VerifiedBadge.svelte";
  import RoundPlay from "./lists/RoundPlay.svelte";
  import HeroIcon from "./lists/HeroIcon.svelte";
  import FollowButton from "./lists/FollowButton.svelte";
  import ArtistCard from "./lists/ArtistCard.svelte";
  import ReleaseCard from "./lists/ReleaseCard.svelte";
  import SkeletonRows from "./lists/SkeletonRows.svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import { revealK, fadeEdges } from "../ui.svelte";
  import { accentTile, themeColors } from "../theme";
  import { artistRadio, compactCount, groupAlbums, playShuffled, playingFrom, releaseKind } from "./lists/actions";
  import { albumLabel } from "./lists/albumMeta.svelte";
  import type { ListKey, Release, Track } from "../api/types";

  const ROW = 6; // ARTIST_ALBUM_ROW
  const name = $derived(app.openArtist ?? "");
  const page = $derived(app.artists[name.toLowerCase()]);
  const tracks = $derived(page?.tracks ?? []);
  const loading = $derived(page?.loading ?? true);
  const info = $derived(page?.info ?? null);
  const infoLoading = $derived(loading && !info);
  const compact = $derived(app.compact);
  const has = $derived(tracks.length > 0);
  const fans = $derived(info?.fans && info.fans > 0 ? info.fans : null);
  const compactMeta = $derived.by(() => {
    const parts: string[] = [];
    if (fans) parts.push(tf("artist.fans", { n: compactCount(fans) }));
    if (has) parts.push(tf("common.n_songs", { n: tracks.length }));
    return parts.join(" · ");
  });
  const verified = $derived(!!info?.verified);
  let showAll = $state(false);
  let aboutOpen = $state(false);

  // Hero picture: the photo, else the top song's cover (only once the info
  // is in, so it never flashes before the photo).
  let coverUrl = $state<string | null>(null);
  $effect(() => {
    const first = tracks[0];
    coverUrl = null;
    if (!first || info?.photo) return;
    let alive = true;
    backend().cover(first).then((u) => alive && (coverUrl = u));
    return () => { alive = false; };
  });
  const src = $derived(info?.photo ?? (!infoLoading ? coverUrl : null));
  let loaded = $state<string | null>(null);
  $effect(() => {
    const s = src;
    loaded = null;
    if (!s) return;
    let active = true;
    const im = new Image();
    im.onload = () => {
      if (active) loaded = s;
    };
    im.onerror = () => {
      if (active) loaded = null;
    };
    im.src = s;
    return () => {
      active = false;
      im.onload = null;
      im.onerror = null;
      im.src = "";
      loaded = null;
    };
  });
  const heroReady = $derived(!!src && loaded === src);
  const stillLoading = $derived(!heroReady && (!!src || infoLoading || loading));
  const heroE = $derived.by(() => {
    if (!heroReady) return 0;
    const k = revealK(`artist:hero:${name}`, 0.8);
    return 1 - (1 - k) * (1 - k);
  });

  const meta = $derived.by(() => {
    const parts: string[] = [];
    if (has) parts.push(`${String(tracks.length).padStart(2, "0")} ${t("list.songs_caps")}`);
    if (fans) parts.push(tf("artist.fans", { n: compactCount(fans) }));
    return parts.length ? `[ ${parts.join(" · ")} ]` : "";
  });
  // One of this artist's songs is on: the button pauses / resumes it.
  const here = $derived(!!app.player.current && (app.player.current.artist.toLowerCase() === name.toLowerCase() || playingFrom(tracks)));
  function play() { if (here) app.togglePlay(); else app.play(tracks, 0, name); }
  function more(e: PointerEvent) {
    e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: tracks[0] ?? null, artist: name });
  }

  const about = $derived(info?.bio?.trim() ? info.bio : null);
  const wiki = $derived(info?.links?.find((x) => /wikipedia/i.test(x.url) || /wikipedia/i.test(x.label))?.url ?? info?.links?.[0]?.url ?? "");
  const aboutK = $derived.by(() => (about ? revealK(`artist:about:${wiki || name}`, 0.5) : 1));
  const showAbout = $derived(!!about || infoLoading);

  // Your liked songs by this artist.
  const likedList = $derived<ListKey>({ kind: "artist_liked", artist: name });
  const liked = $derived(app.listTracks(likedList));
  const accent = $derived(themeColors(app.config?.visuals.theme ?? "glyph").accent);

  // The catalog's discography when known, else the songs found grouped.
  const releases = $derived(page?.releases ?? []);
  const grouped = $derived(releases.length ? [] : groupAlbums(tracks));
  const seed = (r: Release): Track => ({ id: r.id, source: "deezer", title: r.title, artist: r.artist, album: r.title, cover_url: r.cover_url ?? undefined });
  const similar = $derived(page?.similar ?? []);

  function listMenu(e: MouseEvent) {
    e.preventDefault(); e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: liked[0] ?? null, list: likedList });
  }
  /** Mouse wheel scrolls the sideways rows (widgets smooth_wheel, horizontal). */
  function sideways(e: WheelEvent) {
    const el = e.currentTarget as HTMLElement;
    if (Math.abs(e.deltaX) > Math.abs(e.deltaY) || el.scrollWidth <= el.clientWidth) return;
    const max = el.scrollWidth - el.clientWidth;
    if ((e.deltaY < 0 && el.scrollLeft <= 0) || (e.deltaY > 0 && el.scrollLeft >= max)) return;
    e.preventDefault();
    el.scrollBy({ left: e.deltaY, behavior: "smooth" });
  }
</script>

{#snippet aboutCard(mobile: boolean)}
  <div class="about" class:mob={mobile}>
    <div class="at">{t("artist.about")}</div>
    {#if about}
      <div class="ab scroll" class:open={aboutOpen} style:opacity={0.15 + 0.85 * aboutK} use:fadeEdges={{ top: 16, bottom: 16, minOverflow: 8, active: aboutOpen }}>{about}</div>
      <div class="af">
        <button class="more" onclick={() => (aboutOpen = !aboutOpen)}>{aboutOpen ? t("artist.less") : t("artist.more")}</button>
        {#if wiki}
          <button class="wiki" onclick={() => backend().call("open_url", { url: wiki }).catch(() => {})}>Wikipedia<Icon name="external-link" size={11} /></button>
        {/if}
      </div>
    {:else}
      <Skeleton w={mobile ? 260 : 310} h={64} radius={6} id="about-sk-{name}" />
    {/if}
  </div>
{/snippet}

<div class="ap" class:mob={compact}>
  <section class="hero" class:mob={compact}>
    {#if heroReady}
      <img class="photo" class:mob={compact} src={loaded} alt="" style:opacity={0.1 + 0.9 * heroE} />
      {#if heroE < 1}<div class="sk" style:opacity={1 - heroE}><Skeleton w="100%" h="100%" radius={compact ? 0 : 14} id="hero-rv-{name}" /></div>{/if}
    {/if}
    {#if stillLoading}<div class="sk"><Skeleton w="100%" h="100%" radius={compact ? 0 : 14} id="hero-sk-{name}" /></div>{/if}
    {#if compact}<div class="scrim"></div>{/if}
    <div class="back"><BackPill /></div>
    <div class="hinfo" class:mob={compact} style:right={compact ? undefined : `${showAbout ? 400 : 32}px`}>
      {#if !compact}<span class="badge mono"><Decode text="[{t('artist.badge')}]" key="artist:badge:{name}" dur={0.4} /></span>{/if}
      <div class="nrow">
        <h1 class="name" class:trunc={!compact}><Decode text={name} key="artist:name:{name}" dur={0.7} /></h1>
        {#if verified}<VerifiedBadge size={compact ? 20 : 24} />{/if}
      </div>
      {#if !compact}
        {#if meta}<span class="hmeta mono">{meta}</span>{/if}
        <div class="hacts">
          {#if has}
            <span class="pl"><RoundPlay size={48} playing={here && app.playing} loading={here && app.loading} onclick={play} /></span>
            <HeroIcon name="shuffle" onclick={() => playShuffled(tracks, name)} />
            <HeroIcon name="radio" onclick={() => artistRadio(name)} />
            <FollowButton {name} />
            <HeroIcon name="ellipsis-vertical" onpointerdown={more} />
          {:else}
            <FollowButton {name} />
          {/if}
        </div>
      {/if}
    </div>
    {#if showAbout && !compact}
      {@render aboutCard(false)}
    {/if}
  </section>

  {#if compact}
    <div class="mob-bar">
      {#if compactMeta}<div class="mob-meta">{compactMeta}</div>{/if}
      <div class="mob-acts">
        <FollowButton {name} h={34} />
        {#if has}
          <HeroIcon name="radio" onclick={() => artistRadio(name)} />
          <span class="flex1"></span>
          <HeroIcon name="shuffle" onclick={() => playShuffled(tracks, name)} />
          <RoundPlay size={48} accent playing={here && app.playing} loading={here && app.loading} onclick={play} />
        {/if}
      </div>
    </div>
  {/if}

  <div class="top" class:mob={compact}>
    <div class="pop">
      <SectionTitle icon="music-2" title={t("artist.popular")} />
      {#if !has && loading}
        <SkeletonRows prefix="artist" n={5} />
      {:else if !has}
        <EmptyState title={t("artist.empty")} sub={t("artist.empty.sub")} />
      {:else}
        <div class="rows">
          {#each tracks.slice(0, showAll ? tracks.length : 5) as _, i (i)}
            {#if compact}
              <PhoneSongRow {tracks} index={i} context={name} number={i + 1} />
            {:else}
              <SongRow {tracks} index={i} context={name} selected={app.selectedRow === i} />
            {/if}
          {/each}
        </div>
        {#if tracks.length > 5}
          <div class="all"><Button onclick={() => (showAll = !showAll)}>{showAll ? t("artist.show_less") : t("artist.show_more_n")}</Button></div>
        {/if}
      {/if}
    </div>
    {#if liked.length}
      <div class="liked" class:mob={compact}>
        {#if !compact}<SectionTitle icon="heart" title={t("artist.liked")} />{/if}
        <button class="lcard" class:mob={compact} onclick={() => app.openListView(likedList)} oncontextmenu={listMenu}>
          {#if compact}
            <span class="l-av">
              <ArtistAvatar {name} size={48} />
              <span class="l-badge"><Icon name="heart-fill" size={11} color="#0a0a0a" /></span>
            </span>
          {:else}
            <span class="ltile" style:background={accentTile(0.22, 0.45, accent)}><Icon name="heart-fill" size={20} color="var(--accent)" /></span>
          {/if}
          <span class="ltx">
            <span class="ln">{compact ? t("artist.liked") : tf("artist.liked_n", { n: liked.length })}</span>
            <span class="lb trunc">{compact ? `${tf("common.n_songs", { n: liked.length })} · ${name}` : tf("artist.by", { name })}</span>
          </span>
          {#if compact}<span class="flex1"></span><Icon name="chevron-right" size={18} color="var(--dim)" />{/if}
        </button>
      </div>
    {/if}
  </div>

  {#if releases.length || grouped.length}
    <div class="sect albums">
      <div class="sh">
        <SectionTitle icon="disc-3" title={t("artist.albums")} />
        <span class="flex1"></span>
        <Button kind="text" onclick={() => app.navigate("releases")}>{t("artist.show_all")}</Button>
      </div>
      <div class="side" onwheel={sideways} use:fadeEdges={{ axis: "x", size: 28, minOverflow: 16 }}>
        {#if releases.length}
          {#each releases.slice(0, ROW) as r (r.id)}
            <ReleaseCard title={r.title} artist={r.artist} tracks={[seed(r)]} label={r.year ? `${r.year} · ${releaseKind(r.kind)}` : releaseKind(r.kind)} onopen={() => app.openAlbum(r.title, r.artist, [], seed(r))} />
          {/each}
        {:else}
          {#each grouped.slice(0, ROW) as g (g.title)}
            <ReleaseCard title={g.title} artist={name} tracks={g.tracks} label={albumLabel(g.title, name, g.tracks.length)} onopen={() => app.openListView({ kind: "album", title: g.title, artist: name, tracks: g.tracks })} />
          {/each}
        {/if}
      </div>
    </div>
  {/if}

  {#if similar.length}
    <div class="sect sim">
      <div class="sh">
        <SectionTitle icon="user-round" title={t("artist.similar")} />
        <span class="flex1"></span>
        <Button kind="text" onclick={() => app.navigate("similar")}>{t("artist.show_all")}</Button>
      </div>
      <div class="side" onwheel={sideways} use:fadeEdges={{ axis: "x", size: 28, minOverflow: 16 }}>
        {#each similar as n (n)}<ArtistCard name={n} />{/each}
      </div>
    </div>
  {/if}

  {#if compact && showAbout}
    {@render aboutCard(true)}
  {/if}
</div>

<style>
  .ap { display: flex; flex-direction: column; gap: 22px; padding-bottom: 20px; }
  .ap.mob { gap: 18px; }
  .hero { position: relative; margin: -12px -24px 0; height: 420px; border-radius: 14px 14px 0 0; overflow: hidden; flex: none; }
  .hero.mob { margin: calc(-1 * var(--page-top, 8px)) calc(-1 * var(--page-x, 16px)) 0; height: min(320px, 44vh); min-height: 240px; border-radius: 0; }
  /* ascii.rs photo_fade: crop at 30 % from the top, 80 % brightness, alpha
     fades (smoothstep) from 60 % of the height to the bottom. */
  .photo { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; object-position: center 30%; border-radius: 14px 14px 0 0; filter: brightness(.8);
    -webkit-mask-image: linear-gradient(#000 60%, rgba(0,0,0,.84) 68%, rgba(0,0,0,.5) 80%, rgba(0,0,0,.16) 92%, transparent);
    mask-image: linear-gradient(#000 60%, rgba(0,0,0,.84) 68%, rgba(0,0,0,.5) 80%, rgba(0,0,0,.16) 92%, transparent); }
  .photo.mob { object-position: center 22%; border-radius: 0; filter: brightness(.9); }
  .scrim { position: absolute; inset: 0; pointer-events: none; background: linear-gradient(to bottom, rgba(0,0,0,.28) 0%, transparent 30%, rgba(0,0,0,.38) 64%, rgba(0,0,0,.5) 82%, transparent 100%); }
  .sk { position: absolute; inset: 0; }
  .back { position: absolute; top: 20px; left: 32px; }
  .hinfo { position: absolute; left: 32px; bottom: 40px; display: flex; flex-direction: column; min-width: 0; }
  .hinfo.mob { left: var(--page-x, 16px); right: var(--page-x, 16px); bottom: 16px; }
  .badge { font-size: 11px; line-height: 14px; color: rgba(255, 255, 255, .55); }
  .nrow { padding-top: 2px; display: flex; align-items: center; gap: 12px; min-width: 0; }
  .name { margin: 0; font-size: 46px; line-height: 52px; font-weight: 700; color: #fff; }
  .hinfo.mob .name { font-size: clamp(28px, 8.5vw, 38px); line-height: 1.08; display: -webkit-box; line-clamp: 2; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .hmeta { padding-top: 6px; font-size: 11.5px; color: rgba(255, 255, 255, .6); }
  .hacts { padding-top: 18px; display: flex; align-items: center; gap: 10px; }
  .mob-bar { display: flex; flex-direction: column; gap: 12px; }
  .mob-meta { font-size: 13px; color: var(--dim); font-weight: 500; }
  .mob-acts { display: flex; align-items: center; gap: 12px; }
  .pl { margin-right: 6px; display: flex; }
  .about { position: absolute; right: 32px; bottom: 40px; width: 340px; padding: 14px; border-radius: 12px; background: rgba(0, 0, 0, .6); border: 1px solid rgba(255, 255, 255, .1); display: flex; flex-direction: column; gap: 8px; }
  .about.mob { position: static; width: 100%; background: var(--card); border-color: rgba(255, 255, 255, .07); }
  .at { font-size: 14px; font-weight: 600; color: rgba(255, 255, 255, .95); }
  .ab { font-size: 12.5px; line-height: 18px; color: rgba(255, 255, 255, .72); max-height: 72px; overflow: hidden; user-select: text; transition: max-height .25s var(--ease); white-space: pre-line; }
  .ab.open { max-height: 198px; overflow-y: auto; }
  .af { display: flex; align-items: center; justify-content: space-between; }
  .more { font-size: 12px; font-weight: 500; color: rgba(255, 255, 255, .6); transition: color .12s; }
  .more:hover { color: rgba(255, 255, 255, .95); }
  .wiki { display: flex; align-items: center; gap: 4px; font-size: 11.5px; color: rgba(255, 255, 255, .45); transition: color .12s; }
  .wiki:hover { color: rgba(255, 255, 255, .85); }
  .top { display: flex; align-items: flex-start; gap: 32px; }
  .top.mob { flex-direction: column; align-items: stretch; gap: 20px; }
  .pop { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
  .rows { display: flex; flex-direction: column; }
  .all { display: flex; padding-top: 4px; }
  .liked { flex: none; width: 300px; display: flex; flex-direction: column; gap: 12px; }
  .liked.mob { order: -1; width: 100%; }
  .lcard { display: flex; align-items: center; gap: 14px; padding: 10px; border-radius: 8px; background: var(--card); transition: background-color .12s ease-out; }
  .lcard.mob { padding: 8px 10px; border-radius: 10px; }
  .l-av { position: relative; flex: none; }
  .l-badge { position: absolute; right: -3px; bottom: -3px; width: 20px; height: 20px; border-radius: 50%; background: var(--accent); border: 2px solid #0e0e0e; display: grid; place-items: center; }
  .lcard:hover { background: linear-gradient(rgba(255, 255, 255, .04), rgba(255, 255, 255, .04)), var(--card); }
  .ltile { flex: none; width: 56px; height: 56px; border-radius: 8px; display: grid; place-items: center; }
  .ltx { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .ln { font-size: var(--md); font-weight: 500; color: rgba(255, 255, 255, .95); }
  .lb { font-size: var(--sm); color: rgba(255, 255, 255, .55); }
  .sect { display: flex; flex-direction: column; }
  .albums { gap: 12px; }
  .sim { gap: 10px; }
  .sh { display: flex; align-items: center; }
  .sh :global(.text) { font-size: var(--sm); font-weight: 500; }
  .side { display: flex; gap: 4px; margin: 0 calc(-1 * var(--page-x)) 0 -8px; padding-right: var(--page-x); overflow-x: auto; overflow-y: hidden; scrollbar-width: none; }
  .ap.mob .side { margin: 0 calc(-1 * var(--page-x)); padding: 0 var(--page-x); }
  .side::-webkit-scrollbar { display: none; }
</style>
