<script lang="ts">
  // settings/server.rs server + privacy: sub-tabs Connection, API keys,
  // Transparency. Connection: server URL, the status line as a row label
  // (connected to ..., direct, incompatible, offline) with its test result
  // under it (green ok / yellow failed) and Test / Apply; a bearer token row
  // when the server asks for one. Keys: masked fields saved on Enter or when
  // left. Transparency: what this server can see (from the app's own
  // behaviour, never from what the server says) and what it noticed.
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import { tip } from "../ui.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import RowBase from "./RowBase.svelte";
  import SectionLabel from "./SectionLabel.svelte";
  import Field from "./Field.svelte";
  import Spin from "./Spin.svelte";
  import PluginHost from "./PluginHost.svelte";
  import { errText, whenMs } from "./util";
  import type { ConnTestResult, PrivacyReport, ServerInfo } from "./types";
  let { sub = 0 }: { sub?: number } = $props();

  const DEFAULT_URL = "https://api.glyph.my";
  const KEYS = ["youtube", "jamendo_client_id", "soundcloud_client_id", "spotify_client_id", "spotify_client_secret", "lastfm"];

  let url = $state(app.config.server_url);
  let bearer = $state("");
  let info = $state<ServerInfo | null>(null);
  let conn = $state<{ kind: "running" | "ok" | "failed"; text: string } | null>(null);
  let authRequired = $state(false);
  let keys = $state<Record<string, string>>(Object.fromEntries(KEYS.map((k) => [k, app.config.keys[k] ?? ""])));
  let report = $state<PrivacyReport | null>(null);

  async function loadInfo() { info = (await backend().call<ServerInfo>("server_info")) ?? null; authRequired = !!info?.auth_required; }
  async function loadReport() { report = (await backend().call<PrivacyReport>("privacy_report")) ?? report; }
  onMount(() => {
    loadInfo();
    if (sub === 2) { loadReport(); const id = setInterval(loadReport, 3000); return () => clearInterval(id); }
  });
  $effect(() => {
    void app.backendState;
    loadInfo();
  });

  const modeText = $derived.by(() => {
    const s = info ?? ({ kind: "offline", auth_required: false } as ServerInfo);
    if (app.backendState.kind === "connecting") return "";
    if (s.kind === "server") {
      if (s.version === "offline") return t("status.backend_offline");
      const u = (s.url ?? "").replace(/\/+$/, "");
      const name = u === DEFAULT_URL ? t("settings.server.public") : u.split("://")[1] ?? u;
      return tf("settings.server.connected", { name, v: s.version ?? "" });
    }
    if (s.kind === "direct") return t("status.direct");
    if (s.kind === "incompatible") return tf("status.incompatible", { v: s.protocol ?? 0 });
    return `${t("status.backend_offline")} ${s.reason ?? ""}`;
  });
  const connColor = $derived(conn?.kind === "ok" ? "var(--ok)" : conn?.kind === "failed" ? "var(--warn)" : "rgba(255,255,255,.6)");

  async function test() {
    const u = url.trim();
    if (!u) { conn = { kind: "ok", text: t("settings.server.direct") }; return; }
    conn = { kind: "running", text: t("settings.server.testing") };
    try {
      const r = await backend().call<ConnTestResult>("server_test", { url: u });
      if (r) { authRequired = !!r.auth_required; conn = { kind: r.ok ? "ok" : "failed", text: r.text }; }
    } catch (e) { conn = { kind: "failed", text: errText(e) }; }
  }
  async function apply() {
    app.setConfig((c) => { c.server_url = url.trim(); });
    try {
      if (authRequired && bearer.trim()) { await backend().call("server_store_token", { url: url.trim() || DEFAULT_URL, token: bearer.trim() }); bearer = ""; }
      await backend().call("retry_backend");
    } catch (e) { conn = { kind: "failed", text: errText(e) }; }
    loadInfo();
  }
  function saveKey(name: string, v: string) {
    const val = v.trim();
    if ((app.config.keys[name] ?? "") !== val) app.setConfig((c) => { c.keys[name] = val || null; });
  }
  async function clearEvents() { await backend().call("privacy_clear"); loadReport(); }

  const lineColor = (l: string) => (l === "encrypted" || l === "local" ? "var(--ok)" : l === "decryptable" ? "var(--warn)" : "rgba(255,255,255,.6)");
  const eventText = (e: { code: string; detail: string }) => tf(`privacy.event.${e.code}`, { detail: e.detail });
