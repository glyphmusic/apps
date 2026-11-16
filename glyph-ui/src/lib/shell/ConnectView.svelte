<script lang="ts">
  // Shared Connect view used by both the desktop right panel (ConnectPanel)
  // and the compact/mobile bottom sheet (BottomSheet): active device card with
  // inline Jam button and click-through chevron to Streaming Quality + Audio
  // Output, Jam sub-views (main / invite / settings via JamSection), and
  // discovered Glyph apps & network devices.
  import { onMount } from "svelte";
  import { fade } from "svelte/transition";
  import { backend } from "../api/backend";
  import { app } from "../state.svelte";
  import { t, tf } from "../i18n.svelte";
  import { jam, watchJam } from "../jam.svelte";
  import { onWeb } from "../web";
  import { fadeEdges } from "../ui.svelte";
  import { deviceIcon, isGlyphApp, kindLabel, thisIcon, thisName, type DeviceX } from "../connect";
  import { drag } from "../compact/cstate.svelte";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import Button from "../components/Button.svelte";
  import Avatar from "../pages/profile/Avatar.svelte";
  import JamSection from "../pages/jam/JamSection.svelte";

  /** Docs page: what to check when a device is missing. */
  const HELP_URL = "https://docs.glyph.my/using-glyph/devices";
  const QUALITIES = ["auto", "low", "standard", "high"] as const;

  let {
    sheet = false,
    onclose,
    onpull,
    ondrop,
  }: {
    sheet?: boolean;
    onclose: () => void;
    onpull?: (dy: number) => void;
    ondrop?: (dy: number, vy: number) => void;
  } = $props();

  let searching = $state(false);
  let sending = $state<string | null>(null);
  let error = $state<string | null>(null);
  let qualityOpen = $state(false);
  let outputs = $state<string[]>([]);
  let stageEl: HTMLDivElement | undefined = $state();

  const web = $derived(onWeb());
  const devices = $derived(app.devices as DeviceX[]);
  const searchOn = $derived(!web && (app.config?.devices.discovery ?? true));
  const inJam = $derived(!web && jam.state != null);
  const current = $derived(devices.find((d) => d.active) ?? null);
  const remote = $derived(!!current && current.kind !== "this");
  const curName = $derived(current && current.kind !== "this" ? current.name : thisName());
  const curQuality = $derived(app.config?.audio.quality ?? "auto");
  const curQualityLabel = $derived(t(`sp.audio.q.${curQuality}`));
  const curOutput = $derived(app.config?.audio.output_device ?? null);
  const localOut = $derived(curOutput ?? t("sp.output.default"));
  const outputNames = $derived([...new Set([...(outputs.length ? outputs : app.outputDevices), ...(curOutput ? [curOutput] : [])])]);
  const apps = $derived(devices.filter((d) => isGlyphApp(d) && !d.active && !d.offline));
  const network = $derived(devices.filter((d) => !isGlyphApp(d) && d.kind !== "this" && !d.active));
  const signedIn = $derived(app.account != null);

  async function scan(now = false) {
    searching = true;
    try {
      const list = await backend().call<DeviceX[] | undefined>("devices_rescan", { now });
      if (list) app.devices = list;
    } catch { /* offline */ }
    searching = false;
  }
  async function loadOutputs() {
    try {
      const list = await backend().call<string[] | undefined>("output_devices");
      if (list) outputs = list;
    } catch { /* offline */ }
  }
  onMount(() => {
    if (searchOn || (web && signedIn)) scan();
    if (!web) loadOutputs();
    // Rust polls the account's devices faster while this panel is open.
    backend().call("account_devices_open", { open: true });
    const stop = web ? () => {} : watchJam();
    return () => { stop(); backend().call("account_devices_open", { open: false }); };
  });

  $effect(() => { if (app.jamOpen) qualityOpen = false; else if (jam.page !== "main") jam.page = "main"; });

  const curSub = $derived.by((): [string, boolean] => {
    if (!current || current.kind === "this") return [curOutput ? `${curQualityLabel} · ${curOutput}` : curQualityLabel, false];
    if (current.error) return [current.error, true];
    if (isGlyphApp(current)) return [current.connecting ? t("connect.account.confirming") : current.offline ? t("connect.account.reconnecting") : t("connect.account.control"), false];
    if (current.unreachable) return [t("connect.unreachable"), false];
    return [current.connecting ? t("connect.connecting") : kindLabel(current), false];
  });
  const hostMember = $derived(jam.state?.members.find((m) => m.host) ?? null);
  const hostName = $derived(hostMember?.handle ?? jam.state?.host_handle ?? "");
  const hostAvatar = $derived(jam.state?.host ? (hostMember?.avatar ?? app.ownAvatarUrl) : (hostMember?.avatar ?? null));
  const subView = $derived(
    app.jamOpen
      ? jam.state && jam.page === "invite"
        ? "jam_invite"
        : jam.state?.host && jam.page === "settings"
          ? "jam_settings"
          : "jam"
      : qualityOpen
        ? "quality"
        : "main",
  );

  function appSub(d: DeviceX): [string, boolean] {
    if (sending === d.id || d.connecting) return [t("connect.connecting"), false];
    if (d.error) return [d.error, true];
    if (d.busy) return [t("connect.account.busy"), false];
    return [d.track || t("connect.account.ready"), false];
  }

  async function pick(d: DeviceX) {
    error = null;
    sending = d.id;
    try {
      const list = await backend().call<DeviceX[] | undefined>(isGlyphApp(d) ? "device_connect" : "device_select", { id: d.id });
      if (list) app.devices = list;
    } catch (e) { error = String((e as Error)?.message ?? e); }
    sending = null;
  }
  async function useLocal() {
    error = null;
    const list = await backend().call<DeviceX[] | undefined>("device_use_local");
    if (list) app.devices = list;
  }
  function openQuality() {
    qualityOpen = true;
    app.jamOpen = false;
    loadOutputs();
  }
  function openJam() {
    qualityOpen = false;
    jam.page = "main";
    app.jamOpen = true;
  }
  function handleClose() {
    qualityOpen = false;
    onclose();
  }
  const help = () => backend().call("open_url", { url: HELP_URL });

  /** Attach sheet pull-to-dismiss only when rendered inside BottomSheet. */
  function dragIf(el: HTMLElement, opts: Parameters<typeof drag>[1] | null) {
    let inst = opts ? drag(el, opts) : null;
    return {
      update(next: Parameters<typeof drag>[1] | null) {
        if (next && inst) inst.update(next);
        else if (next && !inst) inst = drag(el, next);
        else if (!next && inst) { inst.destroy(); inst = null; }
      },
      destroy() { inst?.destroy(); },
    };
  }

  const stageAtTop = () => {
    const sc = [...(stageEl?.querySelectorAll<HTMLElement>(".body") ?? [])].pop();
    return (sc?.scrollTop ?? 0) <= 0;
  };
