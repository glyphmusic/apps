<script lang="ts">
  // settings/about.rs shortcuts + ui/keys.rs: Settings > About > Shortcuts.
  // Every keyboard shortcut and mouse gesture, grouped. A click on a
  // shortcut's keys records a new key (desktop): Esc cancels, Delete /
  // Backspace removes the key, a key another shortcut had makes that one
  // give it up, fixed keys (lists, sidebar, closing) cannot be taken. Key
  // caps: mono 11.5, min 24 x 24, px 7, radius 6, white 7 %, border white
  // 10 %; `+` joins keys pressed together, `/` separates alternatives.
  import { app } from "../state.svelte";
  import { t, tf } from "../i18n.svelte";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import { tip } from "../ui.svelte";
  import { FIXED, SHORTCUTS, eventKey, keyLabel, keyOf } from "../keys";
  import { onWeb } from "../web";
  import { isMac } from "../window";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import SectionLabel from "./SectionLabel.svelte";
  import Select from "./Select.svelte";

  const web = onWeb();
  const editable = $derived(app.platform !== "android" && app.platform !== "ios" && !app.compact);
  const visibleShortcuts = $derived(web ? SHORTCUTS.filter((s) => s.id !== "download" && s.id !== "quit") : SHORTCUTS);
  const keys = $derived(app.config.ui.keys ?? {});
  const changed = $derived(Object.keys(keys).length > 0);
  let capture = $state<string | null>(null);
  const GROUPS: [string, string][] = [["playback", "sp.keys.group.playback"], ["navigation", "sp.keys.group.nav"], ["app", "sp.keys.group.app"]];
  const MODS = ["Control", "Shift", "Alt", "Meta", "AltGraph", "CapsLock"];

  const caps = (key: string) => key.split("-").map((p) => keyLabel(p));
  const def = (id: string) => SHORTCUTS.find((s) => s.id === id)?.key ?? "";

  // While recording nothing else may fire: listen first and stop the event.
  $effect(() => {
    if (!capture) return;
    const id = capture;
    const on = (e: KeyboardEvent) => {
      e.preventDefault(); e.stopImmediatePropagation();
      const plain = !(e.ctrlKey || e.altKey || e.metaKey || e.shiftKey);
      if (plain && e.key === "Escape") { capture = null; return; }
      let key: string;
      if (plain && (e.key === "Delete" || e.key === "Backspace")) key = "";
      else {
        // A modifier on its own: wait for the key that goes with it.
        if (MODS.includes(e.key)) return;
        key = eventKey(e);
        if (FIXED.includes(key)) { app.toastInfo(tf("toast.key_fixed", { key: keyLabel(key) })); return; }
      }
      app.setConfig((c) => {
        // Another shortcut had this key: it gives it up.
        if (key) for (const s of SHORTCUTS) if (s.id !== id && (c.ui.keys[s.id] ?? s.key) === key) {
          c.ui.keys[s.id] = "";
          app.toastInfo(tf("toast.key_moved", { key: keyLabel(key), other: t(s.label) }));
        }
        if (key === def(id) || (!key && !def(id))) delete c.ui.keys[id]; else c.ui.keys[id] = key;
      });
      capture = null;
    };
    addEventListener("keydown", on, true);
    return () => removeEventListener("keydown", on, true);
  });

  function reset(id: string | null) {
    app.setConfig((c) => {
      if (id == null) { c.ui.keys = {}; return; }
      delete c.ui.keys[id];
      const key = c.ui.keys[id] ?? def(id);
      // A shortcut that took this key meanwhile gives it up.
      if (key) for (const s of SHORTCUTS) if (s.id !== id && (c.ui.keys[s.id] ?? s.key) === key) c.ui.keys[s.id] = "";
    });
    capture = null;
  }

  const m = isMac() ? "⌘" : "Ctrl";
  const fixed: [string, string][] = [
    ["sp.keys.list_move", "↑ / ↓"], ["sp.keys.list_play", "Enter"], ["sp.keys.list_remove", `${t("sp.keys.del")} / ⌫`],
    ["sp.keys.sidebar", `${m}+1 / 9`], ["sp.keys.close", "Esc"],
  ];
  const ui = $derived(app.config.ui);
  const off = () => t("sp.player.off");
  const seekOpts = [0, 1, 5, 10, 15, 30].map((s) => ({ value: s, label: s === 0 ? off() : `${s} s` }));
  const volOpts = [0, 1, 2, 5, 10].map((p) => ({ value: p, label: p === 0 ? off() : `${p} %` }));
