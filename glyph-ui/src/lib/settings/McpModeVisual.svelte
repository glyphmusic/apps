<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import Icon from "../components/Icon.svelte";
  import VisualCard from "./VisualCard.svelte";

  const local = $derived(app.config.mcp.mode !== "server");
  const signedIn = $derived(app.account != null);
  const info = $derived(!local ? (signedIn ? t("sp.mcp.server.desc") : t("sp.mcp.server.signin")) : t("sp.mcp.local.desc"));

  function pick(isLocal: boolean) {
    app.setConfig((c) => {
      c.mcp.mode = isLocal ? "local" : "server";
    });
  }
</script>

<VisualCard label={t("sp.mcp.mode")} {info}>
  <div class="grid" role="radiogroup" aria-label={t("sp.mcp.mode")}>
    {#each [true, false] as isLocal (String(isLocal))}
      {@const on = local === isLocal}
      {@const accent = on ? "var(--accent)" : "rgba(255,255,255,.55)"}
      {@const ink = on ? "rgba(255,255,255,.85)" : "rgba(255,255,255,.42)"}
      <button class="opt" class:on role="radio" aria-checked={on} onclick={() => pick(isLocal)}>
        <div class="stage" aria-hidden="true">
          <div class="flow">
            <span class="node">
              <Icon name="bot" size={15} color={accent} />
            </span>
            <span class="line" class:dashed={!isLocal}></span>
            {#if !isLocal}
              <span class="node mid">
                <Icon name="server" size={14} color={accent} />
              </span>
              <span class="line dashed"></span>
            {/if}
            <span class="node">
              <Icon name="monitor" size={15} color={isLocal ? accent : ink} />
            </span>
          </div>
        </div>
        <span class="lbl">{t(isLocal ? "sp.mcp.local" : "sp.mcp.server")}</span>
      </button>
    {/each}
  </div>
</VisualCard>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 12px;
  }
  .opt {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 9px;
    padding: 12px;
    border-radius: 10px;
    background: #0b0b0b;
    border: 0;
    transition: border-color .15s ease-out, background-color .15s ease-out, transform .15s var(--ease);
  }
  .opt:hover { background: #18181b; }
  /* No outlines: the picked card is the lighter one. */
  .opt.on { background: #26262b; }
  .opt:active { transform: scale(.985); }

  .stage {
    width: 100%;
    height: 68px;
    display: grid;
    place-items: center;
  }
  .flow {
    width: 158px;
    max-width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .node {
    width: 32px;
    height: 32px;
    border-radius: 8px;
    background: rgba(255, 255, 255, .06);
    display: grid;
    place-items: center;
    flex: none;
  }
  .opt.on .node { background: rgba(255, 255, 255, .1); }
  .opt.on .node.mid { background: color-mix(in srgb, var(--accent) 20%, #181818); }

  .line {
    flex: 1;
    height: 2px;
    border-radius: 1px;
    background: rgba(255, 255, 255, .18);
  }
  .opt.on .line { background: var(--accent); opacity: .75; }
  .line.dashed {
    background: repeating-linear-gradient(90deg, rgba(255, 255, 255, .24) 0 4px, transparent 4px 7px);
  }
  .opt.on .line.dashed {
    background: repeating-linear-gradient(90deg, var(--accent) 0 4px, transparent 4px 7px);
    opacity: .75;
  }

  .lbl { font-size: 13px; font-weight: 500; color: rgba(255, 255, 255, .75); }
  .opt.on .lbl { color: #fff; font-weight: 600; }

  @media (max-width: 460px) {
    .grid { grid-template-columns: 1fr; }
  }
</style>
