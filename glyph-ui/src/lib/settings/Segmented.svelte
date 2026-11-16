<script lang="ts" generics="T extends string | number | boolean">
  // controls.rs segmented: one pill (p 3, gap 2, radius 9, white 5 %), the
  // selected option filled (white 12 %, medium); others white 60 % and
  // white + 4 % fill on hover. Options h 28, px 12, radius 7, 13 px.
  let { value, options, onpick, stretch = false, fill = false }: { value: T; options: { value: T; label: string }[]; onpick: (v: T) => void; stretch?: boolean; fill?: boolean } = $props();
</script>

<div class="seg" class:stretch class:fill role="radiogroup">
  {#each options as o (String(o.value))}
    <button class="o" class:on={o.value === value} role="radio" aria-checked={o.value === value} onclick={() => onpick(o.value)}>{o.label}</button>
  {/each}
</div>

<style>
  .seg { display: flex; flex-wrap: wrap; padding: 3px; gap: 2px; border-radius: 9px; background: #161618; max-width: 100%; }
  .seg.stretch { width: 100%; margin-bottom: 4px; }
  .seg.fill { width: 100%; }
  .o { height: 28px; padding: 0 12px; display: flex; align-items: center; border-radius: 7px; font-size: 13px; color: rgba(255, 255, 255, .6); white-space: nowrap; transition: background-color .15s ease-out, color .15s ease-out; }
  .seg.stretch .o { flex: 1; justify-content: center; height: 30px; }
  .seg.fill .o { flex: 1 0 auto; justify-content: center; padding: 0 10px; }
  .o:hover { background: #222226; color: #fff; }
  .o.on { background: #2c2c32; color: #fff; font-weight: 500; }
</style>
