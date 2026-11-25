<script lang="ts">
  // lists/dialogs.rs name_dialog + edit_dialog, over widgets.rs modal_layer
  // (titlebar dimmed but draggable, backdrop below it closes the dialog).
  // New playlist / equalizer preset: w 380, p 20, radius 12,
  // hsla(0,0,.07,.99), 1 px white 8 %, gap 14; title 16 semibold white 95 %;
  // field h 38, px 12, radius 8, black 45 %, 1 px white 12 %, 13.5 px. New
  // playlists add "or paste a link" (link-2 field + what it was recognised
  // as / reading / error) and "Import file"; the button says Create, Import
  // or Importing... Backdrop black 45 %.
  // Edit playlist (rename): w 540, p 24, radius 12, hsla(0,0,.09,.99), gap
  // 18; title 20 bold + × (18, white 60 %); picture 176 radius 8 (click to
  // change, hover: black 60 % + image-plus 28 + "Choose picture"), name field
  // h 40 white 7 % 14 px, "download new songs automatically" switch; Remove
  // picture + Save (h 40, px 24; white when something changed, else white
  // 16 % with white 40 % text). Backdrop black 55 %.
  import { addEqPreset } from "../settings/eq";
  import { fade, fly, scale } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import Button from "../components/Button.svelte";
  import Switch from "../components/Switch.svelte";
  import ListCover from "../components/ListCover.svelte";
  import Decode from "../components/Decode.svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import { press, eatTap } from "../ui.svelte";
  import { pickCover, playlistX, removeCover, setPlaylistStorage, toggleAutoDownload } from "../pages/lists/actions";
  import type { ProfileSettingsData } from "../settings/types";
  import { errText } from "../settings/util";

  const d = $derived(app.nameDialog!);
  const editId = $derived(d.rename ?? null);
  const eq = $derived(!!d.eqPreset);
  let name = $state(app.nameDialog?.initial ?? (app.nameDialog?.rename ? app.playlist(app.nameDialog.rename)?.name ?? "" : ""));
  let input: HTMLInputElement | undefined = $state();
  $effect(() => { input?.focus(); input?.select(); });

  // ---- edit dialog: what was there when it opened ----
  const p = $derived(editId ? playlistX(editId) : undefined);
  const firstId = app.nameDialog?.rename ?? null;
  const initial = firstId ? { name: app.playlist(firstId)?.name ?? "", cover: app.playlist(firstId)?.cover ?? null, auto: !!playlistX(firstId)?.auto_download } : null;

  let initialPub = $state<boolean | null>(null);
  let pub = $state<boolean | null>(null);
  let inProfile = $state(false);
  let pubErr = $state(false);
  let saving = $state(false);
  $effect(() => {
    const id = editId;
    initialPub = null;
    pub = null;
    inProfile = false;
    pubErr = false;
    if (!id || !app.account) return;
    let dead = false;
    backend().call<ProfileSettingsData>("profile_settings")
      .then((s) => {
        if (dead || !s) return;
        const listed = s.playlists.includes(id);
        inProfile = listed;
        const cur = !playlistX(id)?.local_only && listed;
        initialPub = cur;
        pub = cur;
      })
      .catch(() => { if (!dead) pubErr = true; });
    return () => { dead = true; };
  });

  const pubChanged = $derived(initialPub !== null && pub !== null && pub !== initialPub);
  const changed = $derived(!!initial && !!p && name.trim() !== "" && (name.trim() !== initial.name || (p.cover ?? null) !== initial.cover || !!p.auto_download !== initial.auto || pubChanged));

  const okLabel = $derived(d.okLabel ?? t(editId || eq ? "dialog.save" : "dialog.create"));
  const titleLabel = $derived(d.title ?? t(eq ? "dialog.eq.title" : "dialog.playlist.title"));
  const mobileNew = $derived(app.compact && !editId && !eq);
  const mobileTitle = $derived(d.title ?? t("dialog.playlist.mobile_title"));
  const mobilePlaceholder = $derived(d.fallback ?? tf("library.new_playlist_name", { n: app.library.playlists.length + 1 }));
  const submitDisabled = $derived(!!d.nonEmpty && !name.trim());

  function close() { app.nameDialog = null; }
  async function submit() {
    if (saving) return;
    const typed = name.trim();
    if (editId) {
      if (!changed) return;
      saving = true;
      try {
        if (pubChanged) {
          if (pub && p?.local_only) await setPlaylistStorage(editId, p.auto_download ? 2 : 1);
          if (inProfile !== !!pub) await backend().call("profile_toggle_playlist", { id: editId });
        }
        if (typed !== initial?.name) app.renamePlaylist(editId, typed);
        close();
      } catch (e) {
        app.toast(errText(e));
      } finally {
        saving = false;
      }
      return;
    }
    if (eq) {
      close();
      if (typed) { addEqPreset({ name: typed, gains: [...app.config.audio.eq_gains] }); }
      return;
    }
    if (d.onSubmit) {
      if (d.nonEmpty && !typed) return;
      const nm = typed || d.fallback || tf("library.new_playlist_name", { n: app.library.playlists.length + 1 });
      const fn = d.onSubmit;
      close();
      await fn(nm);
      return;
    }
    const nm = typed || tf("library.new_playlist_name", { n: app.library.playlists.length + 1 });
    const tracks = $state.snapshot(d.tracks);
    close();
    const id = await app.createPlaylist(nm, tracks);
    if (!id) return;
    if (!tracks.length) app.openListView({ kind: "playlist", id });
    else app.toast(tracks.length === 1 ? tf("toast.added_to_playlist", { name: nm }) : tf("toast.added_n_to_playlist", { n: tracks.length, name: nm }));
  }
  function key(e: KeyboardEvent) {
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); }
    else if (e.key === "Enter") { e.preventDefault(); e.stopPropagation(); submit(); }
  }
