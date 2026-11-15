<script lang="ts">
  // ui/account.rs page: Settings > Account. Signed out: two ways in centred
  // (browser passkey, or the in-app form with handle + password; other
  // servers: pairing code, and their own sign-in page only behind a warning),
  // the pairing wait (code box 24 mono, dashed). Signed in, sub-tabs:
  // Overview (handle + sync, rename, device limit, library encryption, Glyph
  // links), Devices (list with limit, pair a device), Security (password,
  // passkey, recovery code, danger zone: delete).
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { t, tf } from "../i18n.svelte";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import RowBase from "./RowBase.svelte";
  import SectionLabel from "./SectionLabel.svelte";
  import Segmented from "./Segmented.svelte";
  import Field from "./Field.svelte";
  import Tog from "./Tog.svelte";
  import Spin from "./Spin.svelte";
  import { ac } from "./account-state.svelte";
  import * as A from "./account-state.svelte";
  import Avatar from "../pages/profile/Avatar.svelte";
  import ProfileSettings from "./ProfileSettings.svelte";
  import { platformIconFor } from "../connect";
  import { tip } from "../ui.svelte";
  import { copyText, whenMs } from "./util";
  let { sub = 0 }: { sub?: number } = $props();

  onMount(() => { A.pageOpened(); });

  const st = $derived(ac.st);
  const touch = $derived(app.compact || app.platform === "android" || app.platform === "ios");
  let user = $state(""), pw = $state(""), pw2 = $state("");
  let newHandle = $state(""), code = $state(""), unlock = $state("");

  const status = $derived.by(() => {
    if (!st) return "";
    let s = st.syncing ? t("account.sync.running") : st.sync_error ? st.sync_error : st.last_sync ? tf("account.sync.done", { when: whenMs(st.last_sync, t, tf) }) : t("account.sync.never");
    if (st.sync_pending > 0) s += ` · ${tf("account.sync.pending", { n: st.sync_pending })}`;
    return s;
  });
  const shownCode = $derived(ac.recoveryVisible ? ac.recovery : ac.recovery?.replace(/./g, (c) => (c === "-" ? "-" : "•")));
  const failed = $derived(ac.login.kind === "failed" ? ac.login.text : null);
  const ICON = "rgba(255,255,255,.6)";

  async function submit() { if (await A.submitPassword(user, pw, pw2)) { pw = ""; pw2 = ""; } }
</script>

