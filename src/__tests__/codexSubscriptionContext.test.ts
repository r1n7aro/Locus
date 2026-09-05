import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useModelStore } from "../stores/model";
import { useAuthStore } from "../stores/auth";
import type { CodexTransportMode } from "../types";

const modelServiceMocks = vi.hoisted(() => ({
  getCodexModelConfig: vi.fn(),
  getCodexAvailableModels: vi.fn(),
}));

vi.mock("../services/model", () => modelServiceMocks);

const subscriptionModelIds = [
  "openai/gpt-6-astra",
  "openai/gpt-5.6-sol",
  "openai/gpt-5.6-terra",
  "openai/gpt-5.6-luna",
  "openai/gpt-5.5",
  "openai/gpt-5.4-mini",
  "openai/gpt-5.3-codex-spark",
  "openai/gpt-future",
];

describe("Codex subscription context window", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    useAuthStore().codexAuthenticated = true;
  });

  it("updates every fallback subscription model without changing other providers", () => {
    const store = useModelStore();
    const otherModels = store.allModels.filter((model) => model.provider !== "openai_codex");

    store.applyCodexModelConfig({ contextWindow: 400_000 });

    expect(store.codexModels.length).toBeGreaterThan(0);
    for (const model of store.codexModels) {
      expect(model.contextWindow, model.id).toBe(380_000);
    }
    expect(store.allModels.filter((model) => model.provider !== "openai_codex")).toEqual(otherModels);

    store.applyCodexModelConfig({ extendedContext: true });
    for (const model of store.codexModels) {
      expect(model.contextWindow, model.id).toBe(353_400);
    }
  });

  it.each<CodexTransportMode>(["http", "websocket"])(
    "applies saved settings before and after a remote catalog refresh over %s",
    async (transport) => {
      const store = useModelStore();
      modelServiceMocks.getCodexModelConfig.mockResolvedValue({ transport, contextWindow: 400_000 });
      modelServiceMocks.getCodexAvailableModels.mockResolvedValue(subscriptionModelIds.map((id) => ({
        id,
        name: id,
        provider: "openai_codex",
        contextWindow: 258_400,
      })));

      await store.loadCodexModelConfig();
      await store.loadCodexAvailableModels();

      expect(store.codexModels.map((model) => model.id)).toEqual(subscriptionModelIds);
      for (const model of store.codexModels) {
        expect(model.contextWindow, model.id).toBe(380_000);
      }

      store.selectedModelId = "openai/gpt-6-astra";
      store.applyCodexModelConfig({ transport, contextWindow: 500_000 });
      expect(store.allModels.find((model) => model.id === store.selectedModelId)?.contextWindow)
        .toBe(475_000);

      await store.loadCodexAvailableModels();
      for (const model of store.codexModels) {
        expect(model.contextWindow, model.id).toBe(475_000);
      }
    },
  );
});
