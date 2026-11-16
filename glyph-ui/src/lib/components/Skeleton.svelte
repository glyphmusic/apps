<script lang="ts">
  // Lightweight GPU-composited placeholder shimmer with crisp ASCII characters.
  import { fx } from "../ui.svelte";
  let { w, h, radius = 6, id: _id = "", round = false, ascii }: { w: number | string; h: number | string; radius?: number; id?: string; round?: boolean; ascii?: boolean } = $props();
  const isAscii = $derived(ascii !== undefined ? ascii : fx.ascii);
  const r = $derived(round ? 9999 : radius);
</script>

<div
  class="sk"
  class:plain={!isAscii}
  class:anim={isAscii && !fx.reduce}
  style:width={typeof w === "number" ? `${w}px` : w}
  style:height={typeof h === "number" ? `${h}px` : h}
  style:border-radius="{r}px"
></div>

<style>
  .sk { position: relative; flex: none; overflow: hidden; background: rgba(255, 255, 255, .04); }
  .sk.plain { background: rgba(255, 255, 255, .06); }
  .sk:not(.plain)::before {
    content: "";
    position: absolute;
    inset: 0;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='60' height='40' viewBox='0 0 60 40'%3E%3Cstyle%3Etext%7Bfont-family:'JetBrains Mono',Consolas,monospace;font-size:7px;font-weight:700;fill:rgba(255,255,255,0.45);text-anchor:middle;dominant-baseline:central;%7D%3C/style%3E%3Ctext x='3' y='4'%3E%23%3C/text%3E%3Ctext x='9' y='4'%3E%25%3C/text%3E%3Ctext x='15' y='4'%3E%26amp%3B%3C/text%3E%3Ctext x='21' y='4'%3E*%3C/text%3E%3Ctext x='27' y='4'%3E%2B%3C/text%3E%3Ctext x='33' y='4'%3E%3D%3C/text%3E%3Ctext x='39' y='4'%3E-%3C/text%3E%3Ctext x='45' y='4'%3E%3A%3C/text%3E%3Ctext x='51' y='4'%3E.%3C/text%3E%3Ctext x='57' y='4'%3E%40%3C/text%3E%3Ctext x='3' y='12'%3E%2B%3C/text%3E%3Ctext x='9' y='12'%3E%3D%3C/text%3E%3Ctext x='15' y='12'%3E-%3C/text%3E%3Ctext x='21' y='12'%3E%3A%3C/text%3E%3Ctext x='27' y='12'%3E.%3C/text%3E%3Ctext x='33' y='12'%3E%40%3C/text%3E%3Ctext x='39' y='12'%3E%24%3C/text%3E%3Ctext x='45' y='12'%3E0%3C/text%3E%3Ctext x='51' y='12'%3EX%3C/text%3E%3Ctext x='57' y='12'%3E%23%3C/text%3E%3Ctext x='3' y='20'%3E%40%3C/text%3E%3Ctext x='9' y='20'%3E%24%3C/text%3E%3Ctext x='15' y='20'%3E0%3C/text%3E%3Ctext x='21' y='20'%3EX%3C/text%3E%3Ctext x='27' y='20'%3E%23%3C/text%3E%3Ctext x='33' y='20'%3E%25%3C/text%3E%3Ctext x='39' y='20'%3E%26amp%3B%3C/text%3E%3Ctext x='45' y='20'%3E*%3C/text%3E%3Ctext x='51' y='20'%3E%2B%3C/text%3E%3Ctext x='57' y='20'%3E%3D%3C/text%3E%3Ctext x='3' y='28'%3E%26amp%3B%3C/text%3E%3Ctext x='9' y='28'%3E*%3C/text%3E%3Ctext x='15' y='28'%3E%2B%3C/text%3E%3Ctext x='21' y='28'%3E%3D%3C/text%3E%3Ctext x='27' y='28'%3E-%3C/text%3E%3Ctext x='33' y='28'%3E%3A%3C/text%3E%3Ctext x='39' y='28'%3E.%3C/text%3E%3Ctext x='45' y='28'%3E%40%3C/text%3E%3Ctext x='51' y='28'%3E%24%3C/text%3E%3Ctext x='57' y='28'%3E0%3C/text%3E%3Ctext x='3' y='36'%3E%3A%3C/text%3E%3Ctext x='9' y='36'%3E.%3C/text%3E%3Ctext x='15' y='36'%3E%40%3C/text%3E%3Ctext x='21' y='36'%3E%24%3C/text%3E%3Ctext x='27' y='36'%3E0%3C/text%3E%3Ctext x='33' y='36'%3EX%3C/text%3E%3Ctext x='39' y='36'%3E%23%3C/text%3E%3Ctext x='45' y='36'%3E%25%3C/text%3E%3Ctext x='51' y='36'%3E%26amp%3B%3C/text%3E%3Ctext x='57' y='36'%3E*%3C/text%3E%3C/svg%3E");
    background-size: 60px 40px;
    opacity: .7;
  }
  .sk.anim::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(90deg, transparent 0%, rgba(255, 255, 255, .22) 50%, transparent 100%);
    transform: translateX(-100%);
    animation: sk-sweep 1.6s linear infinite;
  }
  @keyframes sk-sweep { to { transform: translateX(100%); } }
</style>
