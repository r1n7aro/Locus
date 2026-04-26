import { ref, onMounted, onUnmounted } from "vue";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { hasTauriWindowRuntime } from "../services/tauriRuntime";

export type ThemePreference = "system" | "light" | "dark";

const STORAGE_KEY = "locus-theme-preference";
const THEME_BACKGROUND_COLOR: Record<"light" | "dark", string> = {
  light: "#f6f7f8",
  dark: "#1d1d21",
};

const preference = ref<ThemePreference>(readPreference());
let lastNativeBackgroundColor: string | null = null;

function readPreference(): ThemePreference {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "light" || v === "dark" || v === "system") return v;
  } catch { /* ignore */ }
  return "system";
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
  if (preference.value !== "system") return;
  mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
  mediaHandler = (e: MediaQueryListEvent) => {
    if (preference.value === "system") {
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

export function setThemePreference(pref: ThemePreference) {
  preference.value = pref;
  try { localStorage.setItem(STORAGE_KEY, pref); } catch { /* ignore */ }
  applyTheme(resolveTheme(pref));
  bindSystemListener();
}

/** Call once from App.vue (main window only, not canvas) */
export function initTheme() {
  applyTheme(resolveTheme(preference.value));
  bindSystemListener();
}

/** Composable for reactive access in components */
export function useTheme() {
  onMounted(() => {
    // ensure listener is active when component mounts
    bindSystemListener();
  });

  onUnmounted(() => {
    unbindSystemListener();
  });

  return {
    preference,
    setThemePreference,
  };
}
