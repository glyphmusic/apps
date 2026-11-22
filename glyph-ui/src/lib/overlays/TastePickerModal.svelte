<script lang="ts">
  // Standalone Taste Picker modal opened from Taste Profile ("Tune taste")
  // so users can add or change their favorite seed artists and songs anytime.
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t } from "../i18n.svelte";
  import type { TasteProfile, Track } from "../api/types";
  import ModalLayer from "./ModalLayer.svelte";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import TastePicker from "../pages/onboarding/TastePicker.svelte";

  let selectedArtists = $state<string[]>([]);
  let selectedTracks = $state<Track[]>([]);
  let selectedGenres = $state<string[]>([]);
  let saving = $state(false);

  onMount(() => {
    backend()
      .call<TasteProfile>("taste_profile")
      .then((p) => {
        if (!p) return;
        selectedArtists = [...(p.onboarding_artists ?? [])];
        selectedTracks = [...(p.onboarding_tracks ?? [])];
        selectedGenres = [...(p.onboarding_genres ?? [])];
      })
      .catch(() => {});
  });

  function close() {
    if (!saving) app.tastePickerOpen = false;
  }

  async function save() {
    saving = true;
    try {
      await backend().call("taste_onboarding_save", {
        artists: $state.snapshot(selectedArtists),
        tracks: $state.snapshot(selectedTracks),
        genres: $state.snapshot(selectedGenres),
        follow_artists: false,
      });
      app.toast(t("taste.toast.saved"), "ok");
      app.tastePickerOpen = false;
    } catch {
      app.toast(t("taste.toast.save_failed"), "info");
    } finally {
      saving = false;
    }
  }
</script>

<ModalLayer dim={0.5} blur={16} vis={1} z={82} onbackdrop={close}>
  <div class="card" role="dialog" aria-label={t("taste.picker.modal_title")}>
    <div class="head">
      <div class="htx">
        <div class="title">{t("taste.picker.modal_title")}</div>
        <div class="sub">{t("taste.picker.modal_sub")}</div>
      </div>
      <button type="button" class="xbtn" onclick={close} aria-label={t("tip.x")}>
        <Icon name="x" size={16} color="var(--dim)" />
      </button>
    </div>

    <div class="stage">
      <TastePicker bind:selectedArtists bind:selectedTracks bind:selectedGenres />
    </div>

    <div class="foot">
      <Button onclick={close}>{t("common.cancel")}</Button>
      <Button kind="primary" disabled={saving} onclick={save}>
        {t("taste.picker.save")}
      </Button>
    </div>
  </div>
</ModalLayer>

<style>
  .card {
    width: min(calc(100vw - 32px), 660px);
    max-height: min(calc(100vh - 48px), 620px);
    padding: 22px;
    border-radius: 16px;
    background: rgba(19, 19, 19, .985);
    display: flex;
    flex-direction: column;
    gap: 14px;
    font-family: var(--font);
  }
  .head { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; }
  .htx { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .title { font-size: 18px; font-weight: 700; color: var(--text); }
  .sub { font-size: 13px; color: var(--dim); }
  .xbtn { width: 30px; height: 30px; border-radius: 8px; display: grid; place-items: center; flex: none; }
  .xbtn:hover { background: rgba(255, 255, 255, .08); }
  .stage {
    height: min(400px, 56vh);
    border-radius: 12px;
    background: rgba(255, 255, 255, .035);
    overflow: hidden;
    display: flex;
  }
  .foot { display: flex; align-items: center; justify-content: flex-end; gap: 8px; }
</style>