</script>

{#snippet tile(name: string, on: boolean)}
  <span class="tile"><Icon {name} size={20} color={on ? "var(--accent)" : "var(--dim)"} /></span>
{/snippet}

{#snippet row(d: DeviceX, sub: string, warn: boolean, enabled: boolean, click: () => void, playing: boolean)}
  <button class="row" class:off={!enabled} disabled={!enabled} onclick={click}>
    {@render tile(deviceIcon(d), false)}
    <span class="tx">
      <span class="n trunc">{d.name}</span>
      <span class="s trunc" class:warn>{sub}</span>
    </span>
    {#if playing}<span class="pl"><Icon name="audio-lines" size={14} color="var(--dim)" /></span>{/if}
  </button>
{/snippet}

{#snippet headerBar()}
  <header class:sheet>
    <div class="hwrap">
      {#key subView}
        <div class="hslot" class:sub={subView !== "main"} in:fade={{ duration: 160, delay: 30 }} out:fade={{ duration: 120 }}>
          {#if subView === "jam_invite"}
            <IconButton name="chevron-left" size={18} label={t("connect.back")} onclick={() => (jam.page = "main")} />
            <h2 class="back crumb trunc">
              <span class="cdim">{t("jam.title")}</span>
              <Icon name="chevron-right" size={14} color="rgba(255,255,255,.4)" />
              <span>{t("jam.invite")}</span>
            </h2>
          {:else if subView === "jam_settings"}
            <IconButton name="chevron-left" size={18} label={t("connect.back")} onclick={() => (jam.page = "main")} />
            <h2 class="back crumb trunc">
              <span class="cdim">{t("jam.title")}</span>
              <Icon name="chevron-right" size={14} color="rgba(255,255,255,.4)" />
              <span>{t("jam.settings")}</span>
            </h2>
          {:else if subView === "jam"}
            <IconButton name="chevron-left" size={18} label={t("connect.back")} onclick={() => { app.jamOpen = false; jam.page = "main"; }} />
            <h2 class:on={inJam} class="back trunc">{t("jam.title")}</h2>
            {#if inJam && jam.state?.host}
              <IconButton name="settings" size={15} label={t("jam.settings")} onclick={() => (jam.page = "settings")} />
            {/if}
          {:else if subView === "quality"}
            <IconButton name="chevron-left" size={18} label={t("connect.back")} onclick={() => (qualityOpen = false)} />
            <h2 class="back trunc">{curName}</h2>
          {:else}
            <h2 class:big={sheet}>{t("connect.title")}</h2>
            {#if searchOn || (web && signedIn)}<IconButton name="refresh-cw" size={15} label={t("connect.rescan")} onclick={() => scan(true)} />{/if}
          {/if}
        </div>
      {/key}
    </div>
    {#if !sheet}<IconButton name="x" size={16} onclick={handleClose} />{/if}
  </header>
{/snippet}

{#if sheet}
  <div class="grab" use:drag={{ axis: "y", move: (_, dy) => onpull?.(Math.max(0, dy)), end: (_, dy, _vx, vy) => ondrop?.(dy, vy) }}>
    <span class="hdl"></span>
    {@render headerBar()}
  </div>
{:else}
  {@render headerBar()}
{/if}

<div class="stage" bind:this={stageEl} use:dragIf={sheet ? { axis: "y", onlyAtTop: stageAtTop, move: (_, dy) => onpull?.(Math.max(0, dy)), end: (_, dy, _vx, vy) => ondrop?.(dy, vy) } : null}>
  {#key subView}
    <div class="scroll body" class:sheet use:fadeEdges={{ top: sheet ? 14 : 18, bottom: sheet ? 14 : 18, minOverflow: sheet ? 8 : undefined }} in:fade={{ duration: 170, delay: 30 }} out:fade={{ duration: 130 }}>
      {#if subView === "jam" || subView === "jam_invite" || subView === "jam_settings"}
        <JamSection />
      {:else if subView === "quality"}
        <div class="qwrap">
          <div class="card qhero">
            <div class="qlab">{t("sp.audio.quality")}</div>
            <div class="qval">{curQualityLabel}</div>
            <div class="qdesc">{t("sp.audio.quality.desc")}</div>
          </div>

          <div class="sec">{t("sp.audio.quality")}</div>
          <div class="card grp">
            {#each QUALITIES as q, qi (q)}
              {@const on = curQuality === q}
              <button class="opt" class:sep={qi > 0} class:on onclick={() => app.setConfig((c) => { c.audio.quality = q; })}>
                <span class="ol trunc">{t(`sp.audio.q.${q}`)}</span>
                {#if on}<Icon name="check" size={15} color="var(--accent)" />{/if}
              </button>
            {/each}
          </div>

          {#if !remote && !web}
            <div class="sec">{t("sp.output")}</div>
            <div class="card grp">
              <button class="opt" class:on={curOutput === null} onclick={() => app.setConfig((c) => { c.audio.output_device = null; })}>
                <span class="ol trunc">{t("sp.output.default")}</span>
                {#if curOutput === null}<Icon name="check" size={15} color="var(--accent)" />{/if}
              </button>
              {#each outputNames as name (name)}
                {@const on = curOutput === name}
                <button class="opt sep" class:on onclick={() => app.setConfig((c) => { c.audio.output_device = name; })}>
                  <span class="ol trunc">{name}</span>
                  {#if on}<Icon name="check" size={15} color="var(--accent)" />{/if}
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {:else}
        <!-- active device card + inline jam row -->
        <div class="card curcard">
          <button class="cur" onclick={openQuality}>
            {@render tile(current ? deviceIcon(current) : thisIcon(), true)}
            <span class="tx">
              <span class="cn trunc">{curName}</span>
              <span class="s trunc" class:warn={curSub[1]}>{curSub[0]}</span>
            </span>
            <span class="chev"><Icon name="chevron-right" size={16} color="var(--dim)" /></span>
          </button>
          {#if !web}
            {#if inJam}
              <button class="jamrow jamon" onclick={openJam}>
                <Avatar src={hostAvatar} size={22} flat />
                <span class="jamhost trunc">{tf("jam.card_title", { who: hostName })}</span>
                <Icon name="chevron-right" size={15} color="var(--dim)" />
              </button>
            {:else}
              <div class="jamrow">
                <span class="jamdesc">{t("jam.start.sub")}</span>
                <button class="jampill" onclick={openJam}>
                  <Icon name="users" size={14} color="#fff" />
                  <span>{t("jam.start")}</span>
                </button>
              </div>
            {/if}
          {/if}
        </div>

        <div class="sec psec">{t("connect.devices")}</div>
        {#if remote || apps.length || (searchOn && network.length)}
          <div class="card list">
            {#if remote}
              <button class="row" onclick={useLocal}>
                {@render tile(thisIcon(), false)}
                <span class="tx"><span class="n trunc">{thisName()}</span><span class="s trunc">{web ? curQualityLabel : localOut}</span></span>
              </button>
            {/if}
            {#each apps as d (d.id)}
              {@const [sub, warn] = appSub(d)}
              {@render row(d, sub, warn, !d.busy && !sending, () => pick(d), !!d.playing && !!d.track)}
            {/each}
            {#if searchOn}
              {#each network as d (d.id)}
                {@render row(d, kindLabel(d), false, true, () => pick(d), false)}
              {/each}
            {/if}
          </div>
        {/if}

        {#if web && !signedIn}
          <div class="card disc-off">
            <div class="note nopad">{t("connect.account.signin")}</div>
            <div class="fl"><Button onclick={() => { handleClose(); app.openSettings("account"); }}>{t("menu.signin")}</Button></div>
          </div>
        {/if}
        {#if signedIn && apps.length === 0}<div class="note">{t("connect.account.none")}</div>{/if}
        {#if error}<div class="note warn">{error}</div>{/if}
        {#if searchOn && searching}<div class="note">{t("ctl.output.searching")}</div>{/if}
        {#if searchOn && !searching && network.length === 0}<div class="note">{t("ctl.output.none")}</div>{/if}
        {#if !web && !searchOn}
          <div class="card disc-off">
            <div class="note nopad">{t("connect.off")}</div>
            <div class="fl"><Button onclick={() => app.setConfig((c) => (c.devices.discovery = true)).then(() => scan())}>{t("connect.turn_on")}</Button></div>
          </div>
        {/if}
        <div class="spacer"></div>
        <footer>
          <button class="help" onclick={help}>
            <span class="ht trunc">{t("connect.help")}</span>
            <Icon name="external-link" size={14} color="var(--dim)" />
          </button>
        </footer>
      {/if}
    </div>
  {/key}
</div>

<style>
  .grab { flex: none; touch-action: none; }
  .hdl { display: block; width: 36px; height: 4px; margin: 10px auto 8px; border-radius: 2px; background: rgba(255, 255, 255, .25); }
  header { display: flex; align-items: center; gap: 2px; padding: 14px 10px 8px 16px; }
  header.sheet { padding: 0 10px 10px 16px; }
  .hwrap { position: relative; flex: 1; min-width: 0; height: 30px; }
  .hslot { position: absolute; inset: 0; display: flex; align-items: center; gap: 2px; }
  .hslot.sub { margin-left: -8px; }
  h2 { flex: 1; min-width: 0; margin: 0; font-size: var(--lg); font-weight: 600; color: var(--text); }
  h2.big { font-size: 21px; font-weight: 700; color: #fff; }
  h2.back { padding-left: 4px; }
  h2.on { color: var(--accent); }
  h2.crumb { display: flex; align-items: center; gap: 4px; }
  .cdim { color: rgba(255, 255, 255, .5); }
  .stage { position: relative; flex: 1; min-height: 0; }
  .body { position: absolute; inset: 0; padding-bottom: 12px; display: flex; flex-direction: column; }
  .body.sheet { padding-bottom: 16px; }
  .card { border-radius: 10px; background: rgba(255, 255, 255, .035); }
  .tile { flex: none; width: 40px; height: 40px; border-radius: 8px; background: rgba(255, 255, 255, .05); display: grid; place-items: center; transition: background-color .15s; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }

  .curcard { margin: 0 12px 6px; display: flex; flex-direction: column; overflow: hidden; animation: fade-in .25s ease-out both; }
  .cur { width: 100%; padding: 10px; display: flex; align-items: center; gap: 12px; transition: background-color .12s ease-out; text-align: left; }
  .cur:hover { background: rgba(255, 255, 255, .04); }
  .chev { flex: none; display: inline-flex; padding-right: 2px; }
  .cn { font-size: var(--md); font-weight: 600; color: var(--accent); }
  .s { font-size: var(--xs); color: var(--dim); }
  .s.warn, .note.warn { color: var(--warn); }

  .jamrow { width: 100%; display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 10px 12px 12px; border-top: 1px solid rgba(255, 255, 255, .06); text-align: left; }
  .jamon { padding: 10px 12px; justify-content: flex-start; transition: background-color .12s ease-out; }
  .jamon:hover { background: rgba(255, 255, 255, .04); }
  .jamhost { flex: 1; min-width: 0; font-size: var(--sm); font-weight: 600; color: var(--text); }
  .jamdesc { flex: 1; min-width: 0; font-size: var(--xs); line-height: 1.35; color: var(--dim); }
  .jampill { flex: none; height: 32px; padding: 0 12px; border-radius: 999px; border: 1px solid rgba(255, 255, 255, .32); display: inline-flex; align-items: center; gap: 6px; font-size: var(--xs); font-weight: 600; color: #fff; white-space: nowrap; transition: background-color .12s, border-color .12s, color .12s; }
  .jampill:hover { background: rgba(255, 255, 255, .08); border-color: rgba(255, 255, 255, .55); }

  .sec { padding: 12px 4px 6px; font-size: 11.5px; font-weight: 600; color: rgba(255, 255, 255, .45); text-transform: uppercase; letter-spacing: .02em; }
  .psec { padding: 12px 16px 6px; }
  .list { margin: 0 12px; padding: 4px; display: flex; flex-direction: column; }
  .row { position: relative; display: flex; flex-direction: row; align-items: center; gap: 12px; height: 52px; padding: 0 6px; border-radius: 7px; width: 100%; text-align: left; flex: none; transition: background-color .12s ease-out, opacity .15s; animation: fade-in .25s ease-out both; }
  .row + .row::before { content: ""; position: absolute; left: 6px; right: 6px; top: 0; height: 1px; background: rgba(255, 255, 255, .06); pointer-events: none; transition: opacity .12s ease-out; }
  .row:hover:not(.off) { background: rgba(255, 255, 255, .055); }
  .row:hover:not(.off)::before, .row:hover:not(.off) + .row::before { opacity: 0; }
  .row.off { opacity: .45; cursor: default; }
  .row .n { font-size: var(--sm); font-weight: 500; color: var(--text); }
  .pl { flex: none; padding-right: 6px; display: inline-flex; }
  .note { padding: 8px 16px; font-size: var(--sm); color: var(--dim); }
  .disc-off { margin: 4px 12px 0; padding: 12px; display: flex; flex-direction: column; gap: 10px; }
  .nopad { padding: 0; }
  .fl { display: flex; }

  .qwrap { padding: 0 12px; display: flex; flex-direction: column; }
  .qhero { padding: 14px; display: flex; flex-direction: column; gap: 4px; }
  .qlab { font-size: var(--xs); color: rgba(255, 255, 255, .55); }
  .qval { font-size: 18px; font-weight: 700; color: var(--accent); line-height: 1.25; }
  .qdesc { font-size: var(--xs); color: var(--dim); line-height: 1.4; margin-top: 2px; }
  .grp { padding: 4px; display: flex; flex-direction: column; }
  .opt { position: relative; width: 100%; min-height: 40px; padding: 8px 10px; border-radius: 7px; display: flex; align-items: center; justify-content: space-between; gap: 10px; font-size: var(--sm); font-weight: 500; color: rgba(255, 255, 255, .88); transition: background-color .12s; text-align: left; }
  .opt.sep::before { content: ""; position: absolute; left: 8px; right: 8px; top: 0; height: 1px; background: rgba(255, 255, 255, .06); pointer-events: none; transition: opacity .12s; }
  .opt:hover { background: rgba(255, 255, 255, .055); }
  .opt:hover::before, .opt:hover + .opt.sep::before { opacity: 0; }
  .opt.on { color: var(--accent); }
  .ol { flex: 1; min-width: 0; }
  .spacer { flex: 1; min-height: 12px; }

  footer { margin-top: 4px; padding: 6px 0 0; border-top: 1px solid rgba(255, 255, 255, .06); }
  .help { margin: 0 12px; width: calc(100% - 24px); height: 42px; padding: 0 10px; border-radius: 8px; display: flex; align-items: center; gap: 10px; transition: background-color .12s ease-out; text-align: left; }
  .help:hover { background: rgba(255, 255, 255, .055); }
  .ht { flex: 1; font-size: var(--sm); color: var(--text); text-decoration: underline; }
</style>
