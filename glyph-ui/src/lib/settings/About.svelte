<script lang="ts">
  // settings/about.rs about + support: Settings > About. Sub-tabs Version,
  // Support, Shortcuts. Version: a card (surface, centred, py 20) with the
  // mark, "Glyph" (20 bold) and the version chip (h 20, radius 6, white 8 %,
  // green dot 6, mono 11.5), Check for updates / Install / Restart buttons,
  // the status line (mono 11.5) and links (GitHub, Releases, Support); then
  // auto update, channel and the introduction (opens onboarding).
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import Surface from "./Surface.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import Tog from "./Tog.svelte";
  import Segmented from "./Segmented.svelte";
  import LinkButton from "./LinkButton.svelte";
  import PluginHost from "./PluginHost.svelte";
  import Spin from "./Spin.svelte";
  import Support from "./Support.svelte";
  import Shortcuts from "./Shortcuts.svelte";
  import AppOnly from "./AppOnly.svelte";
  import NoteLogo from "../overlays/NoteLogo.svelte";
  import { onWeb } from "../web";
  import { copyText, errText, isMobile } from "./util";
  import type { UpdateState } from "./types";
  let { sub = 0 }: { sub?: number } = $props();

  const REPO_URL = "https://github.com/glyphmusic/apps";
  const RELEASES_URL = "https://github.com/glyphmusic/apps/releases";
  const ISSUES_URL = "https://github.com/glyphmusic/apps/issues/new";

  let u = $state({ checking: false, installing: null as [number, number] | null, installed: false, error: null as string | null, status: { kind: "none" } as UpdateState });
  const android = $derived(app.platform === "android");
  /** The web player is always the newest version: no updates to check. */
  const web = onWeb();

  async function sync() {
    const s = await backend().call<typeof u | undefined>("update_state");
    if (s) Object.assign(u, s);
  }
  onMount(() => { if (web) return; sync(); const id = setInterval(() => u.installing && sync(), 700); return () => clearInterval(id); });

  async function check() {
    u.checking = true; u.error = null;
    try { u.status = (await backend().call<UpdateState>("update_check", { manual: true })) ?? u.status; }
    catch (e) { u.error = errText(e); }
    u.checking = false;
  }
  async function install() {
    u.installing = [0, 0]; u.error = null;
    try { await backend().call("update_install"); u.installed = true; if (android) app.toastInfo(t("update.android_confirm")); }
    catch (e) { u.error = errText(e); }
    u.installing = null;
    sync();
  }
  const restart = () => backend().call("update_restart");

  const status = $derived.by((): [string, string] => {
    if (u.checking) return [t("update.checking"), "rgba(255,255,255,.6)"];
    if (u.installing) { const [d, n] = u.installing; return [tf("update.downloading", { p: n > 0 ? Math.floor((d * 100) / n) : 0 }), "#fff"]; }
    if (u.installed) return [t(android ? "update.android_confirm" : "update.installed"), "var(--ok)"];
    if (u.error) return [tf("update.error", { e: u.error }), "var(--warn)"];
    const s = u.status;
    if (s.kind === "up_to_date") return [tf("update.up_to_date", { v: s.current }), "var(--ok)"];
    if (s.kind === "skipped") return [t("update.skipped"), "rgba(255,255,255,.55)"];
    if (s.kind === "available" && s.downgrade) return [tf("update.back_to_stable", { v: s.version.replace(/^v/, "") }), "rgba(255,255,255,.85)"];
    if (s.kind === "available") return [tf("update.badge", { v: s.version.replace(/^v/, "") }), "var(--ok)"];
    return [t("update.not_checked"), "rgba(255,255,255,.55)"];
  });
  const avail = $derived(u.status.kind === "available" ? u.status : null);
  const installable = $derived(!!avail && avail.can_self_install && (!u.installed || android) && !u.installing);
  const prerelease = $derived(app.config.updates.channel === "prerelease");
  const manual = $derived(avail && !avail.can_self_install
    ? tf(avail.downgrade && android ? "update.back_to_stable_android" : avail.release_package ? "update.release_package" : "update.package_manager", { url: avail.html_url })
    : null);

  /** Switching the channel checks right away: back on Stable after a
   * Latest build, the newest stable version can then be installed. */
  async function setChannel(latest: boolean) {
    await app.setConfig((c) => { c.updates.channel = latest ? "prerelease" : "stable"; });
    if (!u.installing && !u.installed) void check();
  }

  function intro() { app.settingsOpen = false; app.onboarding = true; }
  void copyText;
