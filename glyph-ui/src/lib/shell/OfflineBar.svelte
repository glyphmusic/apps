<script lang="ts">
  // A thin full-width bar under the player once the device has had no
  // connection for a while (OFFLINE_AFTER_MS): "No connection", a few covers
  // of the saved songs and a button that plays them. A tap on the text opens
  // Downloads (phone: the sheet with the saved playlists). Desktop mounts it
  // under the player bar, the phone under the mini bar.
  import { app } from "../state.svelte";
  import { t, tf } from "../i18n.svelte";
  import { offline, OFFLINE_AFTER_MS } from "../offline.svelte";
  import Icon from "../components/Icon.svelte";
  import Cover from "../components/Cover.svelte";

  let { phone = false }: { phone?: boolean } = $props();

  let netDown = $state(typeof navigator !== "undefined" && navigator.onLine === false);
  const down = $derived(netDown || app.backendState.kind === "offline");
  const saved = $derived(app.library.downloads);

  $effect(() => {
    const on = () => (netDown = false);
    const off = () => (netDown = true);
    addEventListener("online", on);
    addEventListener("offline", off);
    return () => { removeEventListener("online", on); removeEventListener("offline", off); };
  });

  // Short drops stay silent; the bar goes as soon as the connection is back.
  // Phones show the saved playlists once per drop.
  $effect(() => {
    if (!down) { offline.shown = false; offline.sheetOpen = false; return; }
    const id = setTimeout(() => {
      offline.shown = true;
      if (phone && saved.length) offline.sheetOpen = true;
    }, OFFLINE_AFTER_MS);
    return () => clearTimeout(id);
  });

  function openSaved() {
    if (phone) offline.sheetOpen = true;
    else app.openListView({ kind: "downloads" });
  }
  function playDownloads() { if (saved.length) void app.play(saved, 0, "downloads"); }
</script>

{#if offline.shown}
  <div class="ob" class:phone role="status">
    <button class="info" onclick={saved.length ? openSaved : undefined} disabled={!saved.length}>
      <Icon name="wifi-off" size={13} color="var(--dim)" />
      <span class="l">{t("offline.bar")}</span>
      {#if saved.length}
        <span class="sep">·</span>
        <span class="covers">
          {#each saved.slice(0, 3) as tr, i (i)}<span class="cv"><Cover track={tr} size={16} radius={4} /></span>{/each}
        </span>
        {#if !phone}<span class="s trunc">{tf("offline.bar_saved", { n: saved.length })}</span>{/if}
      {/if}
    </button>
    {#if saved.length}
      <button class="play" onclick={playDownloads}>
        <Icon name="play" size={10} color="rgba(10,10,10,1)" />
        <span>{phone ? t("library.downloads") : t("offline.bar_play")}</span>
      </button>
    {/if}
  </div>
{/if}

<style>
  .ob { width: 100%; height: 28px; padding: 0 6px 0 14px; display: flex; align-items: center; gap: 8px; background: hsla(0, 0%, 6%, .98); border-top: 1px solid rgba(255, 255, 255, .07); animation: rise-in .2s var(--ease) both; }
  .ob.phone { height: 32px; padding: 0 8px 0 16px; }
  .info { flex: 1; min-width: 0; overflow: hidden; height: 100%; display: flex; align-items: center; gap: 8px; color: var(--text); text-align: left; }
  .info:disabled { cursor: default; }
  .l { font-size: 11.5px; font-weight: 600; white-space: nowrap; }
  .sep { color: var(--faint, var(--dim)); font-size: 12px; }
  .s { font-size: 11.5px; color: var(--dim); }
  .info:not(:disabled):hover .s { color: var(--text); }
  .covers { display: flex; flex: none; }
  .cv { display: block; border-radius: 5px; box-shadow: 0 0 0 1.5px hsla(0, 0%, 6%, 1); }
  .cv + .cv { margin-left: -5px; }
  .play { flex: none; height: 20px; padding: 0 9px 0 7px; border-radius: 6px; display: inline-flex; align-items: center; gap: 5px; background: var(--accent); color: rgba(10, 10, 10, 1); font-size: 11px; font-weight: 700; transition: opacity .15s; }
  .play:hover { opacity: .92; }
  .phone .play { height: 22px; border-radius: 7px; }
</style>
