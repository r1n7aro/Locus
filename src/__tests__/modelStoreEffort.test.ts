import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { nextTick } from "vue";
import { useModelStore } from "../stores/model";
import { useAuthStore } from "../stores/auth";
import { loadClaudeCodeCliEnabled, setClaudeCodeCliEnabled } from "../services/claudeCodeCli";

const ipcMocks = vi.hoisted(() => ({
  claudeCodeCliEnabled: false,
  ipcInvoke: vi.fn(),
}));

vi.mock("../services/ipc", () => ({ ipcInvoke: ipcMocks.ipcInvoke }));

const modelServiceMocks = vi.hoisted(() => ({
  getModelDefaults: vi.fn(),
  getLastModel: vi.fn(),
  getLastEffort: vi.fn(),
  getAgentModelPreferences: vi.fn(),
  getCodexFastMode: vi.fn(),
  getCustomProviders: vi.fn(),
  getCodexModelConfig: vi.fn(),
  getCodexAvailableModels: vi.fn(),
  getClaudeCodeModels: vi.fn(),
  saveLastModel: vi.fn(),
  saveLastEffort: vi.fn(),
  saveAgentModelPreference: vi.fn(),
  saveCodexFastMode: vi.fn(),
}));

vi.mock("../services/model", () => modelServiceMocks);

