<script lang="ts">
  // ui/transfer.rs page: Settings > Backup & share. Copy the settings as a
  // code, save them as a file, and apply a code or file from someone else
  // (with undo). Format: glyph_core::settings_share (no API keys, no server,
  // no device values).
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import SectionLabel from "./SectionLabel.svelte";
  import Field from "./Field.svelte";
  import { copyText, errText, pickFiles, saveText } from "./util";

  let {}: { sub?: number } = $props();
  // A code pasted into the search field lands here.
  let code = $state(app.pendingSettingsCode ?? "");
  app.pendingSettingsCode = null;
  let undo = $state(false);
  const ICON = "rgba(255,255,255,.6)";

  async function copy() {
    const c = await backend().call<string>("settings_code");
    if (c && (await copyText(c))) app.copied(t("sp.transfer.copied"));
  }
  async function save() {
    const f = await backend().call<{ name: string; text: string }>("settings_file");
    if (!f) return;
    try { saveText(f.name, f.text, "application/json"); app.toast(`${t("sp.transfer.saved")} · ${f.name}`); }
    catch (e) { app.toastInfo(tf("sp.transfer.error", { e: errText(e) })); }
  }
  /** Applies a settings code or file content; the old settings stay for undo. */
  async function importText(text: string): Promise<boolean> {
    try {
      const r = await backend().call<{ changed: boolean }>("settings_import", { text });
      if (r?.changed) { undo = true; app.toast(t("sp.transfer.applied")); } else app.toastInfo(t("sp.transfer.same"));
      return true;
    } catch (e) { app.toastInfo(tf("sp.transfer.invalid", { e: errText(e) })); return false; }
  }
  async function apply() { if (code.trim() && (await importText(code))) code = ""; }
  async function openFile() {
    const [f] = await pickFiles(".json,.txt");
    if (f) await importText(f.text);
  }
  async function undoImport() { await backend().call("settings_undo"); undo = false; app.toast(t("sp.transfer.undone")); }
</script>

<div class="col">
  <SectionLabel text={t("sp.transfer.share")} />
  <Rows>
    <Row label={t("sp.transfer.code")} info={t("sp.transfer.code.desc")}>
      <Button onclick={copy}>{t("sp.transfer.code.button")}<Icon name="copy" size={14} color={ICON} /></Button>
    </Row>
    <Row label={t("sp.transfer.file")} info={t("sp.transfer.file.desc")}>
      <Button onclick={save}>{t("sp.transfer.file.button")}<Icon name="download" size={14} color={ICON} /></Button>
    </Row>
  </Rows>
  <SectionLabel text={t("sp.transfer.apply")} />
  <Rows>
    <Row label={t("sp.transfer.import")} info={t("sp.transfer.import.desc")}>
      <Field bind:value={code} width={200} placeholder={t("sp.transfer.placeholder")} onenter={apply} label={t("sp.transfer.import")} />
      <Button onclick={apply}>{t("sp.transfer.import.button")}</Button>
    </Row>
    <Row label={t("sp.transfer.open")} info={t("sp.transfer.open.desc")}>
      <Button onclick={openFile}>{t("sp.transfer.open.button")}<Icon name="folder-open" size={14} color={ICON} /></Button>
    </Row>
    {#if undo}
      <Row label={t("sp.transfer.undo")} info={t("sp.transfer.undo.desc")}>
        <Button onclick={undoImport}>{t("sp.transfer.undo.button")}</Button>
      </Row>
    {/if}
  </Rows>
  <div class="note">{t("sp.transfer.note")}</div>
</div>

<style>
  .col { display: flex; flex-direction: column; }
  .note { padding-top: 14px; font-size: 12px; color: rgba(255, 255, 255, .4); }
</style>
