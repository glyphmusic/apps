<script lang="ts">
  // Easy Taste view: plays random songs one by one and asks "good or not?".
  // "Good" only adds the song to the taste picks (onboarding_tracks), never to
  // Liked Songs. "Not for me" marks it disliked (restorable on the advanced page).
  import { onDestroy, onMount } from "svelte";
  import { backend } from "../../api/backend";
  import { app } from "../../state.svelte";
  import { t, tf } from "../../i18n.svelte";
  import { trackKey, type DailyMix, type OnboardingCatalogResponse, type TasteProfile, type Track } from "../../api/types";
  import Button from "../../components/Button.svelte";
  import Cover from "../../components/Cover.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import Icon from "../../components/Icon.svelte";
  import Skeleton from "../../components/Skeleton.svelte";

  let queue = $state<Track[]>([]);
  let idx = $state(0);
  let rated = $state(0);
  let loading = $state(true);
  let started = $state(false);
  let seekedKey = "";

  // The taste picks as saved; good songs wait in `pending` and are saved together.
  let base = { artists: [] as string[], tracks: [] as Track[], genres: [] as string[] };
  let pending: Track[] = [];
  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  const card = $derived(queue[idx] ?? null);
  const cardKey = $derived(card ? trackKey(card) : "");
  const isCardPlaying = $derived(!!card && !!app.player.current && trackKey(app.player.current) === cardKey);

  function shuffle<T>(arr: T[]): T[] {
    const a = [...arr];
    for (let i = a.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [a[i], a[j]] = [a[j], a[i]];
    }
    return a;
  }

  async function build() {
    loading = true;
    idx = 0;
    const [profile, mixes, catalog] = await Promise.all([
      backend().call<TasteProfile>("taste_profile").catch(() => null),
      backend().call<DailyMix[]>("taste_mixes").catch(() => null),
      backend().call<OnboardingCatalogResponse>("taste_onboarding_catalog").catch(() => null),
    ]);
    if (profile) {
      base = {
        artists: [...(profile.onboarding_artists ?? [])],
        tracks: [...(profile.onboarding_tracks ?? [])],
        genres: [...(profile.onboarding_genres ?? [])],
      };
    }
    const skip = new Set<string>([
      ...app.library.liked.map(trackKey),
      ...(app.library.disliked_tracks ?? []),
      ...base.tracks.map(trackKey),
      ...pending.map(trackKey),
    ]);
    const pool: Track[] = [
      ...app.home.recs,
      ...(app.home.mixes ?? []).flatMap((m) => m.tracks),
      ...(mixes ?? []).flatMap((m) => m.tracks),
      ...app.home.trending,
      ...(catalog?.tracks ?? []),
      ...(catalog?.artists ?? []).flatMap((a) => (a.sample_track ? [a.sample_track] : [])),
    ];
    const seen = new Set<string>();
    const out: Track[] = [];
    for (const tr of pool) {
      const k = trackKey(tr);
      if (!tr.title?.trim() || skip.has(k) || seen.has(k)) continue;
      seen.add(k);
      out.push(tr);
    }
    queue = shuffle(out).slice(0, 60);
    loading = false;
    if (started) playCard();
  }

  function playCard() {
    if (!card) return;
    seekedKey = "";
    void app.play([card], 0, t("taste.title"));
  }

  function start() {
    started = true;
    playCard();
  }

  // Jump into the middle of the song once it plays (full songs only, not 30 s previews).
  $effect(() => {
    if (!started || !isCardPlaying || !app.playing || seekedKey === cardKey) return;
    seekedKey = cardKey;
    const dur = app.player.duration ?? 0;
    if (!app.player.preview && dur > 60 && app.position < 5) void app.seek(Math.min(dur * 0.35, 75));
  });

  /** Next card; after the last one the old test song stops instead of playing on. */
  function advance(count: boolean) {
    const wasPlaying = isCardPlaying;
    if (count) rated += 1;
    idx += 1;
    if (!started) return;
    if (idx < queue.length) playCard();
    else if (wasPlaying) void app.pause();
  }

  function good() {
    if (!card) return;
    const k = cardKey;
    pending = [card, ...pending.filter((x) => trackKey(x) !== k)];
    scheduleSave();
    advance(true);
  }

  function bad() {
    if (!card) return;
    const tr = $state.snapshot(card) as Track;
    const k = cardKey;
    const cur = app.library.disliked_tracks ?? [];
    if (!cur.includes(k)) app.library.disliked_tracks = [...cur, k];
    void backend().call("taste_dislike_track", { track: tr, disliked: true }).catch(() => {});
    advance(true);
  }

  function skipCard() {
    if (card) advance(false);
  }

  function scheduleSave() {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => void flush(), 4000);
  }

  async function flush() {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = null;
    if (!pending.length) return;
    const keys = new Set(pending.map(trackKey));
    const tracks = [...pending, ...base.tracks.filter((x) => !keys.has(trackKey(x)))];
    pending = [];
    base = { ...base, tracks };
    await backend()
      .call("taste_onboarding_save", {
        artists: base.artists,
        tracks: $state.snapshot(tracks),
        genres: base.genres,
        follow_artists: false,
      })
      .catch(() => app.toast(t("taste.toast.save_failed"), "info"));
  }

  onMount(() => void build());

  onDestroy(() => {
    void flush();
    // Leaving the page stops the test song (the rest of the app plays on as usual).
    if (started && app.playing && card && isCardPlaying) void app.pause();
  });
