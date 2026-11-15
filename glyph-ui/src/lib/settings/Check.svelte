<script lang="ts">
  // storage.rs checkbox: 16 px, radius 4, white with a black check when on,
  // white 35 % outline when off.
  import Icon from "../components/Icon.svelte";
  let { on }: { on: boolean } = $props();
</script>

<span class="cb" class:on>{#if on}<Icon name="check" size={12} color="#000" />{/if}</span>

<style>
  .cb { flex: none; width: 16px; height: 16px; border-radius: 4px; display: grid; place-items: center; background: rgba(255, 255, 255, .16); transition: background-color .12s; }
  .cb.on { background: #fff; }
</style>
