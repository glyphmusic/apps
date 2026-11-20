<script lang="ts">
  // Server dashboard (admins only, sidebar "Server"), built from the stats
  // page's pieces. The overview keeps the important bits: four tiles, two
  // ring charts (where the space goes, space per person), 30 days of history,
  // the five people seen last, the main settings, both update channels,
  // cache and bans. "All" opens a sub page (/server/people, /server/settings,
  // /server/versions); back returns to the overview. On phones the role and
  // refresh sit in the top bar next to back (compact/Compact.svelte).
  // Everything goes through the server's admin API with the app sign-in.
  import { app } from "../state.svelte";
  import { t, tf } from "../i18n.svelte";
  import Button from "../components/Button.svelte";
  import IconButton from "../components/IconButton.svelte";
  import Icon from "../components/Icon.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import ErrorCard from "../components/ErrorCard.svelte";
  import SectionTitle from "../components/SectionTitle.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import Segmented from "../settings/Segmented.svelte";
  import Field from "../settings/Field.svelte";
  import Tog from "../settings/Tog.svelte";
  import { whenMs } from "../settings/util";
  import BackPill from "./lists/BackPill.svelte";
  import Tile from "./stats/Tile.svelte";
  import Donut from "./server/Donut.svelte";
  import Trend from "./server/Trend.svelte";
  import { admin, adminCall, type AdminSub } from "../admin/admin.svelte";
  import {
    fmtBytes, OTHER, SERIES,
    type AdminAccount, type AdminAccountRow, type AppRelease, type Ban, type DayStats, type Limits, type ServerStats, type Slice,
  } from "../admin/api";

  type Metric = "accounts" | "active" | "space";

  let stats = $state<ServerStats | null>(null);
  let days = $state<DayStats[]>([]);
  let people = $state<AdminAccountRow[]>([]);
  let total = $state(0);
  let limits = $state<Limits | null>(null);
  let draft = $state<Limits | null>(null);
  let bans = $state<Ban[]>([]);
  let releases = $state<AppRelease[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let metric = $state<Metric>("accounts");
  let query = $state("");
  let open = $state<string | null>(null);
  let detail = $state<AdminAccount | null>(null);
  let busy = $state(false);

  async function load() {
    loading = true;
    error = null;
    try {
      const [s, h, a, l, b, r] = await Promise.all([
        adminCall<ServerStats>("GET", "stats"),
        adminCall<{ days: DayStats[] }>("GET", "history").catch(() => ({ days: [] })),
        adminCall<{ accounts: AdminAccountRow[]; total: number }>("GET", "accounts?limit=500").catch(() => ({ accounts: [], total: 0 })),
        adminCall<Limits>("GET", "limits").catch(() => null),
        adminCall<{ bans: Ban[] }>("GET", "bans").catch(() => ({ bans: [] })),
        adminCall<{ releases: AppRelease[] }>("GET", "app-releases").catch(() => ({ releases: [] })),
      ]);
      stats = s;
      days = h.days ?? [];
      people = a.accounts ?? [];
      total = a.total ?? people.length;
      limits = l;
      draft = l ? { ...l } : null;
      bans = b.bans ?? [];
      releases = r.releases ?? [];
    } catch (e) {
      error = String((e as Error).message ?? e);
    } finally {
      loading = false;
    }
  }
  $effect(() => admin.check(app.account?.handle));
  // `admin.reload` is bumped by the refresh button (also the phone's top bar).
  $effect(() => { void admin.reload; if (admin.on) void load(); });

  const sub = $derived(admin.sub);
  const SUB_TITLE: Record<AdminSub, string> = { "": "admin.title", people: "admin.people.title", settings: "admin.limits", versions: "admin.versions" };
  function go(s: AdminSub) {
    admin.sub = s;
    open = null;
    detail = null;
    document.getElementById("main-scroll")?.scrollTo({ top: 0 });
    document.querySelector("[data-part='page-scroll']")?.scrollTo({ top: 0 });
  }

  /** Error codes as words; unknown ones keep the server's text in the details. */
  function words(code: string): string {
    if (code === "owner_only") return t("admin.err.owner_only");
    if (code === "not_admin") return t("admin.err.not_admin");
    if (code.startsWith("bad_request: ")) return code.slice(13);
    return t("admin.err.generic");
  }
  async function act<T>(run: () => Promise<T>, ok?: string): Promise<T | null> {
    busy = true;
    try {
      const r = await run();
      if (ok) app.toast(ok);
      return r;
    } catch (e) {
      app.toast(words(String((e as Error).message ?? e)), "info");
      return null;
    } finally {
      busy = false;
    }
  }

  // ---- tiles
  const acc = $derived(stats?.accounts ?? null);
  const st = $derived(stats?.storage ?? null);
  const used = $derived((st?.audio_bytes ?? 0) + (st?.db_bytes ?? 0));
  const uptime = $derived.by(() => {
    const s = stats?.uptime_s;
    if (s == null) return null;
    const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600);
    return d > 0 ? tf("admin.uptime_d", { d, h }) : tf("admin.uptime_h", { h, m: Math.floor((s % 3600) / 60) });
  });

  // ---- ring charts
  const space = $derived<Slice[]>(st ? [
    { label: t("admin.space.songs"), value: st.audio_bytes ?? 0, color: SERIES[0] },
    { label: t("admin.space.sync"), value: st.sync_bytes ?? 0, color: SERIES[1] },
    { label: t("admin.space.db"), value: Math.max(0, (st.db_bytes ?? 0) - (st.sync_bytes ?? 0)), color: SERIES[2] },
  ] : []);
  const perPerson = $derived.by<Slice[]>(() => {
    const sorted = [...people].sort((a, b) => b.sync_bytes - a.sync_bytes);
    const top = sorted.slice(0, 5).filter((p) => p.sync_bytes > 0);
    const rest = sorted.slice(top.length).reduce((n, p) => n + p.sync_bytes, 0);
    const out = top.map((p, i) => ({ label: `@${p.handle}`, value: p.sync_bytes, color: SERIES[i] }));
    if (rest > 0) out.push({ label: tf("admin.space.others", { n: sorted.length - top.length }), value: rest, color: OTHER });
    return out;
  });
  const personTotal = $derived(people.reduce((n, p) => n + p.sync_bytes, 0));

  // ---- history (last 30 days, plus today live)
  const series = $derived.by(() => {
    const list = days.slice(-29);
    const today = new Date().toISOString().slice(0, 10);
    const live: DayStats | null = acc && st ? {
      day: today, accounts: acc.accounts, devices: acc.devices, active_1d: acc.active_1d,
      audio_bytes: st.audio_bytes ?? 0, db_bytes: st.db_bytes ?? 0, sync_bytes: st.sync_bytes ?? 0,
    } : null;
    const all = live && list.at(-1)?.day !== today ? [...list, live] : list;
    const pick = (d: DayStats) => (metric === "accounts" ? d.accounts : metric === "active" ? d.active_1d : d.audio_bytes + d.db_bytes);
    return { days: all.map((d) => d.day), values: all.map(pick) };
  });
  const metricOptions = $derived([
    { value: "accounts" as Metric, label: t("admin.metric.accounts") },
    { value: "active" as Metric, label: t("admin.metric.active") },
    { value: "space" as Metric, label: t("admin.metric.space") },
  ]);
  const fmtMetric = (v: number) => (metric === "space" ? fmtBytes(v) : String(v));

  // ---- people
  const PREVIEW = 5;
  const recent = $derived([...people].sort((a, b) => (b.last_seen ?? 0) - (a.last_seen ?? 0)).slice(0, PREVIEW));
  const shown = $derived.by(() => {
    const q = query.trim().replace(/^@/, "").toLowerCase();
    return q ? people.filter((p) => p.handle.toLowerCase().includes(q)) : people;
  });
  const capBytes = $derived((limits?.account_max_mb ?? 0) * 1e6);
  async function toggle(p: AdminAccountRow) {
    if (open === p.handle) { open = null; detail = null; return; }
    open = p.handle;
    detail = null;
    detail = await act(() => adminCall<AdminAccount>("GET", `accounts/${encodeURIComponent(p.handle)}`));
  }
  function updateRow(a: AdminAccount) {
    detail = a;
    people = people.map((p) => (p.id === a.id ? { ...p, banned: a.banned, devices: a.devices.length, device_limit: a.device_limit, device_limit_set: a.device_limit_set } : p));
  }
  function signOut(p: AdminAccountRow) {
    app.ask({
      title: tf("admin.signout.title", { handle: p.handle }), body: t("admin.signout.body"), ok: t("admin.signout.ok"), danger: true,
      run: async () => {
        const a = await act(() => adminCall<AdminAccount>("POST", `accounts/${encodeURIComponent(p.handle)}/revoke`, {}), t("admin.done"));
        if (a) updateRow(a);
      },
    });
  }
  function ban(p: AdminAccountRow) {
    app.ask({
      title: tf("admin.ban.title", { handle: p.handle }), body: t("admin.ban.body"), ok: t("admin.ban.ok"), danger: true,
      run: async () => {
        const r = await act(() => adminCall<{ bans: Ban[] }>("POST", "bans", { target: "account", value: p.handle }), t("admin.done"));
        if (r) { bans = r.bans; people = people.map((x) => (x.id === p.id ? { ...x, banned: true } : x)); if (detail?.id === p.id) detail = { ...detail, banned: true }; }
      },
    });
  }
  async function unban(target: string, id?: string) {
    const r = await act(() => adminCall<{ bans: Ban[] }>("POST", "bans/remove", { target }), t("admin.done"));
    if (r) {
      bans = r.bans;
      const gone = id ?? people.find((x) => x.handle === target)?.id;
      if (gone) { people = people.map((x) => (x.id === gone ? { ...x, banned: false } : x)); if (detail?.id === gone) detail = { ...detail, banned: false }; }
    }
  }
  async function setDevices(p: AdminAccountRow, n: number | null) {
    const a = await act(() => adminCall<AdminAccount>("PUT", `accounts/${encodeURIComponent(p.handle)}`, { device_limit: n }));
    if (a) updateRow(a);
  }

  // ---- settings (all on the sub page, the main ones on the overview)
  type Flag = "require_account" | "youtube_require_account" | "companion_enabled" | "mcp_relay";
  const TOGS: [Flag, string][] = [
    ["require_account", "admin.limit.require_account"],
    ["youtube_require_account", "admin.limit.youtube_account"],
    ["companion_enabled", "admin.limit.companion"],
    ["mcp_relay", "admin.limit.mcp"],
  ];
  const NUMS: [keyof Limits, string][] = [
    ["max_accounts", "admin.limit.max_accounts"],
    ["account_max_mb", "admin.limit.account_mb"],
    ["rate_limit_per_min", "admin.limit.rate"],
    ["youtube_per_day", "admin.limit.youtube_day"],
    ["youtube_daily_budget_units", "admin.limit.youtube_budget"],
    ["playlist_max_tracks", "admin.limit.playlist"],
    ["import_max_pages", "admin.limit.import"],
    ["audio_cache_max_gb", "admin.limit.cache_gb"],
    ["audio_cache_days", "admin.limit.cache_days"],
    ["d1_max_mb", "admin.limit.db_mb"],
  ];
  const MAIN_TOGS = new Set<Flag>(["require_account"]);
  const MAIN_NUMS = new Set<keyof Limits>(["max_accounts", "account_max_mb"]);
  const dirty = $derived(!!draft && !!limits && JSON.stringify(draft) !== JSON.stringify(limits));
  function num(key: keyof Limits, v: string) {
    if (!draft) return;
    const n = Number.parseInt(v.replace(/\D/g, ""), 10);
    (draft as unknown as Record<string, unknown>)[key] = Number.isFinite(n) ? n : 0;
  }
  async function saveLimits() {
    if (!draft) return;
    const l = await act(() => adminCall<Limits>("PUT", "limits", draft), t("admin.saved"));
    if (l) { limits = l; draft = { ...l }; }
  }

  // ---- versions: which app release each update channel offers
  type Channel = "stable_app_version" | "latest_app_version";
  async function pick(ch: Channel, version: string | null) {
    if (!limits) return;
    const l = await act(() => adminCall<Limits>("PUT", "limits", { ...limits, [ch]: version ?? "" }), t("admin.saved"));
    if (l) { limits = l; draft = { ...l, min_app_version: draft?.min_app_version ?? l.min_app_version }; }
  }
  const newestStable = $derived(releases.find((r) => !r.prerelease)?.version ?? null);
  const newest = $derived(releases[0]?.version ?? null);
  const CHANS: [Channel, string, string][] = [
    ["stable_app_version", "admin.versions.stable", "admin.versions.auto_stable"],
    ["latest_app_version", "admin.versions.latest", "admin.versions.auto_latest"],
  ];
  const day = (iso?: string | null) => (iso ? new Date(iso).toLocaleDateString() : "");

  // ---- cache
  const hitRate = $derived(stats?.cache && stats.cache.hits + stats.cache.misses > 0 ? stats.cache.hits / (stats.cache.hits + stats.cache.misses) : null);
  function clearCache() {
    app.ask({
      title: t("admin.cache.clear.title"), body: t("admin.cache.clear.body"), ok: t("admin.cache.clear"), danger: true,
      run: async () => { if (await act(() => adminCall("POST", "cache/clear", {}), t("admin.done"))) void load(); },
    });
  }

  const initial = (h: string) => h.slice(0, 1).toUpperCase();
