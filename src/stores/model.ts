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

const CLAUDE_CONTEXT_1M = 1_000_000;
const CLAUDE_STANDARD_EFFORTS: EffortLevel[] = ["none", "low", "medium", "high", "max"];
const CLAUDE_XHIGH_EFFORTS: EffortLevel[] = ["none", "low", "medium", "high", "xhigh", "max"];

const builtinModels: ModelOption[] = [
  {
    id: "openrouter/claude-fable-5",
    name: "Claude Fable 5[1m]",
    provider: "openrouter",
    contextWindow: CLAUDE_CONTEXT_1M,
    supportedEfforts: CLAUDE_XHIGH_EFFORTS,
  },
  {
    id: "openrouter/claude-opus-4.8",
    name: "Claude Opus 4.8[1m]",
    provider: "openrouter",
    contextWindow: CLAUDE_CONTEXT_1M,
    supportedEfforts: CLAUDE_XHIGH_EFFORTS,
    isDefault: true,
  },
  {
    id: "openrouter/claude-sonnet-5",
    name: "Claude Sonnet 5[1m]",
    provider: "openrouter",
    contextWindow: CLAUDE_CONTEXT_1M,
    supportedEfforts: CLAUDE_XHIGH_EFFORTS,
  },
  {
    id: "openrouter/claude-opus-4.6",
    name: "Claude Opus 4.6[1m]",
    provider: "openrouter",
    contextWindow: CLAUDE_CONTEXT_1M,
    supportedEfforts: CLAUDE_STANDARD_EFFORTS,
  },
  { id: "openrouter/glm-5", name: "GLM 5", provider: "openrouter" },
  { id: "openrouter/minimax-m2.5", name: "MiniMax M2.5", provider: "openrouter" },
  {
    id: "claude-fable-5",
    name: "Claude Fable 5[1m]",
    provider: "anthropic",
    contextWindow: CLAUDE_CONTEXT_1M,
    supportedEfforts: CLAUDE_XHIGH_EFFORTS,
  },
  {
    id: "claude-opus-4.8",
    name: "Claude Opus 4.8[1m]",
    provider: "anthropic",
    contextWindow: CLAUDE_CONTEXT_1M,
    supportedEfforts: CLAUDE_XHIGH_EFFORTS,
    isDefault: true,
  },
  {
    id: "claude-sonnet-5",
    name: "Claude Sonnet 5[1m]",
    provider: "anthropic",
    contextWindow: CLAUDE_CONTEXT_1M,
    supportedEfforts: CLAUDE_XHIGH_EFFORTS,
  },
  {
    id: "claude-opus-4.6",
    name: "Claude Opus 4.6[1m]",
    provider: "anthropic",
    contextWindow: CLAUDE_CONTEXT_1M,
    supportedEfforts: CLAUDE_STANDARD_EFFORTS,
  },
  {
    id: "claude_code/claude-fable-5",
    name: "Claude Fable 5[1m]",
    provider: "claude_code",
    contextWindow: CLAUDE_CONTEXT_1M,
    supportedEfforts: CLAUDE_XHIGH_EFFORTS,
  },
  {
    id: "claude_code/claude-opus-4.8[1m]",
    name: "Claude Opus 4.8[1m]",
    provider: "claude_code",
    contextWindow: CLAUDE_CONTEXT_1M,
    supportedEfforts: CLAUDE_XHIGH_EFFORTS,
    isDefault: true,
  },
  {
    id: "claude_code/claude-sonnet-5",
    name: "Claude Sonnet 5[1m]",
    provider: "claude_code",
    contextWindow: CLAUDE_CONTEXT_1M,
    supportedEfforts: CLAUDE_XHIGH_EFFORTS,
  },
  {
    id: "claude_code/claude-opus-4.6[1m]",
    name: "Claude Opus 4.6[1m]",
    provider: "claude_code",
    contextWindow: CLAUDE_CONTEXT_1M,
    supportedEfforts: CLAUDE_STANDARD_EFFORTS,
  },
];

