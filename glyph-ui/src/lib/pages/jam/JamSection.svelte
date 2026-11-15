<script lang="ts">
  // ui/jam.rs section_el / start / active / now_card / song_row: the Jam
  // page of the Connect panel (320 px wide). Not in a jam: waiting card
  // (while the host has to let us in), or "Start a Jam" row (h 52) plus the
  // code field with "Join". In a jam: now card (guest), invite card (p 12,
  // radius 10: reactions, copy, QR on white with a 10 px quiet zone, code
  // mono 20, hosted-by, scan hint, copy link), people waiting, perms (host)
  // or listen options (guest), people, up next (rows h 52 with who added,
  // vote, remove), activity, leave / end.
  import { onMount } from "svelte";
  import Icon from "../../components/Icon.svelte";
  import IconButton from "../../components/IconButton.svelte";
  import ToggleButton from "../../components/ToggleButton.svelte";
  import Button from "../../components/Button.svelte";
  import Cover from "../../components/Cover.svelte";
  import Chip from "../../components/Chip.svelte";
  import SongTitle from "../../components/SongTitle.svelte";
  import Avatar from "../profile/Avatar.svelte";
  import Tog from "../../settings/Tog.svelte";
  import { app } from "../../state.svelte";
  import { t, tf } from "../../i18n.svelte";
  import { tip } from "../../ui.svelte";
  import { fmtMs } from "../../fmt";
  import { rating } from "../../api/types";
  import { qrMatrix, qrPath } from "../../qr";
  import {
    ago, admit, copy, FEED_SHOWN, jam, join, kick, leave, QUEUE_SHOWN, react, REACTIONS, removeItem, setApprove, setListenHere, setPerms, showAll, start, stopWaiting,
    vote, watchJam, type JamEvent, type JamItem, type JamPerms,
  } from "../../jam.svelte";

  onMount(() => watchJam());

  let code = $state("");
  const j = $derived(jam.state);
  const copied = $derived.by(() => { void app.clock; return performance.now() - jam.copied < 2000; });
  const copiedCode = $derived(copied && jam.copiedKind === "code");
  const copiedLink = $derived(copied && jam.copiedKind === "link");
  const hostName = $derived(j?.members.find((m) => m.host)?.handle ?? j?.host_handle ?? "");
  const avatarOf = (h: string) => j?.members.find((m) => m.handle === h)?.avatar ?? null;
  const qr = $derived(j ? qrMatrix(j.link) : null);
  const qrD = $derived(qr ? qrPath(qr) : "");
  const queue = $derived(j ? (jam.queueAll ? j.queue : j.queue.slice(0, QUEUE_SHOWN)) : []);
  const more = $derived(!!j && j.queue_len > queue.length);
  const feed = $derived(j ? [...j.feed].filter((e) => e.what !== "react").reverse().slice(0, FEED_SHOWN) : []);
  const anyControl = $derived(!!j && Object.values(j.perms).some(Boolean) && (j.perms.skip || j.perms.seek || j.perms.pause || j.perms.play_now));

  /** Position of the host's song now (ms): it keeps running between reports. */
  const pos = $derived.by(() => {
    void app.clock;
    const n = j?.now;
    if (!n) return 0;
    const dur = n.track.duration_ms ?? Infinity;
    return Math.min(dur, n.position_ms + (n.playing ? performance.now() - jam.at : 0));
  });
  const frac = $derived(j?.now?.track.duration_ms ? Math.min(1, pos / j.now.track.duration_ms) : 0);

  function feedText(e: JamEvent): string {
    const who = `@${e.who}`;
    switch (e.what) {
      case "joined": return tf("jam.f.joined", { who });
      case "left": return tf("jam.f.left", { who });
      case "added": return tf("jam.f.added", { who, title: e.title ?? "" });
      case "played": return tf("jam.f.played", { who, title: e.title ?? "" });
      case "host": return tf("jam.f.host", { who });
      default: return `${who} · ${t(REACTIONS.find((r) => r.id === e.reaction)?.label ?? "jam.r.heart")}`;
    }
  }
  const perm = (key: keyof JamPerms) => (on: boolean) => j && setPerms({ ...j.perms, [key]: on });