</script>

{#snippet more(label: string, to: AdminSub)}
  <button class="more-link" onclick={() => go(to)}>{label}<Icon name="chevron-right" size={14} color="var(--dim)" /></button>
{/snippet}

{#snippet person(p: AdminAccountRow)}
  {@const share = capBytes > 0 ? Math.min(1, p.sync_bytes / capBytes) : 0}
  <button class="prow" class:open={open === p.handle} onclick={() => toggle(p)}>
    <span class="av" class:adm={p.admin}>{initial(p.handle)}</span>
    <span class="who">
      <span class="hd trunc">@{p.handle}</span>
      <span class="sub trunc">{p.last_seen ? whenMs(p.last_seen, t, tf) : t("admin.people.never")} · {tf("admin.people.devices", { n: p.devices, max: p.device_limit })}</span>
    </span>
    {#if p.admin}<span class="badge">{t("admin.badge.admin")}</span>{/if}
    {#if p.banned}<span class="badge bad">{t("admin.badge.banned")}</span>{/if}
    {#if !app.compact}<span class="track"><span class="fill" class:full={share >= 0.9} style:width="{share * 100}%"></span></span>{/if}
    <span class="sz mono">{fmtBytes(p.sync_bytes)}</span>
    <span class="chev"><Icon name="chevron-down" size={14} color="var(--faint)" /></span>
  </button>
  {#if open === p.handle}
    <div class="more">
      {#if !detail}
        <Skeleton w="100%" h={60} radius={8} id="admin-d" />
      {:else}
        {@const may = !p.admin || !!admin.me?.owner || p.handle === admin.me?.handle}
        {#each detail.devices as d (d.id)}
          <div class="dev"><Icon name="smartphone" size={14} color="var(--dim)" /><span class="dn trunc">{d.name}</span><span class="dm">{d.platform} · {whenMs(d.last_seen, t, tf)}</span></div>
        {:else}
          <div class="na">{t("admin.people.no_devices")}</div>
        {/each}
        {#if may}
          <div class="lim">
            <span class="ll">{t("admin.people.limit")}</span>
            <IconButton name="minus" label={t("admin.people.fewer")} disabled={busy || detail.device_limit <= 1} onclick={() => setDevices(p, detail!.device_limit - 1)} />
            <span class="lv mono">{detail.device_limit}</span>
            <IconButton name="plus" label={t("admin.people.more")} disabled={busy || detail.device_limit >= 100} onclick={() => setDevices(p, detail!.device_limit + 1)} />
            {#if detail.device_limit_set != null}<Button kind="text" onclick={() => setDevices(p, null)} disabled={busy}>{t("admin.people.default")}</Button>{/if}
          </div>
          <div class="acts">
            <Button icon="log-out" onclick={() => signOut(p)} disabled={busy || !detail.devices.length}>{t("admin.signout.ok")}</Button>
            {#if detail.banned}
              <Button icon="ban" onclick={() => unban(p.handle, p.id)} disabled={busy}>{t("admin.unban")}</Button>
            {:else}
              <Button kind="danger" icon="ban" onclick={() => ban(p)} disabled={busy}>{t("admin.ban.ok")}</Button>
            {/if}
          </div>
        {:else}
          <span class="note">{t("admin.people.other_admin")}</span>
        {/if}
      {/if}
    </div>
  {/if}
{/snippet}

{#snippet chans()}
  <div class="chans">
    {#each CHANS as [ch, label, autoLabel] (ch)}
      {@const set = limits?.[ch]}
      {@const auto = ch === "stable_app_version" ? newestStable : newest}
      <div class="card chan">
        <span class="ct">
          <span class="cl">{t(label)}</span>
          <span class="cv mono">{set ?? auto ?? "–"}</span>
          <span class="cs">{set ? t("admin.versions.picked") : t(autoLabel)}</span>
        </span>
        {#if set}<Button kind="text" onclick={() => pick(ch, null)} disabled={busy}>{t("admin.versions.unpin")}</Button>{/if}
      </div>
    {/each}
  </div>
{/snippet}

{#snippet bansCard(max: number)}
  <div class="card cbox">
    <div class="ch"><SectionTitle icon="ban" title={tf("admin.bans", { n: bans.length })} /></div>
    {#each bans.slice(0, max) as b (b.id)}
      <div class="ban">
        <span class="bl trunc">{b.label ?? `${b.kind} ${b.value.slice(0, 12)}`}</span>
        <span class="bm">{b.expires_at ? tf("admin.bans.until", { d: new Date(b.expires_at).toLocaleDateString() }) : t("admin.bans.forever")}</span>
        <Button kind="text" onclick={() => unban(b.id)} disabled={busy}>{t("admin.unban")}</Button>
      </div>
    {:else}
      <div class="na">{t("admin.bans.none")}</div>
    {/each}
  </div>
{/snippet}

{#snippet cacheCard()}
  <div class="card cbox">
    <div class="ch row">
      <SectionTitle icon="database" title={t("admin.cache")} />
      <Button kind="danger" icon="trash-2" onclick={clearCache} disabled={busy}>{t("admin.cache.clear")}</Button>
    </div>
    {#if stats?.cache}
      <div class="kv"><span>{t("admin.cache.entries")}</span><span class="mono">{stats.cache.entries}</span></div>
      <div class="kv"><span>{t("admin.cache.hits")}</span><span class="mono">{hitRate != null ? `${Math.round(hitRate * 100)} %` : "–"}</span></div>
    {/if}
    {#if st?.audio_files != null}<div class="kv"><span>{t("admin.cache.songs")}</span><span class="mono">{st.audio_files} · {fmtBytes(st.audio_bytes)}</span></div>{/if}
  </div>
{/snippet}

{#snippet settingsRows(all: boolean)}
  {#if draft}
    <div class="card list">
      {#each TOGS.filter(([k]) => all || MAIN_TOGS.has(k)) as [key, label] (key)}
        <div class="lrow">
          <span class="lt"><span class="lname">{t(label)}</span><span class="linfo">{t(`${label}.info`)}</span></span>
          <Tog on={draft[key]} onchange={(v) => (draft![key] = v)} label={t(label)} />
        </div>
      {/each}
      {#each NUMS.filter(([k]) => all || MAIN_NUMS.has(k)) as [key, label] (key)}
        <div class="lrow">
          <span class="lt"><span class="lname">{t(label)}</span><span class="linfo">{t(`${label}.info`)}</span></span>
          <Field value={String(draft[key] ?? "")} width={84} oninput={(v) => num(key, v)} label={t(label)} />
        </div>
      {/each}
      <div class="lrow">
        <span class="lt"><span class="lname">{t("admin.limit.min_version")}</span><span class="linfo">{t("admin.limit.min_version.info")}</span></span>
        <Field value={draft.min_app_version ?? ""} width={84} placeholder="–" oninput={(v) => (draft!.min_app_version = v.trim() || null)} label={t("admin.limit.min_version")} />
      </div>
      {#if dirty}
        <div class="lfoot">
          <Button kind="text" disabled={busy} onclick={() => (draft = limits ? { ...limits } : null)}>{t("admin.reset")}</Button>
          <Button kind="primary" disabled={busy} onclick={saveLimits}>{t("admin.save")}</Button>
        </div>
      {/if}
    </div>
  {/if}
{/snippet}

<div class="pg">
  {#if sub}
    {#if !app.compact}<div class="bp"><Button kind="back" icon="arrow-left" onclick={() => go("")}>{t("admin.title")}</Button></div>{/if}
  {:else}
    <BackPill />
  {/if}

  <div class="head">
    {#if !app.compact}
      <SectionTitle heading title={t(SUB_TITLE[sub])} />
      <div class="hr">
        {#if admin.me}<span class="role">{admin.me.owner ? t("admin.role.owner") : tf("admin.role.admin", { handle: admin.me.handle ?? "" })}</span>{/if}
        <IconButton name="refresh-cw" label={t("admin.refresh")} onclick={() => admin.reload++} disabled={loading} />
      </div>
    {/if}
  </div>

  {#if !admin.on && admin.pending}
    <div class="tiles">{#each [0, 1, 2, 3] as i (i)}<div class="sk"><Skeleton w="100%" h={92} radius={12} id="admin-{i}" /></div>{/each}</div>
  {:else if !admin.on}
    <EmptyState icon="server" title={t("admin.err.not_admin")} sub={t("admin.err.not_admin.sub")} />
  {:else if error && !stats}
    <ErrorCard title={t("admin.err.title")} body={words(error)} detail={error} onretry={() => load()} />
  {:else if loading && !stats}
    <div class="tiles">{#each [0, 1, 2, 3] as i (i)}<div class="sk"><Skeleton w="100%" h={92} radius={12} id="admin-{i}" /></div>{/each}</div>
    <div class="two">{#each [0, 1] as i (i)}<div class="sk"><Skeleton w="100%" h={200} radius={12} id="admin-r{i}" /></div>{/each}</div>
  {:else if stats && sub === "people"}
    <!-- ---- all people -->
    <Field bind:value={query} placeholder={t("admin.people.search")} icon="search" width="100%" mono={false} />
    <div class="card list">
      {#each shown as p (p.id)}
        {@render person(p)}
      {:else}
        <div class="na pad">{query ? t("admin.people.no_match") : t("admin.people.none")}</div>
      {/each}
    </div>
    {@render bansCard(100)}
  {:else if stats && sub === "settings"}
    <!-- ---- all settings -->
    {@render settingsRows(true)}
    {@render cacheCard()}
  {:else if stats && sub === "versions"}
    <!-- ---- version manager -->
    {@render chans()}
    <div class="card list">
      {#each releases as r (r.version)}
        {@const isStable = (limits?.stable_app_version ?? newestStable) === r.version}
        {@const isLatest = (limits?.latest_app_version ?? newest) === r.version}
        <div class="rel">
          <span class="rinfo">
            <span class="rv mono">{r.version}</span>
            <span class="rmeta">
              <span class="rd">{day(r.published_at)}</span>
              {#if r.prerelease}<span class="badge sm">{t("admin.versions.pre")}</span>{/if}
              {#if limits?.min_app_version === r.version}<span class="badge sm bad">{t("admin.versions.min")}</span>{/if}
            </span>
          </span>
          <span class="pills">
            <button class="pill" class:on={isStable} disabled={busy} aria-pressed={isStable} onclick={() => pick("stable_app_version", isStable && limits?.stable_app_version ? null : r.version)}>
              <Icon name="badge-check" size={13} color={isStable ? "#000" : "var(--dim)"} />{t("admin.versions.stable")}
            </button>
            <button class="pill" class:on={isLatest} disabled={busy} aria-pressed={isLatest} onclick={() => pick("latest_app_version", isLatest && limits?.latest_app_version ? null : r.version)}>
              <Icon name="sparkles" size={13} color={isLatest ? "#000" : "var(--dim)"} />{t("admin.versions.latest")}
            </button>
          </span>
        </div>
      {:else}
        <div class="na pad">{t("admin.versions.none")}</div>
      {/each}
    </div>
    <div class="note">{t("admin.versions.note")}</div>
  {:else if stats}
    <!-- ---- overview -->
    <div class="tiles">
      <Tile label={t("admin.tile.accounts")} value={String(acc?.accounts ?? total)} sub={stats.max_accounts ? tf("admin.tile.of", { n: stats.max_accounts }) : t("admin.tile.no_cap")} />
      <Tile label={t("admin.tile.active")} value={String(acc?.active_1d ?? 0)} sub={tf("admin.tile.active_sub", { w: acc?.active_7d ?? 0, m: acc?.active_30d ?? 0 })} delay={40} />
      <Tile label={t("admin.tile.devices")} value={String(acc?.devices ?? 0)} sub={tf("admin.tile.bans", { n: stats.bans })} delay={80} />
      <Tile label={t("admin.tile.space")} value={st ? fmtBytes(used) : "–"} sub={uptime ?? (st?.audio_files != null ? tf("admin.tile.files", { n: st.audio_files }) : null)} delay={120} />
    </div>

    <div class="two">
      <div class="card cbox">
        <div class="ch"><SectionTitle icon="hard-drive" title={t("admin.space.title")} /></div>
        {#if st}
          <Donut slices={space} total={fmtBytes(used)} />
          <div class="caps">
            {#if st.audio_cap_bytes}<span>{tf("admin.space.songs_cap", { used: fmtBytes(st.audio_bytes), cap: fmtBytes(st.audio_cap_bytes) })}</span>{/if}
            {#if st.db_cap_bytes}<span>{tf("admin.space.db_cap", { used: fmtBytes(st.db_bytes), cap: fmtBytes(st.db_cap_bytes) })}</span>{/if}
          </div>
        {:else}
          <div class="na">{t("admin.space.unknown")}</div>
        {/if}
      </div>
      <div class="card cbox">
        <div class="ch"><SectionTitle icon="users" title={t("admin.people_space")} /></div>
        {#if perPerson.length}
          <Donut slices={perPerson} total={fmtBytes(personTotal)} />
        {:else}
          <div class="na">{t("admin.people_space.none")}</div>
        {/if}
      </div>
    </div>

    <div class="card cbox">
      <div class="ch row">
        <SectionTitle icon="chart-column" title={t("admin.history")} />
        <Segmented value={metric} options={metricOptions} onpick={(v) => (metric = v)} fill={app.compact} />
      </div>
      {#if series.values.length > 1}
        <Trend days={series.days} values={series.values} fmt={fmtMetric} />
      {:else}
        <div class="na">{t("admin.history.soon")}</div>
      {/if}
    </div>

    <section class="sec">
      <div class="ch row">
        <SectionTitle icon="user-round" title={tf("admin.people", { n: total })} />
        {@render more(t("admin.all"), "people")}
      </div>
      <div class="card list">
        {#each recent as p (p.id)}
          {@render person(p)}
        {:else}
          <div class="na pad">{t("admin.people.none")}</div>
        {/each}
        {#if total > PREVIEW}
          <button class="all-row" onclick={() => go("people")}>{tf("admin.people.all", { n: total })}<Icon name="chevron-right" size={14} color="var(--dim)" /></button>
        {/if}
      </div>
    </section>

    <section class="sec">
      <div class="ch row">
        <SectionTitle icon="sliders-horizontal" title={t("admin.limits")} />
        {@render more(t("admin.all"), "settings")}
      </div>
      {@render settingsRows(false)}
    </section>

    <section class="sec">
      <div class="ch row">
        <SectionTitle icon="github" title={t("admin.versions")} />
        {@render more(t("admin.versions.manage"), "versions")}
      </div>
      {@render chans()}
    </section>

    <div class="two">
      {@render cacheCard()}
      {@render bansCard(3)}
    </div>

    <div class="note">{admin.me?.owner ? t("admin.note.owner") : t("admin.note.admin")}</div>
  {/if}
</div>

<style>
  .pg { display: flex; flex-direction: column; gap: 24px; }
  .bp { flex: none; display: flex; align-self: flex-start; }
  .head { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 12px; }
  .hr { display: flex; align-items: center; gap: 8px; margin-left: auto; }
  .role { height: 26px; padding: 0 10px; border-radius: 13px; background: rgba(255, 255, 255, .06); display: inline-flex; align-items: center; font-size: var(--xs); color: var(--dim); white-space: nowrap; }
  .tiles { display: flex; flex-wrap: wrap; gap: 10px; }
  .sk { flex: 1; min-width: 150px; }
  :global(html.compact) .tiles :global(.tile) { min-width: calc(50% - 5px); }
  .two { display: flex; flex-wrap: wrap; gap: 12px; }
  .two > :global(*) { flex: 1; min-width: 320px; }
  :global(html.compact) .two > :global(*) { min-width: 0; flex-basis: 100%; }
  .card { background: var(--card); border-radius: 12px; }
  .cbox { padding: 16px 18px; display: flex; flex-direction: column; gap: 14px; animation: rise-in .3s var(--ease) both; }
  .ch :global(.sec) { font-size: var(--md); }
  .ch.row { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
  :global(html.compact) .cbox .ch.row :global(.seg) { width: 100%; }
  .sec { display: flex; flex-direction: column; gap: 12px; }
  .more-link { display: inline-flex; align-items: center; gap: 4px; height: 28px; padding: 0 6px 0 10px; border-radius: 8px; font-size: var(--sm); color: var(--dim); transition: background-color .12s ease-out, color .12s; }
  .more-link:hover { background: rgba(255, 255, 255, .06); color: var(--text); }
  .all-row { display: flex; align-items: center; justify-content: center; gap: 6px; height: 40px; border-radius: 8px; font-size: var(--sm); color: var(--dim); transition: background-color .12s ease-out; }
  .all-row:hover { background: rgba(255, 255, 255, .05); color: var(--text); }
  .caps { display: flex; flex-wrap: wrap; gap: 6px 16px; font-size: var(--xs); color: var(--faint); }
  .na { font-size: var(--sm); color: var(--faint); }
  .na.pad { padding: 14px 10px; }
  .list { display: flex; flex-direction: column; gap: 2px; padding: 8px; }
  .prow { display: flex; align-items: center; gap: 12px; min-height: 52px; padding: 0 10px; border-radius: 8px; text-align: left; transition: background-color .12s ease-out; }
  .prow:hover, .prow.open { background: rgba(255, 255, 255, .05); }
  .av { flex: none; width: 32px; height: 32px; border-radius: 50%; background: rgba(255, 255, 255, .08); display: grid; place-items: center; font-size: var(--sm); font-weight: 600; color: var(--dim); }
  .av.adm { background: var(--accent); color: #000; }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .hd { font-size: var(--md); font-weight: 500; color: var(--text); }
  .sub { font-size: var(--xs); color: var(--faint); }
  .badge { flex: none; height: 20px; padding: 0 8px; border-radius: 10px; background: rgba(255, 255, 255, .08); font-size: 11px; font-weight: 600; color: var(--dim); display: inline-flex; align-items: center; white-space: nowrap; }
  .badge.sm { height: 18px; padding: 0 7px; font-size: 10.5px; }
  .badge.bad { color: var(--danger); }
  .track { flex: none; width: 160px; height: 4px; border-radius: 2px; background: rgba(255, 255, 255, .07); overflow: hidden; }
  .fill { display: block; height: 100%; border-radius: 2px; background: var(--accent); transform-origin: left; animation: fillin .35s var(--ease) both; }
  .fill.full { background: var(--danger); }
  @keyframes fillin { from { transform: scaleX(0); } to { transform: scaleX(1); } }
  .chev { flex: none; display: grid; transition: transform .2s var(--ease); }
  .prow.open .chev { transform: rotate(180deg); }
  .sz { flex: none; width: 64px; text-align: right; font-size: var(--sm); color: var(--dim); }
  .more { margin: 2px 0 8px 54px; padding: 10px 12px; border-radius: 8px; background: rgba(255, 255, 255, .03); display: flex; flex-direction: column; gap: 8px; }
  :global(html.compact) .more { margin-left: 0; }
  .dev { display: flex; align-items: center; gap: 8px; font-size: var(--sm); min-width: 0; }
  .dn { color: var(--text); min-width: 0; }
  .dm { margin-left: auto; flex: none; font-size: var(--xs); color: var(--faint); }
  .lim { display: flex; align-items: center; gap: 4px; }
  .ll { font-size: var(--sm); color: var(--dim); margin-right: 4px; }
  .lv { width: 24px; text-align: center; font-size: var(--sm); }
  .acts { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
  .lrow { display: flex; align-items: center; gap: 16px; min-height: 56px; padding: 6px 10px; }
  .lt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .lname { font-size: var(--md); color: var(--text); }
  .linfo { font-size: var(--xs); color: var(--faint); }
  .lfoot { display: flex; justify-content: flex-end; gap: 8px; padding: 8px 10px 4px; }
  .kv { display: flex; justify-content: space-between; font-size: var(--sm); color: var(--dim); }
  .kv .mono { color: var(--text); }
  .ban { display: flex; align-items: center; gap: 10px; font-size: var(--sm); }
  .bl { flex: 1; min-width: 0; color: var(--text); }
  .bm { flex: none; font-size: var(--xs); color: var(--faint); }
  .chans { display: flex; flex-wrap: wrap; gap: 10px; }
  .chan { flex: 1; min-width: 220px; padding: 14px 18px; display: flex; align-items: center; gap: 8px; animation: rise-in .3s var(--ease) both; }
  :global(html.compact) .chan { min-width: calc(50% - 5px); padding: 12px 14px; flex-direction: column; align-items: flex-start; }
  :global(html.compact) .chan :global(.btn) { height: 26px; padding: 0; }
  .ct { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .cl { font-size: var(--sm); font-weight: 500; color: var(--dim); }
  .cv { font-size: 22px; font-weight: 700; color: var(--text); }
  .cs { font-size: var(--xs); color: var(--faint); }
  .rel { display: flex; align-items: center; gap: 12px; min-height: 52px; padding: 6px 10px; border-radius: 8px; }
  .rel:hover { background: rgba(255, 255, 255, .03); }
  .rinfo { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .rv { font-size: var(--md); font-weight: 600; color: var(--text); }
  .rmeta { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .rd { font-size: var(--xs); color: var(--faint); }
  .pills { flex: none; display: flex; gap: 6px; }
  .pill { height: 28px; padding: 0 11px; border-radius: 14px; display: inline-flex; align-items: center; gap: 5px; background: rgba(255, 255, 255, .06); font-size: var(--xs); font-weight: 600; color: var(--dim); white-space: nowrap; transition: background-color .12s ease-out, transform .15s var(--ease); }
  .pill:hover { background: rgba(255, 255, 255, .1); }
  .pill.on { background: var(--text); color: #000; }
  .pill.on:active { transform: scale(.94); }
  .pill:disabled { opacity: .5; }
  .note { font-size: var(--xs); color: var(--faint); }
</style>
