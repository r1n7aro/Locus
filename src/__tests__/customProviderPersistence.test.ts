import { describe, expect, it, vi, beforeEach } from "vitest";

const modelServiceMocks = vi.hoisted(() => ({
  getCustomProviders: vi.fn(),
  saveCustomProviders: vi.fn(),
  testCustomEndpoint: vi.fn(),
  getModelCatalog: vi.fn(),
  refreshModelCatalog: vi.fn(),
  getProviders: vi.fn(),
  saveProviderKey: vi.fn(),
  deleteProviderKey: vi.fn(),
  getAuthUrl: vi.fn(),
  exchangeAuthCode: vi.fn(),
  authLogout: vi.fn(),
  importClaudeCodeOAuth: vi.fn(),
  codexStatus: vi.fn(),
  codexRateLimits: vi.fn(),
  codexConsumeRateLimitResetCredit: vi.fn(),
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
  getFileToolWorkspaceBoundary: vi.fn(),
  setFileToolWorkspaceBoundary: vi.fn(),
  resetAllConfig: vi.fn(),
  setWarmup: vi.fn(),
  getWarmup: vi.fn(),
  clearWarmup: vi.fn(),
  confirm: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: modelServiceMocks.confirm,
}));

vi.mock("../services/auth", () => ({
  getProviders: modelServiceMocks.getProviders,
  saveProviderKey: modelServiceMocks.saveProviderKey,
  deleteProviderKey: modelServiceMocks.deleteProviderKey,
  getAuthUrl: modelServiceMocks.getAuthUrl,
  exchangeAuthCode: modelServiceMocks.exchangeAuthCode,
  authLogout: modelServiceMocks.authLogout,
  importClaudeCodeOAuth: modelServiceMocks.importClaudeCodeOAuth,
  codexStatus: modelServiceMocks.codexStatus,
  codexRateLimits: modelServiceMocks.codexRateLimits,
  codexConsumeRateLimitResetCredit: modelServiceMocks.codexConsumeRateLimitResetCredit,
  codexStartLogin: modelServiceMocks.codexStartLogin,
  codexPollLogin: modelServiceMocks.codexPollLogin,
  codexLogout: modelServiceMocks.codexLogout,
  codexRetryAuth: modelServiceMocks.codexRetryAuth,
}));

vi.mock("../services/model", () => ({
  getCustomProviders: modelServiceMocks.getCustomProviders,
  saveCustomProviders: modelServiceMocks.saveCustomProviders,
  testCustomEndpoint: modelServiceMocks.testCustomEndpoint,
  getModelCatalog: modelServiceMocks.getModelCatalog,
  refreshModelCatalog: modelServiceMocks.refreshModelCatalog,
  getModelDefaults: modelServiceMocks.getModelDefaults,
  saveModelDefaults: modelServiceMocks.saveModelDefaults,
  getCodexModelConfig: modelServiceMocks.getCodexModelConfig,
  saveCodexModelConfig: modelServiceMocks.saveCodexModelConfig,
}));

