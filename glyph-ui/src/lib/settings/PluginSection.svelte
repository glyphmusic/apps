<script lang="ts">
  // settings/plugin_ui.rs section + item_row: one plugin section (title, and
  // on Glyph's pages the plugin's name with a small puzzle icon so nobody
  // mistakes it for Glyph's own), then its rows: progress, toggle, slider,
  // select, text, button, link or a note. Texts and patches arrive resolved.
  import Icon from "../components/Icon.svelte";
  import Button from "../components/Button.svelte";
  import ThinBar from "../components/ThinBar.svelte";
  import RowBase from "./RowBase.svelte";
  import Rows from "./Rows.svelte";
  import SectionLabel from "./SectionLabel.svelte";
  import Tog from "./Tog.svelte";
  import Select from "./Select.svelte";
  import Field from "./Field.svelte";
  import { pluginAction, setValue } from "./plugins-state.svelte";
  import { openUrl } from "./util";
  import { t } from "../i18n.svelte";
  import type { PluginItem, PluginSection } from "./types";
  let { section, named = true }: { section: PluginSection; named?: boolean } = $props();

  const head = $derived(named ? (section.title ? `${section.title} · ${section.plugin_name}` : section.plugin_name) : section.title);
  const items = $derived(section.items);
  const num = (i: PluginItem) => (typeof i.value === "number" ? i.value : (i.min ?? 0));
  const shown = (i: PluginItem) => { const x = num(i); return `${Math.abs(x - Math.round(x)) < 1e-9 ? x.toFixed(0) : x.toFixed(1)}${i.unit ?? ""}`; };
  const frac = (i: PluginItem) => { const lo = i.min ?? 0, hi = i.max ?? 100; return Math.min(1, Math.max(0, (num(i) - lo) / (hi - lo))); };
  const status = (i: PluginItem) => (i.secret ? (i.configured ? t("plugins.setup.configured") : t("plugins.setup.enter")) : i.status || null);
  let texts = $state<Record<string, string>>({});
</script>

<div class="sec">
  {#if head}
    <div class="head">
      <SectionLabel text={head} />
      {#if named}<span class="pz"><Icon name="puzzle" size={12} color="rgba(255,255,255,.4)" /></span>{/if}
    </div>
  {/if}
  <Rows>
    {#each items as i (i.id)}
      {#if i.kind === "note"}
        <div class="srow note">
          {#if i.label}<div class="nl">{i.label}</div>{/if}
          <div class="nb">{i.desc}</div>
          {#if status(i)}<div class="ns">{status(i)}</div>{/if}
        </div>
      {:else}
        <div class="srow">
          <RowBase label={i.label} info={i.desc || null} status={status(i)}>
            {#if i.kind === "progress"}
              <div class="prog"><div class="pf" style:width="{Math.min(1, Math.max(0, num(i))) * 160}px"></div></div>
            {:else if i.kind === "toggle"}
              <Tog on={i.value === true} disabled={i.disabled} onchange={(v) => setValue(section.plugin, i.id, v)} label={i.label} />
            {:else if i.kind === "slider"}
              <div class="sl" class:dis={i.disabled}>
                {#if i.disabled}<div style:width="160px"></div>
                {:else}<ThinBar width={160} value={frac(i)} step={(i.step ?? 1) / ((i.max ?? 100) - (i.min ?? 0))} live={false}
                    onpick={(f) => setValue(section.plugin, i.id, (i.min ?? 0) + ((i.max ?? 100) - (i.min ?? 0)) * f)} />{/if}
                <span class="mono sv">{shown(i)}</span>
              </div>
            {:else if i.kind === "select"}
              <Select value={String(i.value ?? "")} options={i.options} onpick={(v) => setValue(section.plugin, i.id, v)} />
            {:else if i.kind === "text"}
              <Field value={texts[i.id] ?? (i.secret ? "" : String(i.value ?? ""))} masked={i.secret} oninput={(v) => (texts[i.id] = v)}
                onenter={(v) => setValue(section.plugin, i.id, v)} onblur={(v) => texts[i.id] != null && setValue(section.plugin, i.id, v)} label={i.label} />
            {:else if i.kind === "button"}
              <Button onclick={() => !i.disabled && pluginAction(section.plugin, i.id)} disabled={i.disabled}>{i.caption || i.label}</Button>
            {:else if i.kind === "link"}
              <Button onclick={() => openUrl(i.url)}>{i.caption || i.label}<Icon name="external-link" size={13} color="rgba(255,255,255,.7)" /></Button>
            {/if}
          </RowBase>
        </div>
      {/if}
    {/each}
  </Rows>
</div>

<style>
  .sec { display: flex; flex-direction: column; }
  .head { display: flex; align-items: center; gap: 6px; }
  .pz { padding-top: 16px; display: flex; }
  .note { display: flex; flex-direction: column; gap: 4px; padding: 14px 0; }
  .nl { font-size: 14px; font-weight: 500; color: #fff; }
  .nb { font-size: 12.5px; line-height: 18px; color: rgba(255, 255, 255, .6); }
  .ns { font-size: 12.5px; color: rgba(255, 255, 255, .55); }
  .prog { width: 160px; height: 6px; border-radius: 3px; background: rgba(255, 255, 255, .08); }
  .pf { height: 6px; border-radius: 3px; background: var(--accent); transition: width .3s var(--ease); }
  .sl { display: flex; align-items: center; gap: 10px; }
  .sl.dis { opacity: .5; }
  .sv { width: 48px; display: flex; justify-content: flex-end; font-size: 12px; color: rgba(255, 255, 255, .6); }
</style>
