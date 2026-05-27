import { ref } from "vue";
import { defineStore } from "pinia";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Window as TauriWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { hasTauriWindowRuntime } from "../services/tauriRuntime";

const WINDOW_RESIZE_SETTLE_DELAY_MS = 420;
const MIN_TRACKABLE_WINDOW_WIDTH_PX = 320;
const MIN_TRACKABLE_WINDOW_HEIGHT_PX = 120;
const NATIVE_WINDOW_CLIENT_SIZE_EVENT = "locus-native-window-client-size";

interface NativeWindowClientSizeEvent {
  width: number;
  height: number;
}

export const useUiStore = defineStore("ui", () => {
  const activeTab = ref<"chat" | "collab" | "knowledge" | "asset" | "views" | "agent" | "settings">("chat");
  const settingsCategoryHint = ref<"api" | "models" | "permissions" | "proxy" | "general" | "display" | "notifications" | "shortcuts" | "knowledge" | "archived" | "console" | "about" | null>(null);
  const alwaysOnTop = ref(false);
  const isMaximized = ref(false);
  const isWindowResizing = ref(false);
  const nativeWindowWidth = ref<number | null>(null);
  const nativeWindowHeight = ref<number | null>(null);
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
  const viewMounted = ref(false);
  const agentMounted = ref(false);
  const settingsMounted = ref(false);

  let appWindow: TauriWindow | null = null;
  let unlistenResize: UnlistenFn | null = null;
  let unlistenNativeClientSize: UnlistenFn | null = null;
  let resizeSettleTimer: ReturnType<typeof setTimeout> | null = null;
  let maximizedSyncSeq = 0;

  function resolveAppWindow(): TauriWindow | null {
    if (appWindow) return appWindow;
    if (!hasTauriWindowRuntime()) return null;
    try {
      appWindow = getCurrentWindow();
      return appWindow;
    } catch (error) {
      console.warn("Failed to resolve current Tauri window:", error);
      return null;
    }
  }

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

  function devicePixelRatioScale(): number {
    if (typeof window === "undefined") return 1;
    return Number.isFinite(window.devicePixelRatio) && window.devicePixelRatio > 0
      ? window.devicePixelRatio
      : 1;
  }

  function normalizeNativeDimension(value?: number): number | undefined {
    if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) return undefined;
    return Math.max(1, Math.round(value / devicePixelRatioScale()));
  }

  function isTrackableWindowSize(width: number | undefined, height?: number): width is number {
    if (width === undefined || width < MIN_TRACKABLE_WINDOW_WIDTH_PX) return false;
    return height === undefined || height >= MIN_TRACKABLE_WINDOW_HEIGHT_PX;
  }

  async function syncMaximizedState() {
    const syncSeq = ++maximizedSyncSeq;
    const window = resolveAppWindow();
    if (!window) {
      isMaximized.value = false;
      return;
    }
    try {
      const nextValue = await window.isMaximized();
      if (syncSeq !== maximizedSyncSeq) return;
      isMaximized.value = nextValue;
    } catch (error) {
      if (syncSeq !== maximizedSyncSeq) return;
      isMaximized.value = false;
      if (isUnsupportedMaximizedCheck(error)) return;
      console.error("Failed to sync maximized state:", error);
    }
  }

  function markWindowResizeInProgress() {
    isWindowResizing.value = true;
    clearResizeSettleTimer();
    resizeSettleTimer = setTimeout(() => {
      resizeSettleTimer = null;
      isWindowResizing.value = false;
      nativeWindowWidth.value = null;
      nativeWindowHeight.value = null;
      void syncMaximizedState();
    }, WINDOW_RESIZE_SETTLE_DELAY_MS);
  }

  function scheduleWindowResizeSettle(width?: number, height?: number) {
    const observedWidth = normalizeNativeDimension(width);
    const observedHeight = normalizeNativeDimension(height);
    if (width !== undefined && !isTrackableWindowSize(observedWidth, observedHeight)) return;
    markWindowResizeInProgress();
  }

  function applyNativeWindowClientSize(width: number, height: number) {
    const observedWidth = normalizeNativeDimension(width);
    const observedHeight = normalizeNativeDimension(height);
    if (observedHeight === undefined || !isTrackableWindowSize(observedWidth, observedHeight)) return;
    nativeWindowWidth.value = observedWidth;
    nativeWindowHeight.value = observedHeight;
    markWindowResizeInProgress();
  }

  async function init() {
    await syncMaximizedState();
    const window = resolveAppWindow();
    if (window) {
      try {
        unlistenResize = await window.onResized((event) => {
          scheduleWindowResizeSettle(event.payload.width, event.payload.height);
        });
      } catch (error) {
        console.error("Failed to listen for window resize:", error);
      }
    }
    if (hasTauriWindowRuntime()) {
      try {
        unlistenNativeClientSize = await listen<NativeWindowClientSizeEvent>(
          NATIVE_WINDOW_CLIENT_SIZE_EVENT,
          (event) => {
            applyNativeWindowClientSize(event.payload.width, event.payload.height);
          },
        );
      } catch (error) {
        console.error("Failed to listen for native window client size:", error);
      }
    }
    try {
      setTab("chat");
      showOnboarding.value = !localStorage.getItem("locus-onboarding-completed");
    } catch (error) {
      setTab("chat");
      console.error("Failed to read onboarding completion state:", error);
      showOnboarding.value = false;
    }
  }

  function cleanup() {
    clearResizeSettleTimer();
    isWindowResizing.value = false;
    nativeWindowWidth.value = null;
    nativeWindowHeight.value = null;
    maximizedSyncSeq += 1;
    unlistenResize?.();
    unlistenResize = null;
    unlistenNativeClientSize?.();
    unlistenNativeClientSize = null;
  }

  function setTab(tab: typeof activeTab.value) {
    activeTab.value = tab;
    if (tab === "collab") collabMounted.value = true;
    if (tab === "knowledge") knowledgeMounted.value = true;
    if (tab === "asset") assetMounted.value = true;
    if (tab === "views") viewMounted.value = true;
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
    const window = resolveAppWindow();
    if (!window) return;
    alwaysOnTop.value = !alwaysOnTop.value;
    try {
      await window.setAlwaysOnTop(alwaysOnTop.value);
    } catch (e) {
      console.error("Failed to set always on top:", e);
      alwaysOnTop.value = !alwaysOnTop.value;
    }
  }

  async function winMinimize() {
    await resolveAppWindow()?.minimize();
  }
  async function winToggleMaximize() {
    await resolveAppWindow()?.toggleMaximize();
    clearResizeSettleTimer();
    isWindowResizing.value = false;
    nativeWindowWidth.value = null;
    nativeWindowHeight.value = null;
    await syncMaximizedState();
  }
  async function winClose() {
    await resolveAppWindow()?.close();
  }

  function completeOnboarding() {
    try {
      localStorage.setItem("locus-onboarding-completed", "1");
    } catch (error) {
      console.error("Failed to persist onboarding completion state:", error);
    }
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
    nativeWindowWidth,
    nativeWindowHeight,
    showOnboarding,
    pendingChatPrefill,
    pendingKnowledgeSelection,
    collabMounted,
    knowledgeMounted,
    assetMounted,
    viewMounted,
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
