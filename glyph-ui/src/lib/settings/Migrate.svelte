<script lang="ts">
  // ui/migrate.rs page: Settings > Transfer music. Moves a whole library from
  // another service (liked songs, playlists, saved albums, followed artists)
  // in one go, or imports a single link / file into a chosen playlist.
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import RowBase from "./RowBase.svelte";
  import SectionLabel from "./SectionLabel.svelte";
  import Surface from "./Surface.svelte";
  import Field from "./Field.svelte";
  import Select from "./Select.svelte";
  import Check from "./Check.svelte";
  import Spin from "./Spin.svelte";
  import { openUrl, readClipboard } from "./util";
  import * as M from "./migrate-state.svelte";
  import { mg, SERVICES, serviceOf } from "./migrate-state.svelte";
  import type { ListKey } from "../api/types";
  import type { MigrateCollection, MigrateTotals } from "./types";

  let {}: { sub?: number } = $props();
  onMount(() => { M.loadResume(); M.syncRunning(); });

  const ICON = "rgba(255,255,255,.6)";
  const svc = $derived(mg.service ? serviceOf(mg.service) : null);
  const picked = $derived(Object.keys(mg.picked).length);
  const ready = $derived(!!svc && (svc.connect === "paste" || svc.connect === "takeout" ? mg.pasted != null || mg.files.length > 0 : true));
  const mainCols = $derived(mg.overview?.collections.filter((c) => c.kind !== "playlist") ?? []);
  const listCols = $derived(mg.overview?.collections.filter((c) => c.kind === "playlist") ?? []);

  // Single-link import state (when entering via "By link")
  let linkUrl = $state("");
  let linkName = $state("");
  let linkTarget = $state("new");
  let linkFound = $state<string | null>(null);
  let linkPasted = $state<[number, number]>([0, 0]);
  let linkBusy = $state(false);
  let linkProg = $state({ done: 0, total: 0 });
  let linkSeq = 0;

  $effect(() => backend().on("import_progress", (p) => (linkProg = p)));
  $effect(() => {
    const v = linkUrl.trim();
    linkFound = null;
    linkPasted = [0, 0];
    if (!v) return;
    const n = ++linkSeq;
    backend().call<string | null>("import_parse_url", { url: v }).then((k) => { if (n === linkSeq) linkFound = k ?? null; });
    backend().call<[number, number]>("migrate_pasted", { text: v }).then((c) => { if (n === linkSeq) linkPasted = c ?? [0, 0]; });
  });

  const userPlaylistOptions = $derived(
    app.library.playlists.map((p) => ({
      value: `pl:${p.id}`,
      label: p.name,
      sub: tf("common.n_songs", { n: p.tracks.length }),
      list: { kind: "playlist", id: p.id } as ListKey,
    })),
  );
  function playlistMenuItems(newLabel: string) {
    return [
      { value: "new", label: newLabel, icon: "plus" },
      ...(userPlaylistOptions.length ? [{ sep: true as const }, ...userPlaylistOptions] : []),
    ];
  }
  const linkDestOptions = $derived(playlistMenuItems(t("migrate.target.new")));

  function targetOptionsFor(c: MigrateCollection) {
    const newLabel = c.kind === "playlist" && c.name.trim() ? tf("migrate.target.new_named", { name: c.name.trim() }) : t("migrate.target.new");
    return [
      { value: "liked", label: t("migrate.liked"), icon: "heart" },
      { label: t("list.kind.playlist"), icon: "list-music", children: playlistMenuItems(newLabel) },
    ];
  }

  function displayName(c: MigrateCollection): string {
    if (c.kind === "liked" && svc?.connect === "paste" && !c.name.trim()) return t("migrate.copied_songs");
    return M.collectionName(c);
  }

  function summaryParts(x: MigrateTotals): string[] {
    const out: string[] = [];
    if (x.liked > 0) out.push(`${x.liked} ${t("migrate.liked")}`);
    if (x.playlists > 0) out.push(`${x.playlists} ${t("migrate.playlists.done")}`);
    if (x.albums > 0) out.push(`${x.albums} ${t("migrate.albums")}`);
    if (x.artists > 0) out.push(`${x.artists} ${t("migrate.artists")}`);
    return out;
  }

  async function pasteIntoLink() {
    linkUrl = await readClipboard();
  }

  async function importSingleLink() {
    const v = linkUrl.trim();
    if (!v || linkBusy) return;
    mg.error = null;
    linkBusy = true;
    const isNew = linkTarget === "new";
    const r = await backend().call<{ error?: string | null; playlist?: string } | null>("import_link", {
      url: v,
      name: isNew ? linkName.trim() || null : null,
      playlist_id: !isNew && linkTarget.startsWith("pl:") ? linkTarget.slice(3) : null,
    }).catch((e) => ({ error: String(e) }));
    linkBusy = false;
    if (r?.error) { mg.error = r.error; return; }
    linkUrl = "";
    linkName = "";
  }

  function toLibrary() { app.settingsOpen = false; app.navigate("library"); }
  function playlistFile() { app.settingsOpen = false; backend().call("import_playlist_file"); }
