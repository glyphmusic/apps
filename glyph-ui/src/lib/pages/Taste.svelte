<script lang="ts">
  // Visible Taste Profile page (/taste): genres, moods, listening habits
  // (time-of-day, completion/skip rate, version/style preferences), interactive
  // Likes & Dislikes ("Mag ich / Mag ich nicht"), and optional server
  // collaborative recommendations. Follows DESIGN.md flat UI rules.
  import { backend } from "../api/backend";
  import { app } from "../state.svelte";
  import { t, tf } from "../i18n.svelte";
  import { trackKey, type TasteProfile, type Track } from "../api/types";
  import BackPill from "./lists/BackPill.svelte";
  import SectionTitle from "../components/SectionTitle.svelte";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import ArtistAvatar from "../components/ArtistAvatar.svelte";
  import Cover from "../components/Cover.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import SkeletonSwap from "../components/SkeletonSwap.svelte";
  import Tile from "./stats/Tile.svelte";
  import Bars from "./stats/Bars.svelte";

  let profile = $state<TasteProfile | null>(null);
  let loading = $state(true);
  let refreshing = $state(false);

  const tasteEnabled = $derived(app.config?.taste?.enabled !== false);
  const serverCollab = $derived(app.config?.taste?.server_collab !== false);

  async function loadProfile() {
    loading = !profile;
    try {
      const res = await backend().call<TasteProfile>("taste_profile");
      if (res) profile = res;
    } catch {
      /* ignore */
    } finally {
      loading = false;
    }
  }

  // Reload whenever Taste opens, config toggles, or library changes after Taste Picker closes.
  $effect(() => {
    void tasteEnabled;
    void serverCollab;
    void app.tastePickerOpen;
    void app.library.liked.length;
    void app.library.followed.length;
    void app.library.hidden_artists?.length;
    void app.library.disliked_tracks?.length;
    void loadProfile();
  });

  async function refresh() {
    if (refreshing) return;
    refreshing = true;
    try {
      const res = await backend().call<TasteProfile>("taste_refresh");
      if (res) profile = res;
      app.toast(t("taste.toast.refreshed"), "ok");
    } finally {
      refreshing = false;
    }
  }

  async function setTagPref(tag: string, cur: number, target: 1 | -1) {
    const next = cur === target ? null : target;
    const res = await backend().call<TasteProfile>("taste_tag_feedback", { tag, preference: next });
    if (res) profile = res;
  }

  const DEFAULT_ODD_STYLES = new Set(["karaoke", "instrumental", "8d", "nightcore", "sped_up", "slowed", "cover"]);

  async function cycleVersion(
    style: string,
    curFeedback: number,
    curState: "prefer" | "avoid" | "neutral",
    plays: number,
    skips: number,
  ) {
    // Cycle cleanly so every click transitions to a new visible state:
    // neutral -> prefer (1) -> avoid (-1) -> neutral (null);
    // default-odd styles (which default to avoid when unplayed) toggle avoid <-> prefer.
    const defaultIsAvoid = DEFAULT_ODD_STYLES.has(style) && plays <= skips;
    let next: number | null;
    if (curState === "neutral") {
      next = 1;
    } else if (curState === "prefer") {
      next = curFeedback > 0 && defaultIsAvoid ? null : -1;
    } else {
      next = curFeedback < 0 && !defaultIsAvoid ? null : 1;
    }
    const res = await backend().call<TasteProfile>("taste_version_feedback", { style, preference: next });
    if (res) profile = res;
  }

  async function restoreArtist(name: string, kind: string) {
    if (kind === "hidden") {
      await backend().call("toggle_hidden_artist", { name });
    }
    await backend().call("recommend_feedback", { artist: name, preference: null });
    await loadProfile();
  }

  async function restoreTrack(track: Track) {
    const res = await backend().call<TasteProfile>("taste_dislike_track", { track, disliked: false });
    if (res) profile = res;
  }

  function confirmReset() {
    app.confirm = {
      title: t("taste.reset.title"),
      body: t("taste.reset.body"),
      ok: t("taste.reset.ok"),
      danger: true,
      run: async () => {
        const res = await backend().call<TasteProfile>("taste_reset");
        if (res) profile = res;
        app.toast(t("taste.toast.reset"), "info");
      },
    };
  }

  const periodLabels = $derived([
    t("taste.period.morning"),
    t("taste.period.afternoon"),
    t("taste.period.evening"),
    t("taste.period.night"),
  ]);
  const peakIdx = $derived.by(() => {
    const p = profile?.habits.peak_period;
    return p === "morning" ? 0 : p === "afternoon" ? 1 : p === "evening" ? 2 : 3;
  });
  const varietyLabel = $derived.by(() => {
    const v = profile?.habits.variety_score ?? 0;
    if (v >= 0.72) return t("taste.variety.high");
    if (v >= 0.4) return t("taste.variety.medium");
    return t("taste.variety.focused");
  });
  const isServerMode = $derived(profile?.mode === "server" || profile?.mode === "hybrid");
  const modeLabel = $derived(
    isServerMode
      ? t("taste.mode.server")
      : profile?.mode === "off"
        ? t("taste.mode.off")
        : t("taste.mode.local"),
  );
  function toggleLocalOptOut() {
    app.setConfig((c) => {
      c.taste = {
        enabled: c.taste?.enabled ?? true,
        server_collab: !(c.taste?.server_collab ?? true),
        metadata_enrichment: c.taste?.metadata_enrichment ?? true,
      };
    });
  }
  const intentMode = $derived(profile?.intent?.mode ?? "balanced");
  const empty = $derived(
    !!profile &&
      profile.signals_count === 0 &&
      !profile.top_genres.length &&
      !profile.liked_artists.length &&
      !profile.disliked_artists.length &&
      !profile.disliked_tracks.length,
  );
