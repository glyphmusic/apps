<script lang="ts">
  // lists/ctx_menu.rs menu_row / ctx_item / danger_item: h 32, px 10, radius
  // 6, gap 10, 13 px white 88 %, 15 px icon (white 70 % unless given), hover
  // white 7 % fading in and out. Danger rows: red icon hsla(0 75 % 68 % / .9),
  // red hover fill (hsla(0 70 % 50 % / .18)) and the text fades to red.
  // `sub`: a row with a flyout (chevron 14 white 40 %, highlighted while open).
  import type { Snippet } from "svelte";
  import Icon from "../../components/Icon.svelte";
  let {
    icon, label, color = "rgba(255,255,255,.7)", danger = false, open = false, sub = false, onclick, onenter, children,
  }: {
    icon: string; label: string; color?: string; danger?: boolean; open?: boolean; sub?: boolean;
    onclick?: (e: MouseEvent) => void; onenter?: () => void; children?: Snippet;
  } = $props();
</script>

<div class="mi-wrap">
  <div class="mi" class:danger class:open role="menuitem" tabindex="-1" onclick={onclick} onpointerenter={onenter}
    onkeydown={(e) => e.key === "Enter" && onclick?.(e as unknown as MouseEvent)}>
    <Icon name={icon} size={15} color={danger ? "hsla(0, 75%, 68%, .9)" : color} />
    <span class="lb trunc">{label}</span>
    {#if sub}<Icon name="chevron-right" size={14} color="rgba(255,255,255,.4)" />{/if}
  </div>
  {#if children}{@render children()}{/if}
</div>

<style>
  .mi-wrap { position: relative; flex: none; }
  .mi { position: relative; flex: none; display: flex; align-items: center; gap: var(--mi-gap, 10px); height: var(--mi-h, 32px); padding: 0 var(--mi-px, 10px); border-radius: var(--mi-r, 6px); cursor: pointer; font-size: var(--mi-fs, 13px); color: rgba(255, 255, 255, .88); transition: background-color .12s ease-out, color .12s ease-out, transform .16s var(--ease); }
  .mi:hover, .mi.open { background: rgba(255, 255, 255, .07); }
  .mi:active { background: rgba(255, 255, 255, .1); transform: scale(.975); }
  .mi.danger:hover { background: hsla(0, 70%, 50%, .18); color: hsl(0 85% 75%); }
  .lb { flex: 1; }
</style>
