<script lang="ts">
  // settings/server.rs integrations + mcp_section: services on this
  // computer Glyph can talk to. Discord rich presence (switch + application
  // id); MCP for AI assistants: switch, mode (this computer / through the
  // server), access per area (none / read / read and change), address, token
  // (copy, copy config, rotate or create) and a status line.
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t } from "../i18n.svelte";
  import Button from "../components/Button.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import SectionLabel from "./SectionLabel.svelte";
  import Tog from "./Tog.svelte";
  import Segmented from "./Segmented.svelte";
  import Field from "./Field.svelte";
  import PluginHost from "./PluginHost.svelte";
  import McpModeVisual from "./McpModeVisual.svelte";
  import { copyText } from "./util";
  import { onWeb } from "../web";
  import type { McpInfo, ServerInfo } from "./types";

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

  async function loadInfo() { const i = await backend().call<McpInfo>("mcp_info"); if (i) info = i; }
  onMount(() => {
    if (!onWeb()) backend().call<ServerInfo>("server_info").then((s) => (builtin = !!s?.discord_builtin));
    loadInfo();
    const id = setInterval(loadInfo, 2000);
    return () => clearInterval(id);
  });

  const custom = $derived(app.config.integrations.discord_client_id.trim() !== "");
  const idDesc = $derived(!custom && builtin ? t("sp.discord.id.builtin") : t("sp.discord.id.desc"));
  const local = $derived(!onWeb() && mcp.mode !== "server");
  const modeDesc = $derived(!local ? (signedIn ? t("sp.mcp.server.desc") : t("sp.mcp.server.signin")) : t("sp.mcp.local.desc"));
  const access = (area: string): Access => mcp.access?.[area] ?? DEFAULTS[area];
  const status = $derived(info.error ? `${t("sp.mcp.problem")} ${info.error}` : !local && info.relay_running ? t("sp.mcp.relay_on") : local ? t("sp.mcp.running") : !local && !signedIn ? t("sp.mcp.server.signin") : t("sp.mcp.idle"));

  async function copy(text: string | null) { if (text && (await copyText(text))) app.toast(t("sp.mcp.copied")); }
  async function rotate() { await backend().call(local ? "mcp_rotate_token" : "mcp_server_token"); loadInfo(); }
  async function create() { info.token_loading = true; await backend().call("mcp_server_token"); loadInfo(); }
</script>

<PluginHost page="integrations">
  <div class="col">
    {#if tab === 0}
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
              <Segmented value={access(area)}
                options={[{ value: "none" as Access, label: t("sp.mcp.access.none") }, { value: "read" as Access, label: t("sp.mcp.access.read") }, ...(write ? [{ value: "write" as Access, label: t("sp.mcp.access.write") }] : [])]}
                onpick={(v) => app.setConfig((c) => { const m = c.mcp as typeof mcp; m.access = { ...(m.access ?? {}), [area]: v }; })} />
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
              <Button onclick={rotate}>{t("sp.mcp.rotate")}</Button>
            </Row>
          {:else if !local && signedIn}
            <Row label={t("sp.mcp.token")} info={t("sp.mcp.token.server_desc")}>
              <Button onclick={create}>{info.token_loading ? t("sp.mcp.creating") : t("sp.mcp.create_token")}</Button>
            </Row>
          {/if}
          <div class="srow stat" class:warn={!!info.error}>{status}</div>
        </Rows>
      {/if}
    {/if}
  </div>
</PluginHost>

<style>
  .col { display: flex; flex-direction: column; }
  .vis { margin-top: 8px; }
  .stat { padding: 8px 2px; font-size: 12.5px; color: rgba(255, 255, 255, .55); }
  .stat.warn { color: var(--warn); }
</style>