</script>

{#if sub === 1}
  <Support />
{:else if sub === 2 && !isMobile()}
  <Shortcuts />
{:else}
  <PluginHost page="about">
    <div class="col">
      <Surface gap={10} pad={20}>
        <div class="card">
          <NoteLogo rowH={4} />
          <div class="nm"><span class="g">Glyph</span><span class="ver"><span class="dot"></span><span class="mono">v{app.version}</span></span></div>
          <div class="acts">
            {#if !web}<button class="chk" onclick={check} disabled={u.checking}><Icon name="refresh-cw" size={14} color="rgba(255,255,255,.85)" /><span>{t("update.check")}</span></button>{/if}
            {#if installable}<Button kind="primary" onclick={install}>{avail?.downgrade ? tf("update.install_stable", { v: avail.version.replace(/^v/, "") }) : t("update.install")}</Button>{/if}
            {#if u.installed && !android}<Button kind="primary" onclick={restart}>{t("update.restart")}</Button>{/if}
          </div>
          {#if !web}<div class="mono st" style:color={status[1]}>{#if u.checking}<Spin label={status[0]} />{:else}{status[0]}{/if}</div>{/if}
          <div class="links">
            <LinkButton icon="github" label="GitHub" url={REPO_URL} />
            <LinkButton icon="external-link" label={t("sp.link.releases")} url={RELEASES_URL} />
            <LinkButton icon="life-buoy" label={t("sp.support")} url={ISSUES_URL} />
          </div>
        </div>
      </Surface>
      <div class="gap"></div>
      <Rows>
        {#if web}<AppOnly />{:else}
        <Row label={t("update.auto")} info={t("sp.about.auto.desc")}>
          <Tog on={app.config.updates.auto_install} onchange={(v) => app.setConfig((c) => { c.updates.auto_install = v; })} label={t("update.auto")} />
        </Row>
        <Row label={t("update.channel")} info={t("sp.about.channel.latest_desc")}>
          <Segmented value={prerelease} options={[{ value: false, label: t("update.channel.stable") }, { value: true, label: t("update.channel.latest") }]}
            onpick={setChannel} />
        </Row>
        {/if}
        <Row label={t("sp.about.intro")} info={t("sp.about.intro.desc")}>
          <Button onclick={intro}>{t("sp.about.intro.open")}</Button>
        </Row>
        {#if manual}<div class="srow man">{manual}</div>{/if}
      </Rows>
    </div>
  </PluginHost>
{/if}

<style>
  .col { display: flex; flex-direction: column; }
  .gap { height: 10px; }
  .card { display: flex; flex-direction: column; align-items: center; gap: 10px; }
  .nm { display: flex; align-items: center; gap: 8px; }
  .g { font-size: 20px; font-weight: 700; color: #fff; }
  .ver { height: 20px; padding: 0 7px; display: flex; align-items: center; gap: 5px; border-radius: 6px; background: rgba(255, 255, 255, .08); }
  .ver .mono { font-size: 11.5px; color: rgba(255, 255, 255, .85); }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--ok); }
  .acts { padding-top: 6px; display: flex; flex-wrap: wrap; justify-content: center; align-items: center; gap: 8px; }
  .chk { height: 34px; padding: 0 14px; display: flex; align-items: center; gap: 8px; border-radius: 8px; background: rgba(255, 255, 255, .08); font-size: 13.5px; color: #fff; transition: background-color .12s; }
  .chk:hover:not(:disabled) { background: rgba(255, 255, 255, .13); }
  .chk:disabled { opacity: .6; cursor: default; }
  .st { font-size: 11.5px; text-align: center; }
  .links { padding-top: 14px; display: flex; flex-wrap: wrap; justify-content: center; gap: 8px; }
  .man { padding: 12px 0; font-size: 12.5px; color: rgba(255, 255, 255, .55); }
</style>