const codexFallbackModels: ModelOption[] = [
  {
    id: "openai/gpt-5.6-sol",
    name: "GPT-5.6 Sol",
    provider: "openai_codex",
    contextWindow: 353_400,
    defaultEffort: "low",
    supportedEfforts: ["low", "medium", "high", "xhigh", "max"],
    additionalSpeedTiers: ["fast"],
    isDefault: true,
  },
  {
    id: "openai/gpt-5.6-terra",
    name: "GPT-5.6 Terra",
    provider: "openai_codex",
    contextWindow: 353_400,
    defaultEffort: "medium",
    supportedEfforts: ["low", "medium", "high", "xhigh", "max"],
    additionalSpeedTiers: ["fast"],
    isDefault: false,
  },
  {
    id: "openai/gpt-5.6-luna",
    name: "GPT-5.6 Luna",
    provider: "openai_codex",
    contextWindow: 353_400,
    defaultEffort: "medium",
    supportedEfforts: ["low", "medium", "high", "xhigh", "max"],
    additionalSpeedTiers: ["fast"],
    isDefault: false,
  },
  {
    id: "openai/gpt-5.5",
    name: "GPT-5.5",
    provider: "openai_codex",
    defaultEffort: "medium",
    supportedEfforts: ["low", "medium", "high", "xhigh"],
    additionalSpeedTiers: ["fast"],
    isDefault: false,
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
  if (m.includes("gpt-5.6")) return ["low", "medium", "high", "xhigh", "max"];
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
  const codexFastMode = ref(false);
  const selectedModelId = ref("");
  const lastModelId = ref("");
  const effort = ref<EffortLevel>("high");
  const defaultEffort = ref<EffortLevel>("high");
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

  function modelSupportsCodexFastMode(modelId: string): boolean {
    const model = allModels.value.find((candidate) => candidate.id === modelId);
    return model?.provider === "openai_codex"
      && model.additionalSpeedTiers?.some((tier) => tier.toLowerCase() === "fast") === true;
  }

  const codexFastModeAvailable = computed(() =>
    modelSupportsCodexFastMode(selectedModelId.value)
  );

  const effectiveCodexFastMode = computed(() =>
    codexFastMode.value && codexFastModeAvailable.value
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
    const catalogEfforts = selectedModelOption.value?.supportedEfforts ?? [];
    if (catalogEfforts.length > 0) return catalogEfforts;
    if (m.includes("claude")) return ["none", "low", "medium", "high"];
    const openAiModel = selectedOpenAiReasoningModel.value;
    if (!openAiModel || !supportsOpenAiReasoningModel(openAiModel)) return [];
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

  async function loadCodexFastMode() {
    try {
      codexFastMode.value = await modelService.getCodexFastMode();
    } catch {
      codexFastMode.value = false;
    }
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

  function selectCodexFastMode(enabled: boolean) {
    codexFastMode.value = enabled;
    modelService.saveCodexFastMode(enabled)
      .catch((e: unknown) => console.warn("[model] save_codex_fast_mode:", e));
  }

  function codexFastModeForModel(modelId: string): boolean {
    return codexFastMode.value && modelSupportsCodexFastMode(modelId);
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
    codexFastMode,
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
    codexFastModeAvailable,
    effectiveCodexFastMode,
    availableEfforts,
    effortSupported,
    loadModelDefaults,
    loadLastModel,
    loadLastEffort,
    loadCodexFastMode,
    loadCustomEndpoints,
    loadCodexModelConfig,
    loadCodexAvailableModels,
    resolveSelectedModel,
    selectModel,
    selectEffort,
    selectCodexFastMode,
    codexFastModeForModel,
    applyContextEffort,
    restoreDefaultEffort,
    applyModelDefaults,
    applyCustomEndpoints,
    applyCodexModelConfig,
  };
});
