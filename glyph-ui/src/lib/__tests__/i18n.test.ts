import { describe, expect, it } from "vitest";

const LOCALE_FILES = import.meta.glob("$assets/i18n/*.toml", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const UI_FILES = import.meta.glob(
  ["/src/lib/**/*.svelte", "/src/lib/**/*.ts", "!/src/lib/__tests__/**"],
  { query: "?raw", import: "default", eager: true },
) as Record<string, string>;

const LINE_RE = /^\s*"((?:[^"\\]|\\.)*)"\s*=\s*"((?:[^"\\]|\\.)*)"\s*$/;
const PLACEHOLDER_RE = /\{([a-zA-Z0-9_]+)\}/g;

interface ParsedLocale {
  keys: Map<string, string>;
  duplicates: string[];
  malformed: string[];
  emptyValues: string[];
}

function unescapeToml(s: string): string {
  return s.replace(/\\(u[0-9a-fA-F]{4}|U[0-9a-fA-F]{8}|.)/g, (_, c: string) => {
    if (c[0] === "u" || c[0] === "U") return String.fromCodePoint(parseInt(c.slice(1), 16));
    return ({ n: "\n", t: "\t", r: "\r", '"': '"', "\\": "\\", b: "\b", f: "\f" } as Record<string, string>)[c] ?? c;
  });
}

function parseLocale(text: string): ParsedLocale {
  const keys = new Map<string, string>();
  const duplicates: string[] = [];
  const malformed: string[] = [];
  const emptyValues: string[] = [];

  for (const [idx, rawLine] of text.split(/\r?\n/).entries()) {
    const trimmed = rawLine.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const m = LINE_RE.exec(rawLine);
    if (!m) {
      malformed.push(`line ${idx + 1}: ${rawLine}`);
      continue;
    }
    const rawKey = m[1];
    const rawVal = m[2];
    if (!rawLine.startsWith(`"${rawKey}" = "`) || !rawLine.endsWith('"')) {
      malformed.push(`line ${idx + 1} (expected canonical "key" = "value"): ${rawLine}`);
    }
    const key = unescapeToml(rawKey);
    const val = unescapeToml(rawVal);
    if (keys.has(key)) duplicates.push(key);
    if (!val.trim()) emptyValues.push(key);
    keys.set(key, val);
  }

  return { keys, duplicates, malformed, emptyValues };
}

function placeholdersOf(s: string): string[] {
  return Array.from(s.matchAll(PLACEHOLDER_RE), (m) => m[1]).sort();
}

describe("i18n translation completeness across all languages", () => {
  const parsed = new Map<string, ParsedLocale>();
  for (const [filePath, src] of Object.entries(LOCALE_FILES)) {
    const code = filePath.split("/").pop()!.replace(/\.toml$/, "");
    parsed.set(code, parseLocale(src));
  }
  const localeCodes = Array.from(parsed.keys()).sort();
  const en = parsed.get("en")!;

  it("loads all locale files and matches LANGS in i18n.svelte.ts", () => {
    expect(localeCodes.length).toBeGreaterThanOrEqual(2);
    expect(localeCodes).toContain("en");
    const i18nSrc = UI_FILES["/src/lib/i18n.svelte.ts"];
    expect(i18nSrc, "i18n.svelte.ts not loaded by glob").toBeTruthy();
    const block = /export const LANGS\s*=\s*\[([\s\S]*?)\]\s*as const;/.exec(i18nSrc);
    expect(block, "LANGS declaration missing in i18n.svelte.ts").not.toBeNull();
    const uiCodes = Array.from(block![1].matchAll(/\[\s*"([a-z]{2,3})"\s*,/g), (m) => m[1]).sort();
    expect(uiCodes).toEqual(localeCodes);
  });

  it("parses cleanly with no malformed lines, duplicates, or empty values in any language", () => {
    for (const [code, data] of parsed) {
      expect(data.malformed, `${code}.toml has malformed lines`).toEqual([]);
      expect(data.duplicates, `${code}.toml has duplicate keys`).toEqual([]);
      expect(data.emptyValues, `${code}.toml has empty values`).toEqual([]);
    }
  });

  it("keeps every language in exact key parity with en.toml (not just en and de)", () => {
    const enKeys = Array.from(en.keys.keys());
    for (const [code, data] of parsed) {
      if (code === "en") continue;
      const missing = enKeys.filter((k) => !data.keys.has(k));
      const extra = Array.from(data.keys.keys()).filter((k) => !en.keys.has(k));
      expect(missing, `${code}.toml is missing ${missing.length} keys from en.toml`).toEqual([]);
      expect(extra, `${code}.toml has ${extra.length} extra keys not in en.toml`).toEqual([]);
    }
  });

  it("preserves identical {placeholders} in every language", () => {
    for (const [code, data] of parsed) {
      if (code === "en") continue;
      const mismatches: string[] = [];
      for (const [key, enVal] of en.keys) {
        const val = data.keys.get(key);
        if (val === undefined) continue;
        const expected = placeholdersOf(enVal);
        const actual = placeholdersOf(val);
        if (expected.join(",") !== actual.join(",")) {
          mismatches.push(`${key}: expected {${expected.join(", ")}} got {${actual.join(", ")}}`);
        }
      }
      expect(mismatches, `${code}.toml has placeholder mismatches`).toEqual([]);
    }
  });

  it("defines every static UI translation key referenced in glyph-ui/src", () => {
    expect(Object.keys(UI_FILES).length).toBeGreaterThan(20);
    const callRe = /\b(?:t|tf|has)\(([^)\n]{1,200})\)/g;
    const keyLitRe = /["']([a-z][a-z0-9_]*\.[a-z0-9_.-]+)["']/g;
    const missing: string[] = [];

    for (const [rel, text] of Object.entries(UI_FILES)) {
      for (const call of text.matchAll(callRe)) {
        for (const lit of call[1].matchAll(keyLitRe)) {
          const key = lit[1];
          if (!en.keys.has(key)) missing.push(`${key} (${rel})`);
        }
      }
    }

    const pagesSrc = UI_FILES["/src/lib/settings/pages.ts"];
    expect(pagesSrc, "settings/pages.ts not loaded by glob").toBeTruthy();
    for (const lit of pagesSrc.matchAll(/"([a-z][a-z0-9_]*\.[a-z0-9_.-]+)"/g)) {
      const key = lit[1];
      if (!en.keys.has(key)) missing.push(`${key} (/src/lib/settings/pages.ts)`);
    }

    expect(missing).toEqual([]);
  });
});
