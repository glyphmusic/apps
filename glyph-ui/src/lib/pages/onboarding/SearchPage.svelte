<script lang="ts">
  // onboarding.rs search(): a query types itself (75 ms per letter, caret in
  // the accent), the sources search (their dots pulse), results from all of
  // them fall into one list. The chips switch sources on and off. Field h 36,
  // px 12, radius 10, white 6 %; chips h 24, px 10, round, gap 6; result rows
  // h 28, gap 10 (cover 24, title 13, artist 12, the source as a small chip).
  import Icon from "../../components/Icon.svelte";
  import Skeleton from "../../components/Skeleton.svelte";
  import { t } from "../../i18n.svelte";
  import { easeOut, k, ob, QUERY, SONGS, SOURCES } from "./anim.svelte";
  let { s, now, still, h }: { s: number; now: number; still: boolean; h: number } = $props();

  const typingDone = 0.3 + QUERY.length * 0.075;
  const replay = $derived(ob.resultsAt != null || still);
  const typed = $derived(replay ? QUERY.length : Math.min(QUERY.length, Math.floor(Math.max(0, (s - 0.3) / 0.075))));
  const caret = $derived(!replay && s < typingDone + 0.5 && Math.floor(s * 2.4) % 2 === 0);
  const searching = $derived(!replay && s > typingDone && s < typingDone + 0.55);
  const since = $derived(ob.resultsAt != null ? (now - ob.resultsAt) / 1000 : still ? 100 : s - typingDone - 0.55);
  const room = $derived(Math.max(1, Math.floor((h - 36 - 24 - 36 - 20) / 32)));
  const shown = $derived(SONGS.filter((x) => ob.sources & (1 << x[2])).slice(0, room));

  function toggle(i: number) { ob.sources ^= 1 << i; ob.resultsAt = performance.now(); }
</script>

<div class="p">
  <div class="field">
    <Icon name="search" size={15} color="rgba(255,255,255,.5)" />
    <span class="q">{QUERY.slice(0, typed)}<i class="caret" style:opacity={caret ? 1 : 0}></i></span>
  </div>
  <div class="chips">
    {#each SOURCES as [name, hue, icon], i (name)}
      {@const on = !!(ob.sources & (1 << i))}
      {@const pulse = searching ? 0.35 + 0.65 * Math.abs(Math.sin((s - typingDone) * 9 + i * 0.8)) : 1}
      <button class="chip" class:on style:opacity={easeOut(k(s, 0.05 * i, 0.35))} onclick={() => toggle(i)}>
        <Icon name={icon} size={13} color={on ? `hsla(${hue * 360}, 78%, 68%, ${pulse})` : "rgba(255,255,255,.3)"} />
        <span class="nm" style:color="rgba(255,255,255,{on ? 0.88 : 0.35})">{name}</span>
      </button>
    {/each}
  </div>
  {#if searching}
    <div class="sk">{#each Array.from({ length: Math.min(room, 3) }, (_, i) => i) as i (i)}<Skeleton w={160 + 50 * (i % 2)} h={18} radius={5} id="onb-sk-{i}" />{/each}</div>
  {:else if shown.length === 0 && since > 0}
    <div class="none">{t("onboarding.search.none")}</div>
  {:else}
    <div class="rows">
      {#each shown as [title, artist, src, cover], j (`${title}-${artist}-${src}`)}
        {@const e = easeOut(k(since, j * 0.08, 0.3))}
        <div class="r" style:opacity={e} style:transform="translateY({8 * (1 - e)}px)">
          <span class="cv" style:background="hsl({SOURCES[src][1] * 360} 45% 42%)" style:background-image="url({cover})" style:background-size="cover"></span>
          <span class="tt"><span class="a">{title}</span><span class="b">{artist}</span></span>
          <span class="src">{SOURCES[src][0]}</span>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .p { width: 100%; height: 100%; display: flex; flex-direction: column; gap: 10px; padding: 18px 20px; }
  .field { display: flex; align-items: center; gap: 8px; height: 36px; padding: 0 12px; border-radius: 10px; background: rgba(255, 255, 255, .06); flex: none; }
  .q { display: flex; align-items: center; font-size: 13.5px; color: rgba(255, 255, 255, .92); }
  .caret { margin-left: 1px; width: 1.5px; height: 16px; background: var(--accent); }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; flex: none; }
  .chip { display: flex; align-items: center; gap: 6px; height: 24px; padding: 0 10px; border-radius: 12px; background: rgba(255, 255, 255, .03); transition: background-color .15s ease-out; }
  .chip.on { background: rgba(255, 255, 255, .11); }
  .chip:hover { background: rgba(255, 255, 255, .14); }
  .nm { font-size: 11.5px; transition: color .15s; }
  .sk { display: flex; flex-direction: column; gap: 8px; padding-top: 4px; }
  .none { padding-top: 10px; font-size: 12px; color: rgba(255, 255, 255, .45); }
  .rows { display: flex; flex-direction: column; gap: 4px; min-height: 0; }
  .r { display: flex; align-items: center; gap: 10px; height: 28px; }
  .cv { flex: none; width: 24px; height: 24px; border-radius: 4.8px; }
  .tt { flex: 1; min-width: 0; overflow: hidden; white-space: nowrap; display: flex; align-items: center; gap: 7px; }
  .a { font-size: 13px; color: rgba(255, 255, 255, .9); }
  .b { font-size: 12px; color: rgba(255, 255, 255, .42); }
  .src { flex: none; padding: 1px 6px; border-radius: 4px; font-size: 10.5px; font-weight: 500; color: rgba(255, 255, 255, .6); background: rgba(255, 255, 255, .07); }
</style>
