<script lang="ts">
  // Settings > Transfer music: original flat Liner settings layout on the start
  // screen, accumulative paste & drag-drop on Connect, SkeletonSwap chooser with
  // covers/counts/rename, live event-driven progress queue, and Done summary.
  import { onDestroy, onMount } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import { fx } from "../ui.svelte";
  import { createDraftPlaylists, draftIdOf, draftValue, isDraftValue, type DraftPlaylist } from "../draft-playlists.svelte";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import SkeletonSwap from "../components/SkeletonSwap.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import SectionLabel from "./SectionLabel.svelte";
  import Surface from "./Surface.svelte";
  import Field from "./Field.svelte";
  import Select from "./Select.svelte";
  import Check from "./Check.svelte";
  import Spin from "./Spin.svelte";
  import { openUrl, readClipboard } from "./util";
  import * as M from "./migrate-state.svelte";
  import { mg, migrateDrafts, SERVICES, serviceOf } from "./migrate-state.svelte";
  import type { ListKey } from "../api/types";
  import type { MigrateCollection, MigrateRunItem, MigrateTotals } from "./types";

  let {}: { sub?: number } = $props();
  onMount(() => {
    M.loadResume();
    M.installMigrateEvents();
  });

  const ICON = "rgba(255,255,255,.6)";
  const instant = $derived(fx.reduce || fx.still);
  const stepIn = $derived(instant ? { y: 0, duration: 0 } : { y: 6, duration: 190, easing: cubicOut });
  const stepOut = $derived(instant ? { duration: 0 } : { duration: 110, easing: cubicOut });

  const svc = $derived(mg.service ? serviceOf(mg.service) : null);
  const pickedList = $derived(mg.overview?.collections.filter((c) => mg.picked[c.id]) ?? []);
  const picked = $derived(pickedList.length);
  const pickedSongs = $derived(pickedList.reduce((acc, c) => acc + (c.kind === "liked" || c.kind === "playlist" ? (c.count ?? 0) : 0), 0));
  const ready = $derived(
    !!svc && (svc.connect === "paste" || svc.connect === "takeout" ? mg.pasted != null || mg.files.length > 0 : mg.profile.trim().length > 0),
  );
  const mainCols = $derived(mg.overview?.collections.filter((c) => c.kind !== "playlist") ?? []);
  const listCols = $derived(mg.overview?.collections.filter((c) => c.kind === "playlist") ?? []);

  // Filter state in Choose step
  let filterQ = $state("");
  const filteredLists = $derived.by(() => {
    const q = filterQ.trim().toLowerCase();
    if (!q) return listCols;
    return listCols.filter((c) => displayName(c).toLowerCase().includes(q) || c.hint.toLowerCase().includes(q));
  });
  $effect(() => {
    if (mg.step !== "choose") {
      filterQ = "";
    }
    if (mg.step !== "link") {
      linkDrafts.clear();
      if (isDraftValue(linkTarget)) linkTarget = "new";
    }
  });

  // Drag & drop state (Connect step and invisible file drop on Pick)
  let dragging = $state(false);
  let dragDepth = 0;

  // Single-link import state (when entering via "By link")
  let linkUrl = $state("");
  let linkTarget = $state("new");
  const linkDrafts = createDraftPlaylists();
  onDestroy(() => linkDrafts.clear());
  let linkFound = $state<string | null>(null);
  let linkPasted = $state<[number, number]>([0, 0]);
  let linkBusy = $state(false);
  let linkProg = $state<{ done: number; total: number; phase?: "reading" | "matching" }>({ done: 0, total: 0 });
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
  function draftOptions(drafts: DraftPlaylist[]) {
    return drafts.map((d) => ({
      value: draftValue(d.id),
      label: d.name,
      sub: tf("common.n_songs", { n: 0 }),
      list: { kind: "playlist", id: d.id } as ListKey,
    }));
  }
  function playlistMenuItems(drafts: DraftPlaylist[], newLabel = t("migrate.target.new")) {
    const items = [...draftOptions(drafts), ...userPlaylistOptions];
    return [
      { value: "new", label: newLabel, icon: "plus" },
      ...(items.length ? [{ sep: true as const }, ...items] : []),
    ];
  }
  const linkDestOptions = $derived(playlistMenuItems(linkDrafts.items));
  const choosePlaylistOptions = $derived(playlistMenuItems(migrateDrafts.items));
  const targetOptions = $derived([
    { value: "liked", label: t("migrate.liked"), icon: "heart" },
    { label: t("list.kind.playlist"), icon: "list-music", children: choosePlaylistOptions },
  ]);

  function targetLabelFor(c: MigrateCollection, dest: string): string | undefined {
    if (dest === "new" && c.kind === "playlist" && c.name.trim()) {
      return tf("migrate.target.new_named", { name: c.name.trim() });
    }
    return undefined;
  }

  function pickTarget(c: MigrateCollection, v: string) {
    if (v === "new") {
      const cur = mg.targets[c.id] ?? (c.kind === "liked" ? "liked" : "new");
      const initial = isDraftValue(cur) ? "" : c.kind === "playlist" ? c.name.trim() : "";
      M.promptNewTarget(c.id, initial);
      return;
    }
    M.setTarget(c.id, v);
  }

  function pickLinkTarget(v: string) {
    if (v === "new") {
      linkDrafts.promptCreate({
        onCreated: (draft) => {
          linkTarget = draftValue(draft.id);
          linkDrafts.prune([draft.id]);
        },
      });
      return;
    }
    linkTarget = v;
    const did = draftIdOf(v);
    linkDrafts.prune(did ? [did] : []);
  }

  function displayName(c: MigrateCollection): string {
    if (c.kind === "liked" && svc?.connect === "paste" && !c.name.trim()) return t("migrate.copied_songs");
    return M.collectionName(c);
  }

  function kindIcon(kind: MigrateCollection["kind"] | MigrateRunItem["kind"]): string {
    if (kind === "liked") return "heart";
    if (kind === "albums") return "disc-3";
    if (kind === "artists") return "user-round";
    return "list-music";
  }

  function summaryParts(x: MigrateTotals): string[] {
    const out: string[] = [];
    if (x.liked > 0) out.push(`+${x.liked} ${t("migrate.liked")}`);
    if (x.playlists > 0) out.push(`+${x.playlists} ${t("migrate.playlists.done")}`);
    if (x.albums > 0) out.push(`+${x.albums} ${t("migrate.albums")}`);
    if (x.artists > 0) out.push(`+${x.artists} ${t("migrate.artists")}`);
    return out;
  }

  function startLabel(): string {
    if (picked === 1) {
      return pickedSongs > 0 ? tf("migrate.start_one_songs", { songs: pickedSongs }) : t("migrate.start_one");
    }
    return pickedSongs > 0
      ? tf("migrate.start_songs", { n: picked, songs: pickedSongs })
      : tf("migrate.start", { n: picked });
  }

  async function pasteIntoLink() {
    linkUrl = await readClipboard();
  }

  async function pasteIntoProfile() {
    const clip = (await readClipboard()).trim();
    if (clip) { mg.profile = clip; mg.error = null; }
  }

  async function importSingleLink() {
    const v = linkUrl.trim();
    if (!v || linkBusy) return;
    mg.error = null;
    linkBusy = true;
    const did = draftIdOf(linkTarget);
    const draft = did ? linkDrafts.get(did) : undefined;
    const playlistId = linkTarget.startsWith("pl:") ? linkTarget.slice(3) : null;
    const r = await backend()
      .call<{ error?: string | null; playlist?: string } | null>("import_link", {
        url: v,
        name: draft ? draft.name : null,
        playlist_id: playlistId,
      })
      .catch((e) => ({ error: String(e) }));
    linkBusy = false;
    if (r?.error) { mg.error = r.error; return; }
    linkUrl = "";
    linkDrafts.clear();
    if (did) linkTarget = "new";
  }

  // Global Ctrl+V shortcut when not typing inside an input/textarea
  function onWindowPaste(e: ClipboardEvent) {
    const el = e.target as HTMLElement | null;
    if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable)) return;
    const text = e.clipboardData?.getData("text/plain")?.trim();
    if (!text) return;
    if (mg.step === "pick") {
      e.preventDefault();
      void M.quickContinue(text, []);
    } else if (mg.step === "connect" && svc) {
      e.preventDefault();
      if (svc.connect === "paste" || svc.connect === "takeout") void M.appendPasted(text, false);
      else { mg.profile = text; mg.error = null; }
    } else if (mg.step === "link") {
      e.preventDefault();
      linkUrl = text;
    }
  }

  function onDragEnter(e: DragEvent) {
    if (!e.dataTransfer?.types.includes("Files")) return;
    e.preventDefault();
    dragDepth++;
    dragging = true;
  }
  function onDragOver(e: DragEvent) {
    if (!e.dataTransfer?.types.includes("Files")) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
  }
  function onDragLeave(e: DragEvent) {
    if (!e.dataTransfer?.types.includes("Files")) return;
    e.preventDefault();
    dragDepth = Math.max(0, dragDepth - 1);
    if (dragDepth === 0) dragging = false;
  }
  async function onDropFiles(e: DragEvent, mode: "pick" | "connect") {
    e.preventDefault();
    dragDepth = 0;
    dragging = false;
    const fl = e.dataTransfer?.files;
    if (!fl?.length) return;
    const read = await M.readDroppedFiles(fl);
    if (!read.length) {
      mg.error = t("migrate.file.desc");
      return;
    }
    mg.error = null;
    if (mode === "pick") {
      void M.quickContinue("", read);
    } else {
      M.addFiles(read);
    }
  }

  function toLibrary() { app.settingsOpen = false; app.navigate("library"); }
  function playlistFile() { app.settingsOpen = false; backend().call("import_playlist_file"); }

  function itemDisplayName(it: MigrateRunItem): string {
    if (it.kind === "liked" && (!it.name.trim() || it.name === "liked")) return t("migrate.liked");
    if (it.kind === "albums" && (!it.name.trim() || it.name === "albums")) return t("migrate.albums");
    if (it.kind === "artists" && (!it.name.trim() || it.name === "artists")) return t("migrate.artists");
    return it.name;
  }

  function activeTitle(r: NonNullable<typeof mg.progress>): string {
    const it = r.items?.[r.index];
    const name = it ? itemDisplayName(it) : r.current;
    if (r.lists > 1) return name ? tf("migrate.list", { i: r.index + 1, n: r.lists, name }) : `${r.index + 1}/${r.lists}`;
    return name || t("migrate.reading");
  }

  function itemStatusText(it: MigrateRunItem): string {
    if (it.status === "done") {
      if (it.added > 0) return tf("migrate.added_n", { n: it.added });
      return t("migrate.up_to_date");
    }
    if (it.status === "reading") return t("migrate.phase.reading");
    if (it.status === "matching") return t("migrate.phase.matching");
    if (it.status === "failed") return t("migrate.status.failed");
    if (it.status === "skipped") return t("migrate.status.skipped");
    return t("migrate.status.queued");
  }

  const stepKey = $derived(mg.step === "loading" || mg.step === "choose" ? "choose-stage" : mg.step);
  const runningElapsed = $derived(
    mg.step === "running"
      ? M.formatElapsed(Math.max(mg.progress?.elapsed_ms ?? 0, mg.runStartedClient ? mg.clockNow - mg.runStartedClient : 0))
      : "",
  );
