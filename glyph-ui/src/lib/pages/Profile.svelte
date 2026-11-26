<script lang="ts">
  // ui/profile.rs view(): the page of a profile (opened by handle).
  //  - not loaded: back pill + skeleton (banner 220 high radius 14 + 5 rows),
  //    or an empty state (signed out / error with @handle)
  //  - hero: banner 260 high (mx -24, mt -12, rounded top 14, a photo dimmed
  //    to 80 % that fades out over its lower 40 %; without one an accent
  //    gradient) with the back pill at (32, 20)
  //  - name row: items-end gap 20, mt -78, px 8: avatar 112, then [PROFILE]
  //    (mono 11, white 55 %), @handle (36 / 40 bold, white), edit button
  //    (own) at pb 74
  //  - body gap 22: private note, playlists (176 px cards, 160 px art),
  //    recently played (10 song rows), empty state
  // Added for the Svelte profile (types.Profile has them): name, bio, follow
  // button, top artists, top songs; own pictures can be changed right here
  // (crop dialog = pages/profile/CropDialog.svelte).
  import { backend } from "../api/backend";
  import { app } from "../state.svelte";
  import { t, tf } from "../i18n.svelte";
  import { ink } from "../ui.svelte";
  import { themeColors, accentTile } from "../theme";
  import type { Playlist, Profile, Track } from "../api/types";
  import Button from "../components/Button.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import SectionTitle from "../components/SectionTitle.svelte";
  import SongRow from "../components/SongRow.svelte";
  import ListCover from "../components/ListCover.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import SkeletonSwap from "../components/SkeletonSwap.svelte";
  import Icon from "../components/Icon.svelte";
  import ArtistName from "./lists/ArtistName.svelte";
  import Avatar from "./profile/Avatar.svelte";
  import SkeletonRows from "./profile/SkeletonRows.svelte";
  import CropDialog from "./profile/CropDialog.svelte";
  import { crop, pickProfileImage, profileError } from "./profile/crop.svelte";
  import { setTab } from "../settings/settings.svelte";
  import { phone, scrollFx } from "../compact/cstate.svelte";

  /** What the backend sends beyond types.Profile (see api/demo/extra.ts `profile`). */
  type ProfileX = Profile & { private?: boolean; recent?: Track[] };

  const handle = $derived(app.openProfile);
  let page = $state<ProfileX | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let signedOut = $state(false);
  let bannerOk = $state(false);
  let banEl = $state<HTMLImageElement | null>(null);
  let lastBanner: string | null | undefined;

  $effect(() => {
    const h = handle;
    void crop.version;
    if (!h) return;
    let alive = true;
    if (page && page.handle !== h) page = null;
    loading = true; error = null; signedOut = false;
    backend().call<ProfileX | null>("profile", { handle: h }).then(
      (p) => { if (!alive) return; loading = false; page = p ?? null; if (!p) error = t("profile.not_found"); },
      (e) => {
        if (!alive) return;
        loading = false; page = null;
        signedOut = String((e as Error)?.message ?? e).includes("signed_out");
        error = profileError(e);
      },
    );
    return () => { alive = false; };
  });

  const p = $derived(page && page.handle === handle ? page : null);
  // Phones: like the artist page. The top bar (Compact.svelte) has the only
  // back button; the banner fills the top and zooms in as the page scrolls;
  // the picture sits on the banner's edge and the name goes under it in full.
  const compact = $derived(app.compact);
  const heroZoom = $derived.by(() => {
    if (!compact || scrollFx) return undefined;
    const y = Math.min(400, Math.max(0, phone.scroll));
    return y > 0 ? `translateY(${(y * 0.3).toFixed(1)}px) scale(${(1 + y * 0.000625).toFixed(4)})` : undefined;
  });
  const accent = $derived(themeColors(app.config?.visuals.theme ?? "glyph").accent);
  const own = $derived(!!p?.is_me);
  const priv = $derived(!!p?.private);
  const recent = $derived((p?.recent ?? []).slice(0, 10));
  const sections = $derived.by(() => {
    const raw = p?.sections?.length ? p.sections : own ? app.config.sharing.profile_sections : undefined;
    const out: ("playlists" | "recent")[] = [];
    for (const s of raw ?? []) if ((s === "playlists" || s === "recent") && !out.includes(s)) out.push(s);
    if (!out.includes("playlists")) out.push("playlists");
    if (!out.includes("recent")) out.push("recent");
    return out;
  });
  const listKeyOf = (pl: Playlist) => ({ kind: "album" as const, title: pl.name, artist: `@${p!.handle}`, tracks: pl.tracks });

  $effect(() => {
    const url = p?.banner;
    if (url !== lastBanner) {
      lastBanner = url;
      bannerOk = Boolean(url && banEl?.complete && banEl.naturalWidth > 0);
    }
  });

  async function openPlaylist(pl: Playlist) {
    if (!p) return;
    const h = p.handle;
    const initial = listKeyOf(pl);
    app.openListView(pl.tracks.length ? initial : { ...initial, loading: true }, false);
    // The songs of a profile playlist come from the server.
    try {
      const full = await backend().call<Track[] | null>("profile_playlist", { handle: h, id: pl.id });
      if (app.openList?.kind === "album" && app.openList.artist === `@${h}` && app.openList.title === pl.name) {
        app.openList = { ...app.openList, tracks: full?.length ? full : app.openList.tracks, loading: false };
      }
    } catch (e) {
      if (app.openList?.kind === "album" && app.openList.artist === `@${h}` && app.openList.title === pl.name) {
        app.openList = { ...app.openList, loading: false };
      }
      app.toastInfo(profileError(e));
    }
  }

  function follow() {
    if (!p) return;
    const on = !p.follows;
    p.follows = on;
    p.followers += on ? 1 : -1;
    backend().call("profile_follow", { handle: p.handle, on }).catch((e) => { p!.follows = !on; p!.followers += on ? -1 : 1; app.toastInfo(profileError(e)); });
  }

  function edit() { setTab("account", 1); app.openSettings("account"); }
