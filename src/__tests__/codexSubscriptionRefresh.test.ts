import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CodexRateLimitsResponse } from "../services/auth";
import type { ModelOption } from "../types";
import { installFakeSettingsBackend, type FakeSettingsBackend } from "./fixtures/settingsBackend";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), addNotice: vi.fn(), confirm: vi.fn() }));

vi.mock("../services/ipc", () => ({ ipcInvoke: mocks.invoke }));
vi.mock("../stores/notification", () => ({
  useNotificationStore: () => ({ addNotice: mocks.addNotice }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: mocks.confirm }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

const AUTO_UPDATE_KEY = "providers.codex.autoUpdateClientVersion";

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
const signedIn = {
  authenticated: true,
  accountId: "account-1",
  validationFailed: false,
  validationError: null,
};

let backend: FakeSettingsBackend | null = null;

/** Fresh modules on a fake desktop settings backend holding `settings`. */
async function setup(settings: Record<string, unknown> = {}) {
  vi.resetModules();
  backend = await installFakeSettingsBackend(settings);
  const { createPinia, setActivePinia } = await import("pinia");
  setActivePinia(createPinia());
  const { ensureSettingsLoaded } = await import("../composables/useSettingsStore");
  const { useCodexAccountStore } = await import("../stores/codexAccount");
  const { useAuthStore } = await import("../stores/auth");
  const { useModelStore } = await import("../stores/model");
  await ensureSettingsLoaded();

  useAuthStore().codexAuthenticated = true;
  const models = useModelStore();
  models.codexRemoteModels = [existingModel];
  models.selectedModelId = existingModel.id;
  const codex = useCodexAccountStore();
  codex.status = { ...signedIn };
  const modelRequest = deferred<ModelOption[]>();
  const quotaRequest = deferred<CodexRateLimitsResponse>();
  mocks.invoke.mockImplementation((command: string) => {
    if (command === "get_codex_available_models") return modelRequest.promise;
    if (command === "codex_rate_limits") return quotaRequest.promise;
    throw new Error(`Unexpected IPC command: ${command}`);
  });
  return { codex, models, modelRequest, quotaRequest, backend };
}

describe("Codex subscription refresh", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.confirm.mockResolvedValue(true);
  });

  afterEach(() => {
    backend?.uninstall();
    backend = null;
  });

  it("refreshes quota and models together, waits for both, and ignores duplicate clicks", async () => {
    const { codex, models, modelRequest, quotaRequest } = await setup();
    const refresh = codex.refreshSubscription();

    expect(mocks.invoke).toHaveBeenCalledWith("codex_rate_limits");
    expect(mocks.invoke).toHaveBeenCalledWith("get_codex_available_models", { forceRefresh: true });
    expect(codex.refreshing).toBe(true);

    quotaRequest.resolve(quota);
    await vi.waitFor(() => expect(codex.quota.loaded).toBe(true));
    expect(codex.refreshing).toBe(true);
    await codex.refreshSubscription();
    expect(mocks.invoke).toHaveBeenCalledTimes(2);

    modelRequest.resolve([existingModel, newModel]);
    await refresh;

    expect(models.availableModels.map((model) => model.id)).toContain(newModel.id);
    expect(models.selectedModelId).toBe(existingModel.id);
    expect(codex.refreshing).toBe(false);
    expect(mocks.addNotice).not.toHaveBeenCalled();
  });

  it("preserves the model list and reports a failed refresh after quota finishes", async () => {
    const { codex, models, modelRequest, quotaRequest } = await setup();
    const refresh = codex.refreshSubscription();
    modelRequest.reject(new Error("Codex models request failed"));
    await vi.waitFor(() => expect(codex.quota.loading).toBe(true));
    expect(codex.refreshing).toBe(true);

    quotaRequest.resolve(quota);
    await refresh;

    expect(models.codexRemoteModels).toEqual([existingModel]);
    expect(models.selectedModelId).toBe(existingModel.id);
    expect(codex.quota.loaded).toBe(true);
    expect(codex.refreshing).toBe(false);
    expect(mocks.addNotice).toHaveBeenCalledWith("error", "Codex models request failed", {
      code: "unknown",
      operation: "codexModelsRefresh",
    });
  });

  it("still updates models when the quota request fails", async () => {
    const { codex, models, modelRequest, quotaRequest } = await setup();
    const refresh = codex.refreshSubscription();
    quotaRequest.reject(new Error("Quota unavailable"));
    modelRequest.resolve([existingModel, newModel]);
    await refresh;

    expect(models.availableModels.map((model) => model.id)).toContain(newModel.id);
    expect(codex.quota.error).toBe("Quota unavailable");
    expect(codex.refreshing).toBe(false);
  });

  it("keeps ordinary model loading cache-aware", async () => {
    const { models, modelRequest } = await setup();
    modelRequest.resolve([existingModel]);
    await models.loadCodexAvailableModels();
    expect(mocks.invoke).toHaveBeenCalledWith("get_codex_available_models", { forceRefresh: false });
  });

  it("defaults automatic client versions on and follows the saved setting", async () => {
    const { codex, backend } = await setup();
    expect(codex.modelConfig.autoUpdateClientVersion).toBe(true);
    backend.change(AUTO_UPDATE_KEY, false);
    expect(codex.modelConfig.autoUpdateClientVersion).toBe(false);
    backend.change(AUTO_UPDATE_KEY, true);
    expect(codex.modelConfig.autoUpdateClientVersion).toBe(true);
  });

  it.each([true, false])("saves automatic versions=%s before refreshing models and blocks overlapping refreshes", async (enabled) => {
    const { codex, models, modelRequest, backend } = await setup({ [AUTO_UPDATE_KEY]: !enabled });
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "get_codex_available_models") return modelRequest.promise;
      throw new Error(`Unexpected IPC command: ${command}`);
    });
    const releaseWrite = backend.holdNextWrite();

    const change = codex.setAutoUpdateClientVersion(enabled);
    expect(codex.clientVersionBusy).toBe(true);
    await vi.waitFor(() => expect(backend.writes).toEqual([[{ key: AUTO_UPDATE_KEY, value: enabled }]]));
    await codex.setAutoUpdateClientVersion(!enabled);
    await codex.refreshSubscription();
    expect(backend.writes).toHaveLength(1);
    expect(mocks.invoke).not.toHaveBeenCalled();

    releaseWrite();
    await vi.waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("get_codex_available_models", { forceRefresh: true }));
    modelRequest.resolve([existingModel, newModel]);
    await change;
    expect(codex.modelConfig.autoUpdateClientVersion).toBe(enabled);
    expect(models.codexRemoteModels.map((model) => model.id)).toContain(newModel.id);
    expect(codex.clientVersionBusy).toBe(false);
  });

  it("rolls back the switch when saving fails, without fetching models", async () => {
    const { codex, backend } = await setup();
    backend.failNextWrite("Disk unavailable");
    await codex.setAutoUpdateClientVersion(false);
    expect(codex.modelConfig.autoUpdateClientVersion).toBe(true);
    expect(codex.clientVersionBusy).toBe(false);
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.addNotice).toHaveBeenCalledWith(
      "error",
      expect.stringContaining("Disk unavailable"),
      expect.objectContaining({ operation: "saveCodexModelConfig" }),
    );
  });

  it("keeps a saved switch and the previous models when its immediate refresh fails", async () => {
    const { codex, models } = await setup({ [AUTO_UPDATE_KEY]: false });
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "get_codex_available_models") return Promise.reject(new Error("Models unavailable"));
      throw new Error(`Unexpected IPC command: ${command}`);
    });
    await codex.setAutoUpdateClientVersion(true);
    expect(codex.modelConfig.autoUpdateClientVersion).toBe(true);
    expect(codex.clientVersionBusy).toBe(false);
    expect(models.codexRemoteModels).toEqual([existingModel]);
    expect(mocks.addNotice).toHaveBeenCalledWith("error", "Models unavailable", {
      code: "unknown", operation: "codexModelsRefresh",
    });
  });

  it("can save the setting before login without requesting a subscription catalog", async () => {
    const { codex, backend } = await setup();
    codex.status = { ...signedIn, authenticated: false };
    await codex.setAutoUpdateClientVersion(false);
    expect(codex.modelConfig.autoUpdateClientVersion).toBe(false);
    expect(backend.values[AUTO_UPDATE_KEY]).toBe(false);
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(codex.clientVersionBusy).toBe(false);
  });
});

