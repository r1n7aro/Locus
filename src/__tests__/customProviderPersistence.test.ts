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
  setWarmup: vi.fn(),
  getWarmup: vi.fn(),
  clearWarmup: vi.fn(),
  confirm: vi.fn(),
  applyCustomProviders: vi.fn(),
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

vi.mock("../stores/model", () => ({
  useModelStore: () => ({ applyCustomProviders: modelServiceMocks.applyCustomProviders }),
}));

vi.mock("../stores/notification", () => ({
  useNotificationStore: () => ({
    addNotice: vi.fn(),
  }),
}));

import { createPinia, setActivePinia } from "pinia";
import { useCustomProviderSettingsStore } from "../stores/customProviderSettings";
import { useProviderAccountsStore } from "../stores/providerAccounts";
import {
  DEFAULT_PROVIDER_PREFIX_CACHE_TTL_SECONDS,
  isOpenRouterEndpoint,
} from "../services/modelCatalog";
import type { CustomEndpoint, CustomProvider, CustomProviderModel } from "../types";

function providerModel(partial: Partial<CustomProviderModel> = {}): CustomProviderModel {
  return {
    id: "model",
    apiModel: "model",
    name: "model",
    contextLength: 256000,
    remoteCompactionMode: "disabled",
    codexTransport: null,
    supportsToolLazyLoading: false,
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
    setActivePinia(createPinia());
    vi.clearAllMocks();
    modelServiceMocks.getWarmup.mockReturnValue(undefined);
    modelServiceMocks.getCustomProviders.mockResolvedValue([]);
    modelServiceMocks.saveCustomProviders.mockResolvedValue(undefined);
    modelServiceMocks.confirm.mockResolvedValue(true);
  });

  it("reloads saved providers and refreshes the warmup cache", async () => {
    const state = useCustomProviderSettingsStore();
    const saved = provider({ id: "saved", name: "Saved", apiKey: "sk-live" });
    const normalizedSaved = {
      ...saved,
      prefixCacheTtlSeconds: DEFAULT_PROVIDER_PREFIX_CACHE_TTL_SECONDS,
    };
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([saved]);

    state.startAddCustomProvider();
    state.editingCustomProvider = provider({
      id: "draft",
      name: "Draft",
      apiKey: "sk-draft",
      models: [providerModel({ id: "draft-model", apiModel: "draft-model" })],
    });

    await state.saveCustomProvider();

    expect(modelServiceMocks.saveCustomProviders).toHaveBeenCalledWith([
      expect.objectContaining({ id: "draft", name: "Draft", apiKey: "sk-draft" }),
    ]);
    expect(state.customProviders).toEqual([normalizedSaved]);
    expect(modelServiceMocks.setWarmup).toHaveBeenCalledWith(
      "settings:customProviders",
      [normalizedSaved],
    );
    expect(modelServiceMocks.applyCustomProviders).toHaveBeenCalledWith([normalizedSaved]);
    expect(state.customProviderSaving).toBe(false);
    expect(state.editingCustomProvider).toBeNull();
  });

  it("starts new providers with one blank model row at the 256k default", () => {
    const state = useCustomProviderSettingsStore();

    state.startAddCustomProvider();

    const draft = state.editingCustomProvider;
    expect(draft?.models).toHaveLength(1);
    expect(draft?.models[0].contextLength).toBe(256000);
    expect(draft?.models[0].remoteCompactionMode).toBe("disabled");
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
    const accounts = useProviderAccountsStore();
    const state = useCustomProviderSettingsStore();
    const imported: CustomEndpoint = {
      id: "claude-code-import",
      name: "Claude Code",
      apiModel: "claude-opus-4-8",
      endpoint: "https://proxy.example/v1",
      apiFormat: "anthropic_messages",
      apiKey: "sk-cc",
      contextLength: 1_000_000,
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
    const normalizedSavedProvider = {
      ...savedProvider,
      prefixCacheTtlSeconds: DEFAULT_PROVIDER_PREFIX_CACHE_TTL_SECONDS,
    };
    modelServiceMocks.importClaudeCodeOAuth.mockResolvedValueOnce({
      kind: "custom_endpoint",
      source: "Claude Code settings.json",
      hasRefreshToken: false,
      customEndpoint: imported,
    });
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([savedProvider]);

    await accounts.importClaudeCodeOAuth();

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
    expect(state.customProviders).toEqual([normalizedSavedProvider]);
    expect(modelServiceMocks.applyCustomProviders).toHaveBeenCalledWith([normalizedSavedProvider]);
  });

  it("saves new OpenAI Chat models with reasoning content replay enabled", async () => {
    const state = useCustomProviderSettingsStore();

    state.startAddCustomProvider();
    const draft = state.editingCustomProvider!;
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

  it("persists Codex V2 remote compaction on a Responses model", async () => {
    const state = useCustomProviderSettingsStore();

    state.startAddCustomProvider();
    const draft = state.editingCustomProvider!;
    draft.name = "CPA";
    draft.endpoint = "http://192.168.0.2:8317/v1";
    draft.apiFormat = "openai_responses";
    draft.models[0].apiModel = "gpt-5.6-sol";
    draft.models[0].remoteCompactionMode = "codex_v2";

    await state.saveCustomProvider();

    expect(modelServiceMocks.saveCustomProviders).toHaveBeenCalledWith([
      expect.objectContaining({
        apiFormat: "openai_responses",
        models: [expect.objectContaining({ remoteCompactionMode: "codex_v2" })],
      }),
    ]);
  });

  it("persists a Codex gateway transport, enables compaction, and tests the selected route", async () => {
    const state = useCustomProviderSettingsStore();
    state.startAddCustomProvider();
    const draft = state.editingCustomProvider!;
    draft.name = "CPA";
    draft.endpoint = "http://192.168.0.2:8317/v1";
    draft.apiFormat = "openai_responses";
    draft.models[0].apiModel = "gpt-6-luna";
    draft.models[0].codexTransport = "websocket";
    modelServiceMocks.testCustomEndpoint.mockResolvedValueOnce("OK");
    await state.testCustomProvider();
    expect(modelServiceMocks.testCustomEndpoint).toHaveBeenCalledWith(
      expect.objectContaining({ apiModel: "gpt-6-luna" }), "websocket",
    );
    await state.saveCustomProvider();
    expect(modelServiceMocks.saveCustomProviders).toHaveBeenCalledWith([
      expect.objectContaining({ models: [expect.objectContaining({
        codexTransport: "websocket", remoteCompactionMode: "codex_v2",
      })] }),
    ]);
  });

  it("keeps legacy endpoints on generic HTTP and clears Codex transport on other formats", async () => {
    const state = useCustomProviderSettingsStore();
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([
      provider({ id: "legacy", name: "Legacy", apiFormat: "openai_responses", models: [providerModel({ codexTransport: undefined })] }),
      provider({ id: "chat", name: "Chat", apiFormat: "openai_chat", models: [providerModel({ codexTransport: "websocket" })] }),
    ]);
    await state.loadCustomProviders();
    expect(state.customProviders.map((p) => p.models[0].codexTransport)).toEqual([null, null]);
  });

  it("normalizes legacy OpenAI Chat models to replay reasoning content", async () => {
    const state = useCustomProviderSettingsStore();
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([
      provider({
        id: "openai-chat",
        name: "OpenAI Chat",
        models: [providerModel({ replayReasoningContent: undefined } as never)],
      }),
    ]);

    await state.loadCustomProviders();

    expect(state.customProviders[0].models[0].replayReasoningContent).toBe(true);
  });

  it("normalizes legacy Anthropic Messages models to disabled reasoning replay", async () => {
    const state = useCustomProviderSettingsStore();
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

    expect(state.customProviders[0].models[0].replayReasoningContent).toBe(false);
  });

  it("forces reasoning replay for an existing DeepSeek V4 Anthropic model", async () => {
    const state = useCustomProviderSettingsStore();
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([
      provider({
        id: "deepseek",
        name: "DeepSeek",
        endpoint: "https://api.deepseek.com/anthropic/v1",
        apiFormat: "anthropic_messages",
        models: [providerModel({
          apiModel: "deepseek-v4-pro",
          catalogModelId: "deepseek-v4-pro",
          reasoningParamFormat: "anthropic_thinking",
          replayReasoningContent: false,
          reasoningReplayField: null,
        })],
      }),
    ]);

    await state.loadCustomProviders();

    expect(state.customProviders[0].models[0].replayReasoningContent).toBe(true);
    expect(state.customProviders[0].models[0].reasoningReplayField).toBe("reasoning_content");
  });

  it("normalizes legacy models to disabled server tools", async () => {
    const state = useCustomProviderSettingsStore();
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([
      provider({
        id: "legacy-server-tools",
        name: "Legacy Server Tools",
        models: [providerModel({ serverTools: undefined } as never)],
      }),
    ]);

    await state.loadCustomProviders();

    expect(state.customProviders[0].models[0].serverTools).toEqual({ webSearch: false });
  });

  it("keeps a known OpenRouter search engine and drops unknown ones", async () => {
    const state = useCustomProviderSettingsStore();
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([
      provider({
        id: "openrouter-search",
        name: "OpenRouter",
        models: [
          providerModel({ id: "exa", apiModel: "google/exa", serverTools: { webSearch: true, webSearchEngine: "exa" } }),
          providerModel({
            id: "bogus",
            apiModel: "google/bogus",
            serverTools: { webSearch: true, webSearchEngine: "bogus" } as never,
          }),
        ],
      }),
    ]);

    await state.loadCustomProviders();

    const [exa, bogus] = state.customProviders[0].models;
    expect(exa.serverTools).toEqual({ webSearch: true, webSearchEngine: "exa" });
    expect(bogus.serverTools).toEqual({ webSearch: true });
  });

  it("recognizes OpenRouter endpoints by host", () => {
    expect(isOpenRouterEndpoint("https://openrouter.ai/api/v1")).toBe(true);
    expect(isOpenRouterEndpoint("https://eu.openrouter.ai/api/v1")).toBe(true);
    expect(isOpenRouterEndpoint("https://openrouter.ai.example.com/v1")).toBe(false);
    expect(isOpenRouterEndpoint("")).toBe(false);
  });

  it("normalizes legacy remote compaction and preserves an enabled Codex compact mode", async () => {
    const state = useCustomProviderSettingsStore();
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([
      provider({
        id: "legacy-compaction",
        name: "Legacy compaction",
        apiFormat: "openai_responses",
        models: [providerModel({ remoteCompactionMode: undefined } as never)],
      }),
      provider({
        id: "cpa",
        name: "CPA",
        apiFormat: "openai_responses",
        models: [providerModel({ remoteCompactionMode: "codex_v2" })],
      }),
    ]);

    await state.loadCustomProviders();

    expect(state.customProviders[0].models[0].remoteCompactionMode).toBe("disabled");
    expect(state.customProviders[1].models[0].remoteCompactionMode).toBe("codex_v2");
  });

  it("normalizes legacy models to enabled image understanding", async () => {
    const state = useCustomProviderSettingsStore();
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([
      provider({
        id: "legacy-vision",
        name: "Legacy Vision",
        models: [providerModel({ supportsVision: undefined } as never)],
      }),
    ]);

    await state.loadCustomProviders();

    expect(state.customProviders[0].models[0].supportsVision).toBe(true);
  });

  it("normalizes legacy default reasoning efforts to include xhigh", async () => {
    const state = useCustomProviderSettingsStore();
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

    expect(state.customProviders[0].models[0].supportedReasoningEfforts).toEqual([
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
    ]);
  });

  it("fills empty model row ids from the api model", async () => {
    const state = useCustomProviderSettingsStore();
    modelServiceMocks.getCustomProviders.mockResolvedValueOnce([
      provider({
        id: "row-ids",
        name: "Row Ids",
        models: [providerModel({ id: "", apiModel: "zai-org/GLM-5.2" })],
      }),
    ]);

    await state.loadCustomProviders();

    expect(state.customProviders[0].models[0].id).toBe("zai-org-GLM-5.2");
  });

  it("serializes delete mutations against the latest reloaded list", async () => {
    const state = useCustomProviderSettingsStore();
    const first = provider({ id: "first", name: "First" });
    const second = provider({ id: "second", name: "Second" });
    state.customProviders = [first, second];

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

    expect(state.customProviderSaving).toBe(true);
    expect(modelServiceMocks.saveCustomProviders).toHaveBeenCalledTimes(1);
    expect(modelServiceMocks.saveCustomProviders).toHaveBeenNthCalledWith(1, [second]);

    releaseFirstSave();
    await Promise.all([firstDelete, secondDelete]);

    expect(modelServiceMocks.saveCustomProviders).toHaveBeenCalledTimes(2);
    expect(modelServiceMocks.saveCustomProviders).toHaveBeenNthCalledWith(2, []);
    expect(state.customProviders).toEqual([]);
    expect(state.customProviderSaving).toBe(false);
  });
});
