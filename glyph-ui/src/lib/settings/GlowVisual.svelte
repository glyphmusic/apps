<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { hex, renderGlowPreview, type GlowShape, type Palette } from "../glow";
  import { allThemes, css, solidBase, themeColors } from "../theme";
  import { pl } from "./plugins-state.svelte";
  import { debounced } from "./util";
  import { tip } from "../ui.svelte";
  import Icon from "../components/Icon.svelte";
  import ThinBar from "../components/ThinBar.svelte";
  import VisualCard from "./VisualCard.svelte";

  const SHAPES: GlowShape[] = ["", "strahl", "band", "sonnenaufgang", "horizont", "solid"];

  const v = $derived(app.config.visuals);
  const themes = $derived((void pl.installed, allThemes()));
  const curTheme = $derived(themes.some((th) => th.id === v.theme) ? v.theme : "glyph");
  const curThemeObj = $derived(themes.find((th) => th.id === curTheme) ?? themes[0]);
  const themeDef = $derived(themeColors(curTheme));
  const themePalette = $derived<Palette | null>(themeDef.glow ? (themeDef.glow.map(hex) as Palette) : null);

  let canvases: Record<string, HTMLCanvasElement | undefined> = $state({});

  $effect(() => {
    const pal = themePalette;
    const inten = v.intensity;
    const base = themeDef.base;
    const solidHex = solidBase(v);
    for (const sh of SHAPES) {
      const c = canvases[sh];
      if (c) renderGlowPreview(c, sh, pal, inten, sh === "solid" ? solidHex : base);
    }
  });

  const saveSoon = debounced(() => app.setConfig(() => {}));
  function slide(key: "intensity" | "music_reactivity", f: number) {
    app.config.visuals[key] = Math.min(1, Math.max(0, f));
    saveSoon();
  }

  // Solid background colours: the theme's own ("") and a few calm dark
  // ones; the last button opens a free colour picker.
  const SOLIDS = ["", "#000000", "#141418", "#0f1a2e", "#1b1030", "#0f2219", "#2a0f14", "#2b2b30"];
  const solidCustom = $derived(!!v.solid_color && !SOLIDS.includes(v.solid_color.toLowerCase()));
  function setSolid(c: string, now = true) {
    app.config.visuals.solid_color = c;
    if (now) app.setConfig(() => {}); else saveSoon();
  }

  const shapeLabel = (n: string) => (n ? t(`sp.appearance.shape.${n}`) : t("sp.appearance.shape.auto_short"));
  const shapeFullLabel = (n: string) => (n ? t(`sp.appearance.shape.${n}`) : t("sp.appearance.shape.auto"));
</script>

<VisualCard
  label={t("sp.appearance.shape")}
  info={t("sp.appearance.shape.desc")}
  extraLabels={[t("sp.appearance.theme"), t("settings.visuals.intensity"), t("settings.visuals.music_reactivity")]}
