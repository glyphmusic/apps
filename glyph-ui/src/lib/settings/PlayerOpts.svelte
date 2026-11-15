<script lang="ts">
  // settings/player.rs player: Playback > Player. Now Playing visual preview
  // (lyrics & download button) and the way to the shortcuts (not on phones).
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import Button from "../components/Button.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import NowPlayingVisual from "./NowPlayingVisual.svelte";
  import { setTab } from "./settings.svelte";
  import { isMobile } from "./util";

  const touch = $derived(isMobile());
</script>

<div class="col">
  <NowPlayingVisual />
  {#if !touch}
    <Rows>
      <!-- All shortcuts (zen mode, download, mouse wheel...) are configured in Shortcuts. -->
      <Row label={t("sp.player.keys")} info={t("sp.player.keys.desc")}>
        <Button onclick={() => setTab("about", 2)}>{t("sp.player.keys.open")}</Button>
      </Row>
    </Rows>
  {/if}
</div>

<style>
  .col { display: flex; flex-direction: column; gap: 14px; }
</style>
