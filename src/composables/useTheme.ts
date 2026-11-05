import { ref, watch } from "vue";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { hasTauriWindowRuntime } from "../services/tauriRuntime";
import { saveSetting, settingValue } from "./useSettingsStore";

export type ThemePreference = "system" | "light" | "dark";
export type ThemeScope = "main" | "unityEmbed";

const SETTING_KEYS: Record<ThemeScope, string> = {
  main: "appearance.theme.main",
  unityEmbed: "appearance.theme.unityEmbed",
};

const DEFAULT_PREFERENCES: Record<ThemeScope, ThemePreference> = {
  main: "dark",
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
let themeInitialized = false;

function preferenceRef(scope: ThemeScope) {
  return scope === "main" ? mainPreference : unityEmbedPreference;
}

function normalizePreference(value: unknown, fallback: ThemePreference): ThemePreference {
  if (value === "light" || value === "dark" || value === "system") return value;
  return fallback;
}

function readPreference(scope: ThemeScope): ThemePreference {
  return normalizePreference(settingValue(SETTING_KEYS[scope]), DEFAULT_PREFERENCES[scope]);
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

/** Follows the settings snapshot, so changes from other windows and the
 *  agent apply here too. */
function applyPreferenceChange(scope: ThemeScope, next: ThemePreference) {
  const preference = preferenceRef(scope);
  if (preference.value === next) return;
  preference.value = next;
  if (scope !== activeScope || !themeInitialized) return;
  applyTheme(resolveTheme(next));
  bindSystemListener();
}

for (const scope of Object.keys(SETTING_KEYS) as ThemeScope[]) {
  watch(() => readPreference(scope), (next) => applyPreferenceChange(scope, next), { flush: "sync" });
}

export function setThemePreference(pref: ThemePreference): void;
export function setThemePreference(scope: ThemeScope, pref: ThemePreference): void;
export function setThemePreference(
  scopeOrPreference: ThemeScope | ThemePreference,
  maybePreference?: ThemePreference,
) {
  const scope = maybePreference ? scopeOrPreference as ThemeScope : "main";
  const pref = maybePreference ?? scopeOrPreference as ThemePreference;
  // The snapshot updates synchronously, which applies the theme here.
  saveSetting(SETTING_KEYS[scope], pref);
}

/** Resolved background color for the current theme; used as the native
 *  window background when creating sub-windows so the pre-paint phase
 *  matches the app instead of flashing WebView2 white. */
export function currentThemeBackgroundColor(scope: ThemeScope = activeScope): string {
  return THEME_BACKGROUND_COLOR[resolveTheme(preferenceRef(scope).value)];
}
export function resolvedTheme(scope: ThemeScope): "light" | "dark" {
  return resolveTheme(preferenceRef(scope).value);
}

/** Call once from App.vue for each window surface that uses shared app tokens. */
export function initTheme(scope: ThemeScope = "main") {
  activeScope = scope;
  themeInitialized = true;
  applyTheme(resolveTheme(preferenceRef(scope).value));
  bindSystemListener();
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
