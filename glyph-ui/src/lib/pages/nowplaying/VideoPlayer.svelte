<script lang="ts">
  // Custom Glyph video player shared by Desktop Now Playing and Mobile NowPlayingSheet.
  // Supports progressive MP4, dual-stream HD (video + synced AAC audio), and the
  // interactive Demo canvas player when `stream.url` starts with `demo://`.
  import { onDestroy } from "svelte";
  import Icon from "../../components/Icon.svelte";
  import ThinBar from "../../components/ThinBar.svelte";
  import { app } from "../../state.svelte";
  import { t } from "../../i18n.svelte";
  import { fmtTime } from "../../fmt";
  import { hash } from "../../api/demo-data";
  import { videoState } from "./video-state.svelte";

  let {
    compact = false,
    onOpenPicker,
  }: {
    compact?: boolean;
    onOpenPicker?: () => void;
  } = $props();

  let stageEl: HTMLDivElement | undefined = $state();
  let videoEl: HTMLVideoElement | undefined = $state();
  let audioEl: HTMLAudioElement | undefined = $state();
  let canvasEl: HTMLCanvasElement | undefined = $state();

  let playing = $state(false);
  let buffering = $state(false);
  let pos = $state(0);
  let dur = $state(0);
  let muted = $state(false);
  let vol = $state(0.85);
  let mediaError = $state<string | null>(null);
  let failedUrls = $state<string[]>([]);
  let controlsShown = $state(true);
  let qualityOpen = $state(false);
  let pseudoFullscreen = $state(false);
  let nativeFullscreen = $state(false);

  let hideTimer = 0;
  let demoRaf = 0;
  let demoLastTs = 0;
  let pendingRestore: { pos: number; resume: boolean } | null = null;
  let resumeOnFallback = false;
  let lastStreamId = "";

  const isFullscreen = $derived(nativeFullscreen || pseudoFullscreen);
  const qualities = $derived(videoState.stream?.qualities ?? []);
  const activeQ = $derived.by(() => {
    const pref = videoState.activeQuality;
    if (!qualities.length) return pref;
    if (pref && !failedUrls.includes(pref.url)) return pref;
    const untried = qualities.filter((q) => !failedUrls.includes(q.url));
    if (!untried.length) return pref;
    return untried.find((q) => !q.audio_url) ?? untried[0];
  });
  const srcUrl = $derived(activeQ?.url ?? videoState.stream?.url ?? "");
  const audioUrl = $derived(activeQ?.audio_url ?? videoState.stream?.audio_url ?? null);
  const isDemo = $derived(srcUrl.startsWith("demo://"));
  const curTrack = $derived(app.player.current);
  const sel = $derived(videoState.selected);
  const frac = $derived(dur > 0 ? Math.min(1, Math.max(0, pos / dur)) : 0);

  // Clear per-stream fallback failures whenever a new video is resolved.
  $effect(() => {
    const id = videoState.stream?.id ?? "";
    if (id !== lastStreamId) {
      lastStreamId = id;
      failedUrls = [];
    }
  });

  function bumpControls() {
    controlsShown = true;
    clearTimeout(hideTimer);
    if (playing && !qualityOpen) {
      hideTimer = window.setTimeout(() => {
        controlsShown = false;
      }, 2800);
    }
  }

  // Keep controls visible while paused or menu open.
  $effect(() => {
    if (!playing || qualityOpen) {
      clearTimeout(hideTimer);
      controlsShown = true;
    } else {
      bumpControls();
    }
  });

  // Reset player state when switching stream / quality (and once <video> is mounted).
  let lastStreamKey = "";
  $effect(() => {
    const streamId = videoState.stream?.id ?? "";
    const url = srcUrl;
    const aUrl = audioUrl;
    const demo = isDemo;
    const v = videoEl;
    const a = audioEl;

    if (!url) {
      lastStreamKey = "";
      pendingRestore = null;
      playing = false;
      buffering = false;
      pos = 0;
      dur = 0;
      return;
    }

    // Wait until the real <video> element is actually mounted before consuming
    // the stream transition so metadata & autoplay hooks can never be missed.
    if (!demo && !v) return;

    const key = `${streamId}|${url}|${aUrl ?? ""}`;
    if (key === lastStreamKey) return;
    const sameVideo = !!streamId && lastStreamKey.startsWith(`${streamId}|`);
    const keepPos = sameVideo ? pos : 0;
    const shouldResume = sameVideo ? (playing || resumeOnFallback) : true;
    resumeOnFallback = false;
    lastStreamKey = key;
    mediaError = null;
    qualityOpen = false;

    if (demo) {
      pendingRestore = null;
      dur = (videoState.stream?.duration_ms ?? sel?.duration_ms ?? 228_000) / 1000;
      pos = sameVideo ? keepPos : Math.min(dur * 0.18, 42);
      buffering = false;
      playing = shouldResume;
      if (playing) videoState.notifyVideoPlaying();
      return;
    }

    // Real <video> element load & optional quality-switch position restore.
    pendingRestore = { pos: keepPos, resume: shouldResume };
    buffering = true;
    playing = false;
    if (!sameVideo) {
      pos = 0;
      dur = (videoState.stream?.duration_ms ?? sel?.duration_ms ?? 0) / 1000;
    }
    if (v) {
      v.volume = muted ? 0 : vol;
      v.muted = !!aUrl || muted;
      if (a) {
        a.volume = muted ? 0 : vol;
        a.muted = muted;
      }
      if (v.currentSrc === url && v.readyState >= 1) {
        onVideoLoadedMetadata();
      } else {
        v.load();
        a?.load();
      }
    }
  });

  // Sync volume & mute to real media elements.
  $effect(() => {
    if (isDemo) return;
    if (audioEl) {
      audioEl.volume = muted ? 0 : vol;
      audioEl.muted = muted;
      if (videoEl) videoEl.muted = true;
    } else if (videoEl) {
      videoEl.volume = muted ? 0 : vol;
      videoEl.muted = muted;
    }
  });

  // Demo cinema renderer loop.
  $effect(() => {
    if (!isDemo || !canvasEl) {
      cancelAnimationFrame(demoRaf);
      return;
    }
    const canvas = canvasEl;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    let running = true;
    demoLastTs = performance.now();

    const draw = (now: number) => {
      if (!running) return;
      const dt = Math.min(0.1, (now - demoLastTs) / 1000);
      demoLastTs = now;
      if (playing && dur > 0) {
        pos += dt;
        if (pos >= dur) {
          pos = 0;
        }
      }
      renderDemoFrame(ctx, canvas.width, canvas.height, pos, dur);
      demoRaf = requestAnimationFrame(draw);
    };
    demoRaf = requestAnimationFrame(draw);
    return () => {
      running = false;
      cancelAnimationFrame(demoRaf);
    };
  });

  function renderDemoFrame(
    ctx: CanvasRenderingContext2D,
    w: number,
    h: number,
    timeS: number,
    totalS: number,
  ) {
    const seed = `${curTrack?.artist ?? ""}|${curTrack?.title ?? ""}|${sel?.id ?? ""}`;
    const baseHue = hash(seed) % 360;
    const kind = sel?.kind ?? "official";
    const accentHue = (baseHue + (kind === "live" ? 36 : kind === "lyric" ? 155 : 52)) % 360;

    // Deep cinematic base gradient.
    const bg = ctx.createLinearGradient(0, 0, w, h);
    bg.addColorStop(0, `hsl(${baseHue}, 46%, 11%)`);
    bg.addColorStop(0.55, `hsl(${(baseHue + 22) % 360}, 34%, 7%)`);
    bg.addColorStop(1, `hsl(${accentHue}, 40%, 10%)`);
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, w, h);

    // Drifting light orbs / stage beams.
    const t0 = timeS * 0.55;
    if (kind === "live") {
      for (let i = 0; i < 4; i++) {
        const sway = Math.sin(t0 * (0.7 + i * 0.22) + i * 1.6) * w * 0.14;
        const gx = w * (0.18 + i * 0.22) + sway;
        ctx.save();
        ctx.beginPath();
        ctx.moveTo(gx - 28, 0);
        ctx.lineTo(gx + 28, 0);
        ctx.lineTo(gx + 155, h);
        ctx.lineTo(gx - 155, h);
        ctx.closePath();
        const beam = ctx.createLinearGradient(gx, 0, gx, h);
        const beamHue = i % 2 === 0 ? baseHue : accentHue;
        beam.addColorStop(0, `hsla(${beamHue}, 92%, 72%, 0.28)`);
        beam.addColorStop(0.75, `hsla(${beamHue}, 86%, 56%, 0.08)`);
        beam.addColorStop(1, "rgba(0,0,0,0)");
        ctx.fillStyle = beam;
        ctx.fill();
        ctx.restore();
      }
    } else {
      for (let i = 0; i < 3; i++) {
        const cx = w * (0.28 + i * 0.24) + Math.cos(t0 * (0.6 + i * 0.18) + i * 2.1) * (w * 0.12);
        const cy = h * (0.42 + (i % 2) * 0.18) + Math.sin(t0 * (0.5 + i * 0.2) + i) * (h * 0.1);
        const r = Math.min(w, h) * (0.42 + i * 0.08);
        const orb = ctx.createRadialGradient(cx, cy, 4, cx, cy, r);
        const oh = i === 1 ? accentHue : baseHue;
        orb.addColorStop(0, `hsla(${oh}, 88%, 62%, 0.34)`);
        orb.addColorStop(0.6, `hsla(${oh}, 78%, 45%, 0.10)`);
        orb.addColorStop(1, "rgba(0,0,0,0)");
        ctx.fillStyle = orb;
        ctx.fillRect(0, 0, w, h);
      }
    }

    // Subtle filmic horizontal scan / grain lines.
    ctx.fillStyle = "rgba(255,255,255,0.025)";
    for (let y = 0; y < h; y += 6) {
      ctx.fillRect(0, y, w, 1);
    }

    const titleText = curTrack?.title ?? sel?.title ?? "Music Video";
    const artistText = curTrack?.artist ?? sel?.channel ?? "";

    if (kind === "lyric") {
      // Kinetic lyric preview lines.
      const sampleLines = app.lyrics.lyrics?.lines?.map((l) => l.text).filter((l) => l.trim().length > 0) ?? [];
      const fallbackLines = [
        `${titleText}`,
        `Feel the signal drifting through the dark`,
        `${artistText} · turning static into light`,
        `Hold the tempo as the skyline fades`,
      ];
      const pool = sampleLines.length >= 3 ? sampleLines : fallbackLines;
      const idx = Math.floor(timeS / 3.8) % pool.length;
      const prevLine = pool[(idx + pool.length - 1) % pool.length];
      const curLine = pool[idx];
      const nextLine = pool[(idx + 1) % pool.length];

      ctx.textAlign = "center";
      ctx.fillStyle = "rgba(255,255,255,0.28)";
      ctx.font = "600 22px Inter, system-ui, sans-serif";
      ctx.fillText(prevLine.slice(0, 52), w / 2, h * 0.38);

      ctx.fillStyle = "rgba(255,255,255,0.95)";
      ctx.font = "700 34px Inter, system-ui, sans-serif";
      ctx.fillText(curLine.slice(0, 44), w / 2, h * 0.51);

      ctx.fillStyle = "rgba(255,255,255,0.34)";
      ctx.font = "600 22px Inter, system-ui, sans-serif";
      ctx.fillText(nextLine.slice(0, 52), w / 2, h * 0.63);
    } else if (kind === "topic") {
      // Centered vinyl/album tile.
      const box = Math.min(w, h) * 0.44;
      const bx = (w - box) / 2;
      const by = (h - box) / 2 - 14;
      ctx.fillStyle = `hsl(${baseHue}, 45%, 24%)`;
      ctx.beginPath();
      ctx.roundRect(bx, by, box, box, 16);
      ctx.fill();
      ctx.strokeStyle = "rgba(255,255,255,0.12)";
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(w / 2, by + box / 2, box * 0.28, 0, Math.PI * 2);
      ctx.stroke();
      ctx.fillStyle = "rgba(255,255,255,0.88)";
      ctx.textAlign = "center";
      ctx.font = "700 24px Inter, system-ui, sans-serif";
      ctx.fillText(titleText.slice(0, 38), w / 2, by + box + 34);
    } else {
      // Cinema widescreen music video frame + reactive horizon wave.
      const bars = 44;
      const bw = 6;
      const gap = 8;
      const totalW = bars * bw + (bars - 1) * gap;
      const sx = (w - totalW) / 2;
      const baseY = h * 0.68;
      for (let i = 0; i < bars; i++) {
        const norm = (i / (bars - 1)) * 2 - 1;
        const env = Math.max(0.18, 1 - norm * norm * 0.85);
        const wave = Math.abs(Math.sin(timeS * 3.1 + i * 0.45) * 0.65 + Math.cos(timeS * 1.9 - i * 0.27) * 0.35);
        const bh = Math.max(6, (playing ? wave : 0.22) * env * h * 0.24);
        ctx.fillStyle = i % 3 === 0 ? `hsla(${accentHue}, 88%, 70%, 0.65)` : "rgba(255,255,255,0.34)";
        ctx.beginPath();
        ctx.roundRect(sx + i * (bw + gap), baseY - bh / 2, bw, bh, 3);
        ctx.fill();
      }

      // Film letterbox bars for official cinema look.
      const barH = Math.round(h * 0.085);
      ctx.fillStyle = "rgba(0,0,0,0.68)";
      ctx.fillRect(0, 0, w, barH);
      ctx.fillRect(0, h - barH, w, barH);

      // Subtle lower-left cinema credit overlay (shifts softly).
      ctx.textAlign = "left";
      ctx.fillStyle = "rgba(255,255,255,0.58)";
      ctx.font = "600 14px Inter, system-ui, sans-serif";
      ctx.fillText(artistText.toUpperCase(), 38, h * 0.38);
      ctx.fillStyle = "rgba(255,255,255,0.94)";
      ctx.font = "700 36px Inter, system-ui, sans-serif";
      ctx.fillText(titleText.slice(0, 34), 38, h * 0.46);
    }

    // Tiny timecode accent in demo cinema mode.
    if (totalS > 0) {
      ctx.fillStyle = "rgba(255,255,255,0.06)";
      ctx.fillRect(0, h - 2, w * Math.min(1, timeS / totalS), 2);
    }
  }

  function onVideoLoadedMetadata() {
    const v = videoEl;
    if (!v || isDemo) return;
    // Reject streams without a video track (e.g., audio-only MP4 returned by an outdated server).
    if (v.videoWidth === 0 && v.videoHeight === 0) {
      onVideoError();
      return;
    }
    dur = Number.isFinite(v.duration) && v.duration > 0 ? v.duration : (videoState.stream?.duration_ms ?? sel?.duration_ms ?? 0) / 1000;
    const restore = pendingRestore;
    pendingRestore = null;
    if (restore && restore.pos > 0 && restore.pos < dur) {
      v.currentTime = restore.pos;
      if (audioEl && audioUrl) audioEl.currentTime = restore.pos;
    }
    buffering = false;
    if (restore?.resume) {
      void startPlayback();
    }
  }

  function onVideoCanPlay() {
    const v = videoEl;
    if (!v || isDemo) return;
    if (v.videoWidth === 0 && v.videoHeight === 0) return;
    if (Number.isFinite(v.duration) && v.duration > 0) dur = v.duration;
    buffering = false;
  }

  async function startPlayback() {
    if (isDemo) {
      playing = true;
      videoState.notifyVideoPlaying();
      bumpControls();
      return;
    }
    const v = videoEl;
    if (!v) return;
    mediaError = null;
    try {
      await v.play();
      if (audioEl && audioUrl) {
        audioEl.currentTime = v.currentTime;
        try {
          await audioEl.play();
        } catch {
          // Browser autoplay policy blocked unmuted audio: pause video cleanly
          // so one click starts both video and audio together instead of silent video.
          v.pause();
          playing = false;
          return;
        }
      }
      playing = true;
      videoState.notifyVideoPlaying();
      bumpControls();
    } catch {
      playing = false;
    }
  }

  function pausePlayback() {
    playing = false;
    videoState.notifyVideoPaused();
    if (!isDemo) {
      videoEl?.pause();
      audioEl?.pause();
    }
  }

  function togglePlay() {
    if (playing) pausePlayback();
    else void startPlayback();
  }

  function seekTo(fraction: number) {
    if (!dur) return;
    const target = Math.max(0, Math.min(dur, fraction * dur));
    pos = target;
    if (!isDemo && videoEl) {
      videoEl.currentTime = target;
      if (audioEl && audioUrl) audioEl.currentTime = target;
    }
    bumpControls();
  }

  function onVideoTimeUpdate() {
    const v = videoEl;
    if (!v || isDemo) return;
    pos = v.currentTime || 0;
    if (Number.isFinite(v.duration) && v.duration > 0) dur = v.duration;
    // Keep adaptive HD audio stream tightly locked to video clock.
    if (audioEl && audioUrl && !v.paused) {
      if (Math.abs(audioEl.currentTime - v.currentTime) > 0.22) {
        audioEl.currentTime = v.currentTime;
      }
      if (audioEl.paused && audioEl.readyState >= 2) {
        void audioEl.play().catch(() => {});
      }
    }
  }

  function onVideoWaiting() {
    buffering = true;
    if (audioEl && audioUrl) audioEl.pause();
  }

  function onVideoPlaying() {
    buffering = false;
    playing = true;
    videoState.notifyVideoPlaying();
    if (audioEl && audioUrl && audioEl.paused) {
      audioEl.currentTime = videoEl?.currentTime ?? 0;
      void audioEl.play().catch(() => {});
    }
  }

  function onAudioError() {
    if (!audioUrl || isDemo) return;
    onVideoError();
  }

  function onVideoError() {
    const wantsResume = playing || !!pendingRestore?.resume;
    buffering = false;
    playing = false;
    pendingRestore = null;
    videoEl?.pause();
    audioEl?.pause();
    // Mark current quality URL as failed for this stream and try the next untried quality.
    if (srcUrl && !failedUrls.includes(srcUrl)) {
      const nextFailed = [...failedUrls, srcUrl];
      if (qualities.some((q) => !nextFailed.includes(q.url))) {
        resumeOnFallback = wantsResume;
        failedUrls = nextFailed;
        return;
      }
      failedUrls = nextFailed;
    }
    mediaError = t("video.error.sub");
  }

  function pickQuality(label: string) {
    failedUrls = [];
    videoState.quality = label;
    qualityOpen = false;
  }

  async function toggleFullscreen() {
    const el = stageEl;
    if (!el) return;
    if (document.fullscreenElement) {
      await document.exitFullscreen().catch(() => {});
      nativeFullscreen = false;
      pseudoFullscreen = false;
      return;
    }
    if (pseudoFullscreen) {
      pseudoFullscreen = false;
      return;
    }
    if (typeof el.requestFullscreen === "function") {
      try {
        await el.requestFullscreen();
        nativeFullscreen = true;
        return;
      } catch {
        /* fall back to viewport overlay fullscreen */
      }
    }
    pseudoFullscreen = true;
  }

  function onFullscreenChange() {
    nativeFullscreen = document.fullscreenElement === stageEl;
  }

  function retry() {
    mediaError = null;
    failedUrls = [];
    lastStreamKey = "";
    if (sel) void videoState.resolve(sel);
  }

  onDestroy(() => {
    clearTimeout(hideTimer);
    cancelAnimationFrame(demoRaf);
    videoEl?.pause();
    audioEl?.pause();
  });
