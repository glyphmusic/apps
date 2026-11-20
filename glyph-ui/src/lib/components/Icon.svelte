<script lang="ts">
  // Flat icon from Glyph's own set (widgets.rs icon, docs/ICONS.md): size in px, colour = currentColor.
  import { iconMask } from "../icons";
  import { iconOv } from "../icon-overrides.svelte";
  let { name, size = 16, color, class: cls = "", style = "" }: { name: string; size?: number; color?: string; class?: string; style?: string } = $props();
  // A plugin's look may replace the icon (icon-overrides.svelte.ts).
  const mask = $derived(iconOv.masks[name] ?? iconMask(name));
</script>

<span class="ic {cls}" style:width="{size}px" style:height="{size}px" style:color={color} {style} aria-hidden="true"
  >{#if mask}<span class="g" style:-webkit-mask-image={mask} style:mask-image={mask}></span>{/if}</span
>

<style>
  .ic { display: inline-flex; flex: none; line-height: 0; }
  .g {
    width: 100%;
    height: 100%;
    display: block;
    background-color: currentColor;
    -webkit-mask-repeat: no-repeat;
    mask-repeat: no-repeat;
    -webkit-mask-position: center;
    mask-position: center;
    -webkit-mask-size: contain;
    mask-size: contain;
  }
</style>
