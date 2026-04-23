import { ref } from "vue";
import { defineStore } from "pinia";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";

const WINDOW_RESIZE_SETTLE_DELAY_MS = 120;
const ACTIVE_TAB_STORAGE_KEY = "locus-active-tab";

function isValidTab(
  value: string | null,
): value is "chat" | "collab" | "knowledge" | "asset" | "agent" | "settings" {
  return value === "chat"
    || value === "collab"
    || value === "knowledge"
    || value === "asset"
    || value === "agent"
    || value === "settings";
}

export const useUiStore = defineStore("ui", () => {
  const activeTab = ref<"chat" | "collab" | "knowledge" | "asset" | "agent" | "settings">("chat");
  const settingsCategoryHint = ref<"api" | "models" | "permissions" | "general" | "display" | "shortcuts" | "knowledge" | "archived" | "console" | "about" | null>(null);
  const alwaysOnTop = ref(false);
  const isMaximized = ref(false);
  const isWindowResizing = ref(false);
  const showOnboarding = ref(false);
  const pendingChatPrefill = ref<{ id: number; text: string } | null>(null);
  const pendingKnowledgeSelection = ref<{
    id: number;
    dashboard: "design" | "memory" | "skill" | "reference";
    path: string;
  } | null>(null);

  const collabMounted = ref(false);
  const knowledgeMounted = ref(false);
  const assetMounted = ref(false);
  const agentMounted = ref(false);
  const settingsMounted = ref(false);

  const appWindow = getCurrentWindow();
  let unlistenResize: UnlistenFn | null = null;
  let resizeSettleTimer: ReturnType<typeof setTimeout> | null = null;
  let maximizedSyncSeq = 0;

  function errorMessage(error: unknown): string {
    if (typeof error === "string") return error;
    if (error instanceof Error) return error.message;
    return String(error ?? "");
  }

  function isUnsupportedMaximizedCheck(error: unknown): boolean {
    const message = errorMessage(error);
    return message.includes("allow-is-maximized") && message.includes("not allowed");
  }

  function clearResizeSettleTimer() {
    if (resizeSettleTimer === null) return;
    clearTimeout(resizeSettleTimer);
    resizeSettleTimer = null;
  }

  async function syncMaximizedState() {
    const syncSeq = ++maximizedSyncSeq;
    try {
      const nextValue = await appWindow.isMaximized();
      if (syncSeq !== maximizedSyncSeq) return;
      isMaximized.value = nextValue;
    } catch (error) {
      if (syncSeq !== maximizedSyncSeq) return;
      isMaximized.value = false;
      if (isUnsupportedMaximizedCheck(error)) return;
      console.error("Failed to sync maximized state:", error);
    }
  }

  function scheduleWindowResizeSettle() {
    isWindowResizing.value = true;
    clearResizeSettleTimer();
    resizeSettleTimer = setTimeout(() => {
      resizeSettleTimer = null;
      isWindowResizing.value = false;
      void syncMaximizedState();
    }, WINDOW_RESIZE_SETTLE_DELAY_MS);
  }

  async function init() {
    await syncMaximizedState();
    unlistenResize = await appWindow.onResized(() => {
      scheduleWindowResizeSettle();
    });
    try {
      const storedTab = localStorage.getItem(ACTIVE_TAB_STORAGE_KEY);
      if (isValidTab(storedTab)) {
        setTab(storedTab);
      } else {
        setTab("chat");
      }
      showOnboarding.value = !localStorage.getItem("locus-onboarding-completed");
    } catch {
      setTab("chat");
      showOnboarding.value = false;
    }
  }

  function cleanup() {
    clearResizeSettleTimer();
    isWindowResizing.value = false;
    maximizedSyncSeq += 1;
    unlistenResize?.();
    unlistenResize = null;
  }

  function setTab(tab: typeof activeTab.value) {
    activeTab.value = tab;
    try {
      localStorage.setItem(ACTIVE_TAB_STORAGE_KEY, tab);
    } catch {
      /* ignore */
    }
    if (tab === "collab") collabMounted.value = true;
    if (tab === "knowledge") knowledgeMounted.value = true;
    if (tab === "asset") assetMounted.value = true;
    if (tab === "agent") agentMounted.value = true;
    if (tab === "settings") settingsMounted.value = true;
  }

  function openSettingsCategory(category: NonNullable<typeof settingsCategoryHint.value>) {
    settingsCategoryHint.value = category;
    setTab("settings");
  }

  function clearSettingsCategoryHint() {
    settingsCategoryHint.value = null;
  }

  function stageChatPrefill(text: string) {
    pendingChatPrefill.value = {
      id: Date.now(),
      text,
    };
  }

  function clearPendingChatPrefill(id?: number) {
    if (!pendingChatPrefill.value) return;
    if (id != null && pendingChatPrefill.value.id !== id) return;
    pendingChatPrefill.value = null;
  }

  function stageKnowledgeSelection(selection: Omit<NonNullable<typeof pendingKnowledgeSelection.value>, "id">) {
    pendingKnowledgeSelection.value = {
      id: Date.now(),
      ...selection,
    };
  }

  function clearPendingKnowledgeSelection(id?: number) {
    if (!pendingKnowledgeSelection.value) return;
    if (id != null && pendingKnowledgeSelection.value.id !== id) return;
    pendingKnowledgeSelection.value = null;
  }

  async function toggleAlwaysOnTop() {
    alwaysOnTop.value = !alwaysOnTop.value;
    try {
      await appWindow.setAlwaysOnTop(alwaysOnTop.value);
    } catch (e) {
      console.error("Failed to set always on top:", e);
      alwaysOnTop.value = !alwaysOnTop.value;
    }
  }

  async function winMinimize() {
    await appWindow.minimize();
  }
  async function winToggleMaximize() {
    await appWindow.toggleMaximize();
    clearResizeSettleTimer();
    isWindowResizing.value = false;
    await syncMaximizedState();
  }
  async function winClose() {
    await appWindow.close();
  }

  function completeOnboarding() {
    try {
      localStorage.setItem("locus-onboarding-completed", "1");
    } catch { /* ignore */ }
    showOnboarding.value = false;
  }

  function resetOnboarding() {
    showOnboarding.value = true;
  }

  return {
    activeTab,
    settingsCategoryHint,
    alwaysOnTop,
    isMaximized,
    isWindowResizing,
    showOnboarding,
    pendingChatPrefill,
    pendingKnowledgeSelection,
    collabMounted,
    knowledgeMounted,
    assetMounted,
    agentMounted,
    settingsMounted,
    init,
    cleanup,
    setTab,
    openSettingsCategory,
    clearSettingsCategoryHint,
    stageChatPrefill,
    clearPendingChatPrefill,
    stageKnowledgeSelection,
    clearPendingKnowledgeSelection,
    toggleAlwaysOnTop,
    winMinimize,
    winToggleMaximize,
    winClose,
    completeOnboarding,
    resetOnboarding,
  };
});
