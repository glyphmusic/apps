<script lang="ts">
  // plugins.rs preview: the plugin's picture. A theme's swatch (glow colour
  // as ring, accent inside 57 %, like Appearance) or a rounded square (radius
  // 28 %, white 7 %) with the puzzle icon (54 %, white 70 %) for other kinds.
  // A plugin with its own app icon shows that instead (until it fails to load).
  import Icon from "../components/Icon.svelte";
  import { css } from "../theme";
  import type { PluginThemeColors } from "./types";
  let { theme, size = 28, icon = null }: { theme: PluginThemeColors | null; size?: number; icon?: string | null } = $props();
  let broken = $state(false);
</script>

{#if icon && !broken}
  <img class="ai" src={icon} alt="" loading="lazy" width={size} height={size} style:border-radius="{size * 0.22}px" onerror={() => (broken = true)} />
{:else if theme}
  <span class="sw" style:width="{size}px" style:height="{size}px" style:background={css(theme.glow ? theme.glow[1] : 0x1d5cff)}>
    <span style:width="{size * 0.57}px" style:height="{size * 0.57}px" style:background={css(theme.accent)}></span>
  </span>
{:else}
  <span class="pz" style:width="{size}px" style:height="{size}px" style:border-radius="{size * 0.28}px"><Icon name="puzzle" size={size * 0.54} color="rgba(255,255,255,.7)" /></span>
{/if}

<style>
  .sw { flex: none; border-radius: 50%; display: grid; place-items: center; }
  .sw span { border-radius: 50%; }
  .ai { flex: none; object-fit: cover; }
  .pz { flex: none; display: grid; place-items: center; background: rgba(255, 255, 255, .07); }
</style>
