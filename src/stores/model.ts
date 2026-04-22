import { ref, computed, watch } from "vue";
import { defineStore } from "pinia";
import { useAuthStore } from "./auth";
import { pickPreferredModelId } from "./modelSelection";
import * as modelService from "../services/model";
import type {
  ModelOption,
  ModelDefaults,
  CustomEndpoint,
  EffortLevel,
  CodexModelConfig,
  CodexTransportMode,
} from "../types";
import { filterVisibleModels } from "../config/providerVisibility";

const builtinModels: ModelOption[] = [
  { id: "openrouter/claude-sonnet-4.6", name: "Claude Sonnet 4.6", provider: "openrouter" },
  { id: "openrouter/claude-opus-4.6", name: "Claude Opus 4.6", provider: "openrouter" },
  { id: "openrouter/glm-5", name: "GLM 5", provider: "openrouter" },
  { id: "openrouter/minimax-m2.5", name: "MiniMax M2.5", provider: "openrouter" },
  { id: "claude-sonnet-4.6", name: "Claude Sonnet 4.6", provider: "anthropic" },
  { id: "claude-opus-4.6", name: "Claude Opus 4.6", provider: "anthropic" },
  { id: "anthropic_sdk/claude-sonnet-4.6", name: "Claude Sonnet 4.6", provider: "anthropic_sdk" },
  { id: "anthropic_sdk/claude-opus-4.6", name: "Claude Opus 4.6", provider: "anthropic_sdk" },
  { id: "openai/gpt-5.4", name: "GPT-5.4", provider: "openai_codex" },
];

const effortLevels: EffortLevel[] = ["none", "low", "medium", "high", "xhigh"];

function normalizeOpenAiReasoningModel(model: string): string {
  return model.trim().toLowerCase();
}

function isEffortLevel(value: string): value is EffortLevel {
  return effortLevels.includes(value as EffortLevel);
}

function supportsOpenAiReasoningModel(model: string): boolean {
  const m = normalizeOpenAiReasoningModel(model);
  return m.includes("codex") || m.includes("gpt-5");
}

function openAiReasoningLevels(model: string): EffortLevel[] {
  const m = normalizeOpenAiReasoningModel(model);
  if (m.includes("gpt-5.4-pro") || m.includes("gpt-5.2-pro")) return ["medium", "high"];
  if (m.includes("gpt-5-pro")) return ["high"];
  if (m.includes("gpt-5.1-codex-mini")) return ["medium", "high"];
  if (m.includes("codex")) return ["low", "medium", "high", "xhigh"];
  if (m.includes("gpt-5.4") || m.includes("gpt-5.2") || m.includes("gpt-5.1")) {
    return ["low", "medium", "high", "xhigh"];
  }
  if (m.includes("gpt-5")) return ["low", "medium", "high", "xhigh"];
  return [];
}

function normalizeCodexTransport(config?: Partial<CodexModelConfig> | null): CodexTransportMode {
  return config?.transport === "http" ? "http" : "websocket";
}

