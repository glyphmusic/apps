// UI strings from glyph-apps/assets/i18n/*.toml (same files as the Rust
// app). English is bundled eagerly as the instant fallback; other locales load
// on demand in their own chunks so startup stays lean.
import enSrc from "$assets/i18n/en.toml?raw";

const loaders = import.meta.glob(["$assets/i18n/*.toml", "!$assets/i18n/en.toml"], { query: "?raw", import: "default" }) as Record<string, () => Promise<string>>;

export const LANGS = [
  ["en", "English"], ["af", "Afrikaans"], ["de", "Deutsch"], ["es", "Español"], ["fr", "Français"], ["it", "Italiano"],
  ["pl", "Polski"], ["pt", "Português"], ["ru", "Русский"], ["tr", "Türkçe"], ["uk", "Українська"],
] as const;

const KNOWN = new Set<string>(LANGS.map(([code]) => code));
const loaderByLang = new Map<string, () => Promise<string>>();
for (const [path, load] of Object.entries(loaders)) {
  const code = path.split("/").pop()!.replace(".toml", "");
  if (code !== "en") loaderByLang.set(code, load);
}

function unescape(s: string): string {
  return s.replace(/\\(u[0-9a-fA-F]{4}|U[0-9a-fA-F]{8}|.)/g, (_, c: string) => {
    if (c[0] === "u" || c[0] === "U") return String.fromCodePoint(parseInt(c.slice(1), 16));
    return ({ n: "\n", t: "\t", r: "\r", '"': '"', "\\": "\\", b: "\b", f: "\f" } as Record<string, string>)[c] ?? c;
  });
}

/** The language files are flat `"key" = "value"` tables. */
function parse(src: string): Map<string, string> {
  const m = new Map<string, string>();
  const re = /^\s*"((?:[^"\\]|\\.)*)"\s*=\s*"((?:[^"\\]|\\.)*)"\s*$/gm;
  for (let r; (r = re.exec(src)); ) m.set(unescape(r[1]), unescape(r[2]));
  return m;
}

const en = parse(enSrc);
const tables = new Map<string, Map<string, string>>([["en", en]]);
const loading = new Set<string>();
let loadedRev = $state(0);

function ensureLoaded(code: string) {
  if (tables.has(code) || loading.has(code)) return;
  const load = loaderByLang.get(code);
  if (!load) return;
  loading.add(code);
  load().then((src) => {
    tables.set(code, parse(src));
    loadedRev += 1;
  }).catch(() => {}).finally(() => {
    loading.delete(code);
  });
}

function pick(code: string): string {
  const c = code.trim().toLowerCase().split(/[-_.]/)[0];
  return KNOWN.has(c) ? c : "en";
}

const initialLang = pick(navigator.language);
let lang = $state(initialLang);
ensureLoaded(initialLang);
/** Plugin texts by language (they win over Glyph's in their language). */
let overrides = $state<Record<string, Record<string, string>>>({});

export function setLang(code: string) {
  lang = pick(code);
  document.documentElement.lang = lang;
  ensureLoaded(lang);
}
export function getLang() { return lang; }
export function setOverrides(o: Record<string, Record<string, string>>) { overrides = o; }

export function t(key: string): string {
  void loadedRev;
  return overrides[lang]?.[key] ?? tables.get(lang)?.get(key) ?? overrides.en?.[key] ?? en.get(key) ?? key;
}

/** Translated string with `{name}` placeholders. */
export function tf(key: string, args: Record<string, string | number>): string {
  let s = t(key);
  for (const [k, v] of Object.entries(args)) s = s.replaceAll(`{${k}}`, String(v));
  return s;
}

/** `tip.<icon>` text, or undefined (widgets/tooltip.rs tip_for). */
export function tipFor(icon: string): string | undefined {
  const k = `tip.${icon}`;
  const v = t(k);
  return v === k ? undefined : v;
}

export function has(key: string): boolean { return t(key) !== key; }
