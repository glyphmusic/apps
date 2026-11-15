<script lang="ts">
  // widgets/buttons.rs button / primary_button / text_button, plus the list
  // view's play pill, round icon button and back pill (lists.rs).
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import { tip } from "../ui.svelte";
  let {
    kind = "default", icon, iconSize, onclick, children, disabled = false, title, class: cls = "", type = "button",
  }: {
    kind?: "default" | "primary" | "text" | "pill" | "round" | "back" | "danger"; icon?: string; iconSize?: number;
    onclick?: (e: MouseEvent) => void; children?: Snippet; disabled?: boolean; title?: string; class?: string; type?: "button" | "submit";
  } = $props();
  const isz = $derived(iconSize ?? (kind === "round" ? 16 : 14));
</script>

<button class="btn {kind} {cls}" {onclick} {disabled} {type} use:tip={title}>
  {#if icon}<Icon name={icon} size={isz} />{/if}
  {#if children}{@render children()}{/if}
</button>

<style>
  .btn { flex: none; display: inline-flex; align-items: center; justify-content: center; gap: 6px; white-space: nowrap; transition: background-color .12s ease-out, opacity .12s ease-out, color .12s; }
  .btn:disabled { opacity: .4; pointer-events: none; }
  /* button: h 32, px 14 */
  .default, .danger { height: 32px; padding: 0 14px; border-radius: 8px; background: #202022; font-size: var(--sm); color: var(--text); }
  .default:hover, .danger:hover { background: #2a2a2e; }
  .default:active, .danger:active { background: #343438; }
  .danger { color: var(--danger); }
  /* primary_button: flat white, black semibold */
  .primary { height: 32px; padding: 0 16px; border-radius: 8px; background: var(--text); color: #000; font-size: var(--sm); font-weight: 600; }
  .primary:hover { opacity: .88; }
  /* text_button */
  .text { padding: 3px 8px; border-radius: 6px; color: var(--dim); }
  .text:hover { background: var(--hover); color: var(--text); }
  /* play pill: h 36, px 18, radius 8, white, black 12.5 semibold, gap 8 */
  .pill { height: 36px; padding: 0 18px; gap: 8px; border-radius: 8px; background: #fff; color: #000; font-size: 12.5px; font-weight: 600; }
  .pill:hover { opacity: .88; }
  /* round icon button: 36, radius 8, solid dark fill, 1 px white 14 % */
  .round { width: 36px; height: 36px; border-radius: 8px; background: #18181a; border: 1px solid rgba(255, 255, 255, .14); color: rgba(255, 255, 255, .9); }
  .round:hover { background: #242428; }
  /* back pill: h 30, px 12, radius 8, solid dark surface visible on dark pages and bright hero banners */
  .back { height: 30px; padding: 0 12px; border-radius: 8px; background: #222226; font-size: 12.5px; font-weight: 500; color: rgba(255, 255, 255, .94); }
  .back:hover { background: #2e2e34; }
  .back:active { background: #3a3a40; }
</style>
