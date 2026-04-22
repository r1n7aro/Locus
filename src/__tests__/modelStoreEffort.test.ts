import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { nextTick } from "vue";
import { useModelStore } from "../stores/model";

const modelServiceMocks = vi.hoisted(() => ({
  getModelDefaults: vi.fn(),
  getLastModel: vi.fn(),
  getLastEffort: vi.fn(),
  getCustomEndpoints: vi.fn(),
  getCodexModelConfig: vi.fn(),
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
    modelServiceMocks.saveLastModel.mockResolvedValue(undefined);
    modelServiceMocks.saveLastEffort.mockResolvedValue(undefined);
  });

  it("exposes xhigh and hides none for GPT-5.4", () => {
    const modelStore = useModelStore();

    modelStore.selectedModelId = "openai/gpt-5.4";

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
      contextLength: 128000,
      betaFlags: [],
    }]);
    modelStore.selectedModelId = "custom/endpoint-1";

    expect(modelStore.availableEfforts).toEqual(["medium", "high"]);
  });

  it("loads the saved effort selection from persistence", async () => {
    const modelStore = useModelStore();
    modelStore.selectedModelId = "openai/gpt-5.4";
    modelServiceMocks.getLastEffort.mockResolvedValue("high");

    await modelStore.loadLastEffort();

    expect(modelStore.effort).toBe("high");
  });

  it("persists effort changes after hydration", async () => {
    const modelStore = useModelStore();

    await modelStore.loadLastEffort();
    modelStore.effort = "high";
    await nextTick();

    expect(modelServiceMocks.saveLastEffort).toHaveBeenCalledWith("high");
  });
});
