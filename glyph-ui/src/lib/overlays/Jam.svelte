<script lang="ts">
  // ui/jam.rs + app/jam.rs show_devices_panel: the Jam page lives inside the
  // Connect panel (shell/ConnectPanel.svelte shows pages/jam/JamSection.svelte
  // while `app.jamOpen`). This overlay is what Overlays.svelte mounts while
  // `app.jamOpen` is set: when the flag is raised from somewhere else (a jam
  // link, a "Jam" menu entry) the Connect panel is not open yet, so it opens
  // it (and closes the queue, which shares its slot) and keeps the jam state
  // fresh. It draws nothing itself.
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { watchJam } from "../jam.svelte";

  onMount(() => {
    if (!app.showDevices) { app.showDevices = true; app.showQueue = false; }
    return watchJam();
  });
</script>
