import { reactive, readonly, watch } from "vue";
import type { GlobalSearchSource } from "../services/globalSearch";
import { GLOBAL_SEARCH_FIELD_KEYS } from "./settingsLegacy";
import { saveSetting, settingValue } from "./useSettingsStore";

export type GlobalSearchSettings = Record<GlobalSearchSource | "enabled" | "includeArchived", boolean>;
const defaults: GlobalSearchSettings = {
  enabled: true, knowledgeTitle: true, knowledgeContent: true, sessionTitle: true, sessionContent: true,
  fileName: true, viewName: true, includeArchived: true,
};
export function normalizeGlobalSearchSettings(value: unknown): GlobalSearchSettings {
  const record = value && typeof value === "object" ? value as Partial<GlobalSearchSettings> : {};
  return Object.fromEntries(Object.entries(defaults).map(([key, fallback]) => [
    key, typeof record[key as keyof GlobalSearchSettings] === "boolean" ? record[key as keyof GlobalSearchSettings] : fallback,
  ])) as GlobalSearchSettings;
}
const FIELD_KEYS: Record<keyof GlobalSearchSettings, string> = GLOBAL_SEARCH_FIELD_KEYS;
function project(): GlobalSearchSettings {
  return normalizeGlobalSearchSettings(Object.fromEntries(
    Object.entries(FIELD_KEYS).map(([field, key]) => [field, settingValue(key)]),
  ));
}
const state = reactive(project());
// Settings can also be opened in an independent native window.
watch(project, (next) => Object.assign(state, next), { flush: "sync" });
export function useGlobalSearchSettings() {
  return {
    state: readonly(state),
    set(key: keyof GlobalSearchSettings, value: boolean) {
      saveSetting(FIELD_KEYS[key], value);
    },
  };
}