</script>

{#snippet combo(text: string)}
  <span class="combo">
    {#each text.split(" / ") as alt, n}
      {#if n > 0}<span class="sep">/</span>{/if}
      {#each alt.split("+") as k, i}
        {#if i > 0}<span class="sep">+</span>{/if}
        <span class="mono cap">{k}</span>
      {/each}
    {/each}
  </span>
{/snippet}

<div class="col">
  {#if editable}
    <div class="top">
      <div class="hint">{t("sp.keys.hint")}</div>
      {#if changed}<Button onclick={() => reset(null)}>{t("sp.keys.reset_all")}</Button>{/if}
    </div>
  {/if}
  {#each GROUPS as [group, title] (group)}
    {@const list = visibleShortcuts.filter((s) => s.group === group)}
    {#if list.length}
      <div class="grp">
        <SectionLabel text={t(title)} />
        <Rows>
          {#each list as s (s.id)}
          {@const key = keyOf(s.id)}
          {@const rec = capture === s.id}
          <Row label={t(s.label)}>
            <span class="ctl">
              {#if editable && s.id in keys && !rec}
                <button class="rs" onclick={() => reset(s.id)} use:tip={t("sp.keys.reset")} aria-label={t("sp.keys.reset")}><Icon name="rotate-ccw" size={14} color="rgba(255,255,255,.6)" /></button>
              {/if}
              {#if editable}
                <button class="ke" class:rec onclick={() => (capture = rec ? null : s.id)} use:tip={t("sp.keys.change")}>
                  {#if rec}<span class="press">{t("sp.keys.press")}</span>
                  {:else if !key}<span class="none">{t("sp.keys.none")}</span>
                  {:else}{@render combo(caps(key).join("+"))}{/if}
                </button>
              {:else if key}
                <span class="ke ro">{@render combo(caps(key).join("+"))}</span>
              {:else}<span class="none">{t("sp.keys.none")}</span>{/if}
            </span>
          </Row>
        {/each}
      </Rows>
    </div>
    {/if}
  {/each}
  <div class="grp">
    <SectionLabel text={t("sp.keys.group.fixed")} />
    <Rows>{#each fixed as [label, k] (label)}<Row label={t(label)}>{@render combo(k)}</Row>{/each}</Rows>
  </div>
  {#if editable}
    <div class="grp">
      <SectionLabel text={t("sp.keys.group.mouse")} />
      <Rows>
        <Row label={t("sp.player.wheel_seek")} info={t("sp.player.wheel_seek.desc")}>
          <Select value={ui.wheel_seek_s} options={seekOpts} onpick={(v) => app.setConfig((c) => { c.ui.wheel_seek_s = v; })} />
        </Row>
        <Row label={t("sp.player.wheel_volume")} info={t("sp.player.wheel_volume.desc")}>
          <Select value={ui.wheel_volume_pct} options={volOpts} onpick={(v) => app.setConfig((c) => { c.ui.wheel_volume_pct = v; })} />
        </Row>
      </Rows>
    </div>
  {/if}
</div>

<style>
  .col { display: flex; flex-direction: column; gap: 14px; }
  .top { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .hint { flex: 1; min-width: 0; font-size: 12.5px; color: rgba(255, 255, 255, .55); }
  .grp { display: flex; flex-direction: column; }
  .ctl { display: flex; align-items: center; gap: 4px; }
  .combo { display: flex; align-items: center; gap: 4px; }
  .sep { font-size: 12px; color: rgba(255, 255, 255, .4); }
  .cap { flex: none; min-width: 24px; height: 24px; padding: 0 7px; display: flex; align-items: center; justify-content: center; border-radius: 6px; background: rgba(255, 255, 255, .07); font-size: 11.5px; color: rgba(255, 255, 255, .85); }
  .ke { height: 32px; padding: 0 6px; display: flex; align-items: center; border-radius: 8px; transition: background-color .12s, box-shadow .15s; }
  .ke:not(.ro):hover { background: rgba(255, 255, 255, .05); }
  .ke.rec { background: color-mix(in srgb, var(--accent) 18%, transparent); }
  .press { font-size: 12.5px; color: var(--accent); }
  .none { font-size: 12.5px; color: rgba(255, 255, 255, .4); }
  .rs { width: 28px; height: 28px; display: grid; place-items: center; border-radius: 8px; transition: background-color .12s; }
  .rs:hover { background: rgba(255, 255, 255, .08); }
</style>