</script>

{#snippet backPill()}
  {#if app.history.length && !compact}<Button kind="back" icon="arrow-left" onclick={() => app.back()}>{t("common.back")}</Button>{/if}
{/snippet}

<SkeletonSwap loading={!p && !signedOut && !error}>
  {#snippet skeleton()}
    <div class="wrap">
      {#if app.history.length && !compact}<div class="bk">{@render backPill()}</div>{/if}
      <div class="load">
        <Skeleton w="100%" h={220} radius={14} id="profile-banner" />
        <SkeletonRows prefix="profile" n={5} />
      </div>
    </div>
  {/snippet}
  {#if !p}
    <div class="wrap">
      {#if app.history.length && !compact}<div class="bk">{@render backPill()}</div>{/if}
      {#if signedOut}
        <EmptyState icon="music-2" title={t("profile.signed_out")} sub={t("profile.signed_out.sub")} />
      {:else if error}
        <EmptyState icon="refresh-cw" title={error} sub={handle ? `@${handle}` : ""} />
      {/if}
    </div>
  {:else}
  <div class="body" class:mob={compact}>
    <div class="hero" class:mob={compact}>
      {#if !p.banner}
        <div class="grad" class:zoom={compact} class:sd={compact && scrollFx} style:transform={heroZoom} style:background="linear-gradient(0deg, hsla(300, 40%, 16%, 0), {accentTile(0.22, 0.45, accent)})"></div>
      {:else}
        <div class="bmask"><img bind:this={banEl} class="ban" class:on={bannerOk} class:zoom={compact} class:sd={compact && scrollFx} style:transform={heroZoom} src={p.banner} alt="" draggable="false" onload={() => (bannerOk = true)} /></div>
      {/if}
      {#if app.history.length && !compact}<div class="back">{@render backPill()}</div>{/if}
      {#if own}
        <div class="bedit">
          <Button kind="round" icon="image-plus" title={t("profile.image.change")} onclick={() => pickProfileImage(true)} />
        </div>
      {/if}
    </div>

    <div class="name" class:mob={compact}>
      <div class="pic">
        <Avatar src={p.avatar} size={compact ? 96 : 112} />
        {#if own}
          <button class="pen" aria-label={t("profile.image.change")} onclick={() => pickProfileImage(false)}><Icon name="pencil" size={20} color="#fff" /></button>
        {/if}
      </div>
      <div class="who">
        <span class="badge">{t(priv ? "profile.badge.private" : "profile.badge")}</span>
        <h1 class="handle" class:trunc={!compact}>@{p.handle}</h1>
        {#if p.name && p.name.toLowerCase() !== p.handle.toLowerCase()}<div class="pname" class:trunc={!compact}>{p.name}</div>{/if}
      </div>
      {#if own}
        <div class="act"><Button onclick={edit}>{t("profile.edit")}</Button></div>
      {:else}
        <div class="act"><Button kind={p.follows ? "default" : "primary"} onclick={follow}>{t(p.follows ? "artist.following" : "artist.follow")}</Button></div>
      {/if}
    </div>

    {#if p.bio}<div class="bio" use:ink>{p.bio}</div>{/if}
    {#if own && priv}<div class="note" use:ink>{t("profile.own_private")}</div>{/if}

    {#if priv && !own}
      <EmptyState icon="music-2" title={t("profile.private")} sub={t("profile.private.sub")} />
    {:else}
      {#each sections as sec (sec)}
        {#if sec === "playlists" && p.playlists.length}
          <section class="sec">
            <SectionTitle icon="library" title={t("profile.playlists")} />
            <div class="cards">
              {#each p.playlists as pl, i (pl.id)}
                <button class="card" style:animation-delay="{Math.min(i, 8) * 30}ms" onclick={() => openPlaylist(pl)}>
                  {#if pl.cover}
                    <img class="art" src={pl.cover} alt="" draggable="false" />
                  {:else if pl.tracks.length}
                    <ListCover list={listKeyOf(pl)} size={160} />
                  {:else}
                    <div class="art ph"><Icon name="music-2" size={32} color="rgba(255,255,255,.4)" /></div>
                  {/if}
                  <div class="pn trunc">{pl.name}</div>
                  <div class="pc">{tf("common.n_songs", { n: pl.tracks.length })}</div>
                </button>
              {/each}
            </div>
          </section>
        {:else if sec === "recent" && recent.length}
          <section class="sec sm">
            <SectionTitle icon="music-2" title={t("profile.recent")} />
            <div class="rows">{#each recent as _, i (i)}<SongRow tracks={recent} index={i} context={`@${p.handle}`} />{/each}</div>
          </section>
        {/if}
      {/each}

      {#if p.top_artists.length}
        <section class="sec">
          <SectionTitle icon="users" title={t("stats.top_artists")} />
          <div class="artists">
            {#each p.top_artists as a, i (a)}
              <button class="arow" onclick={() => app.openArtistPage(a)}>
                <span class="n mono">{String(i + 1).padStart(2, "0")}</span>
                <span class="an"><ArtistName name={a} badge={14} /></span>
              </button>
            {/each}
          </div>
        </section>
      {/if}

      {#if p.top_tracks.length}
        <section class="sec">
          <SectionTitle icon="trophy" title={t("stats.top_songs")} />
          <div class="rows">{#each p.top_tracks.slice(0, 10) as _, i (i)}<SongRow tracks={p.top_tracks} index={i} context={`@${p.handle}`} />{/each}</div>
        </section>
      {/if}

      {#if !p.playlists.length && !recent.length && !p.top_artists.length && !p.top_tracks.length}
        <EmptyState icon="music-2" title={t("profile.empty")} sub={t(own ? "profile.empty.own" : "profile.empty.sub")} />
      {/if}
    {/if}
  </div>
  {/if}
</SkeletonSwap>

{#if crop.busy}<div class="busy" use:ink>{t("profile.image.uploading")}</div>{/if}
<CropDialog />

<style>
  .wrap { display: flex; flex-direction: column; gap: 16px; }
  .bk { display: flex; }
  .load { display: flex; flex-direction: column; gap: 16px; }
  .body { display: flex; flex-direction: column; gap: 22px; }

  /* hero: full frame bleed, mt -12, h 260, rounded top 14 */
  .hero { position: relative; margin: -12px calc(-1 * var(--page-x)) 0; height: 260px; border-radius: 14px 14px 0 0; overflow: hidden; flex: none; }
  .grad { position: absolute; inset: 0; border-radius: 14px 14px 0 0; }
  /* photo_fade: dimmed to 80 %, alpha smoothstep over the lower 40 % */
  .ban { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; object-position: 50% 30%; opacity: 0; filter: brightness(.8); transition: opacity .5s ease-out;
    -webkit-mask-image: linear-gradient(to bottom, #000 0, #000 60%, rgba(0,0,0,.84) 70%, rgba(0,0,0,.5) 80%, rgba(0,0,0,.16) 90%, transparent 100%);
    mask-image: linear-gradient(to bottom, #000 0, #000 60%, rgba(0,0,0,.84) 70%, rgba(0,0,0,.5) 80%, rgba(0,0,0,.16) 90%, transparent 100%); }
  .ban.on { opacity: 1; }
  .bmask { position: absolute; inset: 0; overflow: hidden; }

  /* Phones: full bleed from the very top (under the status bar), like the
     artist photo; scrolling zooms in (same numbers as the artist page). */
  .hero.mob { margin: calc(-1 * var(--page-top, 8px)) calc(-1 * var(--page-x, 16px)) 0; height: calc(min(240px, 34vh) + var(--safe-top, env(safe-area-inset-top, 0px))); border-radius: 0; }
  .hero.mob .grad { border-radius: 0; }
  .zoom { transform-origin: 50% 30%; will-change: transform; }
  .sd { animation: profile-zoom linear both; animation-timeline: --page; animation-range: 0px 400px; }
  @keyframes profile-zoom { from { transform: none; } to { transform: translateY(120px) scale(1.25); } }
  .name.mob { flex-wrap: wrap; align-items: flex-end; gap: 14px 16px; margin-top: -56px; padding: 0; }
  .name.mob .who { flex: 1 1 100%; order: 2; padding-bottom: 0; gap: 4px; }
  .name.mob .act { order: 1; margin-left: auto; padding-bottom: 6px; }
  .name.mob .handle { font-size: clamp(26px, 8vw, 34px); line-height: 1.12; overflow-wrap: anywhere; }
  .name.mob .pname { font-size: 15px; }
  .name.mob .pic { width: 96px; height: 96px; }
  .body.mob { gap: 18px; }
  .back { position: absolute; top: 20px; left: 32px; }
  .bedit { position: absolute; top: 20px; right: 32px; }

  /* name row: items-end, gap 20, mt -(56 + 22), px 8 */
  /* Picture, name and button on one middle line (the picture used to hang
     below the name because the text was pushed up by its bottom padding). */
  .name { display: flex; align-items: center; gap: 20px; margin-top: -64px; padding: 0 8px; position: relative; }
  .pic { position: relative; flex: none; width: 112px; height: 112px; }
  .pen { position: absolute; inset: 0; border-radius: 50%; display: grid; place-items: center; background: rgba(0, 0, 0, .5); opacity: 0; transition: opacity .15s ease-out; }
  .pic:hover .pen, .pen:focus-visible { opacity: 1; }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; }
  .badge { font-size: var(--sm); font-weight: 500; line-height: 18px; color: rgba(255, 255, 255, .65); }
  .handle { margin: 0; font-size: 36px; line-height: 40px; font-weight: 700; color: #fff; }
  .pname { font-size: var(--md); color: rgba(255, 255, 255, .7); }
  .act { flex: none; }
  .bio { font-size: var(--md); color: var(--dim); max-width: 640px; line-height: 1.4; }
  .note { font-size: var(--sm); color: var(--faint); }

  .sec { display: flex; flex-direction: column; gap: 12px; }
  .sec.sm { gap: 8px; }
  .cards { display: flex; flex-wrap: wrap; gap: 4px; margin-left: -8px; }
  .card { flex: none; width: 176px; padding: 8px; border-radius: 10px; display: flex; flex-direction: column; gap: 4px; transition: background-color .12s ease-out; animation: rise-in .3s var(--ease) both; }
  .card:hover { background: rgba(255, 255, 255, .05); }
  .art { width: 160px; height: 160px; border-radius: 8px; object-fit: cover; }
  .art.ph { background: rgba(255, 255, 255, .06); display: grid; place-items: center; }
  .pn { padding-top: 4px; font-size: var(--md); font-weight: 500; color: var(--text); }
  .pc { font-size: 11px; color: var(--faint); }
  .artists { display: flex; flex-direction: column; gap: 6px; }
  .arow { display: flex; align-items: center; gap: 12px; height: 34px; padding: 0 10px; border-radius: 8px; transition: background-color .12s ease-out; }
  .arow:hover { background: rgba(255, 255, 255, .05); }
  .n { width: 22px; font-size: var(--sm); color: var(--faint); }
  .an { flex: 1; min-width: 0; font-size: var(--md); }
  .rows { display: flex; flex-direction: column; }
  .busy { position: fixed; right: 24px; bottom: 100px; font-size: var(--xs); color: var(--faint); }
</style>
