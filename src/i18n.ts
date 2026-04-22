/**
 * i18n - lightweight internationalization system
 * Supports zh / en languages, loaded from JSON at startup, switchable at runtime
 */
import { ref, readonly } from "vue";
import zhMessages from "./language/zh.json";
import enMessages from "./language/en.json";

export type Locale = "zh" | "en";

const STORAGE_KEY = "locus-locale";

type Messages = Record<string, string>;

const messages: Record<Locale, Messages> = {
  zh: zhMessages as Messages,
  en: enMessages as Messages,
};

const currentLocale = ref<Locale>(loadLocale());

function loadLocale(): Locale {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "zh" || saved === "en") return saved;
  } catch { /* ignore */ }
  return "zh";
}

export function setLocale(locale: Locale) {
  currentLocale.value = locale;
  try {
    localStorage.setItem(STORAGE_KEY, locale);
  } catch { /* ignore */ }
}

export const locale = readonly(currentLocale);

/**
 * Translation function: returns the string for the current locale by key
 * Supports {0}, {1} placeholder substitution
 */
export function t(key: string, ...args: (string | number)[]): string {
  const msg = messages[currentLocale.value]?.[key] ?? messages.zh[key] ?? key;
  if (args.length === 0) return msg;
  return msg.replace(/\{(\d+)\}/g, (_, idx) => {
    const i = parseInt(idx);
    return i < args.length ? String(args[i]) : `{${idx}}`;
  });
}

export function useI18n() {
  return { t, locale: currentLocale, setLocale };
}
