<script lang="ts">
  // widgets/indicators.rs song_title + explicit_badge: the "E" box (white
  // 22 %, black 85 % text) or "?" (unknown) in front of the title.
  import type { Snippet } from "svelte";
  import type { Rating } from "../api/types";
  import { tip } from "../ui.svelte";
  import { t } from "../i18n.svelte";
  let { rating = "clean", children, class: cls = "" }: { rating?: Rating; children: Snippet; class?: string } = $props();
</script>

<span class="st {cls}">
  {#if rating !== "clean"}
    <i class="e" class:unk={rating === "unknown"} use:tip={t(rating === "explicit" ? "tip.explicit" : "tip.explicit_unknown")}>{rating === "explicit" ? "E" : "?"}</i>
  {/if}
  <span class="trunc">{@render children()}</span>
</span>

<style>
  .st { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .e { flex: none; width: 15px; height: 15px; border-radius: 3px; background: rgba(255, 255, 255, .22); color: rgba(0, 0, 0, .85); font: 700 9.5px/15px var(--font); text-align: center; font-style: normal; }
  .e.unk { background: rgba(255, 255, 255, .12); color: rgba(255, 255, 255, .6); }
</style>
