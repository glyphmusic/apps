<script lang="ts">
  // widgets/buttons.rs toggle_button: on = filled icon (or a 4 px dot under
  // line-only icons) in text colour or the accent; off = dim outline. The
  // icon pops 0.7 → 1.18 → 1.0 on click; a liked heart throws a few dots.
  import Icon from "./Icon.svelte";
  import { holdMenu, tip } from "../ui.svelte";
  import { tipFor } from "../i18n.svelte";
  import { hasFill } from "../icons";
  let {
    name, onName, size = 15, on, accent, label, onclick, oncontextmenu, onpointerdown, notip = false, class: cls = "",
  }: {
    name: string; onName?: string; size?: number; on: boolean; accent?: string; label?: string; notip?: boolean; class?: string;
    onclick?: (e: MouseEvent) => void; oncontextmenu?: (e: MouseEvent) => void; onpointerdown?: (e: PointerEvent) => void;
  } = $props();
  let pop = $state(0);
  let burst = $state(0);
  const fill = $derived(on && hasFill(name));
  const shownName = $derived(on ? (onName ?? (fill ? `${name}-fill` : name)) : name);
  const color = $derived(on ? (accent ?? "var(--text)") : "var(--dim)");
  const box = $derived(size + 14);
  const text = $derived(label ?? tipFor(name));
  function click(e: MouseEvent) {
    e.stopPropagation();
    pop++;
    if ((name === "heart" || !!onName) && !on) burst++;
    onclick?.(e);
  }
  const dots = Array.from({ length: 8 }, (_, i) => {
    const a = (i / 8) * Math.PI * 2 - Math.PI / 2;
    return { dx: Math.cos(a), dy: Math.sin(a), far: i % 2 === 0 ? 1.05 : 0.8, d: i % 2 === 0 ? 3.5 : 2.5 };
  });
</script>

<button class="tb {cls}" style:width="{box}px" style:height="{box}px" style:color={color} use:tip={notip ? undefined : text} aria-label={text ?? name} aria-pressed={on} onclick={click} {oncontextmenu} {onpointerdown} use:holdMenu={oncontextmenu}>
  {#key pop}<span class="g" class:popping={pop > 0}><Icon name={shownName} {size} /></span>{/key}
  {#if on && !onName && !hasFill(name)}<span class="dot" style:left="{(box - 4) / 2}px"></span>{/if}
  {#key burst}
    {#if burst > 0 && on}
      <span class="burst">
        {#each dots as d}
          <i style:width="{d.d}px" style:height="{d.d}px" style:left="{box / 2 - d.d / 2}px" style:top="{box / 2 - d.d / 2}px"
            style:--tx="{d.dx * (size * 0.35 + size * d.far)}px" style:--ty="{d.dy * (size * 0.35 + size * d.far)}px"></i>
        {/each}
      </span>
    {/if}
  {/key}
</button>

<style>
  .tb { position: relative; flex: none; display: grid; place-items: center; border-radius: 8px; transition: background-color .12s ease-out, color .15s ease-out; }
  .tb:hover { background: var(--hover); }
  .tb:active { background: var(--active); }
  .g { display: grid; place-items: center; }
  .g.popping { animation: pop .26s var(--ease); }
  .dot { position: absolute; bottom: 1px; width: 4px; height: 4px; border-radius: 50%; background: currentColor; }
  .burst { position: absolute; inset: 0; pointer-events: none; }
  .burst i { position: absolute; border-radius: 50%; background: currentColor; animation: burst .6s var(--ease) forwards; }
  @keyframes burst { from { transform: translate(0, 0); opacity: 1; } to { transform: translate(var(--tx), var(--ty)); opacity: 0; } }
</style>
