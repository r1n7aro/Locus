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
    emitResize(width = 1440, height = 900) {
      resizedHandler?.({ payload: { width, height } });
    },
  };
});

const tauriEventMocks = vi.hoisted(() => {
  let listeners = new Map<string, (event: { payload: { width: number; height: number } }) => void>();

  return {
    listen: vi.fn(async (
      eventName: string,
      handler: (event: { payload: { width: number; height: number } }) => void,
    ) => {
      listeners.set(eventName, handler);
      return () => {
        listeners.delete(eventName);
      };
    }),
    emitNativeClientSize(width = 1440, height = 900) {
      listeners.get("locus-native-window-client-size")?.({ payload: { width, height } });
    },
    reset() {
      listeners = new Map<string, (event: { payload: { width: number; height: number } }) => void>();
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

vi.mock("@tauri-apps/api/event", () => ({
  listen: tauriEventMocks.listen,
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
    tauriEventMocks.reset();
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
    expect(store.nativeWindowWidth).toBeNull();
    expect(store.nativeWindowHeight).toBeNull();

    tauriEventMocks.emitNativeClientSize();

    expect(store.isWindowResizing).toBe(true);
    expect(store.nativeWindowWidth).toBe(1440);
    expect(store.nativeWindowHeight).toBe(900);

    tauriWindowMocks.isMaximized.mockResolvedValueOnce(true);
    await vi.advanceTimersByTimeAsync(420);

    expect(tauriWindowMocks.isMaximized).toHaveBeenCalledTimes(2);
    expect(store.isMaximized).toBe(true);
    expect(store.isWindowResizing).toBe(false);
    expect(store.nativeWindowWidth).toBeNull();
    expect(store.nativeWindowHeight).toBeNull();
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

  it("does not derive layout offsets from unstable WebView screen coordinates", async () => {
    const windowMock = {
      innerWidth: 1200,
      innerHeight: 900,
      devicePixelRatio: 2,
      screenX: 100,
      screenLeft: 100,
    };
    vi.stubGlobal("window", windowMock);
    const store = useUiStore();

    await store.init();

    windowMock.innerWidth = 1200;
    windowMock.screenX = 160;
    windowMock.screenLeft = 160;
    tauriWindowMocks.emitResize(2280);

    expect("windowResizeAnchor" in store).toBe(false);
    expect("resizeAnchorWidth" in store).toBe(false);
    expect(store.isWindowResizing).toBe(true);
    expect(store.nativeWindowWidth).toBeNull();

    tauriEventMocks.emitNativeClientSize(2280, 1800);

    expect(store.nativeWindowWidth).toBe(1140);
    expect(store.nativeWindowHeight).toBe(900);

    windowMock.innerWidth = 1140;
    tauriWindowMocks.isMaximized.mockResolvedValueOnce(false);
    await vi.advanceTimersByTimeAsync(420);

    expect(store.isWindowResizing).toBe(false);
  });

  it("ignores minimized offscreen resize dimensions", async () => {
    const windowMock = {
      innerWidth: 1200,
      innerHeight: 900,
      devicePixelRatio: 1,
    };
    vi.stubGlobal("window", windowMock);
    const store = useUiStore();

    await store.init();

    tauriWindowMocks.emitResize(160, 28);
    await vi.advanceTimersByTimeAsync(420);

    expect(store.isWindowResizing).toBe(false);

    windowMock.innerWidth = 1274;
    tauriWindowMocks.emitResize(1274, 900);

    expect(store.isWindowResizing).toBe(true);
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
