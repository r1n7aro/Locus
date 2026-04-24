import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useUiStore } from "../stores/ui";

const tauriWindowMocks = vi.hoisted(() => {
  let resizedHandler: ((event: { payload: { width: number; height: number } }) => void) | null = null;
  const windowMock = {
    isMaximized: vi.fn<() => Promise<boolean>>(),
    onResized: vi.fn(async (handler: (event: { payload: { width: number; height: number } }) => void) => {
      resizedHandler = handler;
      return () => undefined;
    }),
    setAlwaysOnTop: vi.fn(async () => undefined),
    minimize: vi.fn(async () => undefined),
    toggleMaximize: vi.fn(async () => undefined),
    close: vi.fn(async () => undefined),
  };

  return {
    ...windowMock,
    getCurrentWindow: vi.fn(() => windowMock),
    emitResize() {
      resizedHandler?.({ payload: { width: 1440, height: 900 } });
    },
  };
});

const localStorageMock = vi.hoisted(() => {
  let storage = new Map<string, string>();
  return {
    getItem: vi.fn((key: string) => storage.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => {
      storage.set(key, String(value));
    }),
    removeItem: vi.fn((key: string) => {
      storage.delete(key);
    }),
    clear: vi.fn(() => {
      storage = new Map<string, string>();
    }),
  };
});

const tauriRuntimeMocks = vi.hoisted(() => ({
  hasTauriWindowRuntime: vi.fn(() => true),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: tauriWindowMocks.getCurrentWindow,
}));

vi.mock("../services/tauriRuntime", () => ({
  hasTauriWindowRuntime: tauriRuntimeMocks.hasTauriWindowRuntime,
}));

describe("ui store window resize sync", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
    vi.clearAllMocks();
    vi.stubGlobal("localStorage", localStorageMock as unknown as Storage);
    localStorageMock.clear();
    tauriRuntimeMocks.hasTauriWindowRuntime.mockReturnValue(true);
    tauriWindowMocks.isMaximized.mockResolvedValue(false);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("debounces maximized-state sync while the window is being resized", async () => {
    const store = useUiStore();

    await store.init();
    expect(tauriWindowMocks.isMaximized).toHaveBeenCalledTimes(1);
    expect(store.isMaximized).toBe(false);
    expect(store.isWindowResizing).toBe(false);

    tauriWindowMocks.emitResize();
    tauriWindowMocks.emitResize();
    tauriWindowMocks.emitResize();

    expect(tauriWindowMocks.isMaximized).toHaveBeenCalledTimes(1);
    expect(store.isWindowResizing).toBe(true);

    tauriWindowMocks.isMaximized.mockResolvedValueOnce(true);
    await vi.advanceTimersByTimeAsync(120);

    expect(tauriWindowMocks.isMaximized).toHaveBeenCalledTimes(2);
    expect(store.isMaximized).toBe(true);
    expect(store.isWindowResizing).toBe(false);
  });

  it("refreshes maximized state immediately after toggling maximize", async () => {
    const store = useUiStore();

    await store.init();
    tauriWindowMocks.isMaximized.mockResolvedValueOnce(true);

    await store.winToggleMaximize();

    expect(tauriWindowMocks.toggleMaximize).toHaveBeenCalledTimes(1);
    expect(tauriWindowMocks.isMaximized).toHaveBeenCalledTimes(2);
    expect(store.isMaximized).toBe(true);
  });

  it("falls back to non-maximized when the current window lacks maximize capability", async () => {
    const store = useUiStore();

    tauriWindowMocks.isMaximized.mockRejectedValueOnce(
      new Error(
        'window.is_maximized not allowed on window "feishu-reference-import-progress", permission: allow-is-maximized',
      ),
    );

    await expect(store.init()).resolves.toBeUndefined();

    expect(store.isMaximized).toBe(false);
    expect(tauriWindowMocks.onResized).toHaveBeenCalledTimes(1);
  });

  it("starts on the chat tab even when a previous tab was stored", async () => {
    localStorage.setItem("locus-active-tab", "knowledge");
    const store = useUiStore();

    await store.init();

    expect(store.activeTab).toBe("chat");
    expect(store.knowledgeMounted).toBe(false);
  });

  it("initializes local UI state when the Tauri window runtime is unavailable", async () => {
    tauriRuntimeMocks.hasTauriWindowRuntime.mockReturnValue(false);
    localStorage.setItem("locus-active-tab", "knowledge");
    const store = useUiStore();

    await expect(store.init()).resolves.toBeUndefined();

    expect(tauriWindowMocks.getCurrentWindow).not.toHaveBeenCalled();
    expect(store.isMaximized).toBe(false);
    expect(store.activeTab).toBe("chat");
    expect(store.knowledgeMounted).toBe(false);
  });
});
