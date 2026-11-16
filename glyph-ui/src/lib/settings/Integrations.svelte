<script lang="ts">
  // settings/server.rs integrations + mcp_section: services on this
  // computer Glyph can talk to. Discord rich presence (switch + application
  // id); MCP for AI assistants: switch, mode (this computer / through the
  // server), access per area (none / read / read and change), address, token
  // (copy, copy config, rotate or create) and a status line.
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import RowBase from "./RowBase.svelte";
  import SectionLabel from "./SectionLabel.svelte";
  import Tog from "./Tog.svelte";
  import Segmented from "./Segmented.svelte";
  import Field from "./Field.svelte";
  import Spin from "./Spin.svelte";
  import PluginHost from "./PluginHost.svelte";
  import McpModeVisual from "./McpModeVisual.svelte";
  import { copyText, openUrl, whenMs } from "./util";
  import { onWeb } from "../web";
  import type { McpInfo, McpSession, ServerInfo } from "./types";

  let { sub = 0 }: { sub?: number } = $props();
  // Web player only exposes AI assistants (Discord is desktop-only).
  const tab = $derived(onWeb() ? 1 : sub);
  type Access = "none" | "read" | "write";
  const mcp = $derived(app.config.mcp as typeof app.config.mcp & { access?: Partial<Record<string, Access>> });
  const signedIn = $derived(app.account != null);
  const AREAS: [string, string, string, boolean][] = [
    ["playback", "sp.mcp.access.playback", "sp.mcp.access.playback.desc", true],
    ["library", "sp.mcp.access.library", "sp.mcp.access.library.desc", true],
    ["settings", "sp.mcp.access.settings", "sp.mcp.access.settings.desc", true],
    ["stats", "sp.mcp.access.stats", "sp.mcp.access.stats.desc", false],
  ];
  const DEFAULTS: Record<string, Access> = { playback: "write", library: "read", settings: "write", stats: "none" };

  let builtin = $state(false);
  let info = $state<McpInfo>({ url: null, token: null, config: null, error: null, relay_running: false, token_loading: false });
  let discordId = $state(app.config.integrations.discord_client_id);
  let tokenName = $state("");
  let lastNonNoneSettings = $state<"read" | "write">("write");

  let sessions = $state<McpSession[] | null>(null);
  let editingId = $state<string | null>(null);
  let editName = $state("");

  async function loadInfo() {
    const i = await backend().call<McpInfo>("mcp_info");
    if (i) info = i;
  }

  async function loadSessions() {
    if (!signedIn || !mcp.enabled || (local && !onWeb())) {
      sessions = null;
      return;
    }
    try {
      const list = await backend().call<McpSession[]>("mcp_sessions");
      if (list) sessions = list;
    } catch {
      // ignore
    }
  }

  onMount(() => {
    if (!onWeb()) backend().call<ServerInfo>("server_info").then((s) => (builtin = !!s?.discord_builtin));
    loadInfo();
    loadSessions();
    const id = setInterval(loadInfo, 2000);
    const sid = setInterval(loadSessions, 5000);
    return () => {
      clearInterval(id);
      clearInterval(sid);
    };
  });

  $effect(() => {
    if (signedIn && mcp.enabled && (!local || onWeb())) {
      loadSessions();
    }
  });

  const custom = $derived(app.config.integrations.discord_client_id.trim() !== "");
  const idDesc = $derived(!custom && builtin ? t("sp.discord.id.builtin") : t("sp.discord.id.desc"));
  const local = $derived(!onWeb() && mcp.mode !== "server");
  const modeDesc = $derived(!local ? (signedIn ? t("sp.mcp.server.desc") : t("sp.mcp.server.signin")) : t("sp.mcp.local.desc"));
  const access = (area: string): Access => mcp.access?.[area] ?? DEFAULTS[area];
  const status = $derived(info.error ? `${t("sp.mcp.problem")} ${info.error}` : !local && info.relay_running ? t("sp.mcp.relay_on") : local ? t("sp.mcp.running") : !local && !signedIn ? t("sp.mcp.server.signin") : t("sp.mcp.idle"));

  async function copy(text: string | null) {
    if (text && (await copyText(text))) app.toast(t("sp.mcp.copied"));
  }

  async function rotate() {
    if (local) {
      await backend().call("mcp_rotate_token");
    } else {
      await backend().call("mcp_server_token", { name: tokenName.trim() || undefined });
      tokenName = "";
    }
    await loadInfo();
    await loadSessions();
  }

  async function create() {
    info.token_loading = true;
    await backend().call("mcp_server_token", { name: tokenName.trim() || undefined });
    tokenName = "";
    await loadInfo();
    await loadSessions();
  }

  function openWeb() {
    const origin = info.url ? new URL(info.url).origin : (app.config.server_url?.trim() || "https://glyph.my");
    openUrl(`${origin}/web#settings/integrations`);
  }

  function startRename(s: McpSession) {
    editingId = s.id;
    editName = s.name;
  }

  async function saveRename(id: string) {
    const name = editName.trim();
    if (!name) return;
    try {
      await backend().call("mcp_rename_session", { id, name });
      app.toast(t("sp.mcp.sessions.renamed"));
      editingId = null;
      await loadSessions();
    } catch (e) {
      app.toast(String(e));
    }
  }

  async function revoke(id: string) {
    try {
      await backend().call("mcp_revoke_session", { id });
      app.toast(t("sp.mcp.sessions.revoked"));
      await loadSessions();
    } catch (e) {
      app.toast(String(e));
    }
  }

  function sessionStatus(s: McpSession): string {
    const parts: string[] = [];
    if (s.client_name && s.client_name !== s.name) {
      parts.push(s.client_name);
    }
    if (s.site) {
      parts.push(s.site);
    }
    parts.push(s.kind === "oauth" ? t("sp.mcp.sessions.oauth") : t("sp.mcp.sessions.token"));
    const ts = s.last_used_at ?? s.created_at;
    if (ts) {
      parts.push(whenMs(ts, t, tf));
    }
    return parts.join(" · ");
  }