</script>

<div class="col">
  {#if mg.step === "pick"}
    {#if mg.resume}
      <SectionLabel text={t("migrate.resume.section")} />
      <Rows>
        <Row label={tf("migrate.resume", { service: serviceOf(mg.resume.service).name })} info={tf("migrate.resume.desc", { done: mg.resume.done, all: mg.resume.all, account: mg.resume.account })}>
          <Button onclick={M.discardResume}>{t("migrate.resume.discard")}</Button>
          <Button kind="primary" onclick={M.resumeRun}>{t("migrate.resume.button")}</Button>
        </Row>
      </Rows>
    {/if}
    <SectionLabel text={t("migrate.from")} />
    <Rows>
      {#each SERVICES as s (s.id)}
        <Row label={s.name} info={t(`migrate.service.${s.id}.desc`)}>
          <Button onclick={() => M.pick(s.id)}>{t("migrate.connect")}<Icon name="arrow-right" size={14} color={ICON} /></Button>
        </Row>
      {/each}
    </Rows>
    <SectionLabel text={t("migrate.single")} />
    <Rows>
      <Row label={t("migrate.link")} info={t("migrate.link.desc")}>
        <Button onclick={M.openLink}>{t("migrate.link.button")}<Icon name="link-2" size={14} color={ICON} /></Button>
      </Row>
      <Row label={t("migrate.file")} info={t("migrate.file.desc")}>
        <Button onclick={playlistFile}>{t("migrate.file.button")}<Icon name="folder-open" size={14} color={ICON} /></Button>
      </Row>
    </Rows>
    <div class="note">{t("migrate.note")}</div>

  {:else if mg.step === "connect" && svc}
    <div class="top">
      <Button icon="arrow-left" onclick={M.back}>{t("migrate.back")}</Button>
    </div>
    <Rows>
      {#if svc.connect === "paste" || svc.connect === "takeout"}
        <div class="srow">
          <RowBase label={t("migrate.clipboard")} status={mg.pasted != null ? tf("migrate.pasted", { songs: mg.pastedCounts[0], lists: mg.pastedCounts[1] }) : t("migrate.clipboard.desc")}>
            {#if mg.pasted != null}<Button onclick={M.clearPasted}>{t("migrate.clear")}</Button>{/if}
            <Button onclick={M.pasteClipboard}>{t("migrate.clipboard.button")}<Icon name="copy" size={14} color={ICON} /></Button>
          </RowBase>
        </div>
        {#if svc.connect === "takeout"}
          <div class="srow">
            <RowBase label={t("migrate.takeout")} status={mg.files.length ? tf("migrate.files.picked", { n: mg.files.length }) : t("migrate.takeout.desc")}>
              {#if svc.exportUrl}<Button onclick={() => openUrl(svc.exportUrl!)}>{t("migrate.takeout.open")}</Button>{/if}
              <Button onclick={M.pickPlaylistFiles}>{t("migrate.files.button")}<Icon name="folder-open" size={14} color={ICON} /></Button>
            </RowBase>
          </div>
        {/if}
      {:else}
        <Row label={t(`migrate.${svc.id}.profile`)} info={t(`migrate.${svc.id}.profile.desc`)}>
          <Field bind:value={mg.profile} placeholder={t("migrate.profile.placeholder")} onenter={M.connect} label={t(`migrate.${svc.id}.profile`)} />
        </Row>
      {/if}
    </Rows>
    <div class="foot end">
      <Button kind="primary" disabled={!ready} onclick={M.connect}>{t("migrate.next")}</Button>
    </div>

  {:else if mg.step === "link"}
    <div class="top">
      <Button icon="arrow-left" onclick={M.back}>{t("migrate.back")}</Button>
    </div>
    <Rows>
      <div class="srow lf">
        <div class="lrow">
          <input class="lin" bind:value={linkUrl} onkeydown={(e) => e.key === "Enter" && importSingleLink()} placeholder={t("dialog.playlist.link")} spellcheck="false" />
          <Button onclick={pasteIntoLink}>{t("migrate.clipboard.button")}<Icon name="copy" size={14} color={ICON} /></Button>
        </div>
        {#if linkBusy}
          <div class="lst"><Spin label={linkProg.total > 0 ? `${t("dialog.playlist.reading")} ${tf("import.progress", { done: linkProg.done, total: linkProg.total })}` : t("dialog.playlist.reading")} /></div>
        {:else if linkFound}
          <div class="lst ok"><Icon name="check" size={13} color={ICON} /><span>{tf("dialog.playlist.link.found", { kind: linkFound })}</span></div>
        {:else if linkPasted[0] + linkPasted[1] > 0}
          <div class="lst ok"><Icon name="check" size={13} color={ICON} /><span>{tf("migrate.pasted", { songs: linkPasted[0], lists: linkPasted[1] })}</span></div>
        {:else if linkUrl.trim()}
          <div class="lst">{t("import.unknown_link")}</div>
        {/if}
      </div>
      <Row label={t("migrate.target")}>
        <Select value={linkTarget} options={linkDestOptions} onpick={(v) => (linkTarget = v)} />
      </Row>
      {#if linkTarget === "new"}
        <Row label={t("dialog.playlist.placeholder")}>
          <Field bind:value={linkName} placeholder={t("migrate.target.name_optional")} mono={false} onenter={importSingleLink} label={t("dialog.playlist.placeholder")} />
        </Row>
      {/if}
    </Rows>
    <div class="foot end">
      <Button kind="primary" disabled={linkBusy || !linkUrl.trim()} onclick={importSingleLink}>{linkBusy ? t("dialog.importing") : t("dialog.import")}</Button>
    </div>

  {:else if mg.step === "loading" && svc}
    <div class="ld">
      <div class="top">
        <Button icon="arrow-left" onclick={M.back}>{t("migrate.back")}</Button>
      </div>
      <Spin label={t("migrate.loading")} />
      <div class="sk">{#each [0.42, 0.55, 0.36, 0.48, 0.6] as w, i}<div class="skr"><Skeleton w={44} h={44} radius={6} id={`mg-c${i}`} /><div class="skt"><Skeleton w="{w * 100}%" h={12} radius={4} id={`mg-t${i}`} /><Skeleton w="25%" h={10} radius={4} id={`mg-a${i}`} /></div></div>{/each}</div>
    </div>

  {:else if mg.step === "choose" && mg.overview && svc}
    <div class="top">
      <Button icon="arrow-left" onclick={M.back}>{t("migrate.back")}</Button>
    </div>
    <div class="cl">
      {#each mainCols as c (c.id)}{@render coll(c)}{/each}
    </div>
    {#if listCols.length}
      <div class="plh">
        <SectionLabel text={tf("migrate.playlists", { n: listCols.length })} />
        <span class="fl"></span>
        <div class="pla"><Button kind="text" onclick={() => M.allPlaylists(true)}>{t("migrate.all")}</Button><Button kind="text" onclick={() => M.allPlaylists(false)}>{t("migrate.none")}</Button></div>
      </div>
      <div class="cl">{#each listCols as c (c.id)}{@render coll(c)}{/each}</div>
    {/if}
    <div class="foot end big">
      <Button kind="primary" disabled={picked === 0} onclick={M.start}>{tf("migrate.start", { n: picked })}</Button>
    </div>

  {:else if mg.step === "running" && svc}
    {@const r = mg.progress}
    {@const pct = r && r.total > 0 ? Math.min(100, Math.round((r.done * 100) / r.total)) : 0}
    {@const stats = r ? summaryParts(r.totals) : []}
    <div class="run">
      <SectionLabel text={tf("migrate.running", { service: svc.name })} />
      <Surface gap={10}>
        <div class="rl">
          <Spin />
          {#if r}
            <span class="rt">{r.lists > 1 ? tf("migrate.list", { i: r.index + 1, n: r.lists, name: r.current }) : r.current}</span>
          {:else}
            <span class="rt">{t("migrate.reading")}</span>
          {/if}
        </div>
        <div class="pbar"><div class="pfill" class:ind={!r || r.total === 0} style:width={r && r.total > 0 ? `${pct}%` : undefined}></div></div>
        <div class="rsub">
          <span class="mono rb">{r && r.total > 0 ? tf("migrate.songs_progress", { done: r.done, total: r.total, pct }) : t("migrate.reading")}</span>
          {#if stats.length}<span class="mono rs">{stats.join(" · ")}</span>{/if}
        </div>
      </Surface>
      <div class="stop"><Button onclick={() => !r?.stopping && M.cancel()} disabled={!!r?.stopping}>{r?.stopping ? t("migrate.stopping") : t("migrate.stop")}</Button></div>
      <div class="note">{t("migrate.running.note")}</div>
    </div>

  {:else if mg.step === "done" && mg.finished && svc}
    {@const f = mg.finished}
    {@const missing = Math.max(0, f.totals.songs - f.totals.matched)}
    {@const matchPct = f.totals.songs === 0 ? 100 : Math.floor((f.totals.matched * 100) / f.totals.songs)}
    <SectionLabel text={f.cancelled ? tf("migrate.stopped.title", { service: svc.name }) : tf("migrate.done", { service: svc.name })} />
    <Rows>
      {#if f.totals.songs > 0}
        <Row label={t("migrate.matched")}><span class="mono v">{f.totals.matched}/{f.totals.songs} ({matchPct}%)</span></Row>
      {/if}
      {#if f.totals.liked > 0}<Row label={t("migrate.liked")}><span class="mono v">+{f.totals.liked}</span></Row>{/if}
      {#if f.totals.playlists > 0}<Row label={t("migrate.playlists.done")}><span class="mono v">+{f.totals.playlists}</span></Row>{/if}
      {#if f.totals.albums > 0}<Row label={t("migrate.albums")}><span class="mono v">+{f.totals.albums}</span></Row>{/if}
      {#if f.totals.artists > 0}<Row label={t("migrate.artists")}><span class="mono v">+{f.totals.artists}</span></Row>{/if}
      {#if f.totals.unsure > 0}<Row label={t("migrate.unsure")} info={t("migrate.unsure.desc")}><span class="mono v">{f.totals.unsure}</span></Row>{/if}
      {#if missing > 0}<Row label={t("migrate.missing")} info={t("migrate.missing.desc")}><span class="mono v">{missing}</span></Row>{/if}
    </Rows>
    {#if f.failed.length}
      <SectionLabel text={t("migrate.failed")} />
      <Rows>{#each f.failed as [name, err] (name)}<Row label={name}><span class="fe">{err}</span></Row>{/each}</Rows>
    {/if}
    {#each f.notes as k (k)}<div class="note">{t(`migrate.note.${k}`)}</div>{/each}
    <div class="foot big">
      <Button icon="arrow-left" onclick={M.back}>{t("migrate.another")}</Button>
      <span class="fl"></span>
      <Button kind="primary" onclick={toLibrary}>{t("migrate.to_library")}</Button>
    </div>
  {/if}

  {#if mg.error}
    <div class="err"><span class="ei"><Icon name="info" size={14} color={ICON} /></span><div>{mg.error}</div></div>
  {/if}
</div>

{#snippet coll(c: MigrateCollection)}
  {@const on = !!mg.picked[c.id]}
  {@const hasDest = c.kind === "liked" || c.kind === "playlist"}
  {@const dest = mg.targets[c.id] ?? (c.kind === "liked" ? "liked" : "new")}
  <div class="cwrap">
    <div class="cr" class:off={!on}>
      <button class="ctog" onclick={() => M.toggle(c.id)}>
        <Check {on} />
        <span class="cn" class:off={!on}>{displayName(c)}</span>
        <span class="mono cc">{c.count ?? "?"}</span>
        {#if c.followed}<span class="cx">{t("migrate.followed")}</span>{/if}
        {#if mg.overview?.done_before.includes(c.id)}<span class="cx">{t("migrate.done_before")}</span>{/if}
      </button>
      {#if hasDest}
        <div class="dest" class:dim={!on}>
          <Select value={dest} options={targetOptionsFor(c)} onpick={(v) => M.setTarget(c.id, v)} />
        </div>
      {/if}
    </div>
    {#if on && hasDest && dest === "new" && c.kind === "liked"}
      <div class="cname">
        <input class="lin sm" value={mg.targetNames[c.id] ?? ""} oninput={(e) => M.setTargetName(c.id, e.currentTarget.value)} placeholder={t("migrate.target.name_optional")} spellcheck="false" />
      </div>
    {/if}
  </div>
{/snippet}

<style>
  .col { display: flex; flex-direction: column; }
  .top { display: flex; align-items: center; gap: 12px; padding-bottom: 12px; }
  .note { padding-top: 14px; font-size: 12px; color: rgba(255, 255, 255, .4); }
  .fl { flex: 1; }
  .foot { padding-top: 16px; display: flex; align-items: center; gap: 8px; }
  .foot.end { justify-content: flex-end; }
  .foot.big { padding-top: 18px; }
  .ld { display: flex; flex-direction: column; gap: 12px; }
  .sk { display: flex; flex-direction: column; gap: 2px; }
  .skr { display: flex; align-items: center; gap: 12px; padding: 6px 8px; }
  .skt { flex: 1; display: flex; flex-direction: column; gap: 8px; }
  .cl { padding-top: 4px; display: flex; flex-direction: column; gap: 2px; }
  .cwrap { display: flex; flex-direction: column; gap: 6px; }
  .cr { min-height: 44px; padding: 4px 6px 4px 10px; display: flex; align-items: center; gap: 10px; border-radius: 8px; transition: background-color .12s; }
  .cr:hover { background: rgba(255, 255, 255, .05); }
  .ctog { flex: 1; min-width: 0; height: 34px; display: flex; align-items: center; gap: 10px; text-align: left; }
  .dest { flex: none; transition: opacity .15s; }
  .dest.dim { opacity: .45; }
  .cname { padding: 0 6px 6px 36px; }
  .cn { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13.5px; font-weight: 500; color: #fff; transition: color .15s; }
  .cn.off { color: rgba(255, 255, 255, .55); }
  .cc { flex: none; padding: 0 6px; border-radius: 4px; background: rgba(255, 255, 255, .08); font-size: 11px; color: rgba(255, 255, 255, .6); }
  .cx { flex: none; font-size: 12px; color: rgba(255, 255, 255, .45); }
  .plh { display: flex; align-items: flex-end; padding-top: 8px; }
  .pla { display: flex; gap: 2px; font-size: var(--sm); }
  .lf { padding: 14px 0; display: flex; flex-direction: column; gap: 10px; }
  .lrow { display: flex; align-items: center; gap: 8px; }
  .lin { flex: 1; min-width: 0; height: 34px; padding: 0 10px; border: 0; outline: none; border-radius: 8px; background: rgba(255, 255, 255, .06); font-size: 13px; color: #fff; transition: background-color .15s ease-out; }
  .lin:focus { background: rgba(255, 255, 255, .1); }
  .lin.sm { width: 100%; height: 32px; font-size: 13px; }
  .lin::placeholder { color: rgba(255, 255, 255, .3); }
  .lst { display: flex; align-items: center; gap: 6px; font-size: 12.5px; color: rgba(255, 255, 255, .5); }
  .lst.ok { color: rgba(255, 255, 255, .75); }
  .run { display: flex; flex-direction: column; gap: 12px; }
  .rl { display: flex; align-items: center; gap: 8px; font-size: 14px; font-weight: 500; }
  .rt { color: #fff; }
  .pbar { height: 6px; width: 100%; border-radius: 999px; background: rgba(255, 255, 255, .1); overflow: hidden; }
  .pfill { height: 100%; border-radius: 999px; background: var(--accent); transition: width .22s ease-out; }
  .pfill.ind { width: 35%; animation: slide 1.3s ease-in-out infinite; }
  @keyframes slide { 0% { transform: translateX(-110%); } 100% { transform: translateX(300%); } }
  .rsub { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
  .rb { font-size: 12.5px; color: rgba(255, 255, 255, .78); }
  .rs { font-size: 12px; color: rgba(255, 255, 255, .5); }
  .stop { display: flex; }
  .v { color: rgba(255, 255, 255, .8); }
  .fe { max-width: 360px; font-size: 12.5px; color: rgba(255, 255, 255, .55); }
  .err { margin-top: 12px; display: flex; align-items: flex-start; gap: 8px; padding: 10px 12px; border-radius: 8px; background: rgba(255, 255, 255, .05); font-size: 13px; color: rgba(255, 255, 255, .85); animation: rise-in .2s var(--ease); }
  .ei { flex: none; padding-top: 1px; display: flex; }
</style>
