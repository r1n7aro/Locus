import { describe, expect, it, vi, beforeEach } from "vitest";

const modelServiceMocks = vi.hoisted(() => ({
  getCustomEndpoints: vi.fn(),
  saveCustomEndpoints: vi.fn(),
  testCustomEndpoint: vi.fn(),
  getProviders: vi.fn(),
  saveProviderKey: vi.fn(),
  deleteProviderKey: vi.fn(),
  getAuthUrl: vi.fn(),
  exchangeAuthCode: vi.fn(),
  authLogout: vi.fn(),
  codexStatus: vi.fn(),
  codexStartLogin: vi.fn(),
  codexPollLogin: vi.fn(),
  codexLogout: vi.fn(),
  codexRetryAuth: vi.fn(),
  getModelDefaults: vi.fn(),
  saveModelDefaults: vi.fn(),
  getCodexModelConfig: vi.fn(),
  saveCodexModelConfig: vi.fn(),
  getToolPermissions: vi.fn(),
  saveToolPermissions: vi.fn(),
  resetAllConfig: vi.fn(),
  setWarmup: vi.fn(),
  getWarmup: vi.fn(),
  clearWarmup: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(),
}));

vi.mock("../services/auth", () => ({
  getProviders: modelServiceMocks.getProviders,
  saveProviderKey: modelServiceMocks.saveProviderKey,
  deleteProviderKey: modelServiceMocks.deleteProviderKey,
  getAuthUrl: modelServiceMocks.getAuthUrl,
  exchangeAuthCode: modelServiceMocks.exchangeAuthCode,
  authLogout: modelServiceMocks.authLogout,
  codexStatus: modelServiceMocks.codexStatus,
  codexStartLogin: modelServiceMocks.codexStartLogin,
  codexPollLogin: modelServiceMocks.codexPollLogin,
  codexLogout: modelServiceMocks.codexLogout,
  codexRetryAuth: modelServiceMocks.codexRetryAuth,
}));

vi.mock("../services/model", () => ({
  getCustomEndpoints: modelServiceMocks.getCustomEndpoints,
  saveCustomEndpoints: modelServiceMocks.saveCustomEndpoints,
  testCustomEndpoint: modelServiceMocks.testCustomEndpoint,
  getModelDefaults: modelServiceMocks.getModelDefaults,
  saveModelDefaults: modelServiceMocks.saveModelDefaults,
  getCodexModelConfig: modelServiceMocks.getCodexModelConfig,
  saveCodexModelConfig: modelServiceMocks.saveCodexModelConfig,
}));

vi.mock("../services/permissions", () => ({
  getToolPermissions: modelServiceMocks.getToolPermissions,
  saveToolPermissions: modelServiceMocks.saveToolPermissions,
}));

vi.mock("../services/project", () => ({
  resetAllConfig: modelServiceMocks.resetAllConfig,
}));

vi.mock("../composables/warmupCache", () => ({
  setWarmup: modelServiceMocks.setWarmup,
  getWarmup: modelServiceMocks.getWarmup,
  clearWarmup: modelServiceMocks.clearWarmup,
}));

vi.mock("../composables/useCopyFeedback", () => ({
  useCopyFeedback: () => ({
    copied: { value: false },
    copyText: vi.fn(),
    reset: vi.fn(),
  }),
}));

vi.mock("../stores/notification", () => ({
  useNotificationStore: () => ({
    addNotice: vi.fn(),
  }),
}));

import { useSettingsState } from "../composables/useSettingsState";
import type { CustomEndpoint } from "../types";

function endpoint(partial: Partial<CustomEndpoint> & Pick<CustomEndpoint, "id" | "name">): CustomEndpoint {
  return {
    apiModel: "model",
    endpoint: "https://example.com/v1",
    apiFormat: "openai_chat",
    apiKey: "",
    contextLength: 128000,
    betaFlags: [],
    supportedReasoningEfforts: ["low", "medium", "high", "max"],
    reasoningParamFormat: "openai_chat_reasoning_effort",
    ...partial,
  };
}

describe("custom endpoint persistence", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    modelServiceMocks.getWarmup.mockReturnValue(undefined);
    modelServiceMocks.getCustomEndpoints.mockResolvedValue([]);
    modelServiceMocks.saveCustomEndpoints.mockResolvedValue(undefined);
  });

  it("reloads saved endpoints and refreshes the warmup cache", async () => {
    const emitted: unknown[][] = [];
    const state = useSettingsState(((...args: unknown[]) => {
      emitted.push(args);
    }) as never);
    const saved = endpoint({ id: "saved", name: "Saved", apiKey: "sk-live" });
    modelServiceMocks.getCustomEndpoints.mockResolvedValueOnce([saved]);

    state.startAddEndpoint();
    state.editingEndpoint.value = endpoint({
      id: "draft",
      name: "Draft",
      apiModel: "draft-model",
      apiKey: "sk-draft",
    });

    await state.saveEndpoint();

    expect(modelServiceMocks.saveCustomEndpoints).toHaveBeenCalledWith([
      expect.objectContaining({ id: "draft", name: "Draft", apiKey: "sk-draft" }),
    ]);
    expect(state.customEndpoints.value).toEqual([saved]);
    expect(modelServiceMocks.setWarmup).toHaveBeenCalledWith("settings:customEndpoints", [saved]);
    expect(emitted).toContainEqual(["customEndpointsChanged", [saved]]);
    expect(state.customEndpointSaving.value).toBe(false);
    expect(state.editingEndpoint.value).toBeNull();
  });

  it("serializes delete mutations against the latest reloaded list", async () => {
    const state = useSettingsState((() => undefined) as never);
    const first = endpoint({ id: "first", name: "First" });
    const second = endpoint({ id: "second", name: "Second" });
    state.customEndpoints.value = [first, second];

    let releaseFirstSave!: () => void;
    modelServiceMocks.saveCustomEndpoints
      .mockImplementationOnce(() => new Promise<void>((resolve) => {
        releaseFirstSave = resolve;
      }))
      .mockResolvedValueOnce(undefined);
    modelServiceMocks.getCustomEndpoints
      .mockResolvedValueOnce([second])
      .mockResolvedValueOnce([]);

    const firstDelete = state.deleteEndpoint("first");
    const secondDelete = state.deleteEndpoint("second");
    await Promise.resolve();
    await Promise.resolve();

    expect(state.customEndpointSaving.value).toBe(true);
    expect(modelServiceMocks.saveCustomEndpoints).toHaveBeenCalledTimes(1);
    expect(modelServiceMocks.saveCustomEndpoints).toHaveBeenNthCalledWith(1, [second]);

    releaseFirstSave();
    await Promise.all([firstDelete, secondDelete]);

    expect(modelServiceMocks.saveCustomEndpoints).toHaveBeenCalledTimes(2);
    expect(modelServiceMocks.saveCustomEndpoints).toHaveBeenNthCalledWith(2, []);
    expect(state.customEndpoints.value).toEqual([]);
    expect(state.customEndpointSaving.value).toBe(false);
  });
});
