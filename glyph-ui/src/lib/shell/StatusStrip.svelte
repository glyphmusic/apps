<script lang="ts">
  // ui/connect.rs status_strip: the strip under the player bar (h 26, full
  // width, px 12, radius 8, content right-aligned, gap 8, icon 13, label xs
  // semibold): in the accent with dark text while music plays on another
  // device ("Playing on ..."), grey (bar colour, card on hover) while another
  // Glyph app plays on its own ("Connect to ..."). A click opens the Connect
  // panel, or takes control of that app. Nothing otherwise. The player bar
  // mounts it (this file is not mounted by anything yet).
  import { backend } from "../api/backend";
  import { app } from "../state.svelte";
  import { tf } from "../i18n.svelte";
  import { jamActive } from "../jam.svelte";
  import { connectTo, deviceIcon, playingOn, type DeviceX } from "../connect";
  import Icon from "../components/Icon.svelte";

  const on = $derived(playingOn(app.devices as DeviceX[]));
  const other = $derived(!on && !app.playing && !jamActive() ? connectTo(app.devices as DeviceX[]) : null);

  function open() { if (!app.showDevices) { app.showDevices = true; app.showQueue = false; } }
  async function take(d: DeviceX) {
    const list = await backend().call<DeviceX[] | undefined>("device_connect", { id: d.id });
    if (list) app.devices = list;
  }
</script>

{#if on}
  <button class="strip on" onclick={open}>
    <Icon name={on.icon} size={12} color="rgba(10,10,10,1)" />
    <span class="l trunc">{tf("connect.playing_on", { name: on.name })}</span>
  </button>
{:else if other}
  <button class="strip" onclick={() => take(other)}>
    <Icon name={deviceIcon(other)} size={12} color="var(--dim)" />
    <span class="l trunc">{tf("connect.connect_to", { name: other.name })}</span>
  </button>
{/if}

<style>
  .strip { width: fit-content; max-width: 100%; height: 20px; padding: 0 8px; border-radius: 6px; display: inline-flex; align-items: center; gap: 6px; background: rgba(255, 255, 255, .06); color: var(--dim); transition: background-color .15s ease-out, color .15s ease-out, opacity .15s; animation: rise-in .2s var(--ease) both; }
  .strip:hover { background: rgba(255, 255, 255, .11); color: var(--text); }
  .strip.on { background: var(--accent); color: rgba(10, 10, 10, 1); }
  .strip.on:hover { background: var(--accent); color: rgba(10, 10, 10, 1); opacity: .92; }
  .l { font-size: 11px; font-weight: 600; line-height: 1; }
</style>
