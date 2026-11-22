<script lang="ts">
  // Onboarding taste calibration stage: pick 3+ favorite artists and/or songs
  // to train the recommendation engine right from first launch.
  import TastePicker from "./TastePicker.svelte";
  import type { Track } from "../../api/types";

  let {
    selectedArtists = $bindable<string[]>([]),
    selectedTracks = $bindable<Track[]>([]),
    selectedGenres = $bindable<string[]>([]),
  }: {
    selectedArtists: string[];
    selectedTracks: Track[];
    selectedGenres: string[];
  } = $props();
</script>

<div class="wrap">
  <TastePicker bind:selectedArtists bind:selectedTracks bind:selectedGenres compact />
</div>

<style>
  .wrap { width: 100%; height: 100%; display: flex; }
</style>
