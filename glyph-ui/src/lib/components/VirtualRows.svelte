<script lang="ts">
  // Fixed-pitch viewport virtualization against `#main-scroll` so long track,
  // album and artist lists mount only visible rows + a small overscan window.
  import { onMount, type Snippet } from "svelte";
  let { count, pitch = 56, overscan = 6, row }: { count: number; pitch?: number; overscan?: number; row: Snippet<[number]> } = $props();
  let host: HTMLDivElement | undefined = $state();
  let from = $state(0), to = $state(30);
  let raf = 0;

  function update() {
    raf = 0;
    const sc = document.getElementById("main-scroll");
    if (!sc || !host) {
      from = 0;
      to = Math.min(count, 30);
      return;
    }
    const top = host.getBoundingClientRect().top - sc.getBoundingClientRect().top;
    from = Math.max(0, Math.floor(-top / pitch) - overscan);
    to = Math.min(count, Math.ceil((sc.clientHeight - top) / pitch) + overscan);
  }

  function schedule() {
    if (!raf) raf = requestAnimationFrame(update);
  }

  onMount(() => {
    const sc = document.getElementById("main-scroll");
    const ro = sc && typeof ResizeObserver !== "undefined" ? new ResizeObserver(schedule) : null;
    if (sc && ro) ro.observe(sc);
    sc?.addEventListener("scroll", schedule, { passive: true });
    addEventListener("resize", schedule);
    update();
    return () => {
      cancelAnimationFrame(raf);
      ro?.disconnect();
      sc?.removeEventListener("scroll", schedule);
      removeEventListener("resize", schedule);
    };
  });

  $effect(() => {
    void count;
    void pitch;
    queueMicrotask(update);
  });

  const visible = $derived(Array.from({ length: Math.max(0, Math.min(count, to) - from) }, (_, k) => from + k));
</script>

<div class="v" bind:this={host} style:height="{count * pitch}px">
  {#each visible as i (i)}
    <div class="r" style:top="{i * pitch}px" style:height="{pitch}px">{@render row(i)}</div>
  {/each}
</div>

<style>
  .v { position: relative; contain: layout style; }
  .r { position: absolute; left: 0; right: 0; }
</style>
