<script lang="ts">
  // lists/rows.rs follow_button: outline button, h 36, px 16, radius 8,
  // black 35 %, 1 px border (following: white 14 %, quiet; not yet: white
  // 45 %, asks for the click), sm semibold white 95 %, hover white 10 %.
  // On a click the check grows in (or shrinks away, 20 px wide) while the
  // label decodes over 0.45 s.
  import Icon from "../../components/Icon.svelte";
  import { app } from "../../state.svelte";
  import { decode } from "../../ui.svelte";
  import { t } from "../../i18n.svelte";
  let { name, h = 36, pill = false }: { name: string; h?: number; pill?: boolean } = $props();
  const following = $derived(app.follows(name));
  let swapAt = $state(0);
  const k = $derived(swapAt ? Math.min(1, (app.clock - swapAt) / 450) : 1);
  const e = $derived(1 - Math.pow(1 - k, 3));
  const w = $derived(following ? e : 1 - e);
  const label = $derived(t(following ? "artist.following" : "artist.follow"));
  function click(ev: MouseEvent) {
    ev.stopPropagation();
    swapAt = performance.now();
    app.pulseClock(600);
    app.toggleFollow(name);
  }
</script>

<button class="fb" class:on={following} class:pill style:height="{h}px" style:padding-left="{16 - 4 * w}px" onclick={click}>
  {#if following || k < 1}
    <span class="ck" style:width="{18 * w}px" style:opacity={w}><Icon name="check" size={14 * (0.6 + 0.4 * w)} color="rgba(255,255,255,.95)" /></span>
  {/if}
  <span>{decode(label, k)}</span>
</button>

<style>
  .fb { position: relative; flex: none; display: flex; align-items: center; padding: 0 16px; border-radius: 8px; border: 1px solid rgba(255, 255, 255, .45); background: #18181a; font-size: var(--sm); font-weight: 600; color: rgba(255, 255, 255, .95); white-space: nowrap; transition: background-color .12s ease-out, border-color .2s ease-out, transform .14s var(--ease); }
  .fb.pill { border-radius: 999px; }
  .fb.on { border-color: rgba(255, 255, 255, .14); }
  .fb::after { content: ""; position: absolute; inset: 0; border-radius: inherit; background: rgba(255, 255, 255, .1); opacity: 0; transition: opacity .12s ease-out; pointer-events: none; }
  .fb:hover { background: #26262a; }
  .fb:hover::after { opacity: 0; }
  .fb:active { background: #303036; transform: scale(.96); }
  .fb:active::after { opacity: 0; }
  .ck { flex: none; overflow: hidden; display: flex; align-items: center; }
</style>
