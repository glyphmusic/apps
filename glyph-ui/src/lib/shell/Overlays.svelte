<script lang="ts">
  // Everything that floats over the page: song / list menu, player control
  // menus, dialogs, the mini player, splash and the first-start tour.
  import { app } from "../state.svelte";
  import { isTauri } from "../api/backend";
  import { isWebMiniFallback } from "../web-mini.svelte";
  import ContextMenu from "../overlays/ContextMenu.svelte";
  import ControlMenu from "../overlays/ControlMenu.svelte";
  import NameDialog from "../overlays/NameDialog.svelte";
  import ConfirmDialog from "../overlays/ConfirmDialog.svelte";
  import McpApprove from "../overlays/McpApprove.svelte";
  import MiniPlayer from "../overlays/MiniPlayer.svelte";
  import Jam from "../overlays/Jam.svelte";
  import Onboarding from "../overlays/Onboarding.svelte";
  import Splash from "../overlays/Splash.svelte";
  import CropDialog from "../pages/profile/CropDialog.svelte";
</script>

<!-- Tauri: the mini player is a window of its own (App.svelte, #mini); Web opens a separate popup, falling back to in-page if blocked. -->
{#if app.miniShown && !isTauri() && isWebMiniFallback()}<MiniPlayer />{/if}
{#if app.ctx}<ContextMenu />{/if}
{#if app.controlMenu}<ControlMenu />{/if}
{#if app.nameDialog}<NameDialog />{/if}
{#if app.confirm}<ConfirmDialog />{/if}
{#if app.jamOpen}<Jam />{/if}
{#if app.onboarding}<Onboarding />{/if}
{#if app.splash}<Splash />{/if}
<McpApprove />
<CropDialog />