<div class="col">
  {#if st?.merge_pending}
    <div class="pad">{t("account.merge.description")}</div>
    <div class="stack">
      <Button kind="primary" onclick={() => A.chooseMerge(true)}>{t("account.merge.yes")}</Button>
      <Button onclick={() => A.chooseMerge(false)}>{t("account.merge.no")}</Button>
      <Button onclick={A.signOut}>{t("account.signout")}</Button>
    </div>
    {#if ac.notice}<div class="nt warn">{ac.notice.text}</div>{/if}
  {:else if st && st.official && st.available === false}
    <!-- The public server always has accounts: it just could not be reached. -->
    <div class="pad dim">{t("account.unreachable")}</div>
    <div><Button onclick={A.pageOpened}>{t("common.retry")}</Button></div>
  {:else if st && st.available === false}
    <div class="pad dim">{t("account.unavailable")}</div>
  {:else if !st}
    <div class="pad"><Spin label={t("account.devices.loading")} /></div>
  {:else if !st.signed_in}
    <!-- ---------------------------------------------------------- signed out -->
    <div class="out">
      {#if ac.login.kind === "starting" || ac.login.kind === "waiting"}
        {#if ac.login.kind === "waiting"}
          {@const l = ac.login}
          <div class="wait">
            <div class="wt">{ac.pairingOnly ? t("account.login.pair_waiting") : t("account.login.waiting")}</div>
            <div class="mono codebox">{l.start.user_code}</div>
            {#if !ac.pairingOnly}<div class="hint">{t("account.login.pair_hint")}</div>{/if}
            {#if l.full}<div class="hint warnc">{t("account.login.full")}</div>{/if}
            <div class="wf">
              <Spin label={t("account.login.polling")} />
              <span class="fl"></span>
              {#if !ac.pairingOnly}<Button onclick={() => window.open(l.start.url, "_blank", "noopener")}>{t("account.login.reopen")}</Button>{/if}
              <Button onclick={A.cancelLogin}>{t("account.login.cancel")}</Button>
            </div>
          </div>
        {:else}
          <Spin label={t("account.login.starting")} />
        {/if}
      {:else if st.official && !ac.inApp}
        <!-- Just the two ways in, centred in the page. -->
        <div class="center" class:touch>
          <div class="ways" class:touch>
            {#if failed}<div class="fail c">{failed}</div>{/if}
            <button class="pb" onclick={() => A.signIn(true)}>{t("account.browser.button")}<Icon name="external-link" size={14} color="rgba(0,0,0,.6)" /></button>
            <Button onclick={() => { ac.inApp = true; ac.login = { kind: "idle" }; }} class="wide">{t("account.browser.in_app")}</Button>
          </div>
        </div>
      {:else}
        <div class="form">
          {#if st.official}
            <button class="back" onclick={() => { ac.inApp = false; ac.login = { kind: "idle" }; }}>
              <Icon name="arrow-left" size={14} color="rgba(255,255,255,.55)" />
              <span>{t("account.browser.back")}</span>
            </button>
          {/if}
          <Segmented stretch value={ac.register} options={[{ value: false, label: t("account.signin.button") }, { value: true, label: t("account.form.new") }]} onpick={A.setMode} />
          <Field bind:value={user} width="100%" height={38} icon="user-round" placeholder={t("account.form.handle")} onenter={submit} />
          {#if ac.register}
            <div class="pw-row">
              <Field bind:value={pw} width="100%" height={38} icon="lock" masked placeholder={t("account.form.password")} onenter={submit} />
              <Field bind:value={pw2} width="100%" height={38} masked placeholder={t("account.form.repeat")} onenter={submit} />
            </div>
            <div class="rules">{t("account.form.rules")}</div>
          {:else}
            <Field bind:value={pw} width="100%" height={38} icon="lock" masked placeholder={t("account.form.password")} onenter={submit} />
          {/if}
          {#if failed}<div class="fail">{failed}</div>{/if}
          <button class="pb" onclick={submit}>{ac.busy ? `| ${t("account.form.working")}` : ac.register ? t("account.form.create") : t("account.signin.button")}</button>
        </div>
        <div class="or"><span class="ln"></span><span>{t("account.form.or")}</span><span class="ln"></span></div>
        {#if st.official}
          <div class="form tight">
            <Button onclick={() => A.signIn(true)} class="wide">{t("account.passkey.button")}<Icon name="external-link" size={14} color={ICON} /></Button>
            <div class="sm">{t("account.passkey.desc")}</div>
          </div>
        {:else}
          <!-- Not the official server: a pairing code, and that server's own page only behind a warning. -->
          <div class="form tight">
            {#if ac.webWarning}
              <div class="warnbox">{t("account.web.warning")}</div>
              <div class="wb">
                <Button kind="danger" onclick={A.webConfirmed}>{t("account.web.continue")}</Button>
                <Button onclick={() => (ac.webWarning = false)}>{t("account.login.cancel")}</Button>
              </div>
            {:else}
              <Button onclick={() => A.signIn(false)} class="wide">{t("account.pair.button")}</Button>
              <div class="sm">{t("account.pair.button.desc")}</div>
              <button class="lnk c" onclick={() => (ac.webWarning = true)}>{t("account.web.link")}</button>
            {/if}
          </div>
        {/if}
      {/if}
    </div>
  {:else if sub === 1}
    <ProfileSettings />
  {:else if sub === 2}
    <!-- -------------------------------------------------------------- devices -->
    <div class="dvh">
      <SectionLabel text={tf("account.devices", { n: st.devices?.length ?? 0, max: st.device_limit })} />
      <div class="fl"></div>
      {#if st.devices?.length}
        <div class="dva"><Button kind="danger" icon="log-out" onclick={A.requestSignOutAll}>{t("account.devices.signout_all")}</Button></div>
      {/if}
    </div>
    <Rows>
      {#if st.devices_error && !st.devices}
        <div class="srow ret"><span class="warnc">{st.devices_error}</span><Button onclick={A.pageOpened}>{t("common.retry")}</Button></div>
      {:else if !st.devices}
        <div class="srow pad"><Spin label={t("account.devices.loading")} /></div>
      {:else}
        {#each st.devices as d (d.id)}
          <div class="srow">
            <RowBase label={d.name} status="{d.platform} · {whenMs(d.last_seen, t, tf)} · {d.approximate_location ? `≈ ${d.approximate_location}` : t('account.devices.location_unknown')}">
              {#snippet leading()}
                <span class="dicon" class:cur={d.current}>
                  <Icon name={platformIconFor(d.platform, d.name)} size={20} color={d.current ? "var(--accent)" : "rgba(255,255,255,.55)"} />
                </span>
              {/snippet}
              {#if d.current}<span class="dim12">{t("account.devices.this")}</span>{/if}
              <IconButton name="log-out" size={15} color="var(--danger)" label={t("account.devices.signout")} onclick={() => (d.current ? A.requestSignOut() : A.removeDevice(d.id))} />
            </RowBase>
          </div>
        {/each}
      {/if}
    </Rows>
    <SectionLabel text={t("account.pair.section")} />
    <Rows>
      <Row label={t("account.pair")} info={t("account.pair.desc")}>
        <Field bind:value={code} width={180} placeholder={t("account.pair.placeholder")} onenter={() => A.approve(code, () => (code = ""))} />
        <Button onclick={() => A.approve(code, () => (code = ""))}>{t("account.pair.approve")}</Button>
      </Row>
    </Rows>
  {:else if sub === 3}
    <!-- ------------------------------------------------------------- security -->
    <Rows>
      <Row label={st.has_password ? t("account.password.change") : t("account.password.set")} info={t("account.password.browser.desc")}>
        <Button onclick={() => A.openSecurity("password")}>{t("account.security.open_browser")}<Icon name="external-link" size={14} color={ICON} /></Button>
      </Row>
      <Row label={t("account.passkey.add")} info={t("account.passkey.add.desc")}>
        <Button onclick={() => A.openSecurity("passkey")}>{t("account.passkey.add.button")}<Icon name="external-link" size={14} color={ICON} /></Button>
      </Row>
      <Row label={t("account.recovery")} info={t("account.recovery.desc")}>
        <Button onclick={() => A.openSecurity("recovery")}>{t("account.security.open_browser")}<Icon name="external-link" size={14} color={ICON} /></Button>
      </Row>
    </Rows>
    <SectionLabel text={t("account.danger")} />
    <Rows>
      <Row label={t("account.delete")} info={t("account.delete.browser.desc")}>
        <Button kind="danger" onclick={() => A.openSecurity("delete")}>{t("account.delete.browser.button")}<Icon name="external-link" size={14} color="var(--danger)" /></Button>
      </Row>
    </Rows>
  {:else}
    <!-- ------------------------------------------------------------- overview -->
    <Rows>
      <Row label={`@${st.handle}`} status={status}>
        {#snippet leading()}
          <button
            class="pfp-btn"
            onclick={A.pickAvatar}
            disabled={ac.avatarBusy}
            use:tip={t(app.ownAvatarUrl ? "profile.image.change" : "profile.image.pick")}
            aria-label={t(app.ownAvatarUrl ? "profile.image.change" : "profile.image.pick")}
          >
            <Avatar src={app.ownAvatarUrl} size={40} flat />
            <span class="pfp-ov" class:busy={ac.avatarBusy}><Icon name="pencil" size={14} color="#fff" /></span>
          </button>
        {/snippet}
        <button
          class="sync-btn"
          onclick={A.syncNow}
          disabled={st.syncing}
          use:tip={t("account.sync.now")}
          aria-label={t("account.sync.now")}
        >
          <span class:spin={st.syncing}><Icon name="refresh-cw" size={14} color="rgba(255,255,255,.88)" /></span>
        </button>
        <Button kind="danger" icon="log-out" onclick={A.requestSignOut}>{t("account.signout")}</Button>
      </Row>
      <Row label={t("account.handle")} info={t("account.handle.desc")}>
        <Field bind:value={newHandle} width={200} placeholder={t("account.handle.placeholder")} onenter={() => A.rename(newHandle, () => (newHandle = ""))} />
        <Button onclick={() => A.rename(newHandle, () => (newHandle = ""))}>{t("account.handle.save")}</Button>
      </Row>
      <!-- Library: end-to-end encrypted with a key only the account's devices have, or why not (yet). -->
      {#if st.crypto === "encrypted"}
        <Row label={t("account.library")} info={t("account.library.encrypted.desc")}><span class="ok13">{t("account.library.encrypted")}</span></Row>
      {:else if st.crypto === "escrow" && st.has_password}
        <Row label={t("account.library")} info={t("account.library.escrow.pw_desc")}>
          <Field bind:value={unlock} width={180} masked placeholder={t("account.form.password")} onenter={() => A.unlockKeys(unlock, () => (unlock = ""))} />
          <Button onclick={() => A.unlockKeys(unlock, () => (unlock = ""))}>{t("account.library.activate")}</Button>
        </Row>
      {:else if st.crypto === "escrow"}
        <Row label={t("account.library")} status={t("account.library.escrow.nopw_desc")}><span class="dim13">{t("account.library.escrow")}</span></Row>
      {:else if st.crypto === "locked"}
        <Row label={t("account.library")} info={st.official ? t("account.library.locked.desc") : t("account.library.locked.desc_pair")}>
          <Field bind:value={unlock} width={180} masked placeholder={t("account.form.password")} onenter={() => A.unlockKeys(unlock, () => (unlock = ""))} />
          <Button onclick={() => A.unlockKeys(unlock, () => (unlock = ""))}>{t("account.library.unlock")}</Button>
        </Row>
      {:else}
        <Row label={t("account.library")} info={t("account.library.starting.desc")}><span class="dim13">{t("account.library.starting")}</span></Row>
      {/if}
      <Row label={t("account.glyph_links")} info={t("account.glyph_links.desc")}>
        <Tog on={app.config.sharing.glyph_links} onchange={(v) => app.setConfig((c) => { c.sharing.glyph_links = v; })} label={t("account.glyph_links")} />
      </Row>
    </Rows>
    {@render recovery()}
    <div class="priv">{t("account.privacy")}</div>
  {/if}
  {#if ac.notice && !st?.merge_pending}<div class="nt" class:warn={!ac.notice.ok} class:okc={ac.notice.ok}>{ac.notice.text}</div>{/if}
</div>

<!-- A fresh recovery code: dots until clicked, then the code; copy and save. -->
{#snippet recovery()}
  {#if ac.recovery}
    <div class="rc">
      <button class="mono codebox rcc" class:shown={ac.recoveryVisible} onclick={() => (ac.recoveryVisible = true)}>
        <span class="rct">{shownCode}</span>
        {#if !ac.recoveryVisible}<span class="rcs">{t("account.recovery.reveal")}</span>{/if}
      </button>
      <div class="rcf">
        <span class="once">{t("account.recovery.once")}</span>
        <Button onclick={() => (ac.recoveryVisible = !ac.recoveryVisible)}>{ac.recoveryVisible ? t("account.recovery.hide") : t("account.recovery.show")}<Icon name={ac.recoveryVisible ? "eye-off" : "eye"} size={14} color={ICON} /></Button>
        <Button onclick={() => A.copyRecovery(copyText)}>{t("account.recovery.copy")}<Icon name="copy" size={14} color={ICON} /></Button>
        <Button onclick={A.downloadRecovery}>{t("account.recovery.download")}<Icon name="download" size={14} color={ICON} /></Button>
      </div>
    </div>
  {/if}
{/snippet}

<style>
  .col { display: flex; flex-direction: column; }
  .pad { padding: 14px 0; font-size: 13px; }
  .pad.dim { color: rgba(255, 255, 255, .6); }
  .stack { display: flex; flex-direction: column; align-items: flex-start; gap: 10px; }
  .nt { padding-top: 8px; font-size: 12.5px; }
  .nt.warn, .warnc { color: var(--warn); }
  .nt.okc { color: var(--ok); }
  .fl { flex: 1; }
  .out { width: 100%; padding-top: 24px; display: flex; flex-direction: column; align-items: center; }
  .center { width: 100%; min-height: 400px; display: flex; flex-direction: column; align-items: center; justify-content: center; margin-top: -24px; }
  .center.touch { min-height: 0; justify-content: flex-start; margin-top: 0; align-items: stretch; }
  .ways { width: 320px; max-width: 100%; display: flex; flex-direction: column; gap: 10px; }
  .ways.touch { width: 100%; }
  .ways :global(.wide), .form :global(.wide) { width: 100%; height: 40px; }
  .pb { height: 38px; width: 100%; display: flex; align-items: center; justify-content: center; gap: 6px; border-radius: 8px; background: var(--text); color: #000; font-size: var(--sm); font-weight: 600; transition: opacity .12s; }
  .ways .pb { height: 40px; }
  .pb:hover { opacity: .88; }
  .form { width: 360px; max-width: 100%; display: flex; flex-direction: column; gap: 10px; }
  .back { align-self: flex-start; display: inline-flex; align-items: center; gap: 6px; padding: 2px 0 4px; font-size: 12.5px; color: rgba(255, 255, 255, .55); transition: color .12s; }
  .back:hover { color: rgba(255, 255, 255, .92); }
  .pw-row { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .pw-row :global(.box) { min-width: 0; }
  @media (max-width: 420px) { .pw-row { grid-template-columns: 1fr; } }
  .form.tight { gap: 8px; }
  .rules { font-size: 12px; color: rgba(255, 255, 255, .5); }
  .fail { font-size: 12.5px; color: var(--warn); }
  .fail.c { text-align: center; }
  .or { width: 360px; max-width: 100%; padding: 14px 0; display: flex; align-items: center; gap: 10px; font-size: 12px; color: rgba(255, 255, 255, .45); }
  .ln { flex: 1; height: 1px; background: rgba(255, 255, 255, .08); }
  .sm { text-align: center; font-size: 12px; color: rgba(255, 255, 255, .5); }
  .lnk { padding-top: 14px; font-size: 12.5px; color: rgba(255, 255, 255, .5); transition: color .12s; }
  .lnk.c { padding-top: 10px; text-align: center; }
  .lnk:hover { color: rgba(255, 255, 255, .9); }
  .warnbox { padding: 12px; border-radius: 8px; background: color-mix(in srgb, var(--warn) 12%, transparent); font-size: 12.5px; color: var(--warn); }
  .wb { padding-top: 4px; display: flex; gap: 8px; }
  .wait { width: 420px; max-width: 100%; display: flex; flex-direction: column; }
  .wt { font-size: 13px; color: rgba(255, 255, 255, .75); }
  .codebox { margin: 8px 0; padding: 14px 0; display: flex; justify-content: center; border-radius: 8px; border: 1px dashed rgba(255, 255, 255, .25); font-size: 24px; color: #fff; }
  .hint { font-size: 12.5px; color: rgba(255, 255, 255, .55); }
  .hint.warnc { padding-top: 8px; color: var(--warn); }
  .wf { padding-top: 12px; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .ret { display: flex; align-items: center; gap: 12px; padding: 14px 0; font-size: 13px; }
  .dim12 { font-size: 12.5px; color: rgba(255, 255, 255, .5); }
  .dim13 { font-size: 13px; color: rgba(255, 255, 255, .55); }
  .ok13 { font-size: 13px; color: var(--ok); }
  .priv { padding-top: 14px; font-size: 12px; color: rgba(255, 255, 255, .4); }
  .rc { display: flex; flex-direction: column; }
  .rcc { flex-direction: column; align-items: center; gap: 4px; font-size: 22px; color: rgba(255, 255, 255, .55); transition: background-color .12s; }
  .rcc:not(.shown):hover { background: rgba(255, 255, 255, .03); }
  .rcc.shown { color: #fff; cursor: default; }
  .rcs { font-family: var(--font); font-size: 12px; color: rgba(255, 255, 255, .5); }
  .rcf { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .once { flex: 1 1 200px; font-size: 12.5px; color: var(--warn); }
  .dicon { width: 40px; height: 40px; border-radius: 8px; background: rgba(255, 255, 255, .04); display: grid; place-items: center; }
  .dicon.cur { background: color-mix(in srgb, var(--accent) 14%, transparent); }
  .dvh { display: flex; align-items: center; gap: 10px; }
  .dva { padding-top: 16px; }
  .pfp-btn { position: relative; flex: none; width: 40px; height: 40px; border-radius: 50%; overflow: hidden; cursor: pointer; padding: 0; }
  .pfp-btn:disabled { cursor: wait; }
  .pfp-ov { position: absolute; inset: 0; display: grid; place-items: center; background: rgba(0, 0, 0, .55); opacity: 0; transition: opacity .12s ease-out; pointer-events: none; }
  .pfp-btn:hover .pfp-ov, .pfp-btn:focus-visible .pfp-ov, .pfp-ov.busy { opacity: 1; }
  .sync-btn { flex: none; width: 32px; height: 32px; display: grid; place-items: center; border-radius: 8px; background: rgba(255, 255, 255, .09); transition: background-color .12s ease-out, opacity .12s ease-out; }
  .sync-btn:hover { background: rgba(255, 255, 255, .14); }
  .sync-btn:active { background: rgba(255, 255, 255, .19); }
  .sync-btn:disabled { opacity: .6; pointer-events: none; }
  .sync-btn .spin { display: inline-flex; animation: sync-spin 1s linear infinite; }
  @keyframes sync-spin { to { transform: rotate(360deg); } }
</style>
