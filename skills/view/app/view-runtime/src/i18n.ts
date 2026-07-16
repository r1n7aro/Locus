/**
 * i18n - lightweight internationalization system
 * Supports zh / en languages, loaded from JSON at startup, switchable at runtime
 */
import { ref, readonly } from "vue";

export type Locale = "zh" | "en";

const STORAGE_KEY = "locus-locale";
const DEFAULT_LOCALE: Locale = "en";

type Messages = Record<string, string>;

// Catalogs load as separate async chunks so entries only pay for the
// active language (each catalog is ~230KB of generated JS).
const messages: Record<Locale, Messages> = {
  zh: {},
  en: {},
};

const catalogLoaders: Record<Locale, () => Promise<{ default: Messages }>> = {
  zh: () => import("./language/zh.json"),
  en: () => import("./language/en.json"),
};

const loadedLocales = new Set<Locale>();

async function loadCatalog(locale: Locale): Promise<void> {
  if (loadedLocales.has(locale)) return;
  const catalog = await catalogLoaders[locale]();
  messages[locale] = catalog.default;
  loadedLocales.add(locale);
}

const currentLocale = ref<Locale>(loadLocale());

export function normalizeLocale(value: string | null | undefined): Locale | null {
  if (!value) return null;
  const normalized = value.trim().toLowerCase().replace(/_/g, "-");
  if (normalized === "zh" || normalized.startsWith("zh-")) return "zh";
  if (normalized === "en" || normalized.startsWith("en-")) return "en";
  return null;
}

function readSavedLocale(): Locale | null {
  try {
    return normalizeLocale(localStorage.getItem(STORAGE_KEY));
  } catch {
    return null;
  }
}

function readNavigatorLocales(): string[] {
  try {
    const candidates = Array.isArray(navigator.languages)
      ? [...navigator.languages]
      : [];
    if (typeof navigator.language === "string" && navigator.language.trim()) {
      candidates.push(navigator.language);
    }
    return candidates;
  } catch {
    return [];
  }
}

export function resolveLocale(options: {
  savedLocale?: string | null;
  systemLocale?: string | null;
  navigatorLocales?: readonly string[] | null;
} = {}): Locale {
  const savedLocale = normalizeLocale(options.savedLocale ?? null);
  if (savedLocale) return savedLocale;

  const systemLocale = normalizeLocale(options.systemLocale ?? null);
  if (systemLocale) return systemLocale;

  for (const candidate of options.navigatorLocales ?? []) {
    const resolved = normalizeLocale(candidate);
    if (resolved) return resolved;
  }

  return DEFAULT_LOCALE;
}

function loadLocale(): Locale {
  return resolveLocale({
    savedLocale: readSavedLocale(),
    navigatorLocales: readNavigatorLocales(),
  });
}

/** Flip the active locale, waiting for its catalog first so `t()` never
 *  renders raw keys mid-switch. */
function activateLocale(locale: Locale) {
  if (loadedLocales.has(locale)) {
    currentLocale.value = locale;
    return;
  }
  void loadCatalog(locale)
    .then(() => {
      currentLocale.value = locale;
    })
    .catch((error) => {
      console.warn(`[i18n] failed to load ${locale} catalog:`, error);
    });
}

export function bootstrapLocale(systemLocale?: string | null): Locale {
  const resolved = resolveLocale({
    savedLocale: readSavedLocale(),
    systemLocale,
    navigatorLocales: readNavigatorLocales(),
  });
  activateLocale(resolved);
  return resolved;
}

export function setLocale(locale: Locale) {
  try {
    localStorage.setItem(STORAGE_KEY, locale);
  } catch { /* ignore */ }
  activateLocale(locale);
}

export const locale = readonly(currentLocale);

// Top-level await: every module that imports `t` only executes once the
// active catalog is parsed, keeping `t()` synchronous with no key flash.
// The other catalog warms in the background for the cross-locale fallback
// in `t()` (and for instant language switching).
await loadCatalog(currentLocale.value);
void loadCatalog(currentLocale.value === "zh" ? "en" : "zh").catch(() => {
  /* fallback catalog is best-effort */
});

/**
 * Translation function: returns the string for the current locale by key
 * Supports {0}, {1} placeholder substitution
 */
export function t(key: string, ...args: (string | number)[]): string {
  const msg = messages[currentLocale.value]?.[key]
    ?? messages.en[key]
    ?? messages.zh[key]
    ?? key;
  if (args.length === 0) return msg;
  return msg.replace(/\{(\d+)\}/g, (_, idx) => {
    const i = parseInt(idx);
    return i < args.length ? String(args[i]) : `{${idx}}`;
  });
}

export function useI18n() {
  return { t, locale: currentLocale, setLocale };
}
