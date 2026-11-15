<script lang="ts">
  // profile.rs avatar(): round, overflow hidden, white 10 % fill, 2 px border
  // black 60 % (none for `flat`: own_avatar / jam person use .border_0()),
  // picture cover-fitted; without one the user-round icon (size x 0.42, white 60 %).
  import Icon from "../../components/Icon.svelte";
  let { src = null, size, flat = false }: { src?: string | null; size: number; flat?: boolean } = $props();
  let failed = $state(false);
  $effect(() => { void src; failed = false; });
</script>

<span class="av" class:flat style:width="{size}px" style:height="{size}px">
  {#if src && !failed}
    <img {src} alt="" draggable="false" onerror={() => (failed = true)} />
  {:else}
    <Icon name="user-round" size={size * 0.42} color="rgba(255,255,255,.6)" />
  {/if}
</span>

<style>
  .av { flex: none; display: inline-flex; align-items: center; justify-content: center; border-radius: 50%; overflow: hidden; background: rgba(255, 255, 255, .10); border: 2px solid rgba(0, 0, 0, .6); position: relative; }
  .av.flat { border: 0; }
  img { width: 100%; height: 100%; object-fit: cover; animation: fade-in .3s var(--ease) both; }
</style>
