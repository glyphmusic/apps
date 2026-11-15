<script lang="ts">
  // controls.rs link_button: h 32, px 12, radius 8, white 7 % (hover + 5 %),
  // 13 px white, icon 14 (white 80 %), gap 8. Opens `url` in the system browser.
  import Icon from "../components/Icon.svelte";
  import { openUrl } from "./util";
  let { icon, label, url }: { icon: string; label: string; url: string } = $props();
</script>

<button class="lb" onclick={() => openUrl(url)}>
  <Icon name={icon} size={14} color="rgba(255,255,255,.8)" />
  <span>{label}</span>
</button>

<style>
  .lb { flex: none; height: 32px; padding: 0 12px; display: inline-flex; align-items: center; gap: 8px; border-radius: 8px; background: rgba(255, 255, 255, .07); font-size: 13px; color: #fff; transition: background-color .12s ease-out; }
  .lb:hover { background: rgba(255, 255, 255, .12); }
</style>