describe("Codex options", () => {
  afterEach(() => {
    backend?.uninstall();
    backend = null;
  });

  it("saves each option under its own key and passes the result to the model store", async () => {
    const { codex, models, backend } = await setup({
      "providers.codex.transport": "http",
      "providers.codex.contextWindow": 400000,
      "providers.codex.autoReview": true,
    });
    const apply = vi.spyOn(models, "applyCodexModelConfig");
    expect(codex.modelConfig.useApplyPatch).toBe(false);

    await codex.setUseApplyPatch(true);
    expect(backend.writes[backend.writes.length - 1]).toEqual([{ key: "providers.codex.useApplyPatch", value: true }]);
    expect(apply).toHaveBeenLastCalledWith(expect.objectContaining({
      transport: "http", contextWindow: 400000, autoReview: true, useApplyPatch: true,
    }));

    await codex.setContextWindow(500000);
    expect(backend.writes[backend.writes.length - 1]).toEqual([{ key: "providers.codex.contextWindow", value: 500000 }]);
    expect(codex.modelConfig).toMatchObject({ useApplyPatch: true, contextWindow: 500000 });
  });

  it("keeps the previous value when saving an option fails", async () => {
    const { codex, models, backend } = await setup();
    const apply = vi.spyOn(models, "applyCodexModelConfig");
    backend.failNextWrite("save failed");
    await codex.setUseApplyPatch(true);
    expect(codex.modelConfig.useApplyPatch).toBe(false);
    expect(apply).not.toHaveBeenCalled();
    expect(mocks.addNotice).toHaveBeenCalledWith(
      "error",
      expect.stringContaining("save failed"),
      expect.objectContaining({ operation: "saveCodexModelConfig" }),
    );
  });

  it("clamps the prefix cache lifetime to the supported range", async () => {
    const { codex, backend } = await setup();
    await codex.setPrefixCacheTtlSeconds(90 * 24 * 60 * 60);
    expect(backend.values["providers.codex.prefixCacheTtlSeconds"]).toBe(30 * 24 * 60 * 60);
  });
});

