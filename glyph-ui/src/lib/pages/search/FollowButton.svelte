<script lang="ts">
  // lists/rows.rs follow_button: "Follow" / "Following" outline button, h 30
  // (search), px 16, radius 8, 1 px border (following: white 14 %, not yet:
  // white 45 % so it asks for the click), black 35 % fill, sm semibold white
  // 95 %, hover white 10 %. A check grows in (or shrinks away) and the label
  // decodes over 0.45 s when it changes.
  import Icon from "../../components/Icon.svelte";
  import Decode from "../../components/Decode.svelte";
  import { app } from "../../state.svelte";
  import { t } from "../../i18n.svelte";
  import { revealRestart } from "../../ui.svelte";
  let { name, h = 30 }: { name: string; h?: number } = $props();
  const following = $derived(app.follows(name));
  const swap = $derived(`follow-swap:${name}`);
  function click(e: MouseEvent) {
    e.stopPropagation();
    revealRestart(swap);
    app.toggleFollow(name);
  }
</script>

<button class="fb" class:on={following} style:height="{h}px" onclick={click}>
  <span class="ck" class:on={following}><Icon name="check" size={14} color="rgba(255,255,255,.95)" /></span>
  <Decode text={following ? t("artist.following") : t("artist.follow")} key={swap} dur={0.45} />
</button>

<style>
  .fb { position: relative; flex: none; display: flex; align-items: center; padding: 0 16px; border-radius: 8px; background: #2a2a2f; font-size: var(--sm); font-weight: 600; color: rgba(255, 255, 255, .95); transition: background-color .12s ease-out, padding-left .3s var(--ease), transform .14s var(--ease); }
  .fb.on { background: #1c1c1f; padding-left: 12px; }
  .fb:hover { background: #34343a; }
  .fb.on:hover { background: #26262a; }
  .fb:active { background: #3c3c42; transform: scale(.96); }
  .ck { display: flex; align-items: center; justify-content: flex-start; width: 0; opacity: 0; overflow: hidden; transform: scale(.6); transform-origin: left center; transition: width .3s var(--ease), opacity .3s var(--ease), transform .3s var(--ease); }
  .ck.on { width: 18px; opacity: 1; transform: none; }
</style>