</script>

<svelte:document onfullscreenchange={onFullscreenChange} />

<div
  class="stage"
  class:compact
  class:fs={isFullscreen}
  class:idle={playing && !controlsShown && !buffering && !mediaError}
  bind:this={stageEl}
  onmousemove={bumpControls}
  onpointerdown={bumpControls}
  role="region"
  aria-label={t("np.mode.video")}
>
  {#if videoState.loading || videoState.resolving}
    <div class="center-state">
      <div class="spin"></div>
      <span class="state-txt">{videoState.loading ? t("video.loading") : t("video.resolving")}</span>
    </div>
  {:else if videoState.error || mediaError}
    <div class="err-card">
      <span class="err-ic"><Icon name="circle-alert" size={20} color="var(--accent)" /></span>
      <div class="err-copy">
        <div class="err-t">{t("video.error.title")}</div>
        <div class="err-s">{mediaError ?? videoState.error ?? t("video.error.sub")}</div>
      </div>
      <button type="button" class="retry" onclick={retry}>
        <Icon name="refresh-cw" size={14} />{t("video.retry")}
      </button>
    </div>
  {:else if !srcUrl}
    <div class="center-state">
      <Icon name="tv" size={28} color="var(--faint)" />
      <span class="state-txt">{t("video.none.title")}</span>
    </div>
  {:else}
    {#if isDemo}
      <canvas
        class="media"
        width="960"
        height="540"
        bind:this={canvasEl}
        onclick={() => (compact ? (controlsShown = !controlsShown) : togglePlay())}
      ></canvas>
    {:else}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        class="media"
        bind:this={videoEl}
        src={srcUrl}
        playsinline
        preload="auto"
        onloadedmetadata={onVideoLoadedMetadata}
        oncanplay={onVideoCanPlay}
        ontimeupdate={onVideoTimeUpdate}
        onwaiting={onVideoWaiting}
        onplaying={onVideoPlaying}
        onpause={() => { playing = false; audioEl?.pause(); videoState.notifyVideoPaused(); }}
        onended={() => { playing = false; audioEl?.pause(); videoState.notifyVideoPaused(); }}
        onerror={onVideoError}
        onclick={() => (compact ? (controlsShown = !controlsShown) : togglePlay())}
      ></video>
      {#if audioUrl}
        <audio bind:this={audioEl} src={audioUrl} preload="auto" onerror={onAudioError}></audio>
      {/if}
    {/if}

    {#if buffering}
      <div class="center-state over"><div class="spin"></div></div>
    {/if}

    <!-- Overlay controls -->
    <div class="overlay" class:show={controlsShown || !playing}>
      {#if !compact || isFullscreen}
        <div class="ov-top">
          <div class="ov-meta">
            {#if sel}
              <span class="kind-pill">{t(videoState.kindLabelKey(sel.kind))}</span>
              <span class="ov-title trunc">{sel.title}</span>
            {/if}
          </div>
          <div class="ov-top-acts">
            {#if isFullscreen && onOpenPicker}
              <button type="button" class="pill-btn" onclick={(e) => { e.stopPropagation(); onOpenPicker(); }}>
                <Icon name="search" size={13} />{t("video.switch")}
              </button>
            {/if}
          </div>
        </div>
      {:else}
        <div></div>
      {/if}

      {#if !playing && !buffering}
        <button
          type="button"
          class="center-play"
          aria-label={t("common.play")}
          onclick={(e) => { e.stopPropagation(); togglePlay(); }}
        >
          <Icon name="play" size={compact ? 22 : 26} color="#0a0a0c" />
        </button>
      {/if}

      <div class="ov-bot" onclick={(e) => e.stopPropagation()} onkeydown={() => {}} role="presentation">
        <div class="seek-row">
          <ThinBar
            width="100%"
            value={frac}
            live={false}
            onpick={seekTo}
            label={dur ? (f) => fmtTime(f * dur) : undefined}
          />
        </div>
        <div class="ctrl-row">
          <div class="ctrl-l">
            <button type="button" class="cbtn" aria-label={playing ? t("common.pause") : t("common.play")} onclick={togglePlay}>
              <Icon name={playing ? "pause" : "play"} size={16} color="#fff" />
            </button>
            <button type="button" class="cbtn" aria-label="Previous" onclick={() => app.previous()}>
              <Icon name="skip-back" size={15} color="#fff" />
            </button>
            <button type="button" class="cbtn" aria-label="Next" onclick={() => app.next()}>
              <Icon name="skip-forward" size={15} color="#fff" />
            </button>
            <span class="time mono">{fmtTime(pos)} / {dur ? fmtTime(dur) : "-:--"}</span>
          </div>

          <div class="ctrl-r">
            <button type="button" class="cbtn" aria-label="Mute" onclick={() => (muted = !muted)}>
              <Icon name={muted || vol <= 0.01 ? "volume-x" : "volume-2"} size={15} color="#fff" />
            </button>
            {#if !compact}
              <div class="vol-bar">
                <ThinBar width={74} value={muted ? 0 : vol} onpick={(f) => { vol = f; muted = f <= 0.01; }} />
              </div>
            {/if}

            {#if qualities.length > 0}
              <div class="q-wrap">
                <button
                  type="button"
                  class="q-btn"
                  class:on={qualityOpen}
                  onclick={() => (qualityOpen = !qualityOpen)}
                >
                  {activeQ?.label ?? (videoState.quality === "auto" ? t("video.quality.auto") : videoState.quality)}
                </button>
                {#if qualityOpen}
                  <div class="q-menu">
                    <button
                      type="button"
                      class="q-item"
                      class:sel={videoState.quality === "auto"}
                      onclick={() => pickQuality("auto")}
                    >
                      <span>{t("video.quality.auto")}</span>
                      {#if videoState.quality === "auto"}<Icon name="check" size={13} color="var(--accent)" />{/if}
                    </button>
                    {#each qualities as q}
                      <button
                        type="button"
                        class="q-item"
                        class:sel={videoState.quality === q.label}
                        onclick={() => pickQuality(q.label)}
                      >
                        <span>{q.label}</span>
                        {#if videoState.quality === q.label}<Icon name="check" size={13} color="var(--accent)" />{/if}
                      </button>
                    {/each}
                  </div>
                {/if}
              </div>
            {/if}

            <button
              type="button"
              class="cbtn"
              aria-label={isFullscreen ? t("video.exit_fullscreen") : t("video.fullscreen")}
              onclick={toggleFullscreen}
            >
              <Icon name={isFullscreen ? "minimize-2" : "maximize-2"} size={15} color="#fff" />
            </button>
          </div>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .stage {
    position: relative;
    width: 100%;
    aspect-ratio: 16 / 9;
    border-radius: 12px;
    overflow: hidden;
    isolation: isolate;
    contain: paint;
    transform: translateZ(0);
    background: #0c0c0e;
    display: flex;
    align-items: center;
    justify-content: center;
    user-select: none;
    -webkit-user-select: none;
  }
  .stage.compact { border-radius: 10px; }
  .stage.fs {
    position: fixed;
    inset: 0;
    z-index: 200;
    width: 100vw;
    height: 100vh;
    aspect-ratio: auto;
    border-radius: 0;
    contain: none;
    transform: none;
    background: #050506;
  }
  .stage.idle { cursor: none; }

  .media {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
    background: #050506;
  }

  .center-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 20px;
    color: var(--dim);
  }
  .center-state.over {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, .35);
    pointer-events: none;
  }
  .state-txt { font-size: 13px; color: var(--dim); }
  .spin {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 2px solid rgba(255, 255, 255, .16);
    border-top-color: var(--accent);
    animation: vspin .75s linear infinite;
  }
  @keyframes vspin { to { transform: rotate(360deg); } }

  .err-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    max-width: 340px;
    padding: 20px 24px;
    border-radius: 12px;
    background: #161619;
    text-align: center;
  }
  .err-ic {
    width: 38px;
    height: 38px;
    border-radius: 50%;
    background: rgba(255, 255, 255, .07);
    display: grid;
    place-items: center;
  }
  .err-copy { display: flex; flex-direction: column; gap: 4px; }
  .err-t { font-size: 14px; font-weight: 600; color: var(--text); }
  .err-s { font-size: 12.5px; color: var(--dim); line-height: 1.4; }
  .retry {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 14px;
    border-radius: 8px;
    background: #26262b;
    color: var(--text);
    font-size: 12.5px;
    font-weight: 500;
    transition: background-color .14s ease-out;
  }
  .retry:hover { background: #323238; }

  .overlay {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    opacity: 0;
    pointer-events: none;
    transition: opacity .18s ease-out;
  }
  .overlay.show { opacity: 1; pointer-events: auto; }

  .ov-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 12px 14px 20px;
    background: linear-gradient(to bottom, rgba(0, 0, 0, .72), rgba(0, 0, 0, 0));
  }
  .ov-meta { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .kind-pill {
    flex: none;
    height: 22px;
    padding: 0 8px;
    border-radius: 6px;
    background: rgba(255, 255, 255, .14);
    color: #fff;
    font-size: 11px;
    font-weight: 600;
    display: inline-flex;
    align-items: center;
  }
  .ov-title { font-size: 12.5px; font-weight: 500; color: rgba(255, 255, 255, .92); min-width: 0; }
  .ov-top-acts { flex: none; display: flex; align-items: center; gap: 6px; }
  .pill-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 10px;
    border-radius: 7px;
    background: rgba(22, 22, 26, .88);
    color: #fff;
    font-size: 12px;
    font-weight: 500;
    transition: background-color .14s ease-out;
  }
  .pill-btn:hover { background: #2c2c32; }

  .center-play {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 56px;
    height: 56px;
    border-radius: 50%;
    background: #fff;
    display: grid;
    place-items: center;
    padding-left: 3px;
    transition: transform .15s ease-out, opacity .15s ease-out;
  }
  .stage.compact .center-play { width: 48px; height: 48px; }
  .center-play:hover { transform: translate(-50%, -50%) scale(1.05); }
  .center-play:active { transform: translate(-50%, -50%) scale(.95); }

  .ov-bot {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 22px 14px 10px;
    background: linear-gradient(to top, rgba(0, 0, 0, .82), rgba(0, 0, 0, 0));
  }
  .seek-row { width: 100%; }
  .ctrl-row { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .ctrl-l, .ctrl-r { display: flex; align-items: center; gap: 6px; min-width: 0; }

  .cbtn {
    width: 30px;
    height: 30px;
    border-radius: 7px;
    display: grid;
    place-items: center;
    color: #fff;
    transition: background-color .14s ease-out, transform .14s ease-out;
  }
  .cbtn:hover { background: rgba(255, 255, 255, .14); }
  .cbtn:active { transform: scale(.93); }

  .time { font-size: 11.5px; color: rgba(255, 255, 255, .76); margin-left: 4px; white-space: nowrap; }
  .vol-bar { display: flex; align-items: center; margin-right: 4px; }

  .q-wrap { position: relative; }
  .q-btn {
    height: 26px;
    padding: 0 8px;
    border-radius: 6px;
    background: rgba(255, 255, 255, .12);
    color: #fff;
    font-size: 11.5px;
    font-weight: 600;
    transition: background-color .14s ease-out;
  }
  .q-btn:hover, .q-btn.on { background: rgba(255, 255, 255, .22); }
  .q-menu {
    position: absolute;
    right: 0;
    bottom: 34px;
    min-width: 108px;
    padding: 4px;
    border-radius: 9px;
    background: #18181b;
    display: flex;
    flex-direction: column;
    gap: 2px;
    z-index: 30;
  }
  .q-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    height: 28px;
    padding: 0 10px;
    border-radius: 6px;
    font-size: 12px;
    color: var(--dim);
    text-align: left;
  }
  .q-item:hover { background: #26262b; color: var(--text); }
  .q-item.sel { color: var(--text); font-weight: 600; }
</style>