>
  {#snippet right()}
    <span class="cur-pill">{shapeFullLabel(v.glow_shape)}</span>
  {/snippet}

  <div class="shapes" role="radiogroup" aria-label={t("sp.appearance.shape")}>
    {#each SHAPES as sh (sh)}
      {@const on = v.glow_shape === sh}
      <button
        class="scard"
        class:on
        role="radio"
        aria-checked={on}
        use:tip={shapeFullLabel(sh)}
        onclick={() => app.setConfig((c) => { c.visuals.glow_shape = sh; })}
      >
        <div class="prev">
          <canvas bind:this={canvases[sh]}></canvas>
          {#if sh === ""}
            <span class="auto-cross" aria-hidden="true"></span>
            <span class="auto-ic"><Icon name="sparkles" size={13} color="rgba(255,255,255,.9)" /></span>
          {/if}
        </div>
        <span class="sl">{shapeLabel(sh)}</span>
      </button>
    {/each}
  </div>

  <div class="sub-rows">
    {#if v.glow_shape === "solid"}
      <div class="srow-inline">
        <div class="s-lab"><span>{t("sp.appearance.solid_color")}</span></div>
        <div class="sw">
          {#each SOLIDS as c (c)}
            {@const on = (v.solid_color ?? "").toLowerCase() === c}
            {@const name = c || t("sp.appearance.solid_color.theme")}
            <button class="th" class:on use:tip={name} aria-label={name} aria-pressed={on} onclick={() => setSolid(c)}>
              <span class="ring solid" style:background={c || css(themeDef.base)}></span>
            </button>
          {/each}
          <label class="th pick" class:on={solidCustom} use:tip={t("sp.appearance.solid_color.custom")}>
            <span class="ring solid" style:background={solidCustom ? v.solid_color : undefined}>
              {#if !solidCustom}<Icon name="plus" size={12} color="rgba(255,255,255,.85)" />{/if}
            </span>
            <input type="color" aria-label={t("sp.appearance.solid_color.custom")} value={v.solid_color || css(solidBase(v))}
              oninput={(e) => setSolid(e.currentTarget.value, false)} onchange={(e) => setSolid(e.currentTarget.value)} />
          </label>
        </div>
      </div>
    {/if}
    <div class="srow-inline">
      <div class="s-lab">
        <span>{t("sp.appearance.theme")}</span>
        <span class="info" use:tip={{ label: t("sp.appearance.theme.desc"), wrap: true }}>
          <Icon name="info" size={13} color="rgba(255,255,255,.42)" />
        </span>
        <span class="th-name">{t(curThemeObj.name)}</span>
      </div>
      <div class="sw">
        {#each themes as th (th.id)}
          <button
            class="th"
            class:on={th.id === curTheme}
            use:tip={t(th.name)}
            aria-label={t(th.name)}
            aria-pressed={th.id === curTheme}
            onclick={() => app.setConfig((c) => { c.visuals.theme = th.id; })}
          >
            <span class="ring" style:background={css(th.glow ? th.glow[1] : 0x1d5cff)}>
              <span class="in" style:background={css(th.accent)}></span>
            </span>
          </button>
        {/each}
      </div>
    </div>

    <div class="srow-inline">
      <div class="s-lab">
        <span>{t("settings.visuals.intensity")}</span>
        <span class="info" use:tip={{ label: t("sp.appearance.intensity.desc"), wrap: true }}>
          <Icon name="info" size={13} color="rgba(255,255,255,.42)" />
        </span>
      </div>
      <div class="bar">
        <ThinBar width={160} value={v.intensity} onpick={(f) => slide("intensity", f)} />
        <span class="mono val">{Math.round(v.intensity * 100)}%</span>
      </div>
    </div>

    <div class="srow-inline">
      <div class="s-lab">
        <span>{t("settings.visuals.music_reactivity")}</span>
        <span class="info" use:tip={{ label: t("sp.appearance.music_reactivity.desc"), wrap: true }}>
          <Icon name="info" size={13} color="rgba(255,255,255,.42)" />
        </span>
      </div>
      <div class="bar">
        <ThinBar width={160} value={v.music_reactivity} onpick={(f) => slide("music_reactivity", f)} />
        <span class="mono val">{Math.round(v.music_reactivity * 100)}%</span>
      </div>
    </div>
  </div>
</VisualCard>

<style>
  .cur-pill {
    padding: 2px 8px;
    border-radius: 6px;
    background: rgba(255, 255, 255, .07);
    font-size: 12px;
    color: rgba(255, 255, 255, .75);
  }
  .shapes {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 10px;
  }
  .scard {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 7px;
    padding: 6px;
    border-radius: 10px;
    transition: background-color .15s ease-out;
  }
  .prev {
    position: relative;
    width: 100%;
    aspect-ratio: 16 / 9;
    border-radius: 8px;
    overflow: hidden;
    background: #030303;
    transition: transform .15s var(--ease);
  }
  .prev canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
  .scard:hover { background: #18181b; }
  .scard.on { background: #26262b; }
  .scard:active .prev { transform: scale(.97); }

  .auto-cross {
    position: absolute;
    inset: 0;
    background:
      linear-gradient(to right, transparent calc(50% - 0.5px), rgba(255, 255, 255, .14) 50%, transparent calc(50% + 0.5px)),
      linear-gradient(to bottom, transparent calc(50% - 0.5px), rgba(255, 255, 255, .14) 50%, transparent calc(50% + 0.5px));
    pointer-events: none;
  }
  .auto-ic {
    position: absolute;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: rgba(10, 10, 12, .72);
    display: grid;
    place-items: center;
    pointer-events: none;
  }

  .sl {
    font-size: 12px;
    color: rgba(255, 255, 255, .68);
    text-align: center;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
    transition: color .15s;
  }
  .scard:hover .sl { color: #fff; }
  .scard.on .sl { color: #fff; font-weight: 600; }

  .sub-rows {
    display: flex;
    flex-direction: column;
    border-top: 1px solid rgba(255, 255, 255, .06);
    padding-top: 4px;
  }
  .srow-inline {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 10px 18px;
    padding: 10px 0;
  }
  .srow-inline:not(:last-child) { border-bottom: 1px solid rgba(255, 255, 255, .05); }
  .s-lab {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13.5px;
    font-weight: 500;
    color: rgba(255, 255, 255, .92);
  }
  .th-name {
    margin-left: 4px;
    font-size: 12px;
    font-weight: 400;
    color: rgba(255, 255, 255, .48);
  }
  .info { flex: none; width: 18px; height: 18px; border-radius: 9px; display: grid; place-items: center; transition: background-color .12s; }
  .info:hover { background: rgba(255, 255, 255, .08); }
  .sw { display: flex; flex-wrap: wrap; justify-content: flex-end; align-items: center; gap: 6px; }
  .th { flex: none; width: 28px; height: 28px; border-radius: 50%; display: grid; place-items: center; border: 2px solid transparent; transition: border-color .15s ease-out, transform .15s var(--ease); }
  .th:hover { transform: scale(1.08); }
  .th.on { border-color: #fff; }
  .ring { width: 20px; height: 20px; border-radius: 50%; display: grid; place-items: center; }
  .in { width: 12px; height: 12px; border-radius: 50%; }
  /* A thin light edge, so near-black swatches still show on the dark card. */
  .ring.solid { box-shadow: inset 0 0 0 1px rgba(255, 255, 255, .2); }
  .th.pick { position: relative; cursor: pointer; }
  .th.pick .ring:not([style*="background"]) { background: conic-gradient(#ff5f6d, #ffc371, #5fff9f, #5fb8ff, #c35fff, #ff5f6d); }
  .th.pick input { position: absolute; inset: 0; width: 100%; height: 100%; opacity: 0; cursor: pointer; border: 0; padding: 0; }
  .bar { display: flex; align-items: center; gap: 10px; margin-left: auto; }
  .val { width: 36px; display: flex; justify-content: flex-end; font-size: 12px; color: rgba(255, 255, 255, .6); }

  @media (max-width: 400px) {
    .shapes { grid-template-columns: repeat(2, 1fr); }
  }
</style>
