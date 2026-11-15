<script lang="ts">
  // compact.rs icon_action: 40 px round touch target, 22 px icon in text
  // colour, white 8 % while pressed.
  import Icon from "../components/Icon.svelte";
  let { name, onclick, label = name, bg, size = 22, color = "var(--text)" }: { name: string; onclick?: (e: MouseEvent) => void; label?: string; bg?: string; size?: number; color?: string } = $props();
</script>

<button class="ia" aria-label={label} {onclick} style:background={bg}><Icon {name} {size} {color} /></button>

<style>
  .ia { flex: none; width: 40px; height: 40px; display: grid; place-items: center; border-radius: 50%; transition: background-color .12s; }
  .ia:active { background: rgba(255, 255, 255, .08) !important; }
</style>