</script>

<div class="rate">
  <div class="intro">
    <div class="it">{t("taste.rate.title")}</div>
    <div class="is">{t("taste.rate.sub")}</div>
  </div>

  {#if loading}
    <div class="card">
      <Skeleton w="180px" h={180} radius={12} id="taste-rate" />
    </div>
  {:else if !queue.length}
    <div class="off">
      <EmptyState icon="music-2" title={t("taste.rate.empty.title")} sub={t("taste.rate.empty.sub")} />
      <Button icon="refresh-cw" onclick={() => { app.loadHome(); void build(); }}>{t("taste.rate.more")}</Button>
    </div>
  {:else if !card}
    <div class="off">
      <EmptyState icon="sparkles" title={t("taste.rate.done.title")} sub={t("taste.rate.done.sub")} />
      <Button icon="refresh-cw" onclick={() => { app.loadHome(); void build(); }}>{t("taste.rate.more")}</Button>
    </div>
  {:else}
    <div class="card">
      {#key cardKey}
        <div class="song">
          <button
            type="button"
            class="cv"
            aria-label={isCardPlaying && app.playing ? t("common.pause") : t("tip.play")}
            onclick={() => (!started ? start() : isCardPlaying ? app.togglePlay() : playCard())}
          >
            <Cover track={card} size={180} radius={12} />
            <span class="pp" class:show={!started || !(isCardPlaying && app.playing)}>
              <Icon name={isCardPlaying && app.playing ? "pause" : "play"} size={26} color="#000" />
            </span>
          </button>
          <div class="meta">
            <div class="ti trunc">{card.title}</div>
            <div class="ar trunc">{card.artist}</div>
          </div>
        </div>
      {/key}

      {#if !started}
        <Button kind="pill" icon="play" onclick={start}>{t("taste.rate.start")}</Button>
      {:else}
        <div class="acts">
          <button type="button" class="act bad" onclick={bad}>
            <Icon name="thumbs-down" size={18} />
            <span>{t("taste.rate.bad")}</span>
          </button>
          <button type="button" class="act skip" onclick={skipCard}>
            <Icon name="skip-forward" size={16} />
            <span>{t("taste.rate.skip")}</span>
          </button>
          <button type="button" class="act good" onclick={good}>
            <Icon name="thumbs-up" size={18} />
            <span>{t("taste.rate.good")}</span>
          </button>
        </div>
      {/if}

      <div class="count mono">{tf("taste.rate.count", { n: rated })}</div>
    </div>
  {/if}
</div>

<style>
  .rate { display: flex; flex-direction: column; gap: 16px; min-width: 0; }
  .intro { display: flex; flex-direction: column; gap: 4px; }
  .it { font-size: var(--lg, 18px); font-weight: 600; color: var(--text); }
  .is { font-size: var(--sm); color: var(--dim); }
  .off { display: flex; flex-direction: column; align-items: flex-start; gap: 14px; }
  .card {
    background: var(--card);
    border-radius: 12px;
    padding: 28px 20px 18px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 22px;
    min-width: 0;
  }
  .song { display: flex; flex-direction: column; align-items: center; gap: 14px; min-width: 0; max-width: 100%; animation: cardin .22s var(--ease) both; }
  @keyframes cardin { from { opacity: 0; transform: translateY(6px); } to { opacity: 1; transform: none; } }
  .cv { position: relative; border-radius: 12px; display: block; }
  .pp {
    position: absolute;
    right: 10px;
    bottom: 10px;
    width: 46px;
    height: 46px;
    border-radius: 999px;
    background: #fff;
    display: grid;
    place-items: center;
    opacity: 0;
    transform: scale(.9);
    transition: opacity .14s ease-out, transform .14s ease-out;
  }
  .cv:hover .pp, .pp.show { opacity: 1; transform: none; }
  .meta { display: flex; flex-direction: column; align-items: center; gap: 3px; min-width: 0; max-width: min(420px, 80vw); text-align: center; }
  .ti { font-size: var(--md); font-weight: 600; color: var(--text); max-width: 100%; }
  .ar { font-size: var(--sm); color: var(--dim); max-width: 100%; }
  .acts { display: flex; align-items: center; justify-content: center; gap: 10px; flex-wrap: wrap; }
  .act {
    height: 44px;
    padding: 0 18px;
    border-radius: 10px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: var(--sm);
    font-weight: 600;
    background: #202022;
    color: var(--text);
    transition: background-color .12s ease-out, transform .18s cubic-bezier(.34, 1.56, .64, 1);
  }
  .act:hover { background: #2a2a2e; }
  .act:active { transform: scale(.94); }
  .act.skip { height: 36px; padding: 0 14px; background: transparent; color: var(--dim); font-weight: 500; }
  .act.skip:hover { background: rgba(255, 255, 255, .06); color: var(--text); }
  .act.good { background: var(--text); color: #000; }
  .act.good:hover { background: #e6e6e6; }
  .count { font-size: var(--xs); color: var(--faint); }
</style>