</script>

<svelte:window onpaste={onWindowPaste} />

<div class="stage">
  {#key stepKey}
    <div class="col" in:fly={stepIn} out:fade={stepOut}>
      {#if mg.step === "pick"}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="pick-wrap"
          ondragenter={onDragEnter}
          ondragover={onDragOver}
          ondragleave={onDragLeave}
          ondrop={(e) => onDropFiles(e, "pick")}
        >
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
        </div>

      {:else if mg.step === "connect" && svc}
        {#if svc.connect === "paste" || svc.connect === "takeout"}
          {#if svc.id === "spotify" || svc.id === "youtube"}
            <Surface pad={14} gap={8}>
              {#each [1, 2, 3] as stepNum}
                <div class="guide-row">
                  <span class="mono gnum">{stepNum}</span>
                  <span class="gtxt">{t(`migrate.${svc.id}.step${stepNum}`)}</span>
                </div>
              {/each}
            </Surface>
            <div class="sp10"></div>
          {/if}

          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="drop-card"
            class:drag={dragging}
            class:pulse={mg.justAdded}
            ondragenter={onDragEnter}
            ondragover={onDragOver}
            ondragleave={onDragLeave}
            ondrop={(e) => onDropFiles(e, "connect")}
          >
            <div class="dc-row">
              <div class="dc-info">
                <div class="dc-title">{t("migrate.clipboard")}</div>
                <div class="dc-sub">
                  {#if mg.pasted != null}
                    <span class="ok-inline"><Icon name="check" size={13} color="var(--accent)" />{M.formatCounts(mg.pastedCounts[0], mg.pastedCounts[1])}</span>
                  {:else}
                    {t("migrate.paste.append_hint")}
                  {/if}
                </div>
              </div>
              <div class="dc-btns">
                {#if mg.pasted != null}<Button onclick={M.clearPasted}>{t("migrate.clear")}</Button>{/if}
                <Button kind={mg.pasted == null ? "primary" : "default"} onclick={M.pasteClipboard}>{t("migrate.clipboard.button")}<Icon name="copy" size={14} color={mg.pasted == null ? "currentColor" : ICON} /></Button>
              </div>
            </div>

            <div class="dc-sep"></div>

            <div class="dc-row">
              <div class="dc-info">
                <div class="dc-title">{svc.connect === "takeout" ? t("migrate.takeout") : t("migrate.file")}</div>
                <div class="dc-sub">{svc.connect === "takeout" ? t("migrate.takeout.desc") : t("migrate.file.desc")}</div>
              </div>
              <div class="dc-btns">
                {#if svc.exportUrl}<Button onclick={() => openUrl(svc.exportUrl!)}>{t("migrate.takeout.open")}<Icon name="external-link" size={13} color={ICON} /></Button>{/if}
                <Button onclick={M.pickPlaylistFiles}>{t("migrate.files.button")}<Icon name="folder-open" size={14} color={ICON} /></Button>
              </div>
            </div>

            {#if mg.files.length}
              <div class="chips">
                {#each mg.files as f (f.name)}
                  <span class="chip">
                    <Icon name="file-text" size={12} color={ICON} />
                    <span class="trunc">{f.name}</span>
                    <button class="cxbtn" onclick={() => M.removeFile(f.name)} aria-label={t("migrate.clear")}><Icon name="x" size={11} color={ICON} /></button>
                  </span>
                {/each}
              </div>
            {/if}

            {#if dragging}
              <div class="qdrop"><Icon name="arrow-down-to-line" size={15} color="var(--accent)" /><span>{t("migrate.quick.drop_active")}</span></div>
            {/if}
          </div>
        {:else}
          <Rows>
            <Row label={t(`migrate.${svc.id}.profile`)} info={t(`migrate.${svc.id}.profile.desc`)}>
              <Field bind:value={mg.profile} width={220} placeholder={t("migrate.profile.placeholder")} onenter={M.connect} label={t(`migrate.${svc.id}.profile`)} />
              <Button onclick={pasteIntoProfile}>{t("migrate.clipboard.button")}<Icon name="copy" size={13} color={ICON} /></Button>
            </Row>
          </Rows>
        {/if}

        <div class="foot end">
          <Button kind="primary" disabled={!ready} onclick={M.connect}>{t("migrate.next")}<Icon name="arrow-right" size={14} color="currentColor" /></Button>
        </div>

      {:else if mg.step === "link"}
        <Rows>
          <div class="srow lf">
            <div class="lrow">
              <input class="lin" bind:value={linkUrl} onkeydown={(e) => e.key === "Enter" && importSingleLink()} placeholder={t("dialog.playlist.link")} spellcheck="false" />
              <Button onclick={pasteIntoLink}>{t("migrate.clipboard.button")}<Icon name="copy" size={14} color={ICON} /></Button>
            </div>
            {#if linkBusy}
              {@const lPct = linkProg.total > 0 ? Math.min(100, Math.round((linkProg.done * 100) / linkProg.total)) : 0}
              <div class="lst">
                <Spin label={linkProg.phase === "matching" ? t("migrate.phase.matching") : t("dialog.playlist.reading")} />
                {#if linkProg.total > 0}<span class="mono">{tf("migrate.songs_progress", { done: linkProg.done, total: linkProg.total, pct: lPct })}</span>{/if}
              </div>
              <div class="pbar sm"><div class="pfill" class:ind={linkProg.total === 0} style:width={linkProg.total > 0 ? `${lPct}%` : undefined}></div></div>
            {:else if linkFound}
              <div class="lst ok"><Icon name="check" size={13} color="var(--accent)" /><span>{tf("dialog.playlist.link.found", { kind: linkFound })}</span></div>
            {:else if linkPasted[0] + linkPasted[1] > 0}
              <div class="lst ok"><Icon name="check" size={13} color="var(--accent)" /><span>{M.formatCounts(linkPasted[0], linkPasted[1])}</span></div>
            {:else if linkUrl.trim()}
              <div class="lst">{t("import.unknown_link")}</div>
            {/if}
          </div>
          <Row label={t("migrate.target")}>
            {#if isDraftValue(linkTarget)}
              {@const did = draftIdOf(linkTarget)!}
              <button
                class="renbtn"
                title={t("migrate.rename")}
                aria-label={t("migrate.rename")}
                onclick={() => linkDrafts.promptRename(did)}
              >
                <Icon name="pencil" size={13} color={ICON} />
              </button>
            {/if}
            <Select value={linkTarget} options={linkDestOptions} onpick={pickLinkTarget} />
          </Row>
        </Rows>
        <div class="foot end">
          <Button kind="primary" disabled={linkBusy || !linkUrl.trim()} onclick={importSingleLink}>{linkBusy ? t("dialog.importing") : t("dialog.import")}</Button>
        </div>

      {:else if (mg.step === "loading" || mg.step === "choose") && svc}
        <SkeletonSwap loading={mg.step === "loading" || !mg.overview} key={mg.overview?.account ?? ""}>
          {#snippet skeleton()}
            <div class="ld">
              <Spin label={t("migrate.loading")} />
              <div class="sk">
                {#each [0.42, 0.56, 0.38, 0.5, 0.62] as w, i}
                  <div class="skr">
                    <Skeleton w={36} h={36} radius={6} id={`mg-c${i}`} />
                    <div class="skt">
                      <Skeleton w="{w * 100}%" h={12} radius={4} id={`mg-t${i}`} />
                      <Skeleton w="24%" h={10} radius={4} id={`mg-a${i}`} />
                    </div>
                  </div>
                {/each}
              </div>
            </div>
          {/snippet}

          {#if mg.overview}
            <div class="col">
              {#if mainCols.length}
                <div class="cl">
                  {#each mainCols as c (c.id)}{@render coll(c)}{/each}
                </div>
              {/if}
              {#if listCols.length}
                <div class="plh">
                  <SectionLabel text={tf("migrate.playlists", { n: filteredLists.length })} />
                  <span class="fl"></span>
                  {#if listCols.length >= 6}
                    <label class="fbox">
                      <Icon name="search" size={13} color="rgba(255,255,255,.45)" />
                      <input bind:value={filterQ} placeholder={t("migrate.filter")} spellcheck="false" />
                    </label>
                  {/if}
                  <div class="pla">
                    <Button kind="text" onclick={() => M.allPlaylists(true)}>{t("migrate.all")}</Button>
                    <Button kind="text" onclick={() => M.allPlaylists(false)}>{t("migrate.none")}</Button>
                  </div>
                </div>
                <div class="cl">
                  {#each filteredLists as c (c.id)}{@render coll(c)}{/each}
                </div>
              {/if}
              <div class="foot big">
                <span class="fsum">{t("migrate.choose.note")}</span>
                <span class="fl"></span>
                <Button kind="primary" disabled={picked === 0} onclick={M.start}>{startLabel()}</Button>
              </div>
            </div>
          {/if}
        </SkeletonSwap>

      {:else if mg.step === "running" && svc}
        {@const r = mg.progress}
        {@const pct = r && r.total > 0 ? Math.min(100, Math.round((r.done * 100) / r.total)) : 0}
        {@const stats = r ? summaryParts(r.totals) : []}
        {@const phaseLabel = r?.phase === "matching" ? t("migrate.phase.matching") : t("migrate.phase.reading")}
        <div class="run">
          <Surface pad={16} gap={12}>
            <div class="rl">
              <Spin />
              {#if r}
                <span class="rt trunc">{activeTitle(r)}</span>
              {:else}
                <span class="rt">{t("migrate.reading")}</span>
              {/if}
              <span class="phase-pill">{phaseLabel}</span>
              <span class="mono elapsed">{runningElapsed}</span>
            </div>

            <div class="pbar"><div class="pfill" class:ind={!r || r.total === 0} style:width={r && r.total > 0 ? `${pct}%` : undefined}></div></div>

            <div class="rsub">
              <span class="mono rb">
                {#if r && r.total > 0}
                  {tf("migrate.songs_progress", { done: r.done, total: r.total, pct })}
                {:else if r && r.done > 0}
                  {M.formatCounts(r.done, 0)}
                {:else}
                  {t("migrate.reading")}
                {/if}
              </span>
              {#if stats.length}<span class="mono rs">{stats.join(" · ")}</span>{/if}
            </div>
          </Surface>

          {#if r?.items?.length}
            <div class="qlist">
              {#each r.items as it (it.id)}
                <div class="qrow" class:active={it.status === "reading" || it.status === "matching"} class:done={it.status === "done"} class:fail={it.status === "failed"}>
                  <span class="qico">
                    {#if it.status === "done"}
                      <Icon name="circle-check" size={15} color="var(--accent)" />
                    {:else if it.status === "failed"}
                      <Icon name="circle-alert" size={15} color="#f87171" />
                    {:else if it.status === "reading" || it.status === "matching"}
                      <Spin />
                    {:else}
                      <span class="qdot"></span>
                    {/if}
                  </span>
                  <span class="qname trunc">{itemDisplayName(it)}</span>
                  {#if it.count != null && it.status !== "done"}
                    <span class="mono cc">{it.count}</span>
                  {/if}
                  <span class="qst" class:ok={it.status === "done"} class:errtxt={it.status === "failed"}>{itemStatusText(it)}</span>
                  {#if it.status === "done" && (it.playlist_id || it.kind === "liked")}
                    <button class="open-mini" onclick={() => M.openTransferred(it)}>{t("migrate.open")}</button>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}

          <div class="stop-row">
            <Button onclick={() => !r?.stopping && M.cancel()} disabled={!!r?.stopping}>{r?.stopping ? t("migrate.stopping") : t("migrate.stop")}</Button>
            <span class="note-inline">{t("migrate.running.note")}</span>
          </div>
        </div>

      {:else if mg.step === "done" && mg.finished && svc}
        {@const f = mg.finished}
        {@const deferred = f.totals.deferred ?? 0}
        {@const checked = Math.max(0, f.totals.songs - deferred)}
        {@const missing = Math.max(0, checked - f.totals.matched)}
        {@const matchPct = checked === 0 ? 100 : Math.floor((f.totals.matched * 100) / checked)}
        {@const doneItems = (f.items ?? []).filter((it) => it.status === "done")}
        {@const failedItems = (f.items ?? []).filter((it) => it.status === "failed")}

        <Surface pad={16} gap={14}>
          <div class="done-head">
            <span class="done-ic" class:stopped={f.cancelled}>
              <Icon name={f.cancelled ? "pause" : "check"} size={18} color={f.cancelled ? "#fff" : "var(--accent)"} />
            </span>
            <div class="done-tx">
              <div class="done-t">{f.cancelled ? tf("migrate.stopped.title", { service: svc.name }) : tf("migrate.done", { service: svc.name })}</div>
              {#if (f.elapsed_ms ?? 0) > 0}
                <div class="done-sub mono">{tf("migrate.done_in", { time: M.formatElapsed(f.elapsed_ms!) })}</div>
              {/if}
            </div>
          </div>

          <div class="stats-grid">
            {#if f.totals.songs > 0}
              <div class="stat">
                <span class="mono sv">{f.totals.songs}</span>
                <span class="sl">{t("migrate.songs_total")}</span>
              </div>
            {/if}
            {#if f.totals.liked > 0}
              <div class="stat">
                <span class="mono sv">+{f.totals.liked}</span>
                <span class="sl">{t("migrate.liked")}</span>
              </div>
            {/if}
            {#if f.totals.playlists > 0}
              <div class="stat">
                <span class="mono sv">+{f.totals.playlists}</span>
                <span class="sl">{t("migrate.playlists.done")}</span>
              </div>
            {/if}
            {#if f.totals.albums > 0}
              <div class="stat">
                <span class="mono sv">+{f.totals.albums}</span>
                <span class="sl">{t("migrate.albums")}</span>
              </div>
            {/if}
            {#if f.totals.artists > 0}
              <div class="stat">
                <span class="mono sv">+{f.totals.artists}</span>
                <span class="sl">{t("migrate.artists")}</span>
              </div>
            {/if}
          </div>

          {#if checked > 0}
            <div class="match-wrap">
              <div class="match-top">
                <span class="ml">{t("migrate.matched")}</span>
                <span class="mono mv">{f.totals.matched}/{checked} ({matchPct}%)</span>
              </div>
              <div class="pbar sm"><div class="pfill" style:width="{matchPct}%"></div></div>
            </div>
          {/if}
        </Surface>

        {#if deferred > 0 || f.totals.unsure > 0 || missing > 0}
          <Rows>
            {#if deferred > 0}
              <Row label={t("migrate.deferred")} info={t("migrate.deferred.desc")}><span class="mono v">{deferred}</span></Row>
            {/if}
            {#if f.totals.unsure > 0}
              <Row label={t("migrate.unsure")} info={t("migrate.unsure.desc")}><span class="mono v">{f.totals.unsure}</span></Row>
            {/if}
            {#if missing > 0}
              <Row label={t("migrate.missing")} info={t("migrate.missing.desc")}><span class="mono v">{missing}</span></Row>
            {/if}
          </Rows>
        {/if}

        {#if doneItems.length}
          <SectionLabel text={t("migrate.transferred_lists")} />
          <div class="qlist">
            {#each doneItems as it (it.id)}
              <div class="qrow done with-thumb">
                {#if it.cover}
                  <img class="thumb sm" src={it.cover} alt="" loading="lazy" />
                {:else}
                  <span class="thumb sm ph"><Icon name={kindIcon(it.kind)} size={13} color={ICON} /></span>
                {/if}
                <span class="qname trunc">{itemDisplayName(it)}</span>
                <span class="mono cc">{it.added > 0 ? tf("migrate.added_n", { n: it.added }) : t("migrate.up_to_date")}</span>
                <Button kind="text" onclick={() => M.openTransferred(it)}>{t("migrate.open")}<Icon name="arrow-right" size={12} color={ICON} /></Button>
              </div>
            {/each}
          </div>
        {/if}

        {#if f.failed.length}
          <div class="fail-head">
            <SectionLabel text={t("migrate.failed")} />
            <span class="fl"></span>
            {#if failedItems.length}
              <Button onclick={M.retryFailed}><Icon name="rotate-cw" size={13} color={ICON} />{tf("migrate.retry_failed", { n: failedItems.length })}</Button>
            {/if}
          </div>
          <Rows>{#each f.failed as [name, err], i (`${name}:${i}`)}<Row label={name || svc.name}><span class="fe">{err}</span></Row>{/each}</Rows>
        {/if}

        {#each f.notes as k (k)}<div class="note">{t(`migrate.note.${k}`)}</div>{/each}

        <div class="foot big">
          <Button icon="arrow-left" onclick={M.toPick}>{t("migrate.another")}</Button>
          <span class="fl"></span>
          <Button kind="primary" onclick={toLibrary}>{t("migrate.to_library")}</Button>
        </div>
      {/if}

      {#if mg.error}
        <div class="err"><span class="ei"><Icon name="info" size={14} color={ICON} /></span><div>{mg.error}</div></div>
      {/if}
    </div>
  {/key}
</div>

{#snippet coll(c: MigrateCollection)}
  {@const on = !!mg.picked[c.id]}
  {@const hasDest = c.kind === "liked" || c.kind === "playlist"}
  {@const dest = mg.targets[c.id] ?? (c.kind === "liked" ? "liked" : "new")}
  <div class="cwrap">
    <div class="cr" class:off={!on}>
      <button class="ctog" onclick={() => M.toggle(c.id)}>
        <Check {on} />
        {#if c.cover}
          <img class="thumb" src={c.cover} alt="" loading="lazy" />
        {:else}
          <span class="thumb ph"><Icon name={kindIcon(c.kind)} size={15} color={on ? "rgba(255,255,255,.75)" : "rgba(255,255,255,.35)"} /></span>
        {/if}
        <span class="cn" class:off={!on}>{displayName(c)}</span>
        {#if c.count != null}
          <span class="mono cc">{c.count}</span>
        {/if}
        {#if c.followed}<span class="cx">{t("migrate.followed")}</span>{/if}
        {#if mg.overview?.done_before.includes(c.id)}<span class="cx">{t("migrate.done_before")}</span>{/if}
      </button>
      {#if hasDest}
        <div class="dest" class:dim={!on}>
          {#if on && (dest === "new" || isDraftValue(dest))}
            <button
              class="renbtn"
              title={t("migrate.rename")}
              aria-label={t("migrate.rename")}
              onclick={() => M.promptRenameTarget(c.id, c.kind === "playlist" ? c.name.trim() : "")}
            >
              <Icon name="pencil" size={13} color={ICON} />
            </button>
          {/if}
          <Select value={dest} label={targetLabelFor(c, dest)} options={targetOptions} onpick={(v) => pickTarget(c, v)} />
        </div>
      {/if}
    </div>
  </div>
{/snippet}

<style>
  .stage { display: grid; }
  .col { grid-area: 1 / 1; min-width: 0; display: flex; flex-direction: column; }
  .pick-wrap { display: flex; flex-direction: column; }
  .sp10 { height: 10px; }
  .fl { flex: 1; }
  .note { padding-top: 14px; font-size: 12px; color: rgba(255, 255, 255, .42); line-height: 1.5; }
  .note-inline { font-size: 12px; color: rgba(255, 255, 255, .42); line-height: 1.4; }
  .foot { padding-top: 16px; display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .foot.end { justify-content: flex-end; }
  .foot.big { padding-top: 18px; }
  .fsum { font-size: 12px; color: rgba(255, 255, 255, .45); }

  /* File chips */
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .chip {
    display: inline-flex; align-items: center; gap: 6px; max-width: 240px;
    height: 26px; padding: 0 6px 0 9px; border-radius: 999px;
    background: rgba(255, 255, 255, .07); font-size: 12px; color: rgba(255, 255, 255, .85);
  }
  .cxbtn { width: 18px; height: 18px; border-radius: 50%; display: grid; place-items: center; opacity: .65; transition: opacity .12s, background-color .12s; }
  .cxbtn:hover { opacity: 1; background: rgba(255, 255, 255, .12); }

  /* Connect guide & accumulative drop card */
  .guide-row { display: flex; align-items: flex-start; gap: 10px; font-size: 12.5px; color: rgba(255, 255, 255, .72); line-height: 1.45; }
  .gnum {
    flex: none; width: 19px; height: 19px; border-radius: 50%; display: grid; place-items: center;
    background: rgba(255, 255, 255, .08); font-size: 11px; font-weight: 600; color: rgba(255, 255, 255, .85);
  }
  .gtxt { padding-top: 1px; }
  .drop-card {
    display: flex; flex-direction: column; gap: 12px; padding: 14px 16px;
    border-radius: 12px; background: rgba(255, 255, 255, .035);
    border: 1px dashed rgba(255, 255, 255, .11);
    transition: border-color .18s ease-out, background-color .18s ease-out;
  }
  .drop-card.drag {
    border-color: var(--accent);
    background: rgba(var(--accent-rgb, 255, 95, 210), .08);
  }
  .drop-card.pulse { border-color: rgba(var(--accent-rgb, 255, 95, 210), .55); background: rgba(var(--accent-rgb, 255, 95, 210), .06); }
  .dc-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
  .dc-info { flex: 1 1 200px; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .dc-title { font-size: 14px; font-weight: 500; color: #fff; }
  .dc-sub { font-size: 12.5px; color: rgba(255, 255, 255, .52); line-height: 1.4; }
  .ok-inline { display: inline-flex; align-items: center; gap: 6px; color: #fff; font-weight: 500; }
  .dc-btns { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .dc-sep { height: 1px; background: rgba(255, 255, 255, .06); }
  .qdrop { display: flex; align-items: center; gap: 8px; font-size: 13px; font-weight: 500; color: var(--accent); }

  /* Filter & loading & choose */
  .fbox {
    display: flex; align-items: center; gap: 7px; height: 28px; padding: 0 9px; margin-right: 6px;
    border-radius: 7px; background: rgba(255, 255, 255, .06); width: 170px; max-width: 100%;
    cursor: text;
  }
  .fbox input { flex: 1; min-width: 0; border: 0; outline: 0; background: none; font-size: 12px; color: #fff; }
  .fbox input::placeholder { color: rgba(255, 255, 255, .36); }

  .ld { display: flex; flex-direction: column; gap: 12px; }
  .sk { display: flex; flex-direction: column; gap: 4px; }
  .skr { display: flex; align-items: center; gap: 12px; padding: 6px 8px; }
  .skt { flex: 1; display: flex; flex-direction: column; gap: 7px; }

  .cl { padding-top: 4px; display: flex; flex-direction: column; gap: 2px; }
  .cwrap { display: flex; flex-direction: column; gap: 6px; }
  .cr { min-height: 48px; padding: 5px 6px 5px 10px; display: flex; align-items: center; gap: 10px; border-radius: 9px; transition: background-color .12s; }
  .cr:hover { background: rgba(255, 255, 255, .05); }
  .ctog { flex: 1; min-width: 0; min-height: 36px; display: flex; align-items: center; gap: 10px; text-align: left; }
  .thumb { flex: none; width: 34px; height: 34px; border-radius: 6px; object-fit: cover; background: rgba(255, 255, 255, .06); }
  .thumb.sm { width: 28px; height: 28px; border-radius: 5px; }
  .thumb.ph { display: grid; place-items: center; }
  .dest { flex: none; display: flex; align-items: center; gap: 6px; transition: opacity .15s; }
  .dest.dim { opacity: .45; }
  .renbtn { width: 28px; height: 28px; border-radius: 7px; display: grid; place-items: center; transition: background-color .12s; }
  .renbtn:hover { background: rgba(255, 255, 255, .08); }
  .cn { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13.5px; font-weight: 500; color: #fff; transition: color .15s; }
  .cn.off { color: rgba(255, 255, 255, .52); }
  .cc { flex: none; padding: 1px 6px; border-radius: 5px; background: rgba(255, 255, 255, .07); font-size: 11px; color: rgba(255, 255, 255, .64); }
  .cx { flex: none; font-size: 11.5px; color: rgba(255, 255, 255, .44); }
  .plh { display: flex; align-items: center; gap: 6px; padding-top: 10px; }
  .pla { display: flex; gap: 2px; font-size: var(--sm); }

  /* Link inputs */
  .lf { padding: 14px 0; display: flex; flex-direction: column; gap: 10px; }
  .lrow { display: flex; align-items: center; gap: 8px; }
  .lin { flex: 1; min-width: 0; height: 34px; padding: 0 10px; border: 0; outline: none; border-radius: 8px; background: rgba(255, 255, 255, .06); font-size: 13px; color: #fff; transition: background-color .15s ease-out; }
  .lin:focus { background: rgba(255, 255, 255, .1); }
  .lin::placeholder { color: rgba(255, 255, 255, .32); }
  .lst { display: flex; align-items: center; justify-content: space-between; gap: 8px; font-size: 12.5px; color: rgba(255, 255, 255, .52); }
  .lst.ok { justify-content: flex-start; color: rgba(255, 255, 255, .82); }

  /* Running & live queue */
  .run { display: flex; flex-direction: column; gap: 12px; }
  .elapsed { flex: none; font-size: 12px; color: rgba(255, 255, 255, .5); }
  .rl { display: flex; align-items: center; gap: 8px; font-size: 14px; font-weight: 500; min-width: 0; }
  .rt { color: #fff; flex: 1; min-width: 0; }
  .phase-pill {
    flex: none; padding: 2px 8px; border-radius: 999px;
    background: rgba(var(--accent-rgb, 255, 95, 210), .14);
    font-size: 11px; font-weight: 500; color: var(--accent);
  }
  .pbar { height: 6px; width: 100%; border-radius: 999px; background: rgba(255, 255, 255, .09); overflow: hidden; }
  .pbar.sm { height: 4px; }
  .pfill { height: 100%; border-radius: 999px; background: var(--accent); transition: width .18s ease-out; }
  .pfill.ind { width: 35%; animation: slide 1.2s ease-in-out infinite; }
  @keyframes slide { 0% { transform: translateX(-110%); } 100% { transform: translateX(300%); } }
  .rsub { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
  .rb { font-size: 12.5px; color: rgba(255, 255, 255, .82); }
  .rs { font-size: 12px; color: rgba(255, 255, 255, .5); }

  .qlist { display: flex; flex-direction: column; gap: 3px; }
  .qrow {
    display: flex; align-items: center; gap: 10px; min-height: 38px; padding: 5px 10px;
    border-radius: 8px; background: rgba(255, 255, 255, .02);
    transition: background-color .14s ease-out;
  }
  /* Thumb rows (28x28 inside 38px) keep equal 5px padding top, bottom and left. */
  .qrow.with-thumb { padding: 5px 10px 5px 5px; }
  .qrow.active { background: rgba(255, 255, 255, .06); }
  .qrow.done { background: rgba(255, 255, 255, .03); }
  .qico { flex: none; width: 18px; display: grid; place-items: center; }
  .qdot { width: 6px; height: 6px; border-radius: 50%; background: rgba(255, 255, 255, .24); }
  .qname { flex: 1; min-width: 0; font-size: 13px; color: rgba(255, 255, 255, .88); }
  .qrow:not(.active):not(.done):not(.fail) .qname { color: rgba(255, 255, 255, .5); }
  .qst { flex: none; font-size: 11.5px; color: rgba(255, 255, 255, .45); }
  .qst.ok { color: rgba(255, 255, 255, .72); }
  .qst.errtxt { color: #f87171; }
  .open-mini {
    flex: none; padding: 3px 8px; border-radius: 6px; font-size: 11.5px; font-weight: 500;
    color: var(--accent); background: rgba(var(--accent-rgb, 255, 95, 210), .12);
    transition: background-color .12s;
  }
  .open-mini:hover { background: rgba(var(--accent-rgb, 255, 95, 210), .22); }
  .stop-row { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; padding-top: 2px; }

  /* Done summary */
  .done-head { display: flex; align-items: center; gap: 12px; }
  .done-ic {
    flex: none; width: 38px; height: 38px; border-radius: 10px; display: grid; place-items: center;
    background: rgba(var(--accent-rgb, 255, 95, 210), .14);
  }
  .done-ic.stopped { background: rgba(255, 255, 255, .08); }
  .done-tx { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .done-t { font-size: 15px; font-weight: 600; color: #fff; }
  .done-sub { font-size: 12px; color: rgba(255, 255, 255, .5); }
  .stats-grid { display: flex; flex-wrap: wrap; gap: 8px; }
  .stat {
    flex: 1 1 92px; padding: 10px 12px; border-radius: 8px;
    background: rgba(255, 255, 255, .04); display: flex; flex-direction: column; gap: 2px;
  }
  .sv { font-size: 17px; font-weight: 600; color: #fff; }
  .sl { font-size: 11.5px; color: rgba(255, 255, 255, .5); }
  .match-wrap { display: flex; flex-direction: column; gap: 6px; }
  .match-top { display: flex; align-items: center; justify-content: space-between; font-size: 12.5px; }
  .ml { color: rgba(255, 255, 255, .6); }
  .mv { color: rgba(255, 255, 255, .86); }
  .fail-head { display: flex; align-items: flex-end; gap: 8px; }
  .v { color: rgba(255, 255, 255, .82); }
  .fe { max-width: 360px; font-size: 12.5px; color: rgba(255, 255, 255, .55); }
  .err { margin-top: 12px; display: flex; align-items: flex-start; gap: 8px; padding: 10px 12px; border-radius: 8px; background: rgba(255, 255, 255, .05); font-size: 13px; color: rgba(255, 255, 255, .85); animation: rise-in .2s var(--ease); }
  .ei { flex: none; padding-top: 1px; display: flex; }
</style>
