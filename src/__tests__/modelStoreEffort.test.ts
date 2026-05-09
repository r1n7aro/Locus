import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { nextTick } from "vue";
import { useModelStore } from "../stores/model";
import { useAuthStore } from "../stores/auth";

const modelServiceMocks = vi.hoisted(() => ({
  getModelDefaults: vi.fn(),
  getLastModel: vi.fn(),
  getLastEffort: vi.fn(),
  getCustomEndpoints: vi.fn(),
  getCodexModelConfig: vi.fn(),
  getCodexAvailableModels: vi.fn(),
  saveLastModel: vi.fn(),
  saveLastEffort: vi.fn(),
}));

vi.mock("../services/model", () => modelServiceMocks);

describe("useModelStore OpenAI effort mapping", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    modelServiceMocks.getModelDefaults.mockResolvedValue({
      mainModel: "",
      planModel: "",
      subagentModels: {},
    });
    modelServiceMocks.getLastModel.mockResolvedValue("");
    modelServiceMocks.getLastEffort.mockResolvedValue("");
    modelServiceMocks.getCustomEndpoints.mockResolvedValue([]);
    modelServiceMocks.getCodexModelConfig.mockResolvedValue({ transport: "websocket" });
    modelServiceMocks.getCodexAvailableModels.mockResolvedValue([]);
    modelServiceMocks.saveLastModel.mockResolvedValue(undefined);
    modelServiceMocks.saveLastEffort.mockResolvedValue(undefined);
  });

  it("uses GPT-5.4 as the Codex fallback catalog", () => {
    const authStore = useAuthStore();
    authStore.codexAuthenticated = true;
    const modelStore = useModelStore();

    expect(modelStore.availableModels.some((model) => model.id === "openai/gpt-5.4")).toBe(true);
    expect(modelStore.availableModels.some((model) => model.id === "openai/gpt-5.5")).toBe(false);
  });

  it("exposes GPT-5.5 only after the remote Codex catalog returns it", async () => {
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
  });

  it("normalizes remote Codex model labels from model slugs", async () => {
    const authStore = useAuthStore();
    authStore.codexAuthenticated = true;
    modelServiceMocks.getCodexAvailableModels.mockResolvedValue([
      { id: "openai/gpt-5.4", name: "gpt-5.4", provider: "openai_codex" },
      { id: "openai/gpt-5.5", name: "GPT-5.5", provider: "openai_codex" },
      { id: "openai/gpt-5.4-mini", name: "GPT-5.4-Mini", provider: "openai_codex" },
      { id: "openai/gpt-5.3-codex", name: "gpt-5.3-codex", provider: "openai_codex" },
      { id: "openai/gpt-5.3-codex-spark", name: "GPT-5.3 Codex-Spark", provider: "openai_codex" },
    ]);
    const modelStore = useModelStore();

    await modelStore.loadCodexAvailableModels();

    expect(modelStore.codexModels.map((model) => model.name)).toEqual([
      "GPT-5.4",
      "GPT-5.5",
      "GPT-5.4 Mini",
      "GPT-5.3 Codex",
      "GPT-5.3 Codex Spark",
    ]);
  });

  it("exposes xhigh and hides none for GPT-5.5", () => {
    const modelStore = useModelStore();

    modelStore.selectedModelId = "openai/gpt-5.5";

    expect(modelStore.availableEfforts).toEqual(["low", "medium", "high", "xhigh"]);
    expect(modelStore.effortSupported).toBe(true);
  });

  it("keeps codex mini limited to medium and high on OpenAI Responses endpoints", () => {
    const modelStore = useModelStore();

    modelStore.applyCustomEndpoints([{
      id: "endpoint-1",
      name: "OpenAI Responses",
      apiModel: "gpt-5.1-codex-mini",
      endpoint: "https://example.com/v1/responses",
      apiFormat: "openai_responses",
      apiKey: "",
      contextLength: 256000,
      betaFlags: [],
      supportedReasoningEfforts: ["medium", "high"],
      reasoningParamFormat: "openai_responses_reasoning_effort",
      replayReasoningContent: false,
      serverTools: { webSearch: false },
    }]);
    modelStore.selectedModelId = "custom/endpoint-1";

    expect(modelStore.availableEfforts).toEqual(["medium", "high"]);
  });

  it("defaults custom endpoints to low medium high xhigh max reasoning controls", () => {
    const modelStore = useModelStore();

    modelStore.applyCustomEndpoints([{
      id: "endpoint-1",
      name: "Custom Chat",
      apiModel: "deepseek-v4-pro",
      endpoint: "https://example.com/v1",
      apiFormat: "openai_chat",
      apiKey: "",
      contextLength: 256000,
      betaFlags: [],
      reasoningParamFormat: "openai_chat_reasoning_effort",
      replayReasoningContent: true,
      serverTools: { webSearch: false },
    } as any]);
    modelStore.selectedModelId = "custom/endpoint-1";

    expect(modelStore.availableEfforts).toEqual(["low", "medium", "high", "xhigh", "max"]);
    expect(modelStore.effortSupported).toBe(true);
  });

  it("upgrades legacy custom endpoint defaults to include xhigh", () => {
    const modelStore = useModelStore();

    modelStore.applyCustomEndpoints([{
      id: "endpoint-1",
      name: "Custom Chat",
      apiModel: "deepseek-v4-pro",
      endpoint: "https://example.com/v1",
      apiFormat: "openai_chat",
      apiKey: "",
      contextLength: 256000,
      betaFlags: [],
      supportedReasoningEfforts: ["low", "medium", "high", "max"],
      reasoningParamFormat: "openai_chat_reasoning_effort",
      replayReasoningContent: true,
      serverTools: { webSearch: false },
    }]);
    modelStore.selectedModelId = "custom/endpoint-1";

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
    modelStore.selectEffort("high");
    await nextTick();

    expect(modelStore.defaultEffort).toBe("high");
    expect(modelStore.hasUserDefaultEffort).toBe(true);
    expect(modelServiceMocks.saveLastEffort).toHaveBeenCalledWith("high");
  });

  it("does not persist context effort changes from session or agent selection", async () => {
    const modelStore = useModelStore();

    await modelStore.loadLastEffort();
    modelStore.applyContextEffort("medium");
    await nextTick();

    expect(modelStore.effort).toBe("medium");
    expect(modelServiceMocks.saveLastEffort).not.toHaveBeenCalled();
  });
});
