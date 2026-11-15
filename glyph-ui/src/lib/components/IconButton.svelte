<script lang="ts">
  // widgets/buttons.rs icon_button: box = icon size + 14, radius 8, hover 6 %, active 10 %.
  import Icon from "./Icon.svelte";
  import { tip } from "../ui.svelte";
  import { tipFor } from "../i18n.svelte";
  let {
    name, size = 16, color = "var(--dim)", label, box, onclick, oncontextmenu, onpointerdown, notip = false, class: cls = "", disabled = false,
  }: {
    name: string; size?: number; color?: string; label?: string; box?: number; notip?: boolean; class?: string; disabled?: boolean;
    onclick?: (e: MouseEvent) => void; oncontextmenu?: (e: MouseEvent) => void; onpointerdown?: (e: PointerEvent) => void;
  } = $props();
  const b = $derived(box ?? size + 14);
  const text = $derived(label ?? tipFor(name));
</script>

<button class="ib {cls}" style:width="{b}px" style:height="{b}px" style:color={color} use:tip={notip ? undefined : text} aria-label={text ?? name} {onclick} {oncontextmenu} {onpointerdown} {disabled}>
  <Icon {name} {size} />
</button>

<style>
  .ib { flex: none; display: grid; place-items: center; border-radius: 8px; transition: background-color .12s ease-out; }
  .ib:hover { background: var(--hover); }
  .ib:active { background: var(--active); }
  .ib:disabled { opacity: .4; pointer-events: none; }
  .ib:global(.pressing) :global(.ic) { animation: press .25s var(--ease); }
</style>
