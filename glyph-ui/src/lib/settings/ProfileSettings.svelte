<script lang="ts">
  // ui/profile.rs settings_page + privacy: Settings > Profile (signed in
  // only). Sub-tabs Profile (picture 40 round, banner 120 x 40, view my
  // profile) and Privacy (public / recent switches, then the playlists
  // shown on the profile: a draggable list, row h 56 with grip, cover 40,
  // name, song count and a switch).
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import { tip } from "../ui.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import Field from "./Field.svelte";
  import Tog from "./Tog.svelte";
  import DragList from "./DragList.svelte";
  import * as A from "./account-state.svelte";
  import Avatar from "../pages/profile/Avatar.svelte";
  import ListCover from "../components/ListCover.svelte";
  import { themeColors, accentTile } from "../theme";
  import { errText } from "./util";
  import type { ProfileSettingsData } from "./types";
  import type { Playlist, ProfileSection } from "../api/types";

  let s = $state<ProfileSettingsData | null>(null);
  let notice = $state<{ text: string; ok: boolean } | null>(null);
  let busy = $state(false);
  let editingHandle = $state(false);
  let draftHandle = $state("");
  const handle = $derived(app.account?.handle ?? "");
  const accent = $derived(themeColors(app.config?.visuals.theme ?? "glyph").accent);

  function normalizeSections(raw?: readonly string[] | null): ProfileSection[] {
    const out: ProfileSection[] = [];
    for (const x of raw ?? []) {
      if ((x === "playlists" || x === "recent") && !out.includes(x)) out.push(x);
    }
    if (!out.includes("playlists")) out.push("playlists");
    if (!out.includes("recent")) out.push("recent");
    return out;
  }

  const sections = $derived(normalizeSections(s?.sections ?? app.config.sharing.profile_sections));

  async function load() {
    try {
      const next = await backend().call<ProfileSettingsData>("profile_settings");
      if (next) {
        s = { ...next, sections: normalizeSections(next.sections?.length ? next.sections : app.config.sharing.profile_sections) };
        app.ownAvatarUrl = next.avatar_url ?? null;
      }
      notice = null;
    }
    catch (e) { notice = { text: errText(e), ok: false }; }
  }
  onMount(load);

  async function image(banner: boolean, remove: boolean) {
    if (busy) return;
    busy = true;
    try { await backend().call(remove ? "profile_remove_image" : "profile_pick_image", { banner }); await load(); }
    catch (e) { notice = { text: errText(e), ok: false }; }
    busy = false;
  }
  async function update(patch: { public?: boolean; show_recent?: boolean; sections?: ProfileSection[] }) {
    if (s) Object.assign(s, patch);
    if (patch.sections) app.config = { ...app.config, sharing: { ...app.config.sharing, profile_sections: patch.sections } };
    try { await backend().call("profile_update", patch); } catch (e) { notice = { text: errText(e), ok: false }; load(); }
  }

  function startHandleEdit() {
    draftHandle = handle;
    editingHandle = true;
  }
  function saveHandle() {
    const h = draftHandle.trim().replace(/^@/, "");
    if (!h || h === handle) { editingHandle = false; return; }
    A.rename(h, () => { editingHandle = false; });
  }

  // Variable-height drag reordering for profile sections (Playlists, Recently played).
  let secWrap: HTMLDivElement | undefined = $state();
  let secFrom = $state<number | null>(null);
  let secSlot = $state(0);
  let secDy = $state(0);
  let secStartY = 0;
  let secRects: { top: number; height: number }[] = [];
  const SEC_GAP = 10;

  function secDown(e: PointerEvent, i: number) {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button, input, [data-nodrag]")) return;
    const nodes = secWrap ? [...secWrap.querySelectorAll<HTMLElement>(":scope > .sec-card")] : [];
    secRects = nodes.map((n) => {
      const r = n.getBoundingClientRect();
      return { top: r.top, height: r.height };
    });
    secFrom = i; secSlot = i; secDy = 0; secStartY = e.clientY;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function secMove(e: PointerEvent) {
    if (secFrom == null) return;
    secDy = e.clientY - secStartY;
    const orig = secRects[secFrom];
    if (!orig) return;
    const cy = orig.top + orig.height / 2 + secDy;
    let target = 0;
    for (let j = 0; j < secRects.length; j++) {
      if (j === secFrom) continue;
      const mid = secRects[j].top + secRects[j].height / 2;
      if (cy > mid) target++;
    }
    secSlot = target;
  }
  function secUp() {
    if (secFrom == null) return;
    const f = secFrom, to = secSlot;
    secFrom = null; secDy = 0;
    if (f !== to) {
      const next = [...sections];
      const [moved] = next.splice(f, 1);
      next.splice(to, 0, moved);
      update({ sections: next });
    }
  }
  function secShift(i: number): number {
    if (secFrom == null) return 0;
    if (i === secFrom) return secDy;
    const h = (secRects[secFrom]?.height ?? 0) + SEC_GAP;
    if (secFrom < i && i <= secSlot) return -h;
    if (secSlot <= i && i < secFrom) return h;
    return 0;
  }

  /** Playlists on the profile first, in their profile order; the rest below. */
  const playlists = $derived.by(() => {
    const order = s?.playlists ?? [];
    const idx = (p: Playlist) => { const i = order.indexOf(p.id); return i < 0 ? Infinity : i; };
    return [...app.library.playlists].sort((a, b) => idx(a) - idx(b));
  });
  function reorder(from: number, to: number) {
    const ids = playlists.map((p) => p.id);
    const [id] = ids.splice(from, 1);
    ids.splice(to, 0, id);
    if (s) s.playlists = ids.filter((x) => s!.playlists.includes(x));
    backend().call("profile_reorder_playlist", { order: ids, from, to });
  }
  function toggle(id: string, on: boolean) {
    if (s) s.playlists = on ? [...s.playlists, id] : s.playlists.filter((x) => x !== id);
    backend().call("profile_toggle_playlist", { id });
  }
</script>

<div class="col">
  {#if !app.account}
    <div class="pad">{t("profile.signed_out.sub")}</div>
  {:else if !s}
    {#if notice}
      <div class="nt warn">{notice.text}</div>
      <div class="ret"><Button onclick={load}>{t("common.retry")}</Button></div>
    {:else}
      <!-- Same shape as the page: profile card, public row, two section cards. -->
      <div class="sk">
        <Skeleton w="100%" h={186} radius={12} id="ps-card" />
        <Skeleton w="100%" h={52} radius={8} id="ps-public" />
        <Skeleton w="100%" h={120} radius={12} id="ps-sec-0" />
        <Skeleton w="100%" h={50} radius={12} id="ps-sec-1" />
      </div>
    {/if}
  {:else}
    <div class="mini" class:busy>
      <div class="mini-hero">
        {#if s.banner_url}
          <img class="mini-ban" src={s.banner_url} alt="" draggable="false" />
        {:else}
          <div class="mini-grad" style:background="linear-gradient(0deg, hsla(300, 40%, 16%, 0), {accentTile(0.22, 0.45, accent)})"></div>
        {/if}
        <button
          class="hero-hit"
          onclick={() => image(true, false)}
          disabled={busy}
          aria-label={t("profile.banner")}
        >
          <span class="hero-ov"><Icon name="image-plus" size={20} color="#fff" /></span>
        </button>
        <div class="hero-acts">
          <button
            class="act-ic"
            onclick={() => image(true, false)}
            disabled={busy}
            use:tip={t(busy ? "profile.image.uploading" : s.banner ? "profile.image.change" : "profile.image.pick")}
            aria-label={t("profile.banner")}
          >
            <Icon name="image-plus" size={14} color="rgba(255,255,255,.88)" />
          </button>
          {#if s.banner}
            <button
              class="act-ic"
              onclick={() => image(true, true)}
              disabled={busy}
              use:tip={t("profile.image.remove")}
              aria-label={t("profile.image.remove")}
            >
              <Icon name="trash-2" size={14} color="var(--danger)" />
            </button>
          {/if}
        </div>
      </div>

      <div class="mini-name">
        <div class="mini-pic">
          <div class="mini-av-ring"><Avatar src={s.avatar_url ?? null} size={76} /></div>
          <button
            class="mini-pen"
            onclick={() => image(false, false)}
            disabled={busy}
            use:tip={t(busy ? "profile.image.uploading" : s.avatar ? "profile.image.change" : "profile.image.pick")}
            aria-label={t("profile.avatar")}
          >
            <Icon name="pencil" size={17} color="#fff" />
          </button>
          {#if s.avatar}
            <button
              class="av-rm"
              onclick={() => image(false, true)}
              disabled={busy}
              use:tip={t("profile.image.remove")}
              aria-label={t("profile.image.remove")}
            >
              <Icon name="trash-2" size={12} color="var(--danger)" />
            </button>
          {/if}
        </div>
        <div class="mini-who">
          <button
            class="mini-badge mono"
            onclick={() => update({ public: !s!.public })}
            use:tip={t("profile.public")}
          >
            [{t(s.public ? "profile.badge" : "profile.badge.private")}]
          </button>
          {#if editingHandle}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="handle-edit" onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); editingHandle = false; } }}>
              <Field bind:value={draftHandle} width={160} height={30} placeholder={t("account.handle.placeholder")} onenter={saveHandle} />
              <Button onclick={saveHandle}>{t("account.handle.save")}</Button>
              <button class="act-ic cancel" onclick={() => (editingHandle = false)} use:tip={t("account.login.cancel")} aria-label={t("account.login.cancel")}>
                <Icon name="x" size={14} color="rgba(255,255,255,.7)" />
              </button>
            </div>
          {:else}
            <button class="mini-handle-btn" onclick={startHandleEdit} use:tip={t("account.handle")}>
              <span class="mini-handle trunc">@{handle}</span>
              <span class="handle-pen"><Icon name="pencil" size={13} color="rgba(255,255,255,.55)" /></span>
            </button>
          {/if}
        </div>
        <Button onclick={() => { app.settingsOpen = false; app.openProfilePage(handle); }}>{t("profile.view.button")}</Button>
      </div>
    </div>

    <div class="priv-box">
      <Rows>
        <Row label={t("profile.public")} info={t("profile.public.desc")}>
          <Tog on={s.public} onchange={(v) => update({ public: v })} label={t("profile.public")} />
        </Row>
      </Rows>
    </div>

    <div class="sections" bind:this={secWrap}>
      {#each sections as sec, i (sec)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="sec-card"
          class:lift={secFrom === i}
          style:transform="translateY({secShift(i)}px)"
        >
          {#if sec === "playlists"}
            <div class="sec-head" onpointerdown={(e) => secDown(e, i)} onpointermove={secMove} onpointerup={secUp} onpointercancel={() => { secFrom = null; secDy = 0; }}>
              <span class="sec-grip"><Icon name="grip-vertical" size={15} color="rgba(255,255,255,.5)" /></span>
              <Icon name="library" size={14} color="rgba(255,255,255,.6)" />
              <span class="sec-title">{t("profile.playlists.section")}</span>
              <span class="sec-sub">{t(s.public ? "profile.playlists.desc" : "profile.playlists.desc_private")}</span>
            </div>
            <div class="sec-body" data-nodrag>
              {#if !playlists.length}
                <div class="pad dim">{t("profile.playlists.none")}</div>
              {:else}
                <DragList items={playlists} pitch={56} key={(p) => p.id} onmove={reorder}>
                  {#snippet row(p)}
                    <div class="pr">
                      <span class="grip"><Icon name="grip-vertical" size={15} color="rgba(255,255,255,.6)" /></span>
                      <span class="cv"><ListCover list={{ kind: "playlist", id: (p as Playlist).id }} size={40} /></span>
                      <span class="tx"><span class="nm">{(p as Playlist).name}</span><span class="ct">{tf("common.n_songs", { n: (p as Playlist).tracks.length })}</span></span>
                      <span data-nodrag><Tog on={s!.playlists.includes((p as Playlist).id)} onchange={(v) => toggle((p as Playlist).id, v)} label={(p as Playlist).name} /></span>
                    </div>
                  {/snippet}
                </DragList>
              {/if}
            </div>
          {:else}
            <div class="sec-head" onpointerdown={(e) => secDown(e, i)} onpointermove={secMove} onpointerup={secUp} onpointercancel={() => { secFrom = null; secDy = 0; }}>
              <span class="sec-grip"><Icon name="grip-vertical" size={15} color="rgba(255,255,255,.5)" /></span>
              <Icon name="music-2" size={14} color="rgba(255,255,255,.6)" />
              <span class="sec-title">{t("profile.recent")}</span>
              <span class="sec-sub">{t("profile.show_recent.desc")}</span>
              <span class="sp"></span>
              <span data-nodrag><Tog on={s.show_recent} onchange={(v) => update({ show_recent: v })} label={t("profile.show_recent")} /></span>
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
  {#if notice && s}<div class="nt" class:warn={!notice.ok}>{notice.text}</div>{/if}
</div>

<style>
  .col { display: flex; flex-direction: column; }
  .pad { padding: 10px 0; font-size: 13px; color: rgba(255, 255, 255, .6); }
  .nt { padding-top: 8px; font-size: 12.5px; color: var(--ok); }
  .nt.warn { color: var(--warn); }
  .ret { display: flex; padding-top: 10px; }
  .sk { display: flex; flex-direction: column; gap: 10px; }
  .pr { height: 100%; padding: 0 6px; display: flex; align-items: center; gap: 12px; }
  .grip { flex: none; display: flex; opacity: .4; transition: opacity .15s; }
  .pr:hover .grip { opacity: 1; }
  .cv { flex: none; width: 40px; height: 40px; display: flex; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .nm { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 14px; font-weight: 500; color: #fff; }
  .ct { font-size: 12.5px; color: rgba(255, 255, 255, .55); }

  .mini { border-radius: 12px; overflow: hidden; background: rgba(255, 255, 255, .035); transition: opacity .15s ease-out; }
  .mini.busy { opacity: .7; }
  .mini-hero { position: relative; height: 128px; overflow: hidden; }
  .mini-grad { position: absolute; inset: 0; }
  .mini-ban { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; object-position: 50% 30%; filter: brightness(.8);
    -webkit-mask-image: linear-gradient(to bottom, #000 0, #000 60%, rgba(0,0,0,.84) 70%, rgba(0,0,0,.5) 80%, rgba(0,0,0,.16) 90%, transparent 100%);
    mask-image: linear-gradient(to bottom, #000 0, #000 60%, rgba(0,0,0,.84) 70%, rgba(0,0,0,.5) 80%, rgba(0,0,0,.16) 90%, transparent 100%); }
  .hero-hit { position: absolute; inset: 0; width: 100%; height: 100%; display: grid; place-items: center; }
  .hero-ov { width: 38px; height: 38px; border-radius: 50%; display: grid; place-items: center; background: rgba(0, 0, 0, .55); opacity: 0; transition: opacity .15s ease-out; }
  .hero-hit:hover .hero-ov, .hero-hit:focus-visible .hero-ov { opacity: 1; }
  .hero-acts { position: absolute; top: 10px; right: 10px; z-index: 2; display: flex; gap: 6px; }
  .act-ic { width: 30px; height: 30px; border-radius: 8px; display: grid; place-items: center; background: rgba(16, 16, 16, .72); backdrop-filter: blur(8px); transition: background-color .12s ease-out; }
  .act-ic:hover { background: rgba(28, 28, 28, .9); }
  .act-ic:active { background: rgba(40, 40, 40, .95); }
  .act-ic.cancel { background: rgba(255, 255, 255, .09); backdrop-filter: none; }
  .act-ic.cancel:hover { background: rgba(255, 255, 255, .14); }

  .mini-name { position: relative; display: flex; align-items: flex-end; gap: 14px; margin-top: -38px; padding: 0 16px 14px; }
  .mini-pic { position: relative; flex: none; width: 76px; height: 76px; }
  .mini-av-ring { width: 76px; height: 76px; border-radius: 50%; box-shadow: 0 0 0 3px #101010; }
  .mini-pen { position: absolute; inset: 0; border-radius: 50%; display: grid; place-items: center; background: rgba(0, 0, 0, .52); opacity: 0; transition: opacity .15s ease-out; }
  .mini-pic:hover .mini-pen, .mini-pen:focus-visible { opacity: 1; }
  .av-rm { position: absolute; right: -3px; bottom: -3px; z-index: 2; width: 24px; height: 24px; border-radius: 50%; display: grid; place-items: center; background: #181818; box-shadow: 0 0 0 2px #101010; transition: background-color .12s ease-out; }
  .av-rm:hover { background: #262626; }
  .mini-who { flex: 1; min-width: 0; padding-bottom: 4px; display: flex; flex-direction: column; align-items: flex-start; gap: 3px; }
  .mini-badge { font-size: 10.5px; color: rgba(255, 255, 255, .52); transition: color .12s; }
  .mini-badge:hover { color: rgba(255, 255, 255, .85); }
  .mini-handle-btn { display: inline-flex; align-items: center; gap: 6px; max-width: 100%; border-radius: 6px; padding: 1px 4px; margin-left: -4px; transition: background-color .12s ease-out; }
  .mini-handle-btn:hover { background: rgba(255, 255, 255, .07); }
  .mini-handle { font-size: 20px; line-height: 24px; font-weight: 700; color: #fff; }
  .handle-pen { opacity: 0; display: inline-flex; transition: opacity .12s ease-out; }
  .mini-handle-btn:hover .handle-pen, .mini-handle-btn:focus-visible .handle-pen { opacity: 1; }
  .handle-edit { display: flex; align-items: center; gap: 6px; padding-top: 2px; }
  .act-ic:disabled, .hero-hit:disabled, .mini-pen:disabled, .av-rm:disabled { pointer-events: none; }

  .priv-box { padding-top: 6px; }
  .sections { display: flex; flex-direction: column; gap: 10px; padding-top: 10px; }
  .sec-card { position: relative; border-radius: 12px; background: rgba(255, 255, 255, .035); padding: 10px 14px; transition: transform .15s var(--ease), background-color .12s; }
  .sec-card.lift { z-index: 4; transition: none; background: rgba(34, 34, 34, .98); }
  .sec-head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; cursor: grab; touch-action: none; user-select: none; min-height: 28px; }
  .sec-card.lift .sec-head { cursor: grabbing; }
  .sec-grip { display: inline-flex; opacity: .45; transition: opacity .12s; }
  .sec-head:hover .sec-grip { opacity: 1; }
  .sec-title { font-size: 13px; font-weight: 600; color: #fff; }
  .sec-sub { font-size: 12px; color: rgba(255, 255, 255, .48); }
  .sp { flex: 1; }
  .sec-body { padding-top: 8px; }
</style>
