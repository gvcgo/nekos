// Minimal dependency-free i18n. Each component carries its own bilingual
// dictionaries (zh/en) and reads the global reactive locale, so the UI
// switches language live.

import { computed, ref } from "vue";

export type Lang = "zh" | "en";

export const locale = ref<Lang>("zh");

const LANGS: Lang[] = ["zh", "en"];

export function normalizeLang(v: unknown): Lang {
  return LANGS.includes(v as Lang) ? (v as Lang) : "zh";
}

/** Switch UI language and adjust <html lang>. */
export function applyLocale(v: unknown) {
  const l = normalizeLang(v);
  locale.value = l;
  document.documentElement.lang = l;
}

export type Dict = Record<string, string>;

/** Reactive dictionary accessor for a component's bilingual tables. */
export function useDict(d: { zh: Dict; en: Dict }) {
  return computed(() => d[locale.value] ?? d.zh);
}

/** {name} placeholder interpolation. */
export function fmt(s: string, p?: Record<string, string | number>): string {
  if (!p) return s;
  return s.replace(/\{(\w+)\}/g, (m, k) =>
    Object.prototype.hasOwnProperty.call(p, k) ? String(p[k]) : m,
  );
}

export function tpl(d: Dict, key: string, p?: Record<string, string | number>): string {
  const s = d[key];
  if (s == null) return key;
  return fmt(s, p);
}
