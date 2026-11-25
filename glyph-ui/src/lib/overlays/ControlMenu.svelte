<script lang="ts">
  // controls.rs control_menu: right-click menus of the player controls.
  // 236 px wide, max h 420, p 6, radius 10, hsla 0 0 7.5 % .98, 1 px white
  // 10 % border; opens upwards from the click (flips down near the top of
  // the window, kept 8 px inside), fades + slides in over 140 ms, out over
  // 110 ms; a click, right click or scroll outside closes it. Rows: h 32,
  // px 10, radius 6, gap 10, 13 px white 88 %, 15 px icon (white 70 %, the
  // chosen one pink with a check at the end), hover white 7 %. Captions 11
  // px white 45 %, separators 1 px white 7 % (margin 4 6).
  // Menus: sleep (timer minutes, end of song, off), sort (lists), mini (card
  // / bar / close), repeat, shuffle, back / forward (10 s, 30 s, restart /
  // previous / next), volume (output, Connect, channels, mute).
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Icon from "../components/Icon.svelte";
  import Avatar from "../pages/profile/Avatar.svelte";
  import { app } from "../state.svelte";
  import { fadeEdges } from "../ui.svelte";
  import { drag, sheetIn, sheetOut, shouldClose } from "../compact/cstate.svelte";
  import { t, tf } from "../i18n.svelte";
  import { SLEEP_MINUTES, setSleep, sleepCountdown, sleepLabel } from "./sleep.svelte";
  import { setTab } from "../settings/settings.svelte";
  import { requestSignOut } from "../settings/account-state.svelte";
  import { listSort, listSortDir, setListSort, type ListSort } from "../pages/lists/actions";
  import type { RepeatMode } from "../api/types";
  import { onWeb } from "../web";
  import { openWebMini } from "../web-mini.svelte";

  type Row = { kind: "row"; icon: string; label: string; on?: boolean; danger?: boolean; run: () => void } | { kind: "account_head"; handle: string; avatar: string | null } | { kind: "caption"; text: string } | { kind: "sep" };
  // Kept after the menu closed so the fade-out can still draw it.
  let m = $state(app.controlMenu);
  $effect.pre(() => { if (app.controlMenu) m = app.controlMenu; });
  const close = () => app.closeMenus();
  const pick = (f: () => void) => () => { close(); f(); };

  let vw = $state(1200), vh = $state(800);
  let pull = $state(0);
  let sheetListEl: HTMLDivElement | undefined = $state();
  const compact = $derived(app.compact);
  $effect(() => { void app.controlMenu; pull = 0; });
  const sheetTitle = $derived.by(() => {
    switch (m?.menu) {
      case "sort": return t("sort.title");
      case "sleep": return t("sleep.title");
      case "repeat": return t("tip.repeat");
      case "shuffle": return t("tip.shuffle");
      case "volume": return t("tip.volume-2");
      default: return "";
    }
  });
  const W = 236;
  const up = $derived(!!m && m.menu !== "account" && m.y > 260);
  const left = $derived(m ? Math.max(8, Math.min(m.menu === "account" ? m.x - W : m.x, vw - W - 8)) : 0);

  function rows(): Row[] {
    if (!m) return [];
    const out: Row[] = [];
    const choice = (icon: string, label: string, on: boolean, run: () => void) => out.push({ kind: "row", icon, label, on, run: pick(run) });
    const plain = (icon: string, label: string, run: () => void, danger = false) => out.push({ kind: "row", icon, label, danger, run: pick(run) });
    switch (m.menu) {
      case "sort": {
        const l = app.openList;
        if (!l) break;
        const cur = listSort(l), dir = listSortDir(l);
        const pickSort = (s: ListSort) => () => {
          if (s === "custom") setListSort(l, "custom", "asc");
          else if (cur === s) setListSort(l, s, dir === "asc" ? "desc" : "asc");
          else setListSort(l, s, "asc");
        };
        const dirTag = (s: ListSort) => cur === s ? (dir === "asc" ? " ↑" : " ↓") : "";
        choice("list-music", l.kind === "liked" ? t("sort.added") : l.kind === "recent" ? t("library.recent") : t("sort.custom"), cur === "custom", pickSort("custom"));
        choice("music-2", `${t("sort.by_title")}${dirTag("title")}`, cur === "title", pickSort("title"));
        choice("user-round", `${t("sort.by_artist")}${dirTag("artist")}`, cur === "artist", pickSort("artist"));
        choice("disc-3", `${t("sort.by_album")}${dirTag("album")}`, cur === "album", pickSort("album"));
        choice("clock", `${t("sort.by_length")}${dirTag("duration")}`, cur === "duration", pickSort("duration"));
        break;
      }
      case "sleep": {
        const s = app.sleep;
        void app.clock;
        out.push({ kind: "caption", text: s?.endOfSong ? t("sleep.status.song") : s?.until ? tf("sleep.status.left", { time: sleepCountdown(s.until - Date.now()) }) : t("sleep.title") });
        for (const mins of SLEEP_MINUTES) choice("timer", sleepLabel(mins), s?.mins === mins, () => setSleep(mins));
        choice("music-2", t("sleep.end_of_song"), !!s?.endOfSong, () => setSleep("song"));
        if (s) { out.push({ kind: "sep" }); plain("x", t("sleep.off"), () => setSleep(null)); }
        break;
      }
      case "mini": {
        const big = app.config.player.mini_large, shown = app.miniShown;
        const open = (large: boolean) => () => {
          app.setConfig((c) => { c.player.mini_large = large; });
          if (onWeb()) openWebMini(app);
          else app.miniShown = true;
        };
        choice("picture-in-picture-2", t("ctl.mini.card"), shown && big, open(true));
        choice("panel-bottom", t("ctl.mini.bar"), shown && !big, open(false));
        if (shown) { out.push({ kind: "sep" }); plain("x", t("ctl.mini.close"), () => (app.miniShown = false)); }
        break;
      }
      case "repeat": {
        const cur = app.player.repeat;
        for (const [icon, key, mode] of [["repeat", "ctl.repeat.off", "off"], ["repeat", "ctl.repeat.all", "all"], ["repeat-1", "ctl.repeat.one", "one"]] as [string, string, RepeatMode][])
          choice(icon, t(key), cur === mode, () => app.setRepeat(mode));
        break;
      }
      case "shuffle": {
        const on = app.player.shuffle, style = app.config.playback.shuffle;
        choice("arrow-right", t("ctl.shuffle.off"), !on, () => app.setShuffle(false));
        for (const [key, s] of [["ctl.shuffle.smart", "smart"], ["ctl.shuffle.random", "random"]] as const)
          choice("shuffle", t(key), on && style === s, () => { app.setConfig((c) => { c.playback.shuffle = s; }); app.setShuffle(true); });
        break;
      }
      case "back": case "forward": {
        const back = m.menu === "back", sign = back ? -1 : 1, icon = back ? "rotate-ccw" : "rotate-cw";
        plain(icon, t(back ? "ctl.back_10" : "ctl.fwd_10"), () => app.seekRelative(sign * 10));
        plain(icon, t(back ? "ctl.back_30" : "ctl.fwd_30"), () => app.seekRelative(sign * 30));
        out.push({ kind: "sep" });
        if (back) {
          plain("skip-back", t("ctl.restart"), () => app.seek(0));
          plain("skip-back", t("ctl.previous"), () => { app.seek(0); app.previous(); });
        } else plain("skip-forward", t("ctl.next"), () => app.next());
        break;
      }
      case "volume": {
        const cur = app.config.audio.output_device, remote = app.devices.find((d) => d.active && d.kind !== "this");
        const out_ = (dev: string | null) => () => app.setConfig((c) => { c.audio.output_device = dev; c.mpd.enabled = false; });
        choice("speaker", t("sp.output.default"), cur == null && !remote, out_(null));
        if (!onWeb() && !app.outputDevices.length) out.push({ kind: "caption", text: t("ctl.output.searching") });
        const mpd = app.config.mpd;
        if (mpd.address.trim()) choice("radio", tf("ctl.output.mpd", { host: mpd.address.trim() }), mpd.enabled, () => app.setConfig((c) => { c.mpd.enabled = !mpd.enabled; }));
        for (const name of app.outputDevices) {
          if (name === app.outputDevices[0] && /default/i.test(name)) continue;
          choice("headphones", name, !remote && cur === name, out_(name));
        }
        if (remote) {
          const icon = ({ glyph: "laptop", cast: "cast", airplay: "airplay", upnp: "speaker", account: "smartphone", this: "laptop" } as Record<string, string>)[remote.kind];
          choice(icon, remote.name, true, () => { if (!app.showDevices) { app.showDevices = true; app.showQueue = false; } });
        }
        plain("monitor-speaker", t("tip.monitor-speaker"), () => { if (!app.showDevices) { app.showDevices = true; app.showQueue = false; } });
        out.push({ kind: "sep" });
        const mode = app.config.audio.channels;
        for (const [icon, key, v] of [["circle", "sp.audio.channels.mono", "mono"], ["audio-lines", "sp.audio.channels.stereo", "stereo"], ["orbit", "sp.audio.channels.spatial", "spatial"]] as const)
          choice(icon, t(key), mode === v, () => app.setConfig((c) => { c.audio.channels = v; }));
        out.push({ kind: "sep" });
        const muted = app.volume <= 0.001;
        plain(muted ? "volume-2" : "volume-x", t(muted ? "ctl.unmute" : "ctl.mute"), () => app.toggleMute());
        break;
      }
      case "account": {
        const acc = app.account;
        if (acc) {
          out.push({ kind: "account_head", handle: acc.handle, avatar: app.ownAvatarUrl });
          out.push({ kind: "sep" });
          plain("user-round", t("menu.account"), () => { setTab("account", 0); app.openSettings("account"); });
          plain("eye", t("menu.profile"), () => app.openProfilePage(acc.handle));
          plain("history", t("menu.recent"), () => app.openListView({ kind: "recent" }));
          plain("sparkles", t("taste.title"), () => app.navigate("taste"));
          plain("chart-column", t("menu.stats"), () => app.navigate("stats"));
          out.push({ kind: "sep" });
          plain("life-buoy", t("menu.support"), () => { setTab("about", 1); app.openSettings("about"); });
          plain("settings", t("menu.settings"), () => app.openSettings());
          out.push({ kind: "sep" });
          plain("log-out", t("account.signout"), requestSignOut, true);
        } else {
          plain("user-round", t("menu.signin"), () => { setTab("account", 0); app.openSettings("account"); });
          out.push({ kind: "sep" });
          plain("history", t("menu.recent"), () => app.openListView({ kind: "recent" }));
          plain("sparkles", t("taste.title"), () => app.navigate("taste"));
          plain("chart-column", t("menu.stats"), () => app.navigate("stats"));
          plain("life-buoy", t("menu.support"), () => { setTab("about", 1); app.openSettings("about"); });
          plain("settings", t("menu.settings"), () => app.openSettings());
        }
        break;
      }
    }
    return out;
  }
  const list = $derived(rows());
