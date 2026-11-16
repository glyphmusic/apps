<script lang="ts">
  import { getContext, onMount } from "svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import Icon from "../components/Icon.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import Tog from "./Tog.svelte";
  import PluginSection from "./PluginSection.svelte";
  import { hiddenLabels, pl } from "./plugins-state.svelte";
  import { sui } from "./settings.svelte";
  import { tip } from "../ui.svelte";
  import type { PluginHost } from "./host";

  const v = $derived(app.config.visuals);
  const toggleCovers = (on: boolean) => app.setConfig((c) => { c.visuals.ascii_covers = on; });
  const toggleFx = (on: boolean) => app.setConfig((c) => { c.visuals.ascii_effects = on; });

  const host = getContext<PluginHost | undefined>("plugin-host");
  const coverLabel = $derived(t("settings.visuals.ascii_covers"));
  const fxLabel = $derived(t("sp.appearance.ascii_fx"));
  const hideCovers = $derived(hiddenLabels().has(coverLabel));
  const hideFx = $derived(hiddenLabels().has(fxLabel));
  const litCovers = $derived(sui.flash != null && sui.flash.label === coverLabel);
  const litFx = $derived(sui.flash != null && sui.flash.label === fxLabel);
  const after = $derived(
    host
      ? pl.ui.sections.filter((s) => s.page === host.page && s.after && (t(s.after) === coverLabel || t(s.after) === fxLabel))
      : [],
  );

  let coverEl: HTMLDivElement | undefined = $state();
  let fxEl: HTMLDivElement | undefined = $state();
  $effect(() => { if (litCovers && coverEl) coverEl.scrollIntoView({ block: "center", behavior: "smooth" }); });
  $effect(() => { if (litFx && fxEl) fxEl.scrollIntoView({ block: "center", behavior: "smooth" }); });

  onMount(() => {
    if (!host) return;
    host.labels.add(coverLabel);
    host.labels.add(fxLabel);
    return () => { host.labels.delete(coverLabel); host.labels.delete(fxLabel); };
  });

  const ASCII_ART = [
    ["#", "X", "+", "=", ".", ":"],
    ["@", "0", "S", "8", "+", "-"],
    ["X", "8", "#", "@", "0", "="],
    ["+", "S", "0", "#", "X", ":"],
    [".", "=", "+", "X", "S", "-"],
  ];
  const ASCII_COLORS = [
    ["#ff5fd2", "#ff7be0", "#b07cff", "#6526d6", "#4da3ff", "#22c3e6"],
    ["#ff8a3d", "#ff5fd2", "#ff7be0", "#b07cff", "#4da3ff", "#1d5cff"],
    ["#ffcc3a", "#ff8a3d", "#ff5fd2", "#ff5fd2", "#b07cff", "#4da3ff"],
    ["#ff6516", "#ff8a3d", "#ff5fd2", "#a970ff", "#6526d6", "#1d5cff"],
    ["#b11d0b", "#ff6516", "#b07cff", "#4da3ff", "#46dcff", "#16052f"],
  ];
</script>

