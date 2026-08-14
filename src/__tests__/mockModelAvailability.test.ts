import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { nextTick } from "vue";
import { useModelStore } from "../stores/model";

const permissionMocks = vi.hoisted(() => {
  let listener: ((enabled: boolean) => void) | null = null;
  return {
    getCachedDebugMode: vi.fn(() => false),
    getDebugMode: vi.fn(async () => true),
    subscribeDebugMode: vi.fn((next: (enabled: boolean) => void) => {
      listener = next;
      return () => {
        listener = null;
      };
    }),
    publish(enabled: boolean) {
      listener?.(enabled);
    },
  };
});

const modelServiceMocks = vi.hoisted(() => ({
  getModelDefaults: vi.fn(),
  getLastModel: vi.fn(),
  getLastEffort: vi.fn(),
  getCodexFastMode: vi.fn(),
  getCustomProviders: vi.fn(),
  getCodexModelConfig: vi.fn(),
  getCodexAvailableModels: vi.fn(),
  saveLastModel: vi.fn(async () => undefined),
  saveLastEffort: vi.fn(async () => undefined),
  saveCodexFastMode: vi.fn(async () => undefined),
}));

vi.mock("../services/permissions", () => permissionMocks);
vi.mock("../services/model", () => modelServiceMocks);

describe("simulated model availability", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    permissionMocks.getCachedDebugMode.mockReturnValue(false);
    permissionMocks.getDebugMode.mockResolvedValue(true);
  });

  it("exposes local model presets only while Debug mode is enabled", async () => {
    const store = useModelStore();

    expect(store.availableModels.filter((model) => model.provider === "mock")).toEqual([]);

    await store.loadDebugMode();
    expect(store.availableModels.filter((model) => model.provider === "mock").map((model) => model.id))
      .toEqual(["mock/stream", "mock/tool", "mock/error"]);

    store.selectModel("mock/tool");
    expect(store.selectedModelId).toBe("mock/tool");

    permissionMocks.publish(false);
    await nextTick();

    expect(store.availableModels.filter((model) => model.provider === "mock")).toEqual([]);
    expect(store.selectedModelId).not.toBe("mock/tool");
  });
});
