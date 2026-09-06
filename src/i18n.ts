// Minimal dependency-free i18n. Each component carries its own three
// dictionaries (zh/en/ar) and reads the global reactive locale, so the UI
// switches language live. Arabic sets dir=rtl on <html>.

import { computed, ref } from "vue";

export type Lang = "zh" | "en" | "ar";

export const locale = ref<Lang>("zh");

const LANGS: Lang[] = ["zh", "en", "ar"];

export function normalizeLang(v: unknown): Lang {
  return LANGS.includes(v as Lang) ? (v as Lang) : "zh";
}

/** Switch UI language and adjust <html lang/dir> (rtl for Arabic). */
export function applyLocale(v: unknown) {
  const l = normalizeLang(v);
  locale.value = l;
  document.documentElement.lang = l;
  document.documentElement.dir = l === "ar" ? "rtl" : "ltr";
}

export type Dict = Record<string, string>;

/** Reactive dictionary accessor for a component's tri-lingual tables. */
export function useDict(d: { zh: Dict; en: Dict; ar: Dict }) {
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
