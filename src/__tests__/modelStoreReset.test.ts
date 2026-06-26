import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useAuthStore } from "../stores/auth";
import { useModelStore } from "../stores/model";

const modelServiceMocks = vi.hoisted(() => ({
  getModelDefaults: vi.fn(),
  getLastModel: vi.fn(),
  getCustomEndpoints: vi.fn(),
  saveLastModel: vi.fn(),
}));

vi.mock("../services/model", () => modelServiceMocks);

describe("useModelStore reset behavior", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    modelServiceMocks.getModelDefaults.mockResolvedValue({
      mainModel: "",
      planModel: "",
      subagentModels: {},
    });
    modelServiceMocks.getCustomEndpoints.mockResolvedValue([]);
  });

  it("clears stale lastModelId when persisted config has been reset", async () => {
    const authStore = useAuthStore();
    authStore.hasApiKey = true;

    const modelStore = useModelStore();

    modelServiceMocks.getLastModel.mockResolvedValueOnce("openrouter/claude-opus-4.6");
    await modelStore.loadLastModel();
    modelStore.resolveSelectedModel(true);
    expect(modelStore.lastModelId).toBe("openrouter/claude-opus-4.6");
    expect(modelStore.selectedModelId).toBe("openrouter/claude-opus-4.6");

    modelServiceMocks.getLastModel.mockResolvedValueOnce("");
    await modelStore.loadLastModel();
    modelStore.resolveSelectedModel(true);

    expect(modelStore.lastModelId).toBe("");
    expect(modelStore.selectedModelId).toBe("openrouter/claude-opus-4.8");
  });
});
