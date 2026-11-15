<script lang="ts">
  // Desktop Connect panel (320 px, Settings-styled): delegates content and
  // state to ConnectView inside the right-hand floating panel shell.
  import { app } from "../state.svelte";
  import { jam } from "../jam.svelte";
  import ConnectView from "./ConnectView.svelte";

  function close() {
    app.showDevices = false;
    app.jamOpen = false;
    jam.page = "main";
  }
</script>

<section class="panel p">
  <ConnectView onclose={close} />
</section>

<style>
  .p { width: 100%; height: 100%; background: rgba(12, 12, 12, .94); border-radius: 12px; display: flex; flex-direction: column; overflow: hidden; }
</style>
