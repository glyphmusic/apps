<script lang="ts">
  // onboarding.rs devices(): one account in the middle (disc 56, white 9 %),
  // four devices around it (44, accent when playing), dotted lines drawn
  // from the middle with data flowing both ways (faster to the playing
  // device). Click a device to play there (a ripple). Everything pops in
  // with a small overshoot.
  import Icon from "../../components/Icon.svelte";
  import { backOut, DEVICES, easeOut, k, ob } from "./anim.svelte";
  let { s, now, still, w, h }: { s: number; now: number; still: boolean; w: number; h: number } = $props();

  const cx0 = $derived(w / 2), cy0 = $derived(h / 2 - 8);
  const rx = $derived(Math.min(w * 0.33, 160)), ry = $derived(Math.min(h * 0.3, 68));
  const spots = $derived<[number, number][]>([[-rx, -ry], [rx, -ry], [-rx, ry], [rx, ry]]);
  const ak = $derived(backOut(k(s, 0, 0.45)));
  const av = $derived(56 * ak);

  function line(i: number) {
    const [dx, dy] = spots[i];
    const len = Math.hypot(dx, dy), ux = dx / len, uy = dy / len, from = 36, to = len - 28;
    const drawn = easeOut(k(s, 0.25 + 0.1 * i, 0.5));
    const dots: [number, number][] = [];
    for (let m = 0; m < 10; m++) {
      const f = m / 9;
      if (f > drawn) break;
      const r = from + (to - from) * f;
      dots.push([cx0 + ux * r, cy0 + uy * r]);
    }
    let flow: [number, number, number] | null = null;
    if (!still && drawn >= 1) {
      const speed = ob.device === i ? 0.9 : 0.35;
      let p = (s * speed + i * 0.27) % 1;
      if (i % 2 === 1) p = 1 - p;
      const r = from + (to - from) * p;
      flow = [cx0 + ux * r, cy0 + uy * r, Math.sin(p * Math.PI)];
    }
    return { dots, flow };
  }
  const lines = $derived([0, 1, 2, 3].map(line));
  function pick(i: number) { ob.device = i; ob.deviceAt = performance.now(); }
</script>

<div class="p">
  {#each lines as l, i (i)}
    {#each l.dots as [x, y], m (m)}<span class="dot" style:left="{x - 1.5}px" style:top="{y - 1.5}px"></span>{/each}
    {#if l.flow}<span class="flow" style:left="{l.flow[0] - 2.5}px" style:top="{l.flow[1] - 2.5}px" style:opacity={l.flow[2]}></span>{/if}
  {/each}
  <span class="me" style:left="{cx0 - av / 2}px" style:top="{cy0 - av / 2}px" style:width="{av}px" style:height="{av}px"><Icon name="user-round" size={26 * ak} color="rgba(255,255,255,.85)" /></span>
  <div class="who" style:left="{cx0 - 60}px" style:top="{cy0 + 34}px" style:opacity={easeOut(k(s, 0.4, 0.4))}>
    <span class="mono h1">@handle</span>
    <span class="mono h2">[ E2E ]</span>
  </div>
  {#each spots as [dx, dy], i (i)}
    {@const nk = backOut(k(s, 0.2 + 0.09 * i, 0.45))}
    {#if nk > 0.01}
      {@const on = ob.device === i}
      {@const size = 44 * nk}
      {@const e = ob.deviceAt && on ? (now - ob.deviceAt) / 600 : 2}
      {#if e < 1}
        {@const r = 44 + 40 * easeOut(e)}
        <span class="rip" style:left="{cx0 + dx - r / 2}px" style:top="{cy0 + dy - r / 2}px" style:width="{r}px" style:height="{r}px" style:border-color="rgba(255,255,255,{0.5 * (1 - e)})"></span>
      {/if}
      <button class="dev" class:on style:left="{cx0 + dx - size / 2}px" style:top="{cy0 + dy - size / 2}px" style:width="{size}px" style:height="{size}px" onclick={() => pick(i)} aria-label={DEVICES[i]}>
        <Icon name={DEVICES[i]} size={20 * nk} color={on ? "rgba(0,0,0,.85)" : "rgba(255,255,255,.8)"} />
      </button>
    {/if}
  {/each}
</div>

<style>
  .p { position: relative; width: 100%; height: 100%; }
  .dot { position: absolute; width: 3px; height: 3px; border-radius: 50%; background: rgba(255, 255, 255, .16); }
  .flow { position: absolute; width: 5px; height: 5px; border-radius: 50%; background: var(--accent); }
  .me { position: absolute; border-radius: 50%; background: rgba(255, 255, 255, .09); display: grid; place-items: center; }
  .who { position: absolute; width: 120px; display: flex; flex-direction: column; align-items: center; }
  .h1 { font-size: 11px; color: rgba(255, 255, 255, .75); }
  .h2 { font-size: 9.5px; color: rgba(255, 255, 255, .4); }
  .rip { position: absolute; border-radius: 50%; border: 2px solid; pointer-events: none; }
  .dev { position: absolute; border-radius: 50%; background: rgba(255, 255, 255, .07); display: grid; place-items: center; transition: background-color .2s ease-out, opacity .12s; }
  .dev.on { background: var(--accent); }
  .dev:hover { opacity: .85; }
</style>
