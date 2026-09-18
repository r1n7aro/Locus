import { reactive, readonly } from "vue";
import type { GlobalSearchSource } from "../services/globalSearch";

export type GlobalSearchSettings = Record<GlobalSearchSource | "enabled", boolean>;
const STORAGE_KEY = "locus-global-search-settings";
const defaults: GlobalSearchSettings = {
  enabled: true, knowledgeTitle: true, knowledgeContent: true, sessionTitle: true, sessionContent: true,
};
export function normalizeGlobalSearchSettings(value: unknown): GlobalSearchSettings {
  const record = value && typeof value === "object" ? value as Partial<GlobalSearchSettings> : {};
  return Object.fromEntries(Object.entries(defaults).map(([key, fallback]) => [
    key, typeof record[key as keyof GlobalSearchSettings] === "boolean" ? record[key as keyof GlobalSearchSettings] : fallback,
  ])) as GlobalSearchSettings;
}
function load(): GlobalSearchSettings {
  try { return normalizeGlobalSearchSettings(JSON.parse(localStorage.getItem(STORAGE_KEY) || "null")); }
  catch { return { ...defaults }; }
}
const state = reactive(load());
// Settings can also be opened in an independent native window.
if (typeof window !== "undefined") window.addEventListener("storage", (event) => {
  if (event.key === STORAGE_KEY || event.key === null) Object.assign(state, load());
});
export function useGlobalSearchSettings() {
  return {
    state: readonly(state),
    set(key: keyof GlobalSearchSettings, value: boolean) {
      state[key] = value;
      try { localStorage.setItem(STORAGE_KEY, JSON.stringify(state)); } catch { /* optional persistence */ }
    },
  };
}
