<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import Icon from "../components/Icon.svelte";
  import VisualCard from "./VisualCard.svelte";

  let { kind }: { kind: "search" | "youtube" } = $props();

  const pv = $derived(app.config.providers);
  const label = $derived(kind === "search" ? t("sp.search.mode") : t("ytdlp.mode"));
  const info = $derived(kind === "search" ? t("sp.search.mode.desc") : t("ytdlp.mode.hint"));
  const isFirst = $derived(kind === "search" ? pv.local_search : pv.youtube_mode !== "server");
  const firstLabel = $derived(kind === "search" ? t("sp.search.mode.device") : t("ytdlp.mode.local"));
  const secondLabel = $derived(kind === "search" ? t("sp.search.mode.server") : t("ytdlp.mode.server"));

  function pick(first: boolean) {
    app.setConfig((c) => {
      if (kind === "search") c.providers.local_search = first;
      else c.providers.youtube_mode = first ? "local" : "server";
    });
  }
</script>

<VisualCard {label} {info}>
  <div class="grid" role="radiogroup" aria-label={label}>
    {#each [true, false] as first (String(first))}
      {@const on = isFirst === first}
      {@const accent = on ? "var(--accent)" : "rgba(255,255,255,.55)"}
      {@const ink = on ? "rgba(255,255,255,.85)" : "rgba(255,255,255,.42)"}
      <button class="opt" class:on role="radio" aria-checked={on} onclick={() => pick(first)}>
        <div class="stage" aria-hidden="true">
          {#if kind === "search"}
            <div class="search-bar">
              <Icon name="search" size={12} color={accent} />
              <span class="query"></span>
            </div>
            <div class="path">
              <Icon name="monitor" size={14} color={first ? accent : ink} />
              <span class="line"></span>
              {#if !first}
                <Icon name="server" size={14} color={accent} />
                <span class="line"></span>
              {/if}
              <Icon name="globe" size={14} color={ink} />
            </div>
          {:else}
            <div class="flow">
              <span class="yt-tile">
                <Icon name="source-youtube" size={16} color={on ? "#fff" : "rgba(255,255,255,.6)"} />
              </span>
              <span class="line"></span>
              <span class="node">
                <Icon name={first ? "monitor" : "server"} size={14} color={accent} />
              </span>
              <span class="line" class:dashed={!first}></span>
              <Icon name="audio-lines" size={15} color={accent} />
            </div>
          {/if}
        </div>
        <span class="lbl">{first ? firstLabel : secondLabel}</span>
      </button>
    {/each}
  </div>
</VisualCard>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 12px;
  }
  .opt {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 9px;
    padding: 12px;
    border-radius: 10px;
    background: #0b0b0b;
    border: 1.5px solid rgba(255, 255, 255, .14);
    transition: border-color .15s ease-out, background-color .15s ease-out, transform .15s var(--ease);
  }
  .opt:hover { border-color: rgba(255, 255, 255, .42); }
  .opt.on { border-color: #fff; background: #101012; }
  .opt:active { transform: scale(.985); }

  .stage {
    width: 100%;
    height: 76px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
  }
  .search-bar {
    width: 152px;
    max-width: 100%;
    height: 26px;
    padding: 0 11px;
    border-radius: 999px;
    background: rgba(255, 255, 255, .07);
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .query {
    width: 68%;
    height: 4px;
    border-radius: 2px;
    background: rgba(255, 255, 255, .24);
  }
  .opt.on .query { background: rgba(255, 255, 255, .42); }

  .path {
    width: 126px;
    max-width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .flow {
    width: 162px;
    max-width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .yt-tile {
    width: 38px;
    height: 28px;
    border-radius: 7px;
    background: rgba(255, 255, 255, .08);
    display: grid;
    place-items: center;
    flex: none;
  }
  .opt.on .yt-tile { background: color-mix(in srgb, var(--accent) 26%, #181818); }
  .node {
    width: 28px;
    height: 28px;
    border-radius: 7px;
    background: rgba(255, 255, 255, .06);
    display: grid;
    place-items: center;
    flex: none;
  }
  .opt.on .node { background: rgba(255, 255, 255, .1); }

  .line {
    flex: 1;
    height: 2px;
    border-radius: 1px;
    background: rgba(255, 255, 255, .18);
  }
  .opt.on .line { background: var(--accent); opacity: .75; }
  .line.dashed {
    background: repeating-linear-gradient(90deg, rgba(255, 255, 255, .24) 0 4px, transparent 4px 7px);
  }
  .opt.on .line.dashed {
    background: repeating-linear-gradient(90deg, var(--accent) 0 4px, transparent 4px 7px);
    opacity: .75;
  }

  .lbl { font-size: 13px; font-weight: 500; color: rgba(255, 255, 255, .75); }
  .opt.on .lbl { color: #fff; font-weight: 600; }

  @media (max-width: 460px) {
    .grid { grid-template-columns: 1fr; }
  }
</style>