</script>

<div class="pg">
  <BackPill />

  <div class="head">
    <div class="hl">
      {#if !app.compact}<SectionTitle heading title={t("taste.title")} />{/if}
      {#if profile}
        <button type="button" class="mode-pill" class:hybrid={isServerMode} onclick={toggleLocalOptOut}>
          <Icon name={isServerMode ? "globe" : profile.mode === "off" ? "eye-off" : "lock"} size={12} color={isServerMode ? "var(--accent)" : "var(--dim)"} />
          <span>{modeLabel}</span>
        </button>
      {/if}
    </div>
    <div class="hacts">
      {#if tasteEnabled}
        <Button icon="sparkles" kind="primary" onclick={() => (app.tastePickerOpen = true)}>{t("taste.tune")}</Button>
        <Button icon="refresh-cw" disabled={refreshing} onclick={refresh}>{t("taste.refresh")}</Button>
      {/if}
      <Button icon="settings" onclick={() => app.openSettings("playback")}>{t("nav.settings")}</Button>
    </div>
  </div>

  {#if !tasteEnabled}
    <div class="off">
      <EmptyState icon="sparkles" title={t("taste.off.title")} sub={t("taste.off.sub")} />
      <Button kind="primary" onclick={() => app.setConfig((c) => { if (!c.taste) c.taste = { enabled: true, server_collab: true, metadata_enrichment: true }; else c.taste.enabled = true; })}>
        {t("taste.enable")}
      </Button>
    </div>
  {:else}
    <SkeletonSwap loading={loading && !profile}>
      {#snippet skeleton()}
        <div class="tiles">{#each [0, 1, 2, 3] as i (i)}<div class="sk"><Skeleton w="100%" h={88} radius={12} id="taste-{i}" /></div>{/each}</div>
      {/snippet}

      {#if empty}
        <div class="off">
          <EmptyState icon="sparkles" title={t("taste.empty.title")} sub={t("taste.empty.sub")} />
          <Button kind="primary" icon="sparkles" onclick={() => (app.tastePickerOpen = true)}>{t("taste.tune")}</Button>
        </div>
      {:else if profile}
        <div class="body">
          <!-- Summary Tiles -->
          <div class="tiles">
            <Tile
              label={t("taste.tile.completion")}
              value={profile.habits.total_plays + profile.habits.total_skips > 0 ? `${Math.round(profile.habits.completion_rate * 100)}%` : "—"}
              sub={tf("taste.tile.skip_rate", { pct: Math.round(profile.habits.early_skip_rate * 100) })}
            />
            <Tile
              label={t("taste.tile.peak")}
              value={periodLabels[peakIdx]}
              sub={t(`taste.period.${profile.habits.peak_period}.hours`)}
              delay={40}
            />
            <Tile
              label={t("taste.tile.variety")}
              value={varietyLabel}
              sub={tf("taste.tile.artists_count", { n: profile.liked_artists.length })}
              delay={80}
            />
            <Tile
              label={t("taste.tile.profile")}
              value={`${Math.round(profile.confidence * 100)}%`}
              sub={tf("taste.tile.signals", { n: profile.signals_count })}
              delay={120}
            />
          </div>

          <!-- Active Session Intention -->
          {#if profile.intent}
            <section class="sec">
              <SectionTitle icon="activity" title={t("taste.intent.title")} />
              <div class="card intent-card">
                <div class="intent-main">
                  <div class="intent-head">
                    <span class="intent-pill">{t(`taste.intent.mode.${intentMode}`)}</span>
                    {#if profile.intent.vibe_pivot}
                      <span class="intent-pivot">{t("taste.intent.vibe_pivot")}</span>
                    {/if}
                  </div>
                  <div class="intent-desc">{t(`taste.intent.desc.${intentMode}`)}</div>
                  {#if profile.intent.focus_genres.length || profile.intent.focus_moods.length}
                    <div class="intent-tags">
                      {#each [...profile.intent.focus_genres, ...profile.intent.focus_moods].slice(0, 5) as tag (tag)}
                        <span class="itag">{tag}</span>
                      {/each}
                    </div>
                  {/if}
                </div>
                <div class="intent-meters">
                  <div class="meter">
                    <div class="mlbl"><span>{t("taste.intent.energy")}</span><span class="mono">{Math.round(profile.intent.energy * 100)}%</span></div>
                    <span class="track"><span class="fill" style:width="{Math.max(8, Math.round(profile.intent.energy * 100))}%"></span></span>
                  </div>
                  <div class="meter">
                    <div class="mlbl"><span>{t("taste.intent.novelty")}</span><span class="mono">{Math.round(profile.intent.novelty * 100)}%</span></div>
                    <span class="track"><span class="fill" style:width="{Math.max(8, Math.round(profile.intent.novelty * 100))}%"></span></span>
                  </div>
                </div>
              </div>
            </section>
          {/if}

          <!-- Top Genres & Moods -->
          <div class="two">
            <section class="sec">
              <SectionTitle icon="music-2" title={t("taste.genres.title")} />
              <div class="card list">
                {#if !profile.top_genres.length}
                  <div class="muted">{t("taste.genres.empty")}</div>
                {:else}
                  {#each profile.top_genres as g, i (g.name)}
                    <div class="grow">
                      <span class="gn trunc">{g.name}</span>
                      <span class="track"><span class="fill" style:width="{Math.max(8, g.score * 100)}%" style:animation-delay="{i * 25}ms"></span></span>
                      <span class="pct mono">{g.share_pct}%</span>
                      <div class="fb">
                        <button type="button" class="fbb" class:up={g.feedback > 0} aria-label={t("taste.like_tag")} onclick={() => setTagPref(g.name, g.feedback, 1)}>
                          <Icon name={g.feedback > 0 ? "thumbs-up-fill" : "thumbs-up"} size={13} color={g.feedback > 0 ? "var(--accent)" : "var(--dim)"} />
                        </button>
                        <button type="button" class="fbb" class:dn={g.feedback < 0} aria-label={t("taste.dislike_tag")} onclick={() => setTagPref(g.name, g.feedback, -1)}>
                          <Icon name="thumbs-down" size={13} color={g.feedback < 0 ? "var(--text)" : "var(--dim)"} />
                        </button>
                      </div>
                    </div>
                  {/each}
                {/if}
              </div>
            </section>

            <section class="sec">
              <SectionTitle icon="sparkles" title={t("taste.moods.title")} />
              <div class="card pad">
                {#if !profile.moods.length}
                  <div class="muted">{t("taste.moods.empty")}</div>
                {:else}
                  <div class="moods">
                    {#each profile.moods as m (m.name)}
                      <button
                        type="button"
                        class="mpill"
                        class:liked={m.feedback > 0}
                        style:--k={m.score}
                        onclick={() => setTagPref(m.name, m.feedback, 1)}
                      >
                        <span>{m.name}</span>
                        <span class="mpct mono">{m.share_pct}%</span>
                      </button>
                    {/each}
                  </div>
                {/if}
                <div class="subnote">{t("taste.moods.hint")}</div>
                {#if profile.top_eras?.length}
                  <div class="eras">
                    <span class="eralbl">{t("taste.eras.title")}</span>
                    {#each profile.top_eras as era (era.name)}
                      <span class="itag">{era.name} · {era.share_pct}%</span>
                    {/each}
                  </div>
                {/if}
              </div>
            </section>
          </div>

          <!-- Listening Behavior: Time of Day + Style / Version Preferences -->
          <div class="two">
            <section class="sec">
              <SectionTitle icon="clock" title={t("taste.habits.time_title")} />
              <div class="card cbox">
                <Bars values={profile.habits.time_of_day} labels={periodLabels} highlight={peakIdx} />
              </div>
            </section>

            <section class="sec">
              <SectionTitle icon="sliders-horizontal" title={t("taste.versions.title")} />
              <div class="card pad">
                <div class="subnote">{t("taste.versions.hint")}</div>
                <div class="vgrid">
                  {#each profile.habits.versions as v (v.style)}
                    {@const state = v.feedback > 0 || (v.feedback === 0 && v.score > 0.15) ? "prefer" : v.feedback < 0 || (v.feedback === 0 && v.score < -0.25) ? "avoid" : "neutral"}
                    <button
                      type="button"
                      class="vpill {state}"
                      onclick={() => cycleVersion(v.style, v.feedback, state, v.plays, v.skips)}
                    >
                      <span class="vn">{t(`taste.version.${v.style}`)}</span>
                      <span class="vs">
                        {state === "prefer" ? t("taste.version.state_prefer") : state === "avoid" ? t("taste.version.state_avoid") : t("taste.version.state_neutral")}
                      </span>
                    </button>
                  {/each}
                </div>
              </div>
            </section>
          </div>

          <!-- Likes & Dislikes ("Mag ich / Mag ich nicht") -->
          <div class="two">
            <section class="sec">
              <SectionTitle icon="thumbs-up" title={t("taste.likes.title")} />
              <div class="card list">
                {#if !profile.liked_artists.length}
                  <div class="muted">{t("taste.likes.empty")}</div>
                {:else}
                  {#each profile.liked_artists.slice(0, 10) as a, i (a.name)}
                    <button type="button" class="arow" onclick={() => app.openArtistPage(a.name)}>
                      <ArtistAvatar name={a.name} size={32} />
                      <span class="atx">
                        <span class="an trunc">{a.name}</span>
                        {#if a.genres.length || a.reasons.length}
                          <span class="ameta trunc">
                            {[...a.genres, ...a.reasons.slice(0, 2).map((r) => t(`taste.reason.${r}`))].join(" · ")}
                          </span>
                        {/if}
                      </span>
                      <span class="mini-track"><span class="fill" style:width="{Math.max(10, a.score * 100)}%" style:animation-delay="{i * 25}ms"></span></span>
                    </button>
                  {/each}
                {/if}
              </div>
            </section>

            <section class="sec">
              <SectionTitle icon="thumbs-down" title={t("taste.dislikes.title")} />
              <div class="card list">
                {#if !profile.disliked_artists.length && !profile.disliked_tracks.length && !profile.avoided_genres.length}
                  <div class="muted">{t("taste.dislikes.empty")}</div>
                {:else}
                  {#each profile.disliked_artists as d (d.name)}
                    <div class="drow">
                      <ArtistAvatar name={d.name} size={30} />
                      <span class="atx">
                        <span class="an trunc">{d.name}</span>
                        <span class="ameta trunc">
                          {d.kind === "high_skips"
                            ? tf("taste.dislike.high_skips", { n: d.skips })
                            : t(`taste.dislike.${d.kind}`)}
                        </span>
                      </span>
                      <button type="button" class="undo" onclick={() => restoreArtist(d.name, d.kind)} aria-label={t("taste.restore")}>
                        <Icon name="rotate-ccw" size={13} color="var(--dim)" />
                      </button>
                    </div>
                  {/each}

                  {#each profile.disliked_tracks.slice(0, 8) as tr (trackKey(tr))}
                    <div class="drow">
                      <Cover track={tr} size={30} radius={6} />
                      <span class="atx">
                        <span class="an trunc">{tr.title}</span>
                        <span class="ameta trunc">{tr.artist}</span>
                      </span>
                      <button type="button" class="undo" onclick={() => restoreTrack(tr)} aria-label={t("taste.restore")}>
                        <Icon name="rotate-ccw" size={13} color="var(--dim)" />
                      </button>
                    </div>
                  {/each}

                  {#if profile.avoided_genres.length}
                    <div class="avtags">
                      {#each profile.avoided_genres as ag (ag.name)}
                        <button type="button" class="avchip" onclick={() => setTagPref(ag.name, ag.feedback, -1)}>
                          <span>{ag.name}</span>
                          <Icon name="x" size={11} color="var(--dim)" />
                        </button>
                      {/each}
                    </div>
                  {/if}
                {/if}
              </div>
            </section>
          </div>

          <!-- Collaborative Recommendations / Privacy Status -->
          <section class="sec">
            <SectionTitle icon="users" title={t("taste.collab.title")} />
            {#if !serverCollab}
              <div class="card pad collab-off">
                <div class="ctext">
                  <div class="ctitle">{t("taste.collab.off_title")}</div>
                  <div class="csub">{t("taste.collab.off_sub")}</div>
                </div>
                <Button onclick={() => app.setConfig((c) => { if (!c.taste) c.taste = { enabled: true, server_collab: true, metadata_enrichment: true }; else c.taste.server_collab = true; })}>
                  {t("taste.collab.enable")}
                </Button>
              </div>
            {:else if !profile.collab_recs.length}
              <div class="card pad">
                <div class="muted">{t("taste.collab.empty")}</div>
              </div>
            {:else}
              <div class="cgrid">
                {#each profile.collab_recs.slice(0, 8) as rec (rec.artist)}
                  <button type="button" class="ccard" onclick={() => app.openArtistPage(rec.artist)}>
                    <ArtistAvatar name={rec.artist} size={40} />
                    <span class="atx">
                      <span class="an trunc">{rec.artist}</span>
                      {#if rec.because_of.length}
                        <span class="ameta trunc">{tf("taste.collab.because", { artists: rec.because_of.slice(0, 2).join(", ") })}</span>
                      {:else if rec.genres.length}
                        <span class="ameta trunc">{rec.genres.slice(0, 2).join(" · ")}</span>
                      {/if}
                    </span>
                  </button>
                {/each}
              </div>
            {/if}
          </section>

          <div class="footer">
            <div class="note">{t(serverCollab ? "taste.note.server" : "taste.note.local")}</div>
            <div class="factions">
              <Button kind="text" onclick={toggleLocalOptOut}>
                {t(serverCollab ? "taste.switch_local" : "taste.switch_server")}
              </Button>
              <Button kind="text" onclick={confirmReset}>{t("taste.reset.button")}</Button>
            </div>
          </div>
        </div>
      {/if}
    </SkeletonSwap>
  {/if}
</div>

<style>
  .pg, .body { display: flex; flex-direction: column; gap: 24px; min-width: 0; }
  .head { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 12px; }
  .hl { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  .hacts { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .mode-pill {
    height: 26px;
    padding: 0 11px;
    border-radius: 999px;
    background: rgba(255, 255, 255, .07);
    color: var(--dim);
    font-size: 12px;
    font-weight: 500;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    transition: background-color .14s ease-out, color .14s ease-out;
  }
  .mode-pill:hover { background: rgba(255, 255, 255, .12); color: var(--text); }
  .mode-pill.hybrid { background: color-mix(in srgb, var(--accent) 16%, rgba(255, 255, 255, .05)); color: var(--text); }
  .off { display: flex; flex-direction: column; align-items: flex-start; gap: 14px; }
  .tiles { display: flex; flex-wrap: wrap; gap: 10px; }
  .sk { flex: 1; min-width: 150px; }
  .two { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px; }
  :global(html.compact) .two { grid-template-columns: minmax(0, 1fr); }
  @media (max-width: 820px) {
    .two { grid-template-columns: minmax(0, 1fr); }
  }
  .sec { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
  .card { background: var(--card); border-radius: 12px; min-width: 0; }
  .list { display: flex; flex-direction: column; gap: 2px; padding: 10px; }
  .pad { padding: 14px 16px; display: flex; flex-direction: column; gap: 12px; }
  .cbox { padding: 16px 18px; }
  .muted { font-size: var(--sm); color: var(--faint); padding: 8px 4px; }
  .subnote { font-size: var(--xs); color: var(--faint); }
  .grow { display: flex; align-items: center; gap: 10px; height: 36px; padding: 0 6px; border-radius: 8px; }
  .grow:hover { background: rgba(255, 255, 255, .04); }
  .gn { width: 122px; flex: none; font-size: var(--sm); font-weight: 500; color: var(--text); }
  .track { flex: 1; height: 5px; border-radius: 3px; background: rgba(255, 255, 255, .07); overflow: hidden; }
  .mini-track { width: 72px; flex: none; height: 4px; border-radius: 2px; background: rgba(255, 255, 255, .07); overflow: hidden; }
  .fill { display: block; height: 100%; border-radius: 3px; background: var(--accent); transform-origin: left; animation: fillin .35s var(--ease) both; }
  @keyframes fillin { from { transform: scaleX(0); } to { transform: scaleX(1); } }
  .pct { width: 34px; flex: none; text-align: right; font-size: var(--xs); color: var(--dim); }
  .fb { display: flex; align-items: center; gap: 2px; flex: none; }
  .fbb { width: 26px; height: 26px; border-radius: 6px; display: grid; place-items: center; transition: background-color .12s; }
  .fbb:hover { background: rgba(255, 255, 255, .09); }
  .moods { display: flex; flex-wrap: wrap; gap: 8px; }
  .mpill {
    height: 32px;
    padding: 0 12px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) calc(12% + var(--k, 0.5) * 24%), rgba(255, 255, 255, .05));
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    transition: transform .14s var(--ease), background-color .14s;
  }
  .mpill:hover { transform: translateY(-1px); }
  .mpill.liked { background: var(--accent); color: #000; font-weight: 600; }
  .mpill.liked .mpct { color: rgba(0, 0, 0, .7); }
  .mpct { font-size: 11px; color: var(--dim); }
  .vgrid { display: flex; flex-wrap: wrap; gap: 6px; }
  .vpill {
    height: 30px;
    padding: 0 10px;
    border-radius: 8px;
    background: rgba(255, 255, 255, .06);
    color: var(--dim);
    font-size: 12px;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    transition: background-color .14s ease-out, color .14s ease-out;
  }
  .vpill:hover { background: rgba(255, 255, 255, .11); color: var(--text); }
  .vpill.prefer { background: color-mix(in srgb, var(--accent) 22%, rgba(255, 255, 255, .06)); color: var(--text); }
  .vpill.avoid { background: rgba(255, 255, 255, .035); color: var(--faint); }
  .vn { font-weight: 500; }
  .vs { font-size: 11px; opacity: .75; }
  .arow, .drow {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 46px;
    padding: 0 8px;
    border-radius: 8px;
    text-align: left;
    min-width: 0;
  }
  .arow { transition: background-color .12s ease-out; }
  .arow:hover { background: rgba(255, 255, 255, .05); }
  .atx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .an { font-size: var(--md); font-weight: 500; color: var(--text); }
  .ameta { font-size: var(--xs); color: var(--dim); }
  .undo { width: 28px; height: 28px; border-radius: 7px; display: grid; place-items: center; flex: none; }
  .undo:hover { background: rgba(255, 255, 255, .09); }
  .avtags { display: flex; flex-wrap: wrap; gap: 6px; padding: 8px 6px 2px; }
  .avchip {
    height: 26px;
    padding: 0 9px;
    border-radius: 999px;
    background: rgba(255, 255, 255, .06);
    color: var(--dim);
    font-size: 12px;
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .avchip:hover { background: rgba(255, 255, 255, .12); color: var(--text); }
  .collab-off { flex-direction: row; align-items: center; justify-content: space-between; flex-wrap: wrap; }
  .ctext { display: flex; flex-direction: column; gap: 3px; min-width: 0; flex: 1; }
  .ctitle { font-size: var(--md); font-weight: 500; color: var(--text); }
  .csub { font-size: var(--sm); color: var(--dim); }
  .cgrid { display: grid; grid-template-columns: repeat(auto-fill, minmax(210px, 1fr)); gap: 10px; }
  .ccard {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--card);
    text-align: left;
    min-width: 0;
    transition: background-color .14s ease-out;
  }
  .ccard:hover { background: var(--card-hover); }
  .footer { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 10px; padding-top: 4px; }
  .factions { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .note { font-size: var(--xs); color: var(--faint); flex: 1; min-width: 240px; }
  .intent-card {
    padding: 14px 16px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 16px;
  }
  .intent-main { display: flex; flex-direction: column; gap: 6px; flex: 1; min-width: 220px; }
  .intent-head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .intent-pill {
    height: 24px;
    padding: 0 10px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 20%, rgba(255, 255, 255, .05));
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
    display: inline-flex;
    align-items: center;
  }
  .intent-pivot {
    height: 22px;
    padding: 0 8px;
    border-radius: 999px;
    background: rgba(255, 255, 255, .08);
    color: var(--dim);
    font-size: 11px;
    display: inline-flex;
    align-items: center;
  }
  .intent-desc { font-size: var(--sm); color: var(--dim); }
  .intent-tags { display: flex; flex-wrap: wrap; gap: 6px; padding-top: 2px; }
  .itag {
    height: 24px;
    padding: 0 9px;
    border-radius: 999px;
    background: rgba(255, 255, 255, .06);
    color: var(--dim);
    font-size: 11.5px;
    display: inline-flex;
    align-items: center;
  }
  .intent-meters { display: flex; flex-direction: column; gap: 10px; width: 210px; flex: none; }
  .meter { display: flex; flex-direction: column; gap: 5px; }
  .mlbl { display: flex; justify-content: space-between; font-size: var(--xs); color: var(--dim); }
  .eras { display: flex; align-items: center; flex-wrap: wrap; gap: 6px; padding-top: 6px; border-top: 1px solid rgba(255, 255, 255, .06); }
  .eralbl { font-size: var(--xs); color: var(--faint); margin-right: 4px; }
</style>