</script>

<PluginHost page="integrations">
  <div class="col">
    {#if onWeb() && !signedIn}
      <div class="out">
        <p class="notice">{t("sp.mcp.signed_out_notice")}</p>
        <Button onclick={() => app.openSettings("account")}>{t("account.signin.button")}</Button>
      </div>
    {:else if tab === 0}
      <Rows>
        <Row label={t("sp.discord")} info={t("sp.discord.desc")}>
          <Tog on={app.config.integrations.discord_presence} onchange={(v) => app.setConfig((c) => { c.integrations.discord_presence = v; })} label={t("sp.discord")} />
        </Row>
        <Row label={t("sp.discord.id")} info={idDesc}>
          <Field bind:value={discordId} onenter={(v) => app.setConfig((c) => { c.integrations.discord_client_id = v.trim(); })} onblur={(v) => v.trim() !== app.config.integrations.discord_client_id && app.setConfig((c) => { c.integrations.discord_client_id = v.trim(); })} label={t("sp.discord.id")} />
        </Row>
      </Rows>
    {:else}
      <Rows>
        <Row label={t("sp.mcp")} info={onWeb() ? modeDesc : t("sp.mcp.desc")}>
          <Tog on={mcp.enabled} onchange={(v) => app.setConfig((c) => { c.mcp.enabled = v; })} label={t("sp.mcp")} />
        </Row>
      </Rows>
      {#if mcp.enabled}
        {#if !onWeb()}
          <div class="vis">
            <McpModeVisual />
          </div>
        {/if}

        <SectionLabel text={t("sp.mcp.access")} />
        <Rows>
          {#each AREAS as [area, label, desc, write] (area)}
            <Row label={t(label)} info={t(desc)}>
              {#if area === "settings"}
                <div class="row-controls">
                  <Tog
                    on={access("settings") !== "none"}
                    onchange={(on) => {
                      const next = on ? lastNonNoneSettings : "none";
                      app.setConfig((c) => {
                        const m = c.mcp as typeof mcp;
                        m.access = { ...(m.access ?? {}), settings: next };
                      });
                    }}
                    label={t("sp.mcp.access.settings")}
                  />
                  <Segmented
                    value={access(area)}
                    options={[
                      { value: "none" as Access, label: t("sp.mcp.access.none") },
                      { value: "read" as Access, label: t("sp.mcp.access.read") },
                      { value: "write" as Access, label: t("sp.mcp.access.write") }
                    ]}
                    onpick={(v) => {
                      if (v !== "none") lastNonNoneSettings = v;
                      app.setConfig((c) => {
                        const m = c.mcp as typeof mcp;
                        m.access = { ...(m.access ?? {}), [area]: v };
                      });
                    }}
                  />
                </div>
              {:else}
                <Segmented
                  value={access(area)}
                  options={[
                    { value: "none" as Access, label: t("sp.mcp.access.none") },
                    { value: "read" as Access, label: t("sp.mcp.access.read") },
                    ...(write ? [{ value: "write" as Access, label: t("sp.mcp.access.write") }] : [])
                  ]}
                  onpick={(v) =>
                    app.setConfig((c) => {
                      const m = c.mcp as typeof mcp;
                      m.access = { ...(m.access ?? {}), [area]: v };
                    })}
                />
              {/if}
            </Row>
          {/each}
        </Rows>

        <SectionLabel text={t("sp.mcp.connection")} />
        <Rows>
          {#if info.url}
            <Row label={t("sp.mcp.url")} info={info.url}>
              <Button onclick={() => copy(info.url)}>{t("sp.mcp.copy")}</Button>
            </Row>
          {/if}
          {#if info.url && info.token}
            <Row label={t("sp.mcp.token")} info={t("sp.mcp.token.desc")}>
              <Button onclick={() => copy(info.token)}>{t("sp.mcp.copy_token")}</Button>
              <Button onclick={() => copy(info.config)}>{t("sp.mcp.copy_config")}</Button>
              {#if !local && signedIn}
                <Field bind:value={tokenName} placeholder={t("sp.mcp.token.name_placeholder")} width={180} />
              {/if}
              <Button onclick={rotate}>{t("sp.mcp.rotate")}</Button>
              {#if !onWeb() && !local}
                <Button onclick={openWeb}>{t("sp.mcp.open_web")}</Button>
              {/if}
            </Row>
          {:else if !local && signedIn}
            <Row label={t("sp.mcp.token")} info={t("sp.mcp.token.server_desc")}>
              <Field bind:value={tokenName} placeholder={t("sp.mcp.token.name_placeholder")} width={180} />
              <Button onclick={create}>{info.token_loading ? t("sp.mcp.creating") : t("sp.mcp.create_token")}</Button>
              {#if !onWeb()}
                <Button onclick={openWeb}>{t("sp.mcp.open_web")}</Button>
              {/if}
            </Row>
          {/if}
          <div class="srow stat" class:warn={!!info.error}>{status}</div>
        </Rows>

        {#if signedIn && (!local || onWeb())}
          <SectionLabel text={t("sp.mcp.sessions")} />
          <Rows>
            {#if sessions === null}
              <div class="srow pad"><Spin label={t("sp.mcp.sessions.loading")} /></div>
            {:else if sessions.length === 0}
              <div class="srow dim13">{t("sp.mcp.sessions.empty")}</div>
            {:else}
              {#each sessions as s (s.id)}
                <div class="srow">
                  {#if editingId === s.id}
                    <div class="inline-edit">
                      <Field bind:value={editName} placeholder={t("sp.mcp.sessions.rename")} width={180} onenter={() => saveRename(s.id)} />
                      <Button onclick={() => saveRename(s.id)}>{t("sp.mcp.sessions.save")}</Button>
                      <Button onclick={() => (editingId = null)}>{t("dialog.cancel")}</Button>
                    </div>
                  {:else}
                    <RowBase label={s.name} status={sessionStatus(s)}>
                      {#snippet leading()}
                        <span class="dicon">
                          <Icon name="bot" size={20} color="rgba(255,255,255,.55)" />
                        </span>
                      {/snippet}
                      <IconButton name="pencil" size={14} label={t("sp.mcp.sessions.rename")} onclick={() => startRename(s)} />
                      <IconButton name="trash-2" size={15} color="var(--danger)" label={t("sp.mcp.sessions.revoke")} onclick={() => revoke(s.id)} />
                    </RowBase>
                  {/if}
                </div>
              {/each}
            {/if}
          </Rows>
        {/if}
      {/if}
    {/if}
  </div>
</PluginHost>

<style>
  .col { display: flex; flex-direction: column; }
  .vis { margin-top: 8px; }
  .stat { padding: 8px 2px; font-size: 12.5px; color: rgba(255, 255, 255, .55); }
  .stat.warn { color: var(--warn); }
  .row-controls { display: flex; align-items: center; gap: 12px; }
  .out { display: flex; flex-direction: column; align-items: flex-start; gap: 12px; padding: 20px 0; }
  .notice { font-size: 13.5px; color: rgba(255, 255, 255, .7); margin: 0; line-height: 1.5; }
  .dicon { width: 40px; height: 40px; border-radius: 8px; background: rgba(255, 255, 255, .04); display: grid; place-items: center; }
  .pad { padding: 14px 0; font-size: 13px; }
  .dim13 { font-size: 13px; color: rgba(255, 255, 255, .55); padding: 10px 0; }
  .inline-edit { display: flex; align-items: center; gap: 8px; padding: 10px 0; width: 100%; }
</style>
