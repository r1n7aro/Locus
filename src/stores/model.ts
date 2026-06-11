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
  { id: "claude_code/claude-sonnet-4.6", name: "Claude Sonnet 4.6", provider: "claude_code" },
  { id: "claude_code/claude-opus-4.6", name: "Claude Opus 4.6", provider: "claude_code" },
];

const codexFallbackModels: ModelOption[] = [
  {
    id: "openai/gpt-5.5",
    name: "GPT-5.5",
    provider: "openai_codex",
    defaultEffort: "medium",
    supportedEfforts: ["low", "medium", "high", "xhigh"],
    additionalSpeedTiers: ["fast"],
    isDefault: true,
  },
  {
    id: "openai/gpt-5.4",
    name: "GPT-5.4",
    provider: "openai_codex",
    defaultEffort: "medium",
    supportedEfforts: ["low", "medium", "high", "xhigh"],
    additionalSpeedTiers: ["fast"],
    isDefault: false,
  },
];

const effortLevels: EffortLevel[] = ["none", "low", "medium", "high", "xhigh", "max"];
const customDefaultReasoningEfforts: EffortLevel[] = ["low", "medium", "high", "xhigh", "max"];
const legacyCustomDefaultReasoningEfforts: EffortLevel[] = ["low", "medium", "high", "max"];

function normalizeOpenAiReasoningModel(model: string): string {
  return model.trim().toLowerCase();
}

function isEffortLevel(value: string): value is EffortLevel {
  return effortLevels.includes(value as EffortLevel);
}

function normalizeEfforts(values?: EffortLevel[] | null): EffortLevel[] {
  if (!Array.isArray(values)) return [];
  return values.filter(isEffortLevel);
}

function normalizeCustomReasoningEfforts(values?: EffortLevel[] | null): EffortLevel[] {
  const normalized = normalizeEfforts(values).filter((value) => value !== "none");
  if (isSameEffortList(normalized, legacyCustomDefaultReasoningEfforts)) {
    return [...customDefaultReasoningEfforts];
  }
  return normalized.length > 0 ? normalized : [...customDefaultReasoningEfforts];
}

function isSameEffortList(a: EffortLevel[], b: EffortLevel[]): boolean {
  return a.length === b.length && a.every((value, index) => value === b[index]);
}

function supportsOpenAiReasoningModel(model: string): boolean {
  const m = normalizeOpenAiReasoningModel(model);
  return m.includes("codex") || m.includes("gpt-5");
}

function openAiReasoningLevels(model: string): EffortLevel[] {
  const m = normalizeOpenAiReasoningModel(model);
  if (m.includes("gpt-5.5-pro") || m.includes("gpt-5.4-pro") || m.includes("gpt-5.2-pro")) return ["medium", "high"];
  if (m.includes("gpt-5-pro")) return ["high"];
  if (m.includes("gpt-5.1-codex-mini")) return ["medium", "high"];
  if (m.includes("codex")) return ["low", "medium", "high", "xhigh"];
  if (m.includes("gpt-5.5") || m.includes("gpt-5.4") || m.includes("gpt-5.2") || m.includes("gpt-5.1")) {
    return ["low", "medium", "high", "xhigh"];
  }
  if (m.includes("gpt-5")) return ["low", "medium", "high", "xhigh"];
  return [];
}

function normalizeCodexTransport(config?: Partial<CodexModelConfig> | null): CodexTransportMode {
  return config?.transport === "http" ? "http" : "websocket";
}

function formatCodexModelName(id: string, fallbackName?: string): string {
  const slug = id.startsWith("openai/") ? id.slice("openai/".length) : id;
  const parts = slug
    .trim()
    .toLowerCase()
    .split("-")
    .filter(Boolean);
  const formatPart = (part: string): string => {
    if (part === "gpt") return "GPT";
    if (part === "codex") return "Codex";
    if (part === "mini") return "Mini";
    if (part === "spark") return "Spark";
    if (part === "pro") return "Pro";
    if (/^\d/.test(part)) return part;
    return part.charAt(0).toUpperCase() + part.slice(1);
  };

  if (parts[0] === "gpt" && parts[1]) {
    const head = `GPT-${parts[1]}`;
    const tail = parts.slice(2).map(formatPart).join(" ");
    return tail ? `${head} ${tail}` : head;
  }

  const formatted = parts.map(formatPart).join(" ");
  return formatted || fallbackName?.trim() || id;
}

function normalizeCodexModels(models?: ModelOption[] | null): ModelOption[] {
  if (!Array.isArray(models)) return [];
  const seen = new Set<string>();
  const normalized: ModelOption[] = [];
  for (const model of models) {
    const id = typeof model.id === "string" ? model.id.trim() : "";
    if (!id.startsWith("openai/") || seen.has(id)) continue;
    seen.add(id);
    const name = formatCodexModelName(id, model.name);
    normalized.push({
      ...model,
      id,
      name,
      provider: "openai_codex",
      supportedEfforts: normalizeEfforts(model.supportedEfforts),
    });
  }
  return normalized;
}