vi.mock("../services/permissions", () => ({
  getToolPermissions: modelServiceMocks.getToolPermissions,
  saveToolPermissions: modelServiceMocks.saveToolPermissions,
  getFileToolWorkspaceBoundary: modelServiceMocks.getFileToolWorkspaceBoundary,
  setFileToolWorkspaceBoundary: modelServiceMocks.setFileToolWorkspaceBoundary,
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
import type { CustomEndpoint, CustomProvider, CustomProviderModel } from "../types";

function providerModel(partial: Partial<CustomProviderModel> = {}): CustomProviderModel {
  return {
    id: "model",
    apiModel: "model",
    name: "model",
    contextLength: 256000,
    betaFlags: [],
    supportedReasoningEfforts: ["low", "medium", "high", "xhigh", "max"],
    reasoningParamFormat: "openai_chat_reasoning_effort",
    replayReasoningContent: true,
    reasoningReplayField: null,
    serverTools: { webSearch: false },
    supportsVision: true,
    ...partial,
  };
}

function provider(
  partial: Partial<CustomProvider> & Pick<CustomProvider, "id" | "name">,
): CustomProvider {
  return {
    endpoint: "https://example.com/v1",
    apiFormat: "openai_chat",
    apiKey: "",
    catalogId: null,
    models: [providerModel()],
    ...partial,
  };
}

describe("custom provider persistence", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    modelServiceMocks.getWarmup.mockReturnValue(undefined);
    modelServiceMocks.getCustomProviders.mockResolvedValue([]);
    modelServiceMocks.saveCustomProviders.mockResolvedValue(undefined);
    modelServiceMocks.getFileToolWorkspaceBoundary.mockResolvedValue(false);
    modelServiceMocks.setFileToolWorkspaceBoundary.mockResolvedValue(undefined);
    modelServiceMocks.confirm.mockResolvedValue(true);
  });

  it("reloads saved providers and refreshes the warmup cache", async () => {
    const emitted: unknown[][] = [];
    const state = useSettingsState(((...args: unknown[]) => {
      emitted.push(args);
    }) as never);
    const saved = provider({ id: "saved", name: "Saved", apiKey: "sk-live" });
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([saved]);

    state.startAddCustomProvider();
    state.editingCustomProvider.value = provider({
      id: "draft",
      name: "Draft",
      apiKey: "sk-draft",
      models: [providerModel({ id: "draft-model", apiModel: "draft-model" })],
    });

    await state.saveCustomProvider();

    expect(modelServiceMocks.saveCustomProviders).toHaveBeenCalledWith([
      expect.objectContaining({ id: "draft", name: "Draft", apiKey: "sk-draft" }),
    ]);
    expect(state.customProviders.value).toEqual([saved]);
    expect(modelServiceMocks.setWarmup).toHaveBeenCalledWith("settings:customProviders", [saved]);
    expect(emitted).toContainEqual(["customProvidersChanged", [saved]]);
    expect(state.customProviderSaving.value).toBe(false);
    expect(state.editingCustomProvider.value).toBeNull();
  });

  it("starts new providers with one blank model row at the 256k default", () => {
    const state = useSettingsState((() => undefined) as never);

    state.startAddCustomProvider();

    const draft = state.editingCustomProvider.value;
    expect(draft?.models).toHaveLength(1);
    expect(draft?.models[0].contextLength).toBe(256000);
    expect(draft?.models[0].supportsVision).toBe(true);
    expect(draft?.models[0].serverTools.webSearch).toBe(false);
    expect(draft?.models[0].supportedReasoningEfforts).toEqual([
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
    ]);
  });

  it("imports a Claude Code custom endpoint as a single-model provider", async () => {
    const emitted: unknown[][] = [];
    const state = useSettingsState(((...args: unknown[]) => {
      emitted.push(args);
    }) as never);
    const imported: CustomEndpoint = {
      id: "claude-code-import",
      name: "Claude Code",
      apiModel: "claude-opus-4-8",
      endpoint: "https://proxy.example/v1",
      apiFormat: "anthropic_messages",
      apiKey: "sk-cc",
      contextLength: 1_000_000,
      betaFlags: [],
      supportedReasoningEfforts: ["low", "medium", "high", "xhigh", "max"],
      reasoningParamFormat: "anthropic_thinking",
      replayReasoningContent: false,
      serverTools: { webSearch: false },
      supportsToolLazyLoading: false,
      supportsVision: true,
    };
    const savedProvider = provider({
      id: "claude-code-import",
      name: "Claude Code",
      endpoint: "https://proxy.example/v1",
      apiFormat: "anthropic_messages",
      models: [providerModel({
        id: "claude-opus-4-8",
        apiModel: "claude-opus-4-8",
        name: "claude-opus-4-8",
        contextLength: 1_000_000,
        reasoningParamFormat: "anthropic_thinking",
        replayReasoningContent: false,
      })],
    });
    modelServiceMocks.importClaudeCodeOAuth.mockResolvedValueOnce({
      kind: "custom_endpoint",
      source: "Claude Code settings.json",
      hasRefreshToken: false,
      customEndpoint: imported,
    });
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([savedProvider]);

    await state.importClaudeCodeOAuth();

    expect(modelServiceMocks.saveCustomProviders).toHaveBeenCalledWith([
      expect.objectContaining({
        id: "claude-code-import",
        name: "Claude Code",
        endpoint: "https://proxy.example/v1",
        apiFormat: "anthropic_messages",
        apiKey: "sk-cc",
        models: [
          expect.objectContaining({
            apiModel: "claude-opus-4-8",
            contextLength: 1_000_000,
            reasoningParamFormat: "anthropic_thinking",
            replayReasoningContent: false,
          }),
        ],
      }),
    ]);
    expect(modelServiceMocks.getProviders).not.toHaveBeenCalled();
    expect(state.customProviders.value).toEqual([savedProvider]);
    expect(emitted).toContainEqual(["customProvidersChanged", [savedProvider]]);
  });

  it("saves new OpenAI Chat models with reasoning content replay enabled", async () => {
    const state = useSettingsState((() => undefined) as never);

    state.startAddCustomProvider();
    const draft = state.editingCustomProvider.value!;
    draft.name = "Chat";
    draft.endpoint = "https://example.com/v1";
    draft.models[0].apiModel = "chat-model";

    await state.saveCustomProvider();

    expect(modelServiceMocks.saveCustomProviders).toHaveBeenCalledWith([
      expect.objectContaining({
        models: [expect.objectContaining({ replayReasoningContent: true })],
      }),
    ]);
  });

  it("normalizes legacy OpenAI Chat models to replay reasoning content", async () => {
    const state = useSettingsState((() => undefined) as never);
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([
      provider({
        id: "openai-chat",
        name: "OpenAI Chat",
        models: [providerModel({ replayReasoningContent: undefined } as never)],
      }),
    ]);

    await state.loadCustomProviders();

    expect(state.customProviders.value[0].models[0].replayReasoningContent).toBe(true);
  });

  it("normalizes legacy Anthropic Messages models to disabled reasoning replay", async () => {
    const state = useSettingsState((() => undefined) as never);
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([
      provider({
        id: "anthropic-messages",
        name: "Anthropic Messages",
        apiFormat: "anthropic_messages",
        models: [providerModel({
          reasoningParamFormat: "anthropic_thinking",
          replayReasoningContent: undefined,
        } as never)],
      }),
    ]);

    await state.loadCustomProviders();

    expect(state.customProviders.value[0].models[0].replayReasoningContent).toBe(false);
  });

  it("normalizes legacy models to disabled server tools", async () => {
    const state = useSettingsState((() => undefined) as never);
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([
      provider({
        id: "legacy-server-tools",
        name: "Legacy Server Tools",
        models: [providerModel({ serverTools: undefined } as never)],
      }),
    ]);

    await state.loadCustomProviders();

    expect(state.customProviders.value[0].models[0].serverTools).toEqual({ webSearch: false });
  });

  it("normalizes legacy models to enabled image understanding", async () => {
    const state = useSettingsState((() => undefined) as never);
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([
      provider({
        id: "legacy-vision",
        name: "Legacy Vision",
        models: [providerModel({ supportsVision: undefined } as never)],
      }),
    ]);

    await state.loadCustomProviders();

    expect(state.customProviders.value[0].models[0].supportsVision).toBe(true);
  });

  it("normalizes legacy default reasoning efforts to include xhigh", async () => {
    const state = useSettingsState((() => undefined) as never);
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([
      provider({
        id: "legacy-efforts",
        name: "Legacy Efforts",
        models: [providerModel({
          supportedReasoningEfforts: ["low", "medium", "high", "max"],
        })],
      }),
    ]);

    await state.loadCustomProviders();

    expect(state.customProviders.value[0].models[0].supportedReasoningEfforts).toEqual([
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
    ]);
  });

  it("fills empty model row ids from the api model", async () => {
    const state = useSettingsState((() => undefined) as never);
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([
      provider({
        id: "row-ids",
        name: "Row Ids",
        models: [providerModel({ id: "", apiModel: "zai-org/GLM-5.2" })],
      }),
    ]);

    await state.loadCustomProviders();

    expect(state.customProviders.value[0].models[0].id).toBe("zai-org-GLM-5.2");
  });

  it("serializes delete mutations against the latest reloaded list", async () => {
    const state = useSettingsState((() => undefined) as never);
    const first = provider({ id: "first", name: "First" });
    const second = provider({ id: "second", name: "Second" });
    state.customProviders.value = [first, second];

    let releaseFirstSave!: () => void;
    modelServiceMocks.saveCustomProviders
      .mockImplementationOnce(() => new Promise<void>((resolve) => {
        releaseFirstSave = resolve;
      }))
      .mockResolvedValueOnce(undefined);
    modelServiceMocks.getCustomProviders
      .mockResolvedValueOnce([second])
      .mockResolvedValueOnce([]);

    const firstDelete = state.deleteCustomProvider("first");
    const secondDelete = state.deleteCustomProvider("second");
    await Promise.resolve();
    await Promise.resolve();

    expect(state.customProviderSaving.value).toBe(true);
    expect(modelServiceMocks.saveCustomProviders).toHaveBeenCalledTimes(1);
    expect(modelServiceMocks.saveCustomProviders).toHaveBeenNthCalledWith(1, [second]);

    releaseFirstSave();
    await Promise.all([firstDelete, secondDelete]);

    expect(modelServiceMocks.saveCustomProviders).toHaveBeenCalledTimes(2);
    expect(modelServiceMocks.saveCustomProviders).toHaveBeenNthCalledWith(2, []);
    expect(state.customProviders.value).toEqual([]);
    expect(state.customProviderSaving.value).toBe(false);
  });

  it("loads and sorts detailed Codex usage reset credits", async () => {
    const state = useSettingsState((() => undefined) as never);
    state.codexStatus.value = {
      authenticated: true,
      accountId: "account-1",
      validationFailed: false,
      validationError: null,
    };
    modelServiceMocks.codexRateLimits.mockResolvedValueOnce({
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

    await state.loadCodexRateLimits();

    expect(state.codexQuota.value.resetCreditsAvailable).toBe(4);
    expect(state.codexQuota.value.resetCredits.map((credit) => credit.id)).toEqual([
      "credit-sooner",
      "credit-later",
    ]);
  });

  it("consumes a selected Codex reset credit and refreshes quota", async () => {
    const state = useSettingsState((() => undefined) as never);
    state.codexStatus.value = {
      authenticated: true,
      accountId: "account-1",
      validationFailed: false,
      validationError: null,
    };
    modelServiceMocks.codexConsumeRateLimitResetCredit.mockResolvedValueOnce({
      outcome: "reset",
      windowsReset: 2,
    });
    modelServiceMocks.codexRateLimits.mockResolvedValueOnce({
      fetchedAtMs: 1_735_689_600_000,
      rateLimits: { limitId: "codex" },
      rateLimitsByLimitId: {},
      rateLimitResetCredits: {
        availableCount: 3,
        credits: [],
      },
    });

    await state.consumeCodexResetCredit("credit-sooner");

    expect(modelServiceMocks.confirm).toHaveBeenCalledOnce();
    expect(modelServiceMocks.codexConsumeRateLimitResetCredit).toHaveBeenCalledWith("credit-sooner");
    expect(modelServiceMocks.codexRateLimits).toHaveBeenCalledOnce();
    expect(state.codexQuota.value.resetCreditsAvailable).toBe(3);
    expect(state.codexResetCreditBusyId.value).toBeNull();
  });
});