</script>

{#if sub === 2}
  <div class="col">
    <SectionLabel text={t("privacy.title")} />
    <div class="lead">{t("privacy.lead")}</div>
    {#if !report}
      <div class="lead"><Spin /></div>
    {:else}
      <Rows>
        {#each report.lines as l (l.area)}
          {@const note = l.note === "no_password" && report.has_password ? "enter_password" : l.note}
          <Row label={t(`privacy.area.${l.area}`)} info={note ? t(`privacy.note.${note}`) : null}>
            <span class="lv" style:color={lineColor(l.level)}>{t(`privacy.level.${l.level}`)}</span>
          </Row>
        {/each}
      </Rows>
      <div class="evh">
        <SectionLabel text={t("privacy.events")} />
        <div class="fl"></div>
        {#if report.events.length}<div class="clr"><Button onclick={clearEvents}>{t("privacy.events.clear")}</Button></div>{/if}
      </div>
      {#if !report.events.length}
        <div class="none">{t("privacy.events.none")}</div>
      {:else}
        <Rows>
          {#each report.events.slice(0, 20) as e (e.at + e.code)}
            <div class="srow">
              <RowBase label={eventText(e)} status={whenMs(e.at, t, tf)}>
                <span class="ev" style:color={e.alarm ? "var(--warn)" : "rgba(255,255,255,.5)"}>{e.alarm ? t("privacy.alarm") : t("privacy.info")}</span>
              </RowBase>
            </div>
          {/each}
        </Rows>
      {/if}
    {/if}
  </div>
{:else if sub === 1}
  <div class="col">
    <div class="kh">
      <SectionLabel text={t("settings.keys")} />
      <span class="ki" use:tip={{ label: t("settings.keys.hint"), wrap: true }}><Icon name="info" size={13} color="rgba(255,255,255,.42)" /></span>
    </div>
    <Rows>
      {#each KEYS as k (k)}
        <Row label={t(`settings.keys.${k}`)}>
          <Field bind:value={keys[k]} masked placeholder={t("settings.keys.unset")} onenter={(v) => saveKey(k, v)} onblur={(v) => saveKey(k, v)} label={t(`settings.keys.${k}`)} />
        </Row>
      {/each}
    </Rows>
  </div>
{:else}
  <Rows>
    <Row label={t("settings.server.url")} info={t("sp.server.url.desc")}>
      <Field bind:value={url} placeholder={t("settings.server.placeholder")} onenter={apply} label={t("settings.server.url")} />
    </Row>
    {#if authRequired}
      <Row label={t("settings.server.bearer")} info={t("settings.server.bearer.required")}>
        <Field bind:value={bearer} masked placeholder={t("settings.server.bearer.placeholder")} label={t("settings.server.bearer")} />
      </Row>
    {/if}
    <div class="srow">
      <RowBase label={modeText || t("status.connecting")} status={conn?.text} statusColor={connColor}>
        <Button onclick={test}>{t("settings.server.test")}</Button>
        <Button onclick={apply}>{t("settings.server.apply")}</Button>
      </RowBase>
      {#if app.backendState.kind === "connecting"}<div class="cn"><Spin label={t("status.connecting")} /></div>{/if}
    </div>
  </Rows>
{/if}

<style>
  .col { display: flex; flex-direction: column; }
  .lead { padding: 0 2px 8px; font-size: 12.5px; color: rgba(255, 255, 255, .55); }
  .lv { font-size: 13px; }
  .evh { display: flex; align-items: center; gap: 10px; }
  .fl { flex: 1; }
  .clr { padding-top: 16px; }
  .none { padding: 8px 2px; font-size: 12.5px; color: rgba(255, 255, 255, .5); }
  .ev { font-size: 12.5px; }
  .kh { display: flex; align-items: center; gap: 6px; }
  .ki { margin-top: 16px; width: 18px; height: 18px; border-radius: 9px; display: grid; place-items: center; }
  .ki:hover { background: rgba(255, 255, 255, .08); }
  .cn { padding: 0 0 12px; }
</style>