describe("Codex reset credits", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.confirm.mockResolvedValue(true);
  });

  afterEach(() => {
    backend?.uninstall();
    backend = null;
  });

  it("loads and sorts detailed Codex usage reset credits", async () => {
    const { codex } = await setup();
    mocks.invoke.mockImplementation((command: string) => {
      if (command !== "codex_rate_limits") throw new Error(`Unexpected IPC command: ${command}`);
      return Promise.resolve({
        fetchedAtMs: 1_735_689_600_000,
        rateLimits: {
          limitId: "codex",
          primary: {
            usedPercent: 25,
            remainingPercent: 75,
            windowMinutes: 300,
            resetsAt: 1_735_707_600,
          },
        },
        rateLimitsByLimitId: {},
        rateLimitResetCredits: {
          availableCount: 4,
          credits: [
            {
              id: "credit-later",
              resetType: "codex_rate_limits",
              status: "available",
              grantedAt: 1_752_796_800,
              expiresAt: 1_784_851_200,
              title: "Full reset (Weekly + 5 hr)",
            },
            {
              id: "credit-used",
              resetType: "codex_rate_limits",
              status: "redeemed",
              grantedAt: 1_752_796_800,
              expiresAt: 1_784_246_400,
            },
            {
              id: "credit-sooner",
              resetType: "codex_rate_limits",
              status: "available",
              grantedAt: 1_752_796_800,
              expiresAt: 1_784_246_400,
              title: "Full reset (Weekly + 5 hr)",
            },
          ],
        },
      });
    });

    await codex.loadRateLimits();

    expect(codex.quota.resetCreditsAvailable).toBe(4);
    expect(codex.quota.resetCredits.map((credit) => credit.id)).toEqual([
      "credit-sooner",
      "credit-later",
    ]);
  });

  it("consumes a selected Codex reset credit and refreshes quota", async () => {
    const { codex } = await setup();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "codex_consume_rate_limit_reset_credit") {
        expect(args).toEqual({ creditId: "credit-sooner" });
        return Promise.resolve({ outcome: "reset", windowsReset: 2 });
      }
      if (command === "codex_rate_limits") {
        return Promise.resolve({
          fetchedAtMs: 1_735_689_600_000,
          rateLimits: { limitId: "codex" },
          rateLimitsByLimitId: {},
          rateLimitResetCredits: { availableCount: 3, credits: [] },
        });
      }
      throw new Error(`Unexpected IPC command: ${command}`);
    });

    await codex.consumeResetCredit("credit-sooner");

    expect(mocks.confirm).toHaveBeenCalledOnce();
    expect(mocks.invoke).toHaveBeenCalledWith("codex_consume_rate_limit_reset_credit", { creditId: "credit-sooner" });
    expect(mocks.invoke.mock.calls.filter(([command]) => command === "codex_rate_limits")).toHaveLength(1);
    expect(codex.quota.resetCreditsAvailable).toBe(3);
    expect(codex.resetCreditBusyId).toBeNull();
  });
});