{#if !hideCovers || !hideFx}
  <div class="cards" class:single={hideCovers || hideFx}>
    {#if !hideCovers}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="acard" class:on={v.ascii_covers} class:lit={litCovers} bind:this={coverEl} onclick={() => toggleCovers(!v.ascii_covers)}>
        <div class="chead">
          <div class="clab">
            <span>{coverLabel}</span>
            <span class="info" use:tip={{ label: t("sp.appearance.ascii.desc"), wrap: true }} onclick={(e) => e.stopPropagation()}>
              <Icon name="info" size={13} color="rgba(255,255,255,.42)" />
            </span>
          </div>
          <span onclick={(e) => e.stopPropagation()}>
            <Tog on={v.ascii_covers} onchange={toggleCovers} label={coverLabel} />
          </span>
        </div>
        <div class="preview">
          <div class="cover-box" class:ascii={v.ascii_covers}>
            {#if v.ascii_covers}
              <div class="mono ascii-grid" aria-hidden="true">
                {#each ASCII_ART as row, r}
                  <div class="arow">
                    {#each row as ch, c}
                      <span style:color={ASCII_COLORS[r][c]}>{ch}</span>
                    {/each}
                  </div>
                {/each}
              </div>
            {:else}
              <div class="smooth-cover">
                <Icon name="disc-3" size={24} color="rgba(255,255,255,.88)" />
              </div>
            {/if}
          </div>
          <div class="meta-bars">
            <span class="b1"></span>
            <span class="b2"></span>
          </div>
        </div>
      </div>
    {/if}

    {#if !hideFx}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="acard" class:on={v.ascii_effects} class:lit={litFx} bind:this={fxEl} onclick={() => toggleFx(!v.ascii_effects)}>
        <div class="chead">
          <div class="clab">
            <span>{fxLabel}</span>
            <span class="info" use:tip={{ label: t("sp.appearance.ascii_fx.desc"), wrap: true }} onclick={(e) => e.stopPropagation()}>
              <Icon name="info" size={13} color="rgba(255,255,255,.42)" />
            </span>
          </div>
          <span onclick={(e) => e.stopPropagation()}>
            <Tog on={v.ascii_effects} onchange={toggleFx} label={fxLabel} />
          </span>
        </div>
        <div class="preview sk-row" aria-hidden="true">
          <Skeleton w={44} h={44} radius={6} id="ascii-prev-c" ascii={v.ascii_effects} />
          <div class="sk-bars">
            <div style:width="68%"><Skeleton w="100%" h={12} radius={4} id="ascii-prev-t" ascii={v.ascii_effects} /></div>
            <div style:width="44%"><Skeleton w="100%" h={10} radius={4} id="ascii-prev-a" ascii={v.ascii_effects} /></div>
          </div>
          <Skeleton w={28} h={10} radius={4} id="ascii-prev-d" ascii={v.ascii_effects} />
        </div>
      </div>
    {/if}
  </div>
  {#each after as s (s.plugin + s.title + s.after)}<PluginSection section={s} named />{/each}
{/if}

<style>
  .cards {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 12px;
  }
  .cards.single { grid-template-columns: 1fr; }
  .acard {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: 12px;
    padding: 14px 16px;
    border-radius: 12px;
    background: rgba(255, 255, 255, .035);
    border: 1.5px solid transparent;
    cursor: pointer;
    transition: border-color .15s ease-out, background-color .15s ease-out;
  }
  .acard:hover { background: rgba(255, 255, 255, .05); }
  .acard.on { border-color: rgba(255, 255, 255, .28); }
  .acard.lit { animation: aflash 1.8s linear forwards; }
  @keyframes aflash {
    0%, 39% { background: rgba(var(--accent-rgb, 255, 95, 210), .14); }
    100% { background: rgba(255, 255, 255, .035); }
  }

  .chead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .clab {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 13.5px;
    font-weight: 500;
    color: #fff;
    min-width: 0;
  }
  .info { flex: none; width: 18px; height: 18px; border-radius: 9px; display: grid; place-items: center; }
  .info:hover { background: rgba(255, 255, 255, .08); }

  .preview {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    border-radius: 9px;
    background: #0b0b0b;
  }
  .cover-box {
    width: 44px;
    height: 44px;
    border-radius: 6px;
    overflow: hidden;
    flex: none;
    background: #141418;
  }
  .smooth-cover {
    width: 100%;
    height: 100%;
    display: grid;
    place-items: center;
    background: linear-gradient(135deg, var(--accent), #4da3ff 65%, #1d5cff);
  }
  .ascii-grid {
    width: 100%;
    height: 100%;
    padding: 2px 3px;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    font-size: 8px;
    line-height: 8px;
    font-weight: 700;
    background: #070709;
  }
  .arow { display: flex; justify-content: space-between; }

  .meta-bars { flex: 1; display: flex; flex-direction: column; gap: 7px; }
  .b1 { width: 72%; height: 8px; border-radius: 4px; background: rgba(255, 255, 255, .14); }
  .b2 { width: 46%; height: 6px; border-radius: 3px; background: rgba(255, 255, 255, .08); }

  .sk-row { gap: 12px; }
  .sk-bars { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }

  @media (max-width: 560px) {
    .cards { grid-template-columns: 1fr; }
  }
</style>