</script>

<svelte:window bind:innerWidth={vw} bind:innerHeight={vh} />

{#snippet rowItems(mobile: boolean)}
  {#each list as r}
    {#if r.kind === "account_head"}
      <div class="ahead">
        <Avatar src={r.avatar} size={mobile ? 28 : 24} flat />
        <span class="ahandle trunc">@{r.handle}</span>
      </div>
    {:else if r.kind === "caption"}
      <div class="cap">{r.text}</div>
    {:else if r.kind === "sep"}
      <div class="sep"></div>
    {:else}
      <button class="item" class:mob={mobile} class:on={r.on} class:danger={r.danger} role="menuitem" onclick={r.run}>
        <Icon name={r.icon} size={mobile ? 18 : 15} />
        <span class="lb trunc">{r.label}</span>
        {#if r.on}<Icon name="check" size={mobile ? 16 : 14} color="var(--accent)" />{/if}
      </button>
    {/if}
  {/each}
{/snippet}

{#if m}
  {#if compact}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div
      class="bs-dim"
      transition:fade|global={{ duration: 180 }}
      onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")}
      onpointerdown={close}
      oncontextmenu={(e) => { e.preventDefault(); close(); }}
      role="presentation"
    >
      <div
        class="bs-panel"
        data-part="menu"
        style:transform="translateY({pull}px)"
        class:settle={pull === 0}
        in:sheetIn|global={{ duration: 340 }}
        out:sheetOut|global={{ duration: 210 }}
        onpointerdown={(e) => e.stopPropagation()}
        oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); }}
        role="dialog"
        tabindex="-1"
      >
        <div
          class="bs-grab"
          use:drag={{
            axis: "y",
            move: (_, dy) => (pull = Math.max(0, dy)),
            end: (_, dy, _vx, vy) => {
              if (shouldClose(dy, vy)) { close(); return; }
              pull = 0;
            },
          }}
        >
          <span class="hdl"></span>
          {#if sheetTitle}<div class="bs-title">{sheetTitle}</div>{/if}
        </div>
        <div
          class="bs-list scroll"
          bind:this={sheetListEl}
          use:fadeEdges={{ top: 10, bottom: 10, minOverflow: 8 }}
          use:drag={{
            axis: "y",
            onlyAtTop: () => (sheetListEl?.scrollTop ?? 0) <= 0,
            move: (_, dy) => (pull = Math.max(0, dy)),
            end: (_, dy, _vx, vy) => {
              if (shouldClose(dy, vy)) { close(); return; }
              pull = 0;
            },
          }}
        >
          {@render rowItems(true)}
        </div>
      </div>
    </div>
  {:else}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="bd" onpointerdown={close} oncontextmenu={(e) => { e.preventDefault(); close(); }} onwheel={close} transition:fade|global={{ duration: 110 }} onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")}>
      <div class="menu" role="menu" tabindex="-1" in:fly|global={{ y: 6, duration: 140, easing: cubicOut }} out:fly|global={{ y: 6, duration: 110, easing: cubicOut }}
        style:left="{left}px" style:bottom={up ? `${vh - m.y}px` : undefined} style:top={up ? undefined : `${m.y}px`}
        onpointerdown={(e) => e.stopPropagation()} oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); }}>
        <div class="menu-in scroll" use:fadeEdges={{ top: 10, bottom: 10, minOverflow: 8 }} style:max-height="{Math.min(420, up ? m.y - 8 : vh - m.y - 8)}px">
          {@render rowItems(false)}
        </div>
      </div>
    </div>
  {/if}
{/if}

<style>
  .bd { position: fixed; inset: 0; z-index: 90; }
  .menu { position: absolute; width: 236px; border-radius: 10px; background: #181818; overflow: hidden; font-family: var(--font); }
  .menu-in { padding: 6px; display: flex; flex-direction: column; }
  .ahead { display: flex; align-items: center; gap: 8px; padding: 6px 10px; }
  .ahandle { min-width: 0; font-size: 13px; font-weight: 500; color: rgba(255, 255, 255, .9); }
  .cap { padding: 6px 10px 2px; font-size: 11px; color: rgba(255, 255, 255, .45); }
  .sep { height: 1px; margin: 4px 6px; background: rgba(255, 255, 255, .07); flex: none; }
  .item { flex: none; display: flex; align-items: center; gap: 10px; height: 32px; padding: 0 10px; border-radius: 6px; font-size: 13px; color: rgba(255, 255, 255, .88); transition: background-color .12s ease-out; }
  .item :global(.ic) { color: rgba(255, 255, 255, .7); }
  .item:hover { background: rgba(255, 255, 255, .07); }
  .item.on :global(.ic:first-child) { color: var(--accent); }
  .item.danger { color: var(--danger); }
  .item.danger :global(.ic) { color: var(--danger); }
  .lb { flex: 1; text-align: left; }

  .bs-dim { position: fixed; inset: 0; z-index: 90; background: rgba(0, 0, 0, .55); }
  .bs-panel {
    position: absolute; left: 0; right: 0; bottom: 0;
    max-height: min(76vh, calc(100vh - var(--safe-top, env(safe-area-inset-top)) - 16px));
    border-radius: 16px 16px 0 0;
    border-top: 1px solid rgba(255, 255, 255, .08);
    background: hsl(0 0% 8%);
    box-shadow: 0 -12px 40px rgba(0, 0, 0, .6);
    padding-bottom: max(10px, var(--safe-bottom, env(safe-area-inset-bottom)), var(--ime-bottom, 0px));
    display: flex; flex-direction: column; overflow: hidden; font-family: var(--font);
  }
  .bs-panel.settle { transition: transform .24s var(--ease); }
  .bs-grab { flex: none; touch-action: none; }
  .hdl { display: block; width: 36px; height: 4px; margin: 10px auto 6px; border-radius: 2px; background: rgba(255, 255, 255, .25); }
  .bs-title { padding: 4px 16px 10px; font-size: 15px; font-weight: 600; color: rgba(255, 255, 255, .95); border-bottom: 1px solid rgba(255, 255, 255, .07); }
  .bs-list { flex: 1; min-height: 0; padding: 6px 8px; display: flex; flex-direction: column; gap: 2px; }
  .bs-list .cap { padding: 8px 12px 4px; font-size: 12px; }
  .bs-list .sep { margin: 6px 8px; }
  .item.mob { height: 48px; padding: 0 12px; gap: 14px; border-radius: 10px; font-size: 15px; }
  .item.mob:active { background: rgba(255, 255, 255, .09); }
  .item.mob.on { color: #fff; font-weight: 500; }
</style>
