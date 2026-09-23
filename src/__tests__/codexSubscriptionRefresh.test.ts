import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useSettingsState } from "../composables/useSettingsState";
import { useAuthStore } from "../stores/auth";
import { useModelStore } from "../stores/model";
import type { CodexRateLimitsResponse } from "../services/auth";
import type { ModelOption } from "../types";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), addNotice: vi.fn() }));

vi.mock("../services/ipc", () => ({ ipcInvoke: mocks.invoke }));
vi.mock("../stores/notification", () => ({
  useNotificationStore: () => ({ addNotice: mocks.addNotice }),
}));
vi.mock("vue", async (importOriginal) => ({
  ...await importOriginal<typeof import("vue")>(),
  onMounted: vi.fn(),
  onUnmounted: vi.fn(),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

const existingModel: ModelOption = {
  id: "openai/gpt-existing",
  name: "GPT Existing",
  provider: "openai_codex",
};
const newModel: ModelOption = {
  id: "openai/gpt-new",
  name: "GPT New",
  provider: "openai_codex",
};
const quota: CodexRateLimitsResponse = {
  fetchedAtMs: 1_735_689_600_000,
  rateLimits: { limitId: "codex" },
  rateLimitsByLimitId: {},
};

function setup() {
  useAuthStore().codexAuthenticated = true;
  const models = useModelStore();
  models.codexRemoteModels = [existingModel];
  models.selectedModelId = existingModel.id;
  const state = useSettingsState(vi.fn());
  state.codexStatus.value = {
    authenticated: true,
    accountId: "account-1",
    validationFailed: false,
    validationError: null,
  };
  const modelRequest = deferred<ModelOption[]>();
  const quotaRequest = deferred<CodexRateLimitsResponse>();
  mocks.invoke.mockImplementation((command: string) => {
    if (command === "get_codex_available_models") return modelRequest.promise;
    if (command === "codex_rate_limits") return quotaRequest.promise;
    throw new Error(`Unexpected IPC command: ${command}`);
  });
  return { state, models, modelRequest, quotaRequest };
}

describe("Codex subscription refresh", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it("refreshes quota and models together, waits for both, and ignores duplicate clicks", async () => {
    const { state, models, modelRequest, quotaRequest } = setup();
    const refresh = state.refreshCodexSubscription();

    expect(mocks.invoke).toHaveBeenCalledWith("codex_rate_limits");
    expect(mocks.invoke).toHaveBeenCalledWith("get_codex_available_models", { forceRefresh: true });
    expect(state.codexRefreshing.value).toBe(true);

    quotaRequest.resolve(quota);
    await vi.waitFor(() => expect(state.codexQuota.value.loaded).toBe(true));
    expect(state.codexRefreshing.value).toBe(true);
    await state.refreshCodexSubscription();
    expect(mocks.invoke).toHaveBeenCalledTimes(2);

    modelRequest.resolve([existingModel, newModel]);
    await refresh;

    expect(models.availableModels.map((model) => model.id)).toContain(newModel.id);
    expect(models.selectedModelId).toBe(existingModel.id);
    expect(state.codexRefreshing.value).toBe(false);
    expect(mocks.addNotice).not.toHaveBeenCalled();
  });

  it("preserves the model list and reports a failed refresh after quota finishes", async () => {
    const { state, models, modelRequest, quotaRequest } = setup();
    const refresh = state.refreshCodexSubscription();
    modelRequest.reject(new Error("Codex models request failed"));
    await vi.waitFor(() => expect(state.codexQuota.value.loading).toBe(true));
    expect(state.codexRefreshing.value).toBe(true);

    quotaRequest.resolve(quota);
    await refresh;

    expect(models.codexRemoteModels).toEqual([existingModel]);
    expect(models.selectedModelId).toBe(existingModel.id);
    expect(state.codexQuota.value.loaded).toBe(true);
    expect(state.codexRefreshing.value).toBe(false);
    expect(mocks.addNotice).toHaveBeenCalledWith("error", "Codex models request failed", {
      code: "unknown",
      operation: "codexModelsRefresh",
    });
  });

  it("still updates models when the quota request fails", async () => {
    const { state, models, modelRequest, quotaRequest } = setup();
    const refresh = state.refreshCodexSubscription();
    quotaRequest.reject(new Error("Quota unavailable"));
    modelRequest.resolve([existingModel, newModel]);
    await refresh;

    expect(models.availableModels.map((model) => model.id)).toContain(newModel.id);
    expect(state.codexQuota.value.error).toBe("Quota unavailable");
    expect(state.codexRefreshing.value).toBe(false);
  });

  it("keeps ordinary model loading cache-aware", async () => {
    const { models, modelRequest } = setup();
    modelRequest.resolve([existingModel]);
    await models.loadCodexAvailableModels();
    expect(mocks.invoke).toHaveBeenCalledWith("get_codex_available_models", { forceRefresh: false });
  });
});
