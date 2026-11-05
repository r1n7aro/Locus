// "Reset onboarding" in System settings: clears every setting and local
// preference, then drops the settings pages' cached state so the next visit
// reloads it.
import { clearWarmup } from "../../composables/warmupCache";
import { resetTextViewerZoomScale } from "../../composables/useTextViewerZoom";
import { useNewSessionSettings } from "../../composables/useNewSessionSettings";
import { setThemePreference } from "../../composables/useTheme";
import { resetAllConfig } from "../../services/project";
import { useCodexAccountStore } from "../../stores/codexAccount";
import { useCustomProviderSettingsStore } from "../../stores/customProviderSettings";
import { useModelStore } from "../../stores/model";
import { useProviderAccountsStore } from "../../stores/providerAccounts";
import { useSettingsFeedbackStore } from "../../stores/settingsFeedback";

/** localStorage entries outside `settings.json`: onboarding progress, panel
 *  layout, and preferences older versions kept here. */
const LOCAL_KEYS = [
  "locus-onboarding-completed",
  "locus-locale",
  "locus-theme-preference",
  "locus-unity-embed-theme-preference",
  "locus-knowledge-access-mode",
  "locus:sessionPanelWidth",
  "locus:unity:sessionPanelWidth",
  "locus:unity:sessionPanelCollapsed",
  "locus:chatSidebarWidth",
  "locus:chatSidebarHeight",
  "locus:unity:chatSidebarWidth",
  "locus:unity:chatSidebarHeight",
  "locus:collabLeftColWidth",
  "locus:collabTerminalHeight",
];

export async function resetToOnboarding(): Promise<void> {
  try {
    for (const key of LOCAL_KEYS) localStorage.removeItem(key);
  } catch { /* ignore */ }
  resetTextViewerZoomScale();
  useNewSessionSettings().reset();
  setThemePreference("main", "dark");
  setThemePreference("unityEmbed", "dark");
  try {
    await resetAllConfig();
  } catch (e) {
    console.error("reset_all_config failed:", e);
  }
  clearWarmup();
  useProviderAccountsStore().reset();
  useCodexAccountStore().reset();
  useCustomProviderSettingsStore().reset();
  useSettingsFeedbackStore().reset();
  const modelStore = useModelStore();
  modelStore.applyModelDefaults({
    mainModel: "",
    planModel: "",
    subagentModels: {},
    subagentEfforts: {},
    subagentFastModes: {},
  });
  modelStore.applyCustomProviders([]);
}
