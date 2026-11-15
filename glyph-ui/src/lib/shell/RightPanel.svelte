<script lang="ts">
  // The slot on the right (ui/mod.rs): "Add songs" of an own playlist, the
  // Connect panel or the queue; they swap with a fade.
  import { fade } from "svelte/transition";
  import { app } from "../state.svelte";
  import QueuePanel from "./QueuePanel.svelte";
  import AddSongsPanel from "./AddSongsPanel.svelte";
  import ConnectPanel from "./ConnectPanel.svelte";
  const which = $derived(app.addSongsOpen && !app.showQueue && !app.showDevices ? "add" : app.showDevices ? "devices" : "queue");
</script>

<div class="wrap">
  {#key which}
    <div class="p" in:fade={{ duration: 180, delay: 30 }} out:fade={{ duration: 140 }}>
      {#if which === "add"}<AddSongsPanel />{:else if which === "devices"}<ConnectPanel />{:else}<QueuePanel />{/if}
    </div>
  {/key}
</div>

<style>
  .wrap { position: relative; width: 100%; height: 100%; }
  .p { position: absolute; inset: 0; }
</style>