describe("useModelStore OpenAI effort mapping", () => {
  beforeEach(async () => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    ipcMocks.claudeCodeCliEnabled = false;
    ipcMocks.ipcInvoke.mockImplementation(async (command: string, args?: { value?: boolean }) => {
      if (command === "get_claude_code_cli_enabled") return ipcMocks.claudeCodeCliEnabled;
      if (command === "set_claude_code_cli_enabled") ipcMocks.claudeCodeCliEnabled = args?.value === true;
      return undefined;
    });
    await loadClaudeCodeCliEnabled();
    modelServiceMocks.getModelDefaults.mockResolvedValue({
      mainModel: "",
      planModel: "",
      subagentModels: {},
      subagentEfforts: {},
      subagentFastModes: {},
    });
    modelServiceMocks.getLastModel.mockResolvedValue("");
    modelServiceMocks.getLastEffort.mockResolvedValue("");
    modelServiceMocks.getAgentModelPreferences.mockResolvedValue({});
    modelServiceMocks.getCodexFastMode.mockResolvedValue(false);
    modelServiceMocks.getCustomProviders.mockResolvedValue([]);
    modelServiceMocks.getCodexModelConfig.mockResolvedValue({ transport: "websocket" });
    modelServiceMocks.getCodexAvailableModels.mockResolvedValue([]);
    modelServiceMocks.getClaudeCodeModels.mockResolvedValue([]);
    modelServiceMocks.saveLastModel.mockResolvedValue(undefined);
    modelServiceMocks.saveLastEffort.mockResolvedValue(undefined);
    modelServiceMocks.saveAgentModelPreference.mockResolvedValue(undefined);
    modelServiceMocks.saveCodexFastMode.mockResolvedValue(undefined);
  });

  it("discovers configured Claude gateway models without adding historical session models to the catalog", async () => {
    useAuthStore().claudeCodeAvailable = true;
    const store = useModelStore();
    ipcMocks.claudeCodeCliEnabled = true;
    const configured = "claude_code/thinkingmachines/inkling-small:free";
    modelServiceMocks.getClaudeCodeModels.mockResolvedValue([{ id: configured, provider: "claude_code" }]);
    await store.loadModelDefaults();
    expect(store.availableModels.some(model => model.id === configured)).toBe(true);
    store.applySessionModel(configured);
    await nextTick();
    expect(store.selectedModelId).toBe(configured);
    expect(store.allModels.find(model => model.id === store.selectedModelId)).toMatchObject({ id: configured, name: "thinkingmachines/inkling-small:free", provider: "claude_code" });
    const historical = "claude_code/vendor/previous-model";
    store.applySessionModel(historical);
    await nextTick();
    expect(store.selectedModelId).toBe(historical);
    expect(store.availableModels.filter(model => model.id === historical)).toHaveLength(0);
    store.applySessionModel(historical);
    expect(store.availableModels.filter(model => model.id === historical)).toHaveLength(0);
    useAuthStore().claudeCodeAvailable = false;
    await nextTick();
    expect(store.selectedModelId).toBe(historical);
  });

  it("includes Astra and the GPT-5.6 family in the Codex fallback catalog", () => {
    const authStore = useAuthStore();
    authStore.codexAuthenticated = true;
    const modelStore = useModelStore();

    expect(modelStore.codexModels.map((model) => model.id)).toEqual([
      "openai/gpt-6-astra",
      "openai/gpt-5.6-sol",
      "openai/gpt-5.6-terra",
      "openai/gpt-5.6-luna",
      "openai/gpt-5.5",
      "openai/gpt-5.4",
    ]);
    expect(modelStore.codexModels[0]).toEqual(expect.objectContaining({
      name: "GPT-6 Astra",
      contextWindow: 258_400,
      defaultEffort: "low",
      supportedEfforts: ["low", "medium", "high", "xhigh", "max"],
      additionalSpeedTiers: ["fast"],
      isDefault: false,
    }));
    expect(modelStore.codexModels[1]).toEqual(expect.objectContaining({
      name: "GPT-5.6 Sol",
      contextWindow: 258_400,
      defaultEffort: "low",
      supportedEfforts: ["low", "medium", "high", "xhigh", "max"],
      isDefault: true,
    }));
    expect(modelStore.availableModels.some((model) => model.id === "openai/gpt-6-astra")).toBe(true);
    expect(modelStore.availableModels.some((model) => model.id === "openai/gpt-5.6-sol")).toBe(true);
    expect(modelStore.availableModels.some((model) => model.id === "openai/gpt-5.5")).toBe(true);
    expect(modelStore.availableModels.some((model) => model.id === "openai/gpt-5.4")).toBe(true);
  });

  it("applies the configured GPT-5.6 context window and migrates the legacy switch", () => {
    const authStore = useAuthStore();
    authStore.codexAuthenticated = true;
    const modelStore = useModelStore();
    modelStore.codexRemoteModels = [
      {
        id: "openai/gpt-5.6-sol",
        name: "GPT-5.6 Sol",
        provider: "openai_codex",
        contextWindow: 258_400,
      },
    ];

    expect(modelStore.codexContextWindow).toBe(272_000);
    expect(modelStore.codexModels[0].contextWindow).toBe(258_400);

    modelStore.applyCodexModelConfig({ transport: "websocket", contextWindow: 500_000 });

    expect(modelStore.codexContextWindow).toBe(500_000);
    expect(modelStore.codexModels[0].contextWindow).toBe(475_000);

    modelStore.applyCodexModelConfig({ transport: "websocket", extendedContext: true });

    expect(modelStore.codexContextWindow).toBe(372_000);
    expect(modelStore.codexModels[0].contextWindow).toBe(353_400);
  });

  it("uses the remote Codex catalog when it is available", async () => {
    const authStore = useAuthStore();
    authStore.codexAuthenticated = true;
    modelServiceMocks.getCodexAvailableModels.mockResolvedValue([
      {
        id: "openai/gpt-5.5",
        name: "GPT-5.5",
        provider: "openai_codex",
        defaultEffort: "medium",
        supportedEfforts: ["low", "medium", "high", "xhigh"],
      },
    ]);
    const modelStore = useModelStore();

    await modelStore.loadCodexAvailableModels();

    expect(modelStore.availableModels.some((model) => model.id === "openai/gpt-5.5")).toBe(true);
    expect(modelStore.availableModels.some((model) => model.id === "openai/gpt-5.4")).toBe(false);
    expect(modelStore.availableModels.some((model) => model.id === "openai/gpt-6-astra")).toBe(false);
  });

  it("normalizes remote Codex model labels from model slugs", async () => {
    const authStore = useAuthStore();
    authStore.codexAuthenticated = true;
    modelServiceMocks.getCodexAvailableModels.mockResolvedValue([
      { id: "openai/gpt-6-astra", name: "GPT-6-Astra", provider: "openai_codex" },
      { id: "openai/gpt-5.4", name: "gpt-5.4", provider: "openai_codex" },
      { id: "openai/gpt-5.6-sol", name: "GPT-5.6-Sol", provider: "openai_codex" },
      { id: "openai/gpt-5.6-terra", name: "GPT-5.6-Terra", provider: "openai_codex" },
      { id: "openai/gpt-5.6-luna", name: "GPT-5.6-Luna", provider: "openai_codex" },
      { id: "openai/gpt-5.5", name: "GPT-5.5", provider: "openai_codex" },
      { id: "openai/gpt-5.4-mini", name: "GPT-5.4-Mini", provider: "openai_codex" },
      { id: "openai/gpt-5.3-codex", name: "gpt-5.3-codex", provider: "openai_codex" },
      { id: "openai/gpt-5.3-codex-spark", name: "GPT-5.3 Codex-Spark", provider: "openai_codex" },
    ]);
    const modelStore = useModelStore();

    await modelStore.loadCodexAvailableModels();

    expect(modelStore.codexModels.map((model) => model.name)).toEqual([
      "GPT-6 Astra",
      "GPT-5.4",
      "GPT-5.6 Sol",
      "GPT-5.6 Terra",
      "GPT-5.6 Luna",
      "GPT-5.5",
      "GPT-5.4 Mini",
      "GPT-5.3 Codex",
      "GPT-5.3 Codex Spark",
    ]);
    modelStore.selectModel("openai/gpt-6-astra");
    expect(modelStore.availableEfforts).toEqual(["low", "medium", "high", "xhigh", "max"]);
  });

  it("exposes xhigh and hides none for GPT-5.5", () => {
    const modelStore = useModelStore();

    modelStore.selectedModelId = "openai/gpt-5.5";

    expect(modelStore.availableEfforts).toEqual(["low", "medium", "high", "xhigh"]);
    expect(modelStore.effortSupported).toBe(true);
  });

  it("uses the current Claude subscription catalog in the same order as Claude CLI", async () => {
    const authStore = useAuthStore();
    authStore.isAuthenticated = true;
    const modelStore = useModelStore();
    const models = modelStore.availableModels.filter((model) => model.provider === "anthropic");

    expect(models.map(({ id, name }) => ({ id, name }))).toEqual([
      { id: "claude-opus-5-5", name: "Claude Opus 5.5" },
      { id: "claude-fable-5-1", name: "Claude Fable 5.1" },
      { id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5" },
      { id: "claude-haiku-5-5", name: "Claude Haiku 5.5" },
    ]);
    expect(models.filter((model) => model.isDefault).map((model) => model.id)).toEqual(["claude-opus-5-5"]);
    expect(models[1].contextWindow).toBe(1_000_000);
    expect(models[2].contextWindow).toBe(1_000_000);
    // Do not infer unverified context limits from a newer model name.
    expect(models[0].contextWindow).toBeUndefined();
    expect(models[3].contextWindow).toBeUndefined();

    await setClaudeCodeCliEnabled(true);
    const cliModels = modelStore.allModels.filter((model) => model.provider === "claude_code");
    expect(models.map(({ id, name, supportedEfforts }) => ({ id, name, supportedEfforts }))).toEqual(
      cliModels.map(({ id, name, supportedEfforts }) => ({ id: id.slice(12), name, supportedEfforts })),
    );
    // Updating the subscription catalog must not replace OpenRouter's catalog.
    expect(modelStore.allModels.find((model) => model.id === "openrouter/claude-opus-4.8")?.isDefault).toBe(true);
  });

  it("exposes max and hides none for GPT-5.6", () => {
    const modelStore = useModelStore();

    modelStore.selectedModelId = "openai/gpt-5.6-sol";

    expect(modelStore.availableEfforts).toEqual(["low", "medium", "high", "xhigh", "max"]);
    expect(modelStore.effortSupported).toBe(true);
  });

  it("supports Astra effort selection and Fast mode through the subscription catalog", async () => {
    const authStore = useAuthStore();
    authStore.codexAuthenticated = true;
    const modelStore = useModelStore();
    modelStore.selectModel("openai/gpt-6-astra");

    expect(modelStore.availableEfforts).toEqual(["low", "medium", "high", "xhigh", "max"]);
    expect(modelStore.effortSupported).toBe(true);
    expect(modelStore.codexFastModeAvailable).toBe(true);
    modelStore.selectCodexFastMode(true);
    expect(modelStore.effectiveCodexFastMode).toBe(true);

    // The release manifest's ultra preset requires Codex's orchestration layer.
    modelServiceMocks.getCodexAvailableModels.mockResolvedValue([{
      id: "openai/gpt-6-astra",
      name: "GPT-6-Astra",
      provider: "openai_codex",
      defaultEffort: "low",
      supportedEfforts: ["low", "medium", "high", "xhigh", "max", "ultra"],
      additionalSpeedTiers: ["fast"],
      contextWindow: 258_400,
    }]);
    await modelStore.loadCodexAvailableModels();

    expect(modelStore.availableEfforts).toEqual(["low", "medium", "high", "xhigh", "max"]);
    expect(modelStore.effectiveCodexFastMode).toBe(true);
    expect(modelStore.codexModels[0].contextWindow).toBe(258_400);
  });

  it("enables one Fast mode state only for supported Codex models", async () => {
    const modelStore = useModelStore();
    modelStore.selectedModelId = "openai/gpt-5.6-sol";

    expect(modelStore.codexFastModeAvailable).toBe(true);
    expect(modelStore.effectiveCodexFastMode).toBe(false);

    modelStore.selectCodexFastMode(true);
    expect(modelStore.effectiveCodexFastMode).toBe(true);
    expect(modelStore.codexFastModeForModel("openai/gpt-5.6-terra")).toBe(true);
    expect(modelStore.codexFastModeForModel("openrouter/claude-opus-4.8")).toBe(false);
    expect(modelServiceMocks.saveCodexFastMode).toHaveBeenCalledWith(true);

    modelStore.selectedModelId = "openrouter/claude-opus-4.8";
    expect(modelStore.codexFastModeAvailable).toBe(false);
    expect(modelStore.effectiveCodexFastMode).toBe(false);

    modelServiceMocks.getCodexFastMode.mockResolvedValue(true);
    await modelStore.loadCodexFastMode();
    expect(modelStore.codexFastMode).toBe(true);
    expect(modelStore.defaultCodexFastMode).toBe(true);

    modelServiceMocks.saveCodexFastMode.mockClear();
    modelStore.applyContextCodexFastMode(false);
    expect(modelStore.codexFastMode).toBe(false);
    expect(modelStore.defaultCodexFastMode).toBe(true);
    expect(modelServiceMocks.saveCodexFastMode).not.toHaveBeenCalled();

    modelStore.restoreDefaultCodexFastMode();
    expect(modelStore.codexFastMode).toBe(true);
  });

  it("uses Claude model effort metadata from the catalog", () => {
    const modelStore = useModelStore();

    for (const id of ["claude-opus-5-5", "claude-sonnet-5-5", "claude-fable-5-1"]) {
      modelStore.selectedModelId = id;
      expect(modelStore.availableEfforts).toEqual(["none", "low", "medium", "high", "xhigh", "max"]);
    }
    modelStore.selectedModelId = "claude-haiku-5-5";
    expect(modelStore.availableEfforts).toEqual(["none", "low", "medium", "high", "max"]);
  });

  it("keeps Claude CLI models hidden until the experimental setting is enabled", async () => {
    useAuthStore().claudeCodeAvailable = true;
    const modelStore = useModelStore();
    await modelStore.loadModelDefaults();
    expect(modelStore.allModels.some(model => model.provider === "claude_code")).toBe(false);

    await setClaudeCodeCliEnabled(true);
    expect(modelStore.availableModels.some(model => model.id === "claude_code/claude-opus-5-5")).toBe(true);
    modelStore.selectModel("claude_code/claude-opus-5-5");

    await setClaudeCodeCliEnabled(false);
    await nextTick();
    expect(modelStore.allModels.some(model => model.provider === "claude_code")).toBe(false);
    expect(modelStore.selectedModelId.startsWith("claude_code/")).toBe(false);
  });

  it("uses stable Claude CLI IDs and the established display name format", async () => {
    const modelStore = useModelStore();
    await setClaudeCodeCliEnabled(true);
    expect(modelStore.allModels.filter(model => model.provider === 'claude_code').map(({ id, name }) => ({ id, name }))).toEqual([
      { id: 'claude_code/claude-opus-5-5', name: 'Claude Opus 5.5' },
      { id: 'claude_code/claude-fable-5-1', name: 'Claude Fable 5.1' },
      { id: 'claude_code/claude-sonnet-5-5', name: 'Claude Sonnet 5.5' },
      { id: 'claude_code/claude-haiku-5-5', name: 'Claude Haiku 5.5' },
    ]);
    modelStore.selectModel("claude_code/claude-opus-5-5");
    expect(modelStore.selectedModelId).toBe("claude_code/claude-opus-5-5");
    expect(modelServiceMocks.saveLastModel).toHaveBeenCalledWith(
      "claude_code/claude-opus-5-5",
    );
  });

  it("keeps codex mini limited to medium and high on OpenAI Responses endpoints", () => {
    const modelStore = useModelStore();

    modelStore.applyCustomProviders([{
      id: "endpoint-1",
      name: "OpenAI Responses",
      endpoint: "https://example.com/v1/responses",
      apiFormat: "openai_responses",
      apiKey: "",
      catalogId: null,
      models: [{
        id: "gpt-5.1-codex-mini",
        apiModel: "gpt-5.1-codex-mini",
        name: "gpt-5.1-codex-mini",
        contextLength: 256000,
        remoteCompactionMode: "disabled",
        supportsToolLazyLoading: false,
        supportedReasoningEfforts: ["medium", "high"],
        reasoningParamFormat: "openai_responses_reasoning_effort",
        replayReasoningContent: false,
        reasoningReplayField: null,
        serverTools: { webSearch: false },
        supportsVision: true,
      }],
    }]);
    modelStore.selectedModelId = "custom/endpoint-1/gpt-5.1-codex-mini";

    expect(modelStore.availableEfforts).toEqual(["medium", "high"]);
  });

  it("defaults custom endpoints to low medium high xhigh max reasoning controls", () => {
    const modelStore = useModelStore();

    modelStore.applyCustomProviders([{
      id: "endpoint-1",
      name: "Custom Chat",
      endpoint: "https://example.com/v1",
      apiFormat: "openai_chat",
      apiKey: "",
      catalogId: null,
      models: [{
        id: "deepseek-v4-pro",
        apiModel: "deepseek-v4-pro",
        name: "deepseek-v4-pro",
        contextLength: 256000,
        remoteCompactionMode: "disabled",
        supportsToolLazyLoading: false,
        reasoningParamFormat: "openai_chat_reasoning_effort",
        replayReasoningContent: true,
        reasoningReplayField: null,
        serverTools: { webSearch: false },
        supportsVision: true,
      } as any],
    }]);
    // Legacy single-segment ids keep resolving to the provider's first model.
    modelStore.selectedModelId = "custom/endpoint-1";

    expect(modelStore.availableEfforts).toEqual(["low", "medium", "high", "xhigh", "max"]);
    expect(modelStore.effortSupported).toBe(true);
  });

  it("upgrades legacy custom endpoint defaults to include xhigh", () => {
    const modelStore = useModelStore();

    modelStore.applyCustomProviders([{
      id: "endpoint-1",
      name: "Custom Chat",
      endpoint: "https://example.com/v1",
      apiFormat: "openai_chat",
      apiKey: "",
      catalogId: null,
      models: [{
        id: "deepseek-v4-pro",
        apiModel: "deepseek-v4-pro",
        name: "deepseek-v4-pro",
        contextLength: 256000,
        remoteCompactionMode: "disabled",
        supportsToolLazyLoading: false,
        supportedReasoningEfforts: ["low", "medium", "high", "max"],
        reasoningParamFormat: "openai_chat_reasoning_effort",
        replayReasoningContent: true,
        reasoningReplayField: null,
        serverTools: { webSearch: false },
        supportsVision: true,
      }],
    }]);
    modelStore.selectedModelId = "custom/endpoint-1/deepseek-v4-pro";

    expect(modelStore.availableEfforts).toEqual(["low", "medium", "high", "xhigh", "max"]);
  });

  it("loads the saved effort selection from persistence", async () => {
    const modelStore = useModelStore();
    modelStore.selectedModelId = "openai/gpt-5.5";
    modelServiceMocks.getLastEffort.mockResolvedValue("high");

    await modelStore.loadLastEffort();

    expect(modelStore.effort).toBe("high");
    expect(modelStore.defaultEffort).toBe("high");
    expect(modelStore.hasUserDefaultEffort).toBe(true);
  });

  it("persists effort changes after hydration", async () => {
    const modelStore = useModelStore();

    await modelStore.loadLastEffort();
    // Select a non-default effort so the change actually triggers persistence
    // (the global default is now "high", so selecting "high" would be a no-op).
    modelStore.selectEffort("low");
    await nextTick();

    expect(modelStore.defaultEffort).toBe("low");
    expect(modelStore.hasUserDefaultEffort).toBe(true);
    expect(modelServiceMocks.saveLastEffort).toHaveBeenCalledWith("low");
  });

  it("restores and persists model defaults independently for each Agent", async () => {
    const authStore = useAuthStore();
    authStore.codexAuthenticated = true;
    modelServiceMocks.getAgentModelPreferences.mockResolvedValue({
      unity: { modelId: "openai/gpt-5.6-terra", effort: "xhigh" },
      explorer: { modelId: "openai/gpt-5.6-luna", effort: "low" },
    });
    const modelStore = useModelStore();
    await modelStore.loadAgentModelPreferences();

    modelStore.activateAgentPreference("unity", "medium", true);
    expect(modelStore.selectedModelId).toBe("openai/gpt-5.6-terra");
    expect(modelStore.effort).toBe("xhigh");

    modelStore.activateAgentPreference("explorer", "none", true);
    expect(modelStore.selectedModelId).toBe("openai/gpt-5.6-luna");
    expect(modelStore.effort).toBe("low");

    modelStore.selectModel("openai/gpt-5.6-sol");
    modelStore.selectEffort("high");
    await vi.waitFor(() => {
      expect(modelServiceMocks.saveAgentModelPreference).toHaveBeenLastCalledWith(
        "explorer",
        "openai/gpt-5.6-sol",
        "high",
      );
    });
  });

  it("does not persist context effort changes from session or agent selection", async () => {
    const modelStore = useModelStore();

    await modelStore.loadLastEffort();
    modelStore.applyContextEffort("medium");
    await nextTick();

    expect(modelStore.effort).toBe("medium");
    expect(modelServiceMocks.saveLastEffort).not.toHaveBeenCalled();
  });

  it("keeps a remembered effort for a Claude CLI model whose catalog has not loaded yet", async () => {
    useAuthStore().claudeCodeAvailable = true;
    const modelStore = useModelStore();
    // The experimental gate is still off/unknown, so the CLI model is not in the catalog.
    modelStore.applySessionModel("claude_code/claude-opus-5-5");
    modelStore.applyNewSessionSelection({ modelId: "claude_code/claude-opus-5-5", effort: "xhigh", fastMode: false });
    await nextTick();
    expect(modelStore.effort).toBe("xhigh");
    expect(modelStore.clampEffortForModel("claude_code/claude-opus-5-5", "max")).toBe("max");

    await setClaudeCodeCliEnabled(true);
    await nextTick();
    expect(modelStore.availableEfforts).toContain("xhigh");
    expect(modelStore.effort).toBe("xhigh");
  });

  it("fits an unsupported effort to the nearest lower level instead of turning thinking off", async () => {
    useAuthStore().isAuthenticated = true;
    const modelStore = useModelStore();
    modelStore.selectModel("claude-opus-5-5");
    modelStore.applyContextEffort("xhigh");
    expect(modelStore.effort).toBe("xhigh");

    modelStore.selectModel("claude-haiku-5-5");
    await nextTick();
    expect(modelStore.effort).toBe("high");
    expect(modelStore.clampEffortForModel("claude-haiku-5-5", "xhigh")).toBe("high");
  });

  it("does not let a model with fewer levels overwrite the saved default effort", async () => {
    useAuthStore().isAuthenticated = true;
    modelServiceMocks.getAgentModelPreferences.mockResolvedValue({
      unity: { modelId: "claude-haiku-5-5", effort: "xhigh" },
    });
    const modelStore = useModelStore();
    await modelStore.loadAgentModelPreferences();
    await modelStore.loadLastEffort();

    modelStore.activateAgentPreference("unity", "medium", true);
    await nextTick();

    expect(modelStore.selectedModelId).toBe("claude-haiku-5-5");
    expect(modelStore.effort).toBe("high");
    expect(modelStore.defaultEffort).toBe("xhigh");
    expect(modelServiceMocks.saveLastEffort).not.toHaveBeenCalledWith("none");
    expect(modelServiceMocks.saveLastEffort).not.toHaveBeenCalledWith("high");
  });
});