</script>

{#if mobileNew}
  <div class="m-stage" transition:fade|global={{ duration: 200 }} role="dialog" aria-modal="true" aria-label={mobileTitle} tabindex="-1">
    <div class="m-inner" in:fly|global={{ y: 16, duration: 240, easing: cubicOut }} out:fly|global={{ y: 8, duration: 150, easing: cubicOut }}>
      <h1 class="m-title"><Decode text={mobileTitle} key="mob-new-pl" /></h1>
      <div class="m-field">
        <input
          class="m-input"
          bind:this={input}
          bind:value={name}
          placeholder={mobilePlaceholder}
          onkeydown={key}
          spellcheck="false"
          enterkeyhint="done"
        />
      </div>
      <div class="m-acts">
        <button type="button" class="m-btn m-cancel" use:press onclick={close}>{t("dialog.cancel")}</button>
        <button type="button" class="m-btn m-create" use:press disabled={submitDisabled} onclick={submit}>{okLabel}</button>
      </div>
    </div>
  </div>
{:else}
<div class="layer" transition:fade|global={{ duration: 160 }}>
  <div class="tdim" style:background="rgba(0,0,0,{editId ? 0.55 : 0.45})"></div>
  <div class="bd" style:background="rgba(0,0,0,{editId ? 0.55 : 0.45})" onpointerdown={(e) => { if (e.target === e.currentTarget) { eatTap(); close(); } }} role="presentation">
    {#if editId}
      <div class="panel edit" data-part="dialog" transition:scale|global={{ start: 0.97, duration: 180, easing: cubicOut }} role="dialog" tabindex="-1">
        <div class="eh">
          <span class="et">{t("dialog.playlist.rename")}</span>
          <IconButton name="x" size={18} color="rgba(255,255,255,.6)" onclick={close} />
        </div>
        <div class="eb">
          <button class="cv" onclick={() => pickCover(editId)} aria-label={t("dialog.playlist.pick")}>
            <ListCover list={{ kind: "playlist", id: editId }} size={app.compact ? 148 : 176} />
            <span class="veil" class:mob={app.compact}><Icon name="image-plus" size={app.compact ? 22 : 28} color="rgba(255,255,255,.95)" /><span>{t("dialog.playlist.pick")}</span></span>
          </button>
          <div class="ef">
            <input class="f2" bind:this={input} bind:value={name} placeholder={t("dialog.playlist.placeholder")} onkeydown={key} spellcheck="false" />
            <div class="sw">
              <Switch on={!!p?.auto_download} label={t("ctx.auto_download")} onchange={() => toggleAutoDownload(editId)} />
              <Switch
                on={!!pub}
                disabled={!app.account || pub === null || pubErr || saving}
                why={!app.account ? t("dialog.playlist.public.signed_out") : pubErr ? t("dialog.playlist.public.unavailable") : undefined}
                label={t("dialog.playlist.public")}
                onchange={(v) => (pub = v)}
              />
            </div>
          </div>
        </div>
        <div class="foot">
          {#if p?.cover}<Button kind="text" onclick={() => removeCover(editId)}>{t("dialog.playlist.unpick")}</Button>{/if}
          <span class="flex1"></span>
          <Button kind="primary" disabled={!changed || saving} onclick={submit}>{t("dialog.save")}</Button>
        </div>
      </div>
    {:else}
      <div class="panel" data-part="dialog" transition:scale|global={{ start: 0.97, duration: 180, easing: cubicOut }} role="dialog" tabindex="-1">
        <div class="tt">{titleLabel}</div>
        <div class="field"><input bind:this={input} bind:value={name} placeholder={eq ? "" : d.placeholder ?? t("dialog.playlist.placeholder")} onkeydown={key} spellcheck="false" /></div>
        <div class="foot">
          <span class="flex1"></span>
          <Button onclick={close}>{t("dialog.cancel")}</Button>
          <Button kind="primary" disabled={submitDisabled} onclick={submit}>{okLabel}</Button>
        </div>
      </div>
    {/if}
  </div>
</div>
{/if}

<style>
  .m-stage {
    position: fixed;
    inset: 0;
    z-index: 95;
    background: rgba(8, 8, 9, .28);
    padding: max(24px, var(--safe-top, env(safe-area-inset-top))) 28px max(28px, var(--safe-bottom, env(safe-area-inset-bottom)), var(--ime-bottom, 0px));
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: var(--font);
  }
  .m-inner {
    width: 100%;
    max-width: 316px;
    margin-bottom: 6vh;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 24px;
  }
  .m-title {
    margin: 0;
    font-size: 19px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: rgba(255, 255, 255, .92);
    text-align: center;
  }
  .m-field {
    width: 100%;
    margin-top: 4px;
    display: flex;
    justify-content: center;
    border-bottom: 1.5px solid rgba(255, 255, 255, .2);
    transition: border-color .18s ease-out;
  }
  .m-field:focus-within {
    border-bottom-color: var(--accent);
  }
  .m-input {
    width: 100%;
    height: auto;
    padding: 8px 4px 12px;
    background: none;
    border: 0;
    outline: none;
    text-align: center;
    font-size: 26px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: rgba(255, 255, 255, .98);
    caret-color: var(--accent);
    font-family: inherit;
  }
  .m-input::placeholder {
    color: rgba(255, 255, 255, .26);
    font-weight: 600;
  }
  .m-acts {
    margin-top: 12px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
  }
  .m-btn {
    height: 40px;
    padding: 0 22px;
    border-radius: 10px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color .14s ease-out, opacity .14s ease-out, transform .14s var(--ease);
  }
  .m-cancel {
    background: #202022;
    color: rgba(255, 255, 255, .88);
  }
  .m-cancel:active {
    background: #2c2c30;
  }
  .m-create {
    padding: 0 26px;
    background: #fff;
    color: #000;
    font-weight: 600;
  }
  .m-create:active {
    opacity: .86;
  }
  .m-create:disabled {
    opacity: .38;
    cursor: default;
  }
  .layer { position: fixed; inset: 0; z-index: 90; }
  .tdim { position: absolute; left: 0; right: 0; top: 0; height: var(--titlebar-h); pointer-events: none; }
  .bd { position: absolute; top: var(--titlebar-h); left: 0; right: 0; bottom: 0; padding-bottom: var(--titlebar-h); display: flex; align-items: center; justify-content: center; }
  .panel { width: 380px; max-width: calc(100vw - 32px); padding: 20px; border-radius: 12px; background: #161616; display: flex; flex-direction: column; gap: 14px; }
  .tt { font-size: 16px; font-weight: 600; color: rgba(255, 255, 255, .95); }
  .field { height: 38px; padding: 0 12px; display: flex; align-items: center; gap: 8px; border-radius: 8px; background: rgba(255, 255, 255, .07); font-size: 13.5px; }
  input { flex: 1; min-width: 0; height: 100%; background: none; border: 0; outline: none; padding: 0; }
  input::placeholder { color: rgba(255, 255, 255, .35); }
  .foot { display: flex; align-items: center; gap: 8px; }
  .foot :global(.text) { font-size: var(--sm); }
  .edit { width: 540px; padding: 24px; background: #171717; gap: 18px; }
  .eh { display: flex; align-items: center; }
  .et { flex: 1; font-size: 20px; font-weight: 700; color: rgba(255, 255, 255, .95); }
  .eb { display: flex; gap: 16px; }
  .cv { position: relative; flex: none; width: 176px; height: 176px; border-radius: 8px; overflow: hidden; }
  .veil { position: absolute; inset: 0; background: rgba(0, 0, 0, .6); display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; font-size: var(--sm); color: rgba(255, 255, 255, .95); opacity: 0; transition: opacity .15s ease-out; }
  .cv:hover .veil { opacity: 1; }
  .veil.mob { inset: auto 8px 8px 8px; flex-direction: row; gap: 6px; padding: 6px 10px; border-radius: 999px; background: rgba(0, 0, 0, .72); font-size: 12px; opacity: 1; }
  .ef { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 12px; }
  .f2 { flex: none; height: 40px; padding: 0 12px; border-radius: 8px; background: rgba(255, 255, 255, .07); font-size: 14px; }
  .sw { display: flex; flex-direction: column; gap: 12px; padding-top: 4px; }
  @media (max-width: 560px) {
    .edit { padding: 18px; gap: 14px; }
    .eb { flex-direction: column; align-items: stretch; }
    .cv { align-self: center; width: 148px; height: 148px; }
  }
</style>
