<script lang="ts">
  // Settings > Appearance > Background picture: the same 16:9 tiles as the
  // glow shapes (GlowVisual), three a row: "None" (the glow), the user's own
  // pictures (remove on hover), pictures from plugins' looks (their name
  // below), then "Add picture". With a picture shown: darken and blur rows.
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { tip } from "../ui.svelte";
  import { bgs, loadBackgrounds, pickBackground, removeBackground, setBackground } from "../backgrounds.svelte";
  import { debounced } from "./util";
  import Icon from "../components/Icon.svelte";
  import ThinBar from "../components/ThinBar.svelte";
  import VisualCard from "./VisualCard.svelte";

  onMount(() => { void loadBackgrounds(); });

  const v = $derived(app.config.visuals);
  const cur = $derived(v.bg_image && bgs.items.some((b) => b.id === v.bg_image) ? v.bg_image : "");
  const curItem = $derived(bgs.items.find((b) => b.id === cur));

  const saveSoon = debounced(() => app.setConfig(() => {}));
  function slide(key: "bg_dim" | "bg_blur", f: number) {
    app.config.visuals[key] = Math.min(key === "bg_dim" ? 0.9 : 1, Math.max(0, f));
    saveSoon();
  }
</script>

<VisualCard label={t("bg.title")} info={t("bg.desc")} extraLabels={[t("bg.dim"), t("bg.blur")]}>
  {#snippet right()}
    <span class="cur-pill">{curItem ? (curItem.plugin ?? t("bg.own")) : t("bg.none")}</span>
  {/snippet}

  <div class="tiles" role="radiogroup" aria-label={t("bg.title")}>
    <button class="scard" class:on={!cur} role="radio" aria-checked={!cur} use:tip={t("bg.none.tip")} onclick={() => setBackground("")}>
      <div class="prev none"><span class="mid"><Icon name="sparkles" size={13} color="rgba(255,255,255,.9)" /></span></div>
      <span class="sl">{t("bg.none")}</span>
    </button>
    {#each bgs.items as b (b.id)}
      {@const on = cur === b.id}
      <div class="tile-wrap">
        <button class="scard" class:on role="radio" aria-checked={on} aria-label={b.plugin ?? t("bg.own")} onclick={() => setBackground(b.id)}>
          <div class="prev" style:background-image={`url("${b.url}")`}></div>
          <span class="sl">{b.plugin ?? t("bg.own")}</span>
        </button>
        {#if !b.plugin}
          <button class="rm" use:tip={t("bg.remove")} aria-label={t("bg.remove")} onclick={() => removeBackground(b.id)}>
            <Icon name="x" size={11} color="rgba(255,255,255,.9)" />
          </button>
        {/if}
      </div>
    {/each}
    <button class="scard" disabled={bgs.adding} use:tip={t("bg.add.tip")} onclick={pickBackground}>
      <div class="prev add" class:busy={bgs.adding}>
        <span class="mid"><Icon name={bgs.adding ? "refresh-cw" : "image-plus"} size={14} color="rgba(255,255,255,.9)" /></span>
      </div>
      <span class="sl">{t(bgs.adding ? "bg.adding" : "bg.add")}</span>
    </button>
  </div>

  {#if cur}
    <div class="sub-rows">
      <div class="srow-inline">
        <div class="s-lab">
          <span>{t("bg.dim")}</span>
          <span class="info" use:tip={{ label: t("bg.dim.desc"), wrap: true }}><Icon name="info" size={13} color="rgba(255,255,255,.42)" /></span>
        </div>
        <div class="bar">
          <ThinBar width={160} value={(v.bg_dim ?? 0.35) / 0.9} onpick={(f) => slide("bg_dim", f * 0.9)} />
          <span class="mono val">{Math.round((v.bg_dim ?? 0.35) * 100)}%</span>
        </div>
      </div>
      <div class="srow-inline">
        <div class="s-lab"><span>{t("bg.blur")}</span></div>
        <div class="bar">
          <ThinBar width={160} value={v.bg_blur ?? 0} onpick={(f) => slide("bg_blur", f)} />
          <span class="mono val">{Math.round((v.bg_blur ?? 0) * 100)}%</span>
        </div>
      </div>
    </div>
  {/if}
</VisualCard>

<style>
  .cur-pill { padding: 2px 8px; border-radius: 6px; background: rgba(255, 255, 255, .07); font-size: 12px; color: rgba(255, 255, 255, .75); max-width: 180px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .tiles { display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px; }
  .tile-wrap { position: relative; min-width: 0; }
  .tile-wrap .scard { width: 100%; }
  .scard { display: flex; flex-direction: column; align-items: center; gap: 7px; padding: 6px; border-radius: 10px; min-width: 0; transition: background-color .15s ease-out; }
  .scard:disabled { cursor: default; }
  .prev { position: relative; width: 100%; aspect-ratio: 16 / 9; border-radius: 8px; overflow: hidden; background: #030303 center / cover no-repeat; transition: transform .15s var(--ease); }
  .prev.none { background: radial-gradient(120% 90% at 30% 110%, rgba(var(--accent-rgb), .55), transparent 60%), #030303; }
  .prev.add { background: #141416; }
  .prev.busy .mid :global(span) { animation: spin 1s linear infinite; }
  .mid { position: absolute; left: 50%; top: 50%; transform: translate(-50%, -50%); width: 24px; height: 24px; border-radius: 50%; background: rgba(10, 10, 12, .72); display: grid; place-items: center; pointer-events: none; }
  .scard:hover { background: #18181b; }
  .scard.on { background: #26262b; }
  .scard:active .prev { transform: scale(.97); }
  .sl { font-size: 12px; color: rgba(255, 255, 255, .68); text-align: center; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 100%; transition: color .15s; }
  .scard:hover .sl { color: #fff; }
  .scard.on .sl { color: #fff; font-weight: 600; }
  .rm { position: absolute; top: 10px; right: 10px; width: 20px; height: 20px; border-radius: 50%; background: rgba(10, 10, 12, .78); display: grid; place-items: center; opacity: 0; transition: opacity .12s ease-out, background-color .12s ease-out; }
  .tile-wrap:hover .rm, .rm:focus-visible { opacity: 1; }
  .rm:hover { background: var(--danger); }
  @media (hover: none) { .rm { opacity: 1; } }
  @keyframes spin { to { transform: rotate(360deg); } }

  .sub-rows { display: flex; flex-direction: column; border-top: 1px solid rgba(255, 255, 255, .06); padding-top: 4px; }
  .srow-inline { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 10px 18px; padding: 10px 0; }
  .srow-inline:not(:last-child) { border-bottom: 1px solid rgba(255, 255, 255, .05); }
  .s-lab { display: flex; align-items: center; gap: 6px; font-size: 13.5px; font-weight: 500; color: rgba(255, 255, 255, .92); }
  .info { flex: none; width: 18px; height: 18px; border-radius: 9px; display: grid; place-items: center; transition: background-color .12s; }
  .info:hover { background: rgba(255, 255, 255, .08); }
  /* Bigger tap target on touch screens (a tap shows the explanation). */
  .info { position: relative; cursor: help; }
  .info::after { content: ""; position: absolute; inset: -10px; }
  .bar { display: flex; align-items: center; gap: 10px; margin-left: auto; }
  .val { width: 36px; display: flex; justify-content: flex-end; font-size: 12px; color: rgba(255, 255, 255, .6); }

  @media (max-width: 400px) {
    .tiles { grid-template-columns: repeat(2, 1fr); }
  }
</style>