export const useModelStore = defineStore("model", () => {
  const authStore = useAuthStore();

  const customEndpoints = ref<CustomEndpoint[]>([]);
  const codexTransport = ref<CodexTransportMode>("websocket");
  const selectedModelId = ref("");
  const lastModelId = ref("");
  const effort = ref<EffortLevel>("none");
  const modelDefaults = ref<ModelDefaults>({ mainModel: "", planModel: "", subagentModels: {} });
  let effortPersistenceReady = false;

  // -- Getters --

  const allModels = computed<ModelOption[]>(() => {
    const customs: ModelOption[] = customEndpoints.value.map((ep) => ({
      id: `custom/${ep.id}`,
      name: ep.name,
      provider: "custom" as const,
    }));
    return filterVisibleModels([...builtinModels, ...customs]);
  });

  const availableModels = computed(() => {
    const providers = new Set<string>();
    if (authStore.hasApiKey) providers.add("openrouter");
    if (authStore.isAuthenticated) providers.add("anthropic");
    if (authStore.anthropicSdkAvailable) providers.add("anthropic_sdk");
    if (authStore.codexAuthenticated) providers.add("openai_codex");
    providers.add("custom");
    return allModels.value.filter((m) => providers.has(m.provider));
  });

  const selectedCustomEndpoint = computed<CustomEndpoint | null>(() =>
    customEndpoints.value.find((ep) => `custom/${ep.id}` === selectedModelId.value) ?? null
  );

  const selectedOpenAiReasoningModel = computed<string | null>(() => {
    const selected = selectedModelId.value;
    if (selected.startsWith("openai/")) {
      return selected.slice("openai/".length);
    }
    if (
      selected.startsWith("custom/")
      && selectedCustomEndpoint.value?.apiFormat === "openai_responses"
    ) {
      return selectedCustomEndpoint.value.apiModel;
    }
    return null;
  });

  const availableEfforts = computed<EffortLevel[]>(() => {
    const m = selectedModelId.value.toLowerCase();
    if (m.includes("claude")) return ["none", "low", "medium", "high"];
    const openAiModel = selectedOpenAiReasoningModel.value;
    if (!openAiModel || !supportsOpenAiReasoningModel(openAiModel)) return [];
    return openAiReasoningLevels(openAiModel);
  });

  const effortSupported = computed(() => availableEfforts.value.length > 0);

  // -- Internal watchers (model-domain only) --

  // Clamp effort when available levels change
  watch(availableEfforts, (levels) => {
    if (levels.length > 0 && !levels.includes(effort.value)) {
      effort.value = levels[0];
    }
  }, { immediate: true });

  watch(effort, (level) => {
    if (!effortPersistenceReady) return;
    Promise.resolve()
      .then(() => modelService.saveLastEffort(level))
      .catch((e: unknown) => console.warn("[model] save_last_effort:", e));
  });

  // Keep the selector valid when provider availability changes.
  watch(availableModels, (models) => {
    if (models.length === 0) {
      selectedModelId.value = "";
      return;
    }

    if (selectedModelId.value && models.some((m) => m.id === selectedModelId.value)) {
      return;
    }

    const next = pickPreferredModelId(models, modelDefaults.value, lastModelId.value);
    if (next) selectedModelId.value = next;
  }, { immediate: true });


  // -- Actions --

  async function loadModelDefaults() {
    try {
      modelDefaults.value = await modelService.getModelDefaults();
    } catch { /* ignore */ }
  }

  async function loadLastModel() {
    try {
      const saved = await modelService.getLastModel();
      lastModelId.value = saved || "";
    } catch { /* ignore */ }
  }

  async function loadLastEffort() {
    effortPersistenceReady = false;
    try {
      const saved = await modelService.getLastEffort();
      if (isEffortLevel(saved)) {
        effort.value = saved;
      }
    } catch { /* ignore */ }
    effortPersistenceReady = true;
  }

  async function loadCustomEndpoints() {
    try {
      customEndpoints.value = await modelService.getCustomEndpoints();
    } catch { /* ignore */ }
  }

  async function loadCodexModelConfig() {
    try {
      codexTransport.value = normalizeCodexTransport(await modelService.getCodexModelConfig());
    } catch {
      codexTransport.value = "websocket";
    }
  }

  function resolveSelectedModel(force = false) {
    const models = availableModels.value;
    if (models.length === 0) {
      selectedModelId.value = "";
      return;
    }

    if (!force && selectedModelId.value && models.some((m) => m.id === selectedModelId.value)) {
      return;
    }

    const next = pickPreferredModelId(models, modelDefaults.value, lastModelId.value);
    if (next) selectedModelId.value = next;
  }

  function rememberLastModel(id: string) {
    lastModelId.value = id;
    modelService.saveLastModel(id).catch((e: unknown) => console.warn("[model] save_last_model:", e));
  }

  function selectModel(id: string) {
    selectedModelId.value = id;
    rememberLastModel(id);
  }

  function applyModelDefaults(defaults: ModelDefaults) {
    modelDefaults.value = defaults;
  }

  function applyCustomEndpoints(endpoints: CustomEndpoint[]) {
    customEndpoints.value = endpoints;
  }

  function applyCodexModelConfig(config?: Partial<CodexModelConfig> | null) {
    codexTransport.value = normalizeCodexTransport(config);
  }

  return {
    customEndpoints,
    codexTransport,
    selectedModelId,
    lastModelId,
    effort,
    modelDefaults,
    allModels,
    availableModels,
    selectedCustomEndpoint,
    selectedOpenAiReasoningModel,
    availableEfforts,
    effortSupported,
    loadModelDefaults,
    loadLastModel,
    loadLastEffort,
    loadCustomEndpoints,
    loadCodexModelConfig,
    resolveSelectedModel,
    selectModel,
    applyModelDefaults,
    applyCustomEndpoints,
    applyCodexModelConfig,
  };
});
