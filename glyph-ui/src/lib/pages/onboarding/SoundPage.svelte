<script lang="ts">
  // onboarding.rs sound(): a 10-band equalizer that moves with the "music":
  // click a block to set a band, or a preset. Bands rise from 0 on entry
  // (eased with a 90 ms time constant), a small wobble rides on top; blocks
  // are `bw` wide (<= 30), pitch 8..14, radius 2.5, lit ones mix white 80 %
  // into the accent from the bottom up, unlit white 6 %; labels mono 9.5.
  import { untrack } from "svelte";
  import { t } from "../../i18n.svelte";
  import { BANDS, BLOCKS, ob, PRESETS } from "./anim.svelte";
  let { s, now, still, w, h }: { s: number; now: number; still: boolean; w: number; h: number } = $props();

  let shown = $state<number[]>(Array(10).fill(0));
  let last = performance.now();
  $effect(() => {
    const n = now;
    untrack(() => {
      const dt = Math.min(0.1, Math.max(0, (n - last) / 1000));
      last = n;
      const a = still ? 1 : 1 - Math.exp(-dt / 0.09);
      shown = shown.map((v, i) => v + (ob.eq[i] - v) * a);
    });
  });

  const innerW = $derived(Math.min(w - 48, 430));
  const gap = 4;
  const bw = $derived(Math.min((innerW - gap * 9) / 10, 30));
  const pitch = $derived(Math.min(14, Math.max(8, (h - 36 - 24 - 16 - 26) / BLOCKS)));
  const lit = (b: number) => {
    const wobble = still ? 0 : 0.07 * Math.sin(s * 5.1 + b * 1.7) * Math.sin(s * 2.3 + b * 0.6);
    return Math.min(BLOCKS, Math.max(0, Math.round((shown[b] + wobble) * BLOCKS)));
  };
</script>

<div class="p">
  <div class="presets">
    {#each PRESETS as [key, levels], i (key)}
      <button class="chip" class:on={ob.preset === i} onclick={() => { ob.eq = [...levels]; ob.preset = i; }}>
        <span style:color="rgba(255,255,255,{ob.preset === i ? 0.9 : 0.5})">{t(key)}</span>
      </button>
    {/each}
  </div>
  <div class="bands" style:gap="{gap}px">
    {#each BANDS as label, b (label)}
      {@const n = lit(b)}
      <div class="band">
        <div class="col">
          {#each Array.from({ length: BLOCKS }, (_, j) => BLOCKS - 1 - j) as j (j)}
            <button class="blk" aria-label="{label} {j + 1}" style:width="{bw}px" style:height="{pitch - 3}px"
              style:background={j < n ? `color-mix(in srgb, var(--accent) ${(j / (BLOCKS - 1)) * 100}%, rgba(255,255,255,.8))` : "rgba(255,255,255,.06)"}
              onclick={() => { ob.eq[b] = (j + 1) / BLOCKS; ob.preset = null; }}></button>
          {/each}
        </div>
        <span class="lb mono">{label}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .p { width: 100%; height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 14px; }
  .presets { display: flex; flex-wrap: wrap; justify-content: center; gap: 6px; }
  .chip { display: flex; align-items: center; gap: 6px; height: 24px; padding: 0 10px; border-radius: 12px; background: rgba(255, 255, 255, .03); font-size: 11.5px; transition: background-color .15s ease-out; }
  .chip.on { background: rgba(255, 255, 255, .11); }
  .chip:hover { background: rgba(255, 255, 255, .14); }
  .bands { display: flex; align-items: flex-end; }
  .band { display: flex; flex-direction: column; align-items: center; gap: 5px; }
  .col { display: flex; flex-direction: column; gap: 3px; }
  .blk { border-radius: 2.5px; transition: background-color .08s linear, opacity .12s; }
  .blk:hover { opacity: .7; }
  .lb { font-size: 9.5px; color: rgba(255, 255, 255, .35); }
</style>