export const useModelStore = defineStore("model", () => {
  const authStore = useAuthStore();

  const customEndpoints = ref<CustomEndpoint[]>([]);
  const codexRemoteModels = ref<ModelOption[]>([]);
  const codexTransport = ref<CodexTransportMode>("websocket");
  const selectedModelId = ref("");
  const lastModelId = ref("");
  const effort = ref<EffortLevel>("medium");
  const defaultEffort = ref<EffortLevel>("medium");
  const hasUserDefaultEffort = ref(false);
  const modelDefaults = ref<ModelDefaults>({ mainModel: "", planModel: "", subagentModels: {} });
  let effortPersistenceReady = false;

  // -- Getters --

  const codexModels = computed<ModelOption[]>(() =>
    codexRemoteModels.value.length > 0 ? codexRemoteModels.value : codexFallbackModels
  );

  const allModels = computed<ModelOption[]>(() => {
    const customs: ModelOption[] = customEndpoints.value.map((ep) => ({
      id: `custom/${ep.id}`,
      name: ep.name,
      provider: "custom" as const,
      supportedEfforts: normalizeCustomReasoningEfforts(ep.supportedReasoningEfforts),
    }));
    // Claude Code CLI models are opt-in: they only join the list after the
    // user explicitly enables them in model configuration.
    const models = [...builtinModels, ...codexModels.value, ...customs].filter(
      (m) => m.provider !== "claude_code" || modelDefaults.value.claudeCodeEnabled === true,
    );
    return filterVisibleModels(models);
  });

  const availableModels = computed(() => {
    const providers = new Set<string>();
    if (authStore.hasApiKey) providers.add("openrouter");
    if (authStore.isAuthenticated) providers.add("anthropic");
    if (authStore.claudeCodeAvailable) providers.add("claude_code");
    if (authStore.codexAuthenticated) providers.add("openai_codex");
    providers.add("custom");
    return allModels.value.filter((m) => providers.has(m.provider));
  });

  const selectedCustomEndpoint = computed<CustomEndpoint | null>(() =>
    customEndpoints.value.find((ep) => `custom/${ep.id}` === selectedModelId.value) ?? null
  );

  const selectedModelOption = computed<ModelOption | null>(() =>
    allModels.value.find((model) => model.id === selectedModelId.value) ?? null
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
    if (selectedModelId.value.startsWith("custom/")) {
      const endpoint = selectedCustomEndpoint.value;
      if (!endpoint || endpoint.reasoningParamFormat === "none") return [];
      return normalizeCustomReasoningEfforts(endpoint.supportedReasoningEfforts);
    }
    if (m.includes("claude")) return ["none", "low", "medium", "high"];
    const openAiModel = selectedOpenAiReasoningModel.value;
    if (!openAiModel || !supportsOpenAiReasoningModel(openAiModel)) return [];
    const catalogEfforts = selectedModelOption.value?.supportedEfforts ?? [];
    if (catalogEfforts.length > 0) return catalogEfforts;
    return openAiReasoningLevels(openAiModel);
  });

  const effortSupported = computed(() => availableEfforts.value.length > 0);

  // -- Internal watchers (model-domain only) --

  function clampEffortForSelectedModel(level: EffortLevel): EffortLevel {
    const levels = availableEfforts.value;
    if (levels.length > 0 && !levels.includes(level)) {
      return levels[0];
    }
    return level;
  }

  // Clamp effort when available levels change
  watch(availableEfforts, (levels) => {
    if (levels.length > 0 && !levels.includes(effort.value)) {
      effort.value = levels[0];
    }
  }, { immediate: true });

  watch(defaultEffort, (level) => {
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
        hasUserDefaultEffort.value = true;
        defaultEffort.value = saved;
        effort.value = clampEffortForSelectedModel(saved);
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

  async function loadCodexAvailableModels() {
    if (!authStore.codexAuthenticated) {
      codexRemoteModels.value = [];
      return;
    }
    try {
      codexRemoteModels.value = normalizeCodexModels(await modelService.getCodexAvailableModels());
    } catch (e: unknown) {
      console.warn("[model] get_codex_available_models:", e);
      codexRemoteModels.value = [];
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

  function selectEffort(level: EffortLevel) {
    if (!isEffortLevel(level)) return;
    hasUserDefaultEffort.value = true;
    defaultEffort.value = level;
    effort.value = clampEffortForSelectedModel(level);
  }

  function applyContextEffort(level: EffortLevel | null | undefined) {
    const normalized = typeof level === "string" && isEffortLevel(level) ? level : "none";
    effort.value = clampEffortForSelectedModel(normalized);
  }

  function restoreDefaultEffort() {
    applyContextEffort(defaultEffort.value);
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
    codexRemoteModels,
    codexTransport,
    selectedModelId,
    lastModelId,
    effort,
    defaultEffort,
    hasUserDefaultEffort,
    modelDefaults,
    allModels,
    availableModels,
    codexModels,
    selectedCustomEndpoint,
    selectedOpenAiReasoningModel,
    availableEfforts,
    effortSupported,
    loadModelDefaults,
    loadLastModel,
    loadLastEffort,
    loadCustomEndpoints,
    loadCodexModelConfig,
    loadCodexAvailableModels,
    resolveSelectedModel,
    selectModel,
    selectEffort,
    applyContextEffort,
    restoreDefaultEffort,
    applyModelDefaults,
    applyCustomEndpoints,
    applyCodexModelConfig,
  };
});