</script>

{#snippet heading(text: string)}<div class="sec">{text}</div>{/snippet}

{#if !j}
  {#if jam.waiting}
    <div class="pad">
      <div class="wait">
        <div class="wr">
          <Icon name="users" size={16} color="var(--dim)" />
          <span class="wt"><span class="spin"></span>{t("jam.waiting")}</span>
          <span class="mono wc">{jam.waiting}</span>
        </div>
        <div class="note">{t("jam.waiting.sub")}</div>
        <div class="fl"><Button onclick={stopWaiting}>{t("jam.cancel")}</Button></div>
      </div>
    </div>
  {:else}
    <div class="pad">
      <div class="card startcard">
        <div class="schead">
          <span class="ic"><Icon name="users" size={20} color="var(--accent)" /></span>
          <span class="tx">
            <span class="sa">{t("jam.start")}</span>
            <span class="sb">{t("jam.start.sub")}</span>
          </span>
        </div>
        <Button kind="primary" icon="users" class="full" disabled={jam.busy} onclick={start}>
          {jam.busy ? t("jam.working") : t("jam.start")}
        </Button>
      </div>

      {@render heading(t("jam.join"))}
      <div class="card joincard">
        <form class="join" onsubmit={(e) => { e.preventDefault(); join(code).then(() => { if (jam.state) code = ""; }); }}>
          <input bind:value={code} placeholder={t("jam.code.placeholder")} maxlength="64" spellcheck="false" />
          <Button kind="default" type="submit" disabled={!code.trim()}>{t("jam.join")}</Button>
        </form>
      </div>
      {#if jam.error}<div class="err note">{jam.error}</div>{/if}
    </div>
  {/if}
{:else if jam.page === "invite"}
  <div class="pad">
    <div class="card invite">
      {#if qr}
        <div class="qrwrap">
          <span class="qr"><svg width={qr.length * 4} height={qr.length * 4} viewBox="0 0 {qr.length} {qr.length}" shape-rendering="crispEdges"><path d={qrD} fill="#000" /></svg></span>
        </div>
      {/if}
      <div class="invmeta">
        <div class="code mono">{j.code}</div>
        <div class="note">{j.host ? t("jam.hosting") : tf("jam.hosted_by", { who: `@${hostName}` })}</div>
        <div class="hint">{t("jam.scan")}</div>
      </div>
      <div class="invacts">
        <Button icon={copiedCode ? "check" : "copy"} class="flexbtn" onclick={() => copy(false)}>
          {copiedCode ? t("jam.copied") : t("jam.copy")}
        </Button>
        <Button icon={copiedLink ? "check" : "link-2"} class="flexbtn" onclick={() => copy(true)}>
          {copiedLink ? t("jam.link_copied") : t("jam.copy_link")}
        </Button>
      </div>
    </div>
  </div>
{:else if jam.page === "settings" && j.host}
  <div class="pad">
    {@render heading(t("jam.perms"))}
    <div class="card sgrp">
      <div class="srow">
        <span class="slab">{t("jam.p.add")}</span>
        <Tog on={j.perms.add} label={t("jam.p.add")} onchange={perm("add")} />
      </div>
      <div class="srow sep">
        <span class="slab">{t("jam.p.play_now")}</span>
        <Tog on={j.perms.play_now} label={t("jam.p.play_now")} onchange={perm("play_now")} />
      </div>
      <div class="srow sep">
        <span class="slab">{t("jam.p.skip")}</span>
        <Tog on={j.perms.skip} label={t("jam.p.skip")} onchange={perm("skip")} />
      </div>
      <div class="srow sep">
        <span class="slab">{t("jam.p.seek")}</span>
        <Tog on={j.perms.seek} label={t("jam.p.seek")} onchange={perm("seek")} />
      </div>
      <div class="srow sep">
        <span class="slab">{t("jam.p.pause")}</span>
        <Tog on={j.perms.pause} label={t("jam.p.pause")} onchange={perm("pause")} />
      </div>
    </div>

    <div class="card sgrp mtop">
      <div class="srow">
        <span class="slab">{t("jam.approve")}</span>
        <Tog on={j.approve_joins} label={t("jam.approve")} onchange={(on) => setApprove(on)} />
      </div>
    </div>
  </div>
{:else}
  <div class="pad">
    <div class="topacts">
      <Button icon="user-round" class="flexbtn" onclick={() => (jam.page = "invite")}>{t("jam.invite")}</Button>
      <Button kind="danger" icon="log-out" class="flexbtn" onclick={leave}>{t(j.host ? "jam.end" : "jam.leave")}</Button>
    </div>

    {#if !j.host && j.now}
      <div class="card now">
        <div class="nr">
          <Cover track={j.now.track} size={48} radius={6} />
          <span class="tx">
            <SongTitle rating={rating(j.now.track)} class="sm">{j.now.track.title}</SongTitle>
            <span class="b trunc">{j.now.track.artist}</span>
          </span>
          <Icon name={j.now.playing ? "audio-lines" : "pause"} size={14} color={j.now.playing ? "var(--accent)" : "var(--dim)"} />
        </div>
        <div class="bar">
          <span class="tm mono">{fmtMs(pos)}</span>
          <span class="tr"><span class="fi" style:width="{frac * 100}%"></span></span>
          <span class="tm mono">{fmtMs(j.now.track.duration_ms ?? 0)}</span>
        </div>
        <div class="chips">
          <span class="b">{j.listen_here ? t("jam.playing_here") : tf("jam.playing_at", { who: `@${j.host_handle}` })}</span>
          {#if !j.host_online}<Chip kind="kind">{t("jam.host_offline")}</Chip>{/if}
          {#if j.listen_here && j.unavailable}<Chip kind="kind">{t("jam.unavailable")}</Chip>{/if}
        </div>
      </div>
    {/if}

    {@render heading(t("jam.reactions"))}
    <div class="card rcard">
      <div class="reacts">
        {#each REACTIONS as r (r.id)}
          <button class="rpill" use:tip={t(r.label)} aria-label={t(r.label)} onclick={() => react(r.id)}>
            <span class="remoji">{r.emoji}</span>
          </button>
        {/each}
      </div>
    </div>

    {#if j.host && j.knocks.length}
      {@render heading(t("jam.knocks"))}
      <div class="card list">
        {#each j.knocks as k (k.handle)}
          <div class="k">
            <Avatar src={k.avatar} size={28} flat />
            <span class="kn trunc">@{k.handle}</span>
            <Button kind="primary" class="allow" onclick={() => admit(k.handle, true)}>{t("jam.allow")}</Button>
            <IconButton name="x" size={13} color="var(--faint)" label={t("jam.deny")} onclick={() => admit(k.handle, false)} />
          </div>
        {/each}
      </div>
    {/if}

    {#if !j.host}
      {@render heading(t("jam.listen"))}
      <div class="card sgrp">
        <div class="srow">
          <span class="slab">{t("jam.listen_here")}</span>
          <Tog on={j.listen_here} label={t("jam.listen_here")} onchange={(on) => setListenHere(on)} />
        </div>
        <div class="lnote">{j.listen_here ? t("jam.listen_here.sub") : tf("jam.listen_host.sub", { who: `@${hostName}` })}</div>
        {#if !anyControl}<div class="lnote">{t("jam.host_controls")}</div>{/if}
      </div>
    {/if}

    {@render heading(tf("jam.people", { n: j.members.length }))}
    <div class="card list">
      {#each j.members as m, mi (m.handle)}
        <div class="p" class:sep={mi > 0}>
          <Avatar src={m.avatar} size={28} flat />
          <span class="kn trunc">@{m.handle}</span>
          {#if m.host}<Chip kind="kind">{t("jam.host")}</Chip>{/if}
          {#if m.handle === j.me && !m.host}<Chip kind="kind">{t("jam.you")}</Chip>{/if}
          {#if j.host && !m.host}<IconButton name="x" size={13} color="var(--faint)" label={t("jam.kick")} onclick={() => kick(m.handle)} />{/if}
        </div>
      {/each}
    </div>

    {@render heading(t("jam.up_next"))}
    {#if !queue.length}
      <div class="card emptyc">{t("jam.empty")}</div>
    {:else}
      <div class="card list">
        {#each queue as item, i (item.id)}
          {@const removable = j.host || item.by === j.me}
          {@const canVote = j.perms.add || j.host}
          <div class="q" class:sep={i > 0} style:animation-delay="{Math.min(i, 8) * 25}ms">
            <Cover track={item.track} size={40} radius={6} />
            <span class="tx">
              <SongTitle rating={rating(item.track)} class="sm">{item.track.title}</SongTitle>
              <span class="b trunc">{item.track.artist}</span>
            </span>
            <span use:tip={tf("jam.added_by", { who: `@${item.by}` })} class="by"><Avatar src={avatarOf(item.by)} size={20} flat /></span>
            <span class="vt">
              {#if canVote}<ToggleButton name="thumbs-up" size={14} on={!!item.voted} accent="var(--accent)" label={t("jam.vote")} onclick={() => vote(item)} />{/if}
              {#if item.votes.length > 0}<span class="vn mono">{item.votes.length}</span>{/if}
            </span>
            {#if removable}<IconButton name="x" size={13} color="var(--faint)" label={t("jam.remove")} onclick={() => removeItem(item)} />{/if}
          </div>
        {/each}
      </div>
    {/if}
    {#if more}<div class="fl pm"><Button onclick={showAll}>{t("artist.more")}</Button></div>{/if}

    {#if feed.length}
      {@render heading(t("jam.activity"))}
      <div class="card list">
        {#each feed as e, i (e.at_ms + e.who + i)}
          <div class="f" class:sep={i > 0}>
            <Avatar src={avatarOf(e.who)} size={18} flat />
            <span class="ft trunc">{feedText(e)}</span>
            <span class="fa mono">{ago(j.server_ms + (performance.now() - jam.at), e.at_ms)}</span>
          </div>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .pad { display: flex; flex-direction: column; padding-bottom: 12px; }
  .sec { padding: 14px 16px 6px; font-size: 11.5px; font-weight: 600; color: rgba(255, 255, 255, .45); text-transform: uppercase; letter-spacing: .02em; }
  .note { font-size: var(--xs); color: var(--dim); line-height: 1.4; }
  .err { margin: 8px 16px 0; color: var(--warn); }
  .b { font-size: var(--xs); color: var(--dim); }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .tx :global(.sm) { font-size: var(--sm); color: var(--text); }
  .fl { display: flex; margin: 0 12px; }
  .pm { margin-top: 6px; }
  .card { border-radius: 10px; background: rgba(255, 255, 255, .035); }
  .mtop { margin-top: 10px; }

  /* waiting card */
  .wait { margin: 0 12px; padding: 12px; border-radius: 10px; background: rgba(255, 255, 255, .035); display: flex; flex-direction: column; gap: 8px; }
  .wr { display: flex; align-items: center; gap: 8px; }
  .wt { flex: 1; font-size: var(--sm); color: var(--text); display: flex; align-items: center; gap: 8px; }
  .wc { font-size: var(--xs); color: var(--dim); }
  .spin { width: 10px; height: 10px; border-radius: 50%; border: 1.5px solid var(--dim); border-top-color: transparent; animation: spin .8s linear infinite; }

  /* not-in-jam cards */
  .startcard { margin: 0 12px; padding: 14px; display: flex; flex-direction: column; gap: 12px; }
  .schead { display: flex; align-items: center; gap: 12px; }
  .ic { flex: none; width: 42px; height: 42px; border-radius: 10px; background: rgba(255, 255, 255, .05); display: grid; place-items: center; }
  .sa { font-size: var(--md); font-weight: 600; color: var(--text); }
  .sb { font-size: var(--xs); color: var(--dim); line-height: 1.35; }
  .startcard :global(.full) { width: 100%; height: 34px; }
  .joincard { margin: 0 12px; padding: 10px; }
  .join { display: flex; align-items: center; gap: 8px; }
  .join input { flex: 1; min-width: 0; height: 32px; padding: 0 10px; border-radius: 8px; background: rgba(20, 20, 20, .92); border: 1px solid rgba(255, 255, 255, .08); font-size: var(--sm); outline: none; text-transform: uppercase; transition: border-color .15s; }
  .join input::placeholder { text-transform: none; color: rgba(255, 255, 255, .4); }
  .join input:focus { border-color: rgba(255, 255, 255, .22); }

  /* top action bar */
  .topacts { margin: 0 12px 4px; display: flex; gap: 8px; }
  .topacts :global(.flexbtn), .invacts :global(.flexbtn) { flex: 1; min-width: 0; }

  /* invite sub-view */
  .invite { margin: 0 12px; padding: 16px 14px; display: flex; flex-direction: column; align-items: center; gap: 14px; text-align: center; }
  .qrwrap { display: flex; justify-content: center; }
  .qr { flex: none; padding: 10px; border-radius: 10px; background: #fff; display: block; line-height: 0; animation: fade-in .3s ease-out both; }
  .invmeta { display: flex; flex-direction: column; gap: 4px; align-items: center; }
  .code { font-size: 22px; font-weight: 700; letter-spacing: .08em; color: var(--text); }
  .hint { font-size: var(--xs); color: var(--faint); }
  .invacts { width: 100%; display: flex; gap: 8px; }

  /* settings rows */
  .sgrp { margin: 0 12px; padding: 4px 12px; display: flex; flex-direction: column; }
  .srow { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 10px 0; }
  .srow.sep, .p.sep, .q.sep, .f.sep { border-top: 1px solid rgba(255, 255, 255, .06); }
  .slab { font-size: 13px; font-weight: 500; color: rgba(255, 255, 255, .88); min-width: 0; }
  .lnote { padding: 0 0 8px; font-size: var(--xs); color: var(--dim); line-height: 1.35; }

  /* guest now playing card */
  .now { margin: 8px 12px 2px; padding: 10px; display: flex; flex-direction: column; gap: 8px; }
  .nr { display: flex; align-items: center; gap: 10px; }
  .bar { display: flex; align-items: center; gap: 8px; }
  .tm { font-size: 10px; color: var(--faint); }
  .tr { flex: 1; height: 3px; border-radius: 2px; background: var(--active); overflow: hidden; }
  .fi { display: block; height: 100%; border-radius: 2px; background: var(--dim); transition: width .25s linear; }
  .chips { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }

  /* reactions strip */
  .rcard { margin: 0 12px; padding: 6px; }
  .reacts { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 6px; }
  .rpill { height: 34px; border-radius: 8px; background: rgba(255, 255, 255, .04); display: grid; place-items: center; transition: background-color .12s, transform .12s; }
  .rpill:hover { background: rgba(255, 255, 255, .09); }
  .rpill:hover .remoji { transform: scale(1.14); }
  .rpill:active { transform: scale(.93); }
  .remoji { font-size: 16px; line-height: 1; transition: transform .12s; }

  .list { margin: 0 12px; padding: 4px 8px; display: flex; flex-direction: column; }
  .emptyc { margin: 0 12px; padding: 12px; font-size: var(--sm); color: var(--dim); }
  .k { display: flex; align-items: center; gap: 10px; height: 42px; }
  .k :global(.allow) { height: 26px; padding: 0 10px; }
  .p { display: flex; align-items: center; gap: 10px; min-height: 40px; padding: 4px 0; }
  .kn { flex: 1; min-width: 0; font-size: var(--sm); color: var(--text); }
  .q { display: flex; align-items: center; gap: 10px; min-height: 52px; padding: 4px 0; animation: fade-in .25s ease-out both; }
  .by { flex: none; display: inline-flex; }
  .vt { display: flex; align-items: center; gap: 2px; }
  .vn { width: 14px; font-size: var(--xs); color: var(--dim); }
  .f { display: flex; align-items: center; gap: 8px; min-height: 28px; padding: 4px 0; }
  .ft { flex: 1; min-width: 0; font-size: var(--xs); color: var(--dim); }
  .fa { flex: none; font-size: 10px; color: var(--faint); }
</style>
