import { ref } from "vue";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { hasTauriWindowRuntime } from "../services/tauriRuntime";

export type ThemePreference = "system" | "light" | "dark";
export type ThemeScope = "main" | "unityEmbed";

const STORAGE_KEYS: Record<ThemeScope, string> = {
  main: "locus-theme-preference",
  unityEmbed: "locus-unity-embed-theme-preference",
};

const DEFAULT_PREFERENCES: Record<ThemeScope, ThemePreference> = {
  main: "system",
  unityEmbed: "dark",
};

const THEME_BACKGROUND_COLOR: Record<"light" | "dark", string> = {
  light: "#f6f7f8",
  dark: "#1d1d21",
};

const mainPreference = ref<ThemePreference>(readPreference("main"));
const unityEmbedPreference = ref<ThemePreference>(readPreference("unityEmbed"));
let lastNativeBackgroundColor: string | null = null;
let activeScope: ThemeScope = "main";
let storageListenerBound = false;

function preferenceRef(scope: ThemeScope) {
  return scope === "main" ? mainPreference : unityEmbedPreference;
}

function normalizePreference(value: string | null, fallback: ThemePreference): ThemePreference {
  if (value === "light" || value === "dark" || value === "system") return value;
  return fallback;
}

function readPreference(scope: ThemeScope): ThemePreference {
  try {
    return normalizePreference(
      localStorage.getItem(STORAGE_KEYS[scope]),
      DEFAULT_PREFERENCES[scope],
    );
  } catch { /* ignore */ }
  return DEFAULT_PREFERENCES[scope];
}

function resolveTheme(pref: ThemePreference): "light" | "dark" {
  if (pref === "light" || pref === "dark") return pref;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function applyTheme(theme: "light" | "dark") {
  document.documentElement.setAttribute("data-theme", theme);
  syncNativeBackgroundColor(theme);
}

function syncNativeBackgroundColor(theme: "light" | "dark") {
  if (!hasTauriWindowRuntime()) return;
  const color = THEME_BACKGROUND_COLOR[theme];
  if (lastNativeBackgroundColor === color) return;
  lastNativeBackgroundColor = color;
  void getCurrentWebviewWindow().setBackgroundColor(color).catch((error) => {
    lastNativeBackgroundColor = null;
    console.warn("Failed to sync native window background color:", error);
  });
}

let mediaQuery: MediaQueryList | null = null;
let mediaHandler: ((e: MediaQueryListEvent) => void) | null = null;

function bindSystemListener() {
  unbindSystemListener();
  const currentPreference = preferenceRef(activeScope);
  if (currentPreference.value !== "system") return;
  mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
  mediaHandler = (e: MediaQueryListEvent) => {
    if (preferenceRef(activeScope).value === "system") {
      applyTheme(e.matches ? "dark" : "light");
    }
  };
  mediaQuery.addEventListener("change", mediaHandler);
}

function unbindSystemListener() {
  if (mediaQuery && mediaHandler) {
    mediaQuery.removeEventListener("change", mediaHandler);
  }
  mediaQuery = null;
  mediaHandler = null;
}

function bindStorageListener() {
  if (storageListenerBound || typeof window === "undefined") return;
  window.addEventListener("storage", handleStorageChange);
  storageListenerBound = true;
}

function handleStorageChange(event: StorageEvent) {
  const scope = (Object.keys(STORAGE_KEYS) as ThemeScope[])
    .find((candidate) => STORAGE_KEYS[candidate] === event.key);
  if (!scope) return;

  const nextPreference = normalizePreference(event.newValue, DEFAULT_PREFERENCES[scope]);
  preferenceRef(scope).value = nextPreference;
  if (scope !== activeScope) return;

  applyTheme(resolveTheme(nextPreference));
  bindSystemListener();
}

export function setThemePreference(pref: ThemePreference): void;
export function setThemePreference(scope: ThemeScope, pref: ThemePreference): void;
export function setThemePreference(
  scopeOrPreference: ThemeScope | ThemePreference,
  maybePreference?: ThemePreference,
) {
  const scope = maybePreference ? scopeOrPreference as ThemeScope : "main";
  const pref = maybePreference ?? scopeOrPreference as ThemePreference;
  preferenceRef(scope).value = pref;
  try { localStorage.setItem(STORAGE_KEYS[scope], pref); } catch { /* ignore */ }
  if (scope !== activeScope) return;

  applyTheme(resolveTheme(pref));
  bindSystemListener();
}

/** Call once from App.vue for each window surface that uses shared app tokens. */
export function initTheme(scope: ThemeScope = "main") {
  activeScope = scope;
  applyTheme(resolveTheme(preferenceRef(scope).value));
  bindSystemListener();
  bindStorageListener();
}

/** Composable for reactive access in components. */
export function useTheme() {
  return {
    preference: mainPreference,
    mainPreference,
    unityEmbedPreference,
    setThemePreference,
  };
}
