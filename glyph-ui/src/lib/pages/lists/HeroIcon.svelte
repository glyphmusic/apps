<script lang="ts">
  // lists/rows.rs hero_icon: borderless 36 px icon action, radius 8, 18 px
  // icon white 85 %, hover white 10 %.
  import Icon from "../../components/Icon.svelte";
  import { tip } from "../../ui.svelte";
  import { tipFor } from "../../i18n.svelte";
  let { name, onclick, onpointerdown }: { name: string; onclick?: (e: MouseEvent) => void; onpointerdown?: (e: PointerEvent) => void } = $props();
</script>

<button class="hi" use:tip={tipFor(name)} aria-label={tipFor(name) ?? name} {onclick} {onpointerdown}><Icon {name} size={18} color="rgba(255,255,255,.85)" /></button>

<style>
  .hi { flex: none; width: 36px; height: 36px; display: grid; place-items: center; border-radius: 8px; transition: background-color .12s ease-out; }
  .hi:hover { background: rgba(255, 255, 255, .1); }
</style>
