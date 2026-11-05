// Custom providers as edited in Settings and onboarding: the saved list, the
// provider being edited, connectivity tests and the model catalog. Mutations
// run one at a time against the latest saved list.
import { ref } from "vue";
import { defineStore } from "pinia";
import {
  getCustomProviders,
  getModelCatalog,
  refreshModelCatalog,
  saveCustomProviders,
  testCustomEndpoint,
} from "../services/model";
import {
  customEndpointTestStatusForReply,
  normalizeCustomEndpointTestErrorMessage,
} from "../services/customEndpointTestResult";
import {
  DEFAULT_CATALOG_CONTEXT_LENGTH,
  DEFAULT_PROVIDER_PREFIX_CACHE_TTL_SECONDS,
  DEFAULT_REASONING_EFFORTS,
  defaultReplayReasoningContent,
  defaultReasoningParamFormat,
  inferredReasoningReplayField,
  isDeepSeekV4Model,
  modelRowIdFromApiModel,
  newCustomProvider,
  OPENROUTER_SEARCH_ENGINES,
} from "../services/modelCatalog";
import { normalizeAppError } from "../services/errors";
import { getWarmup, setWarmup } from "../composables/warmupCache";
import { t } from "../i18n";
import type {
  ApiFormat,
  CustomEndpoint,
  CustomProvider,
  CustomProviderModel,
  EffortLevel,
  ModelCatalogResponse,
} from "../types";
import { useModelStore } from "./model";
import { useNotificationStore } from "./notification";
import { useSettingsFeedbackStore } from "./settingsFeedback";

export type EndpointTestStatus = "idle" | "testing" | "success" | "error";

const LEGACY_DEFAULT_REASONING_EFFORTS: EffortLevel[] = ["low", "medium", "high", "max"];
const REASONING_EFFORTS = new Set<EffortLevel>(["none", "low", "medium", "high", "xhigh", "max"]);

function isSameEffortList(a: EffortLevel[], b: EffortLevel[]): boolean {
  return a.length === b.length && a.every((value, index) => value === b[index]);
}

function normalizeReasoningEfforts(values?: EffortLevel[] | null): EffortLevel[] {
  const normalized = Array.isArray(values)
    ? values.filter((value): value is EffortLevel => REASONING_EFFORTS.has(value))
    : [];
  if (isSameEffortList(normalized, LEGACY_DEFAULT_REASONING_EFFORTS)) {
    return [...DEFAULT_REASONING_EFFORTS];
  }
  return normalized.length > 0 ? normalized : [...DEFAULT_REASONING_EFFORTS];
}

function normalizeServerTools(
  value?: Partial<CustomProviderModel["serverTools"]> | null,
): CustomProviderModel["serverTools"] {
  const engine = value?.webSearchEngine;
  return {
    webSearch: value?.webSearch === true,
    // Absent = OpenRouter's automatic engine; keeps the legacy shape intact.
    ...(engine && OPENROUTER_SEARCH_ENGINES.includes(engine) ? { webSearchEngine: engine } : {}),
  };
}

function normalizeProviderModel(
  model: CustomProviderModel,
  apiFormat: ApiFormat,
): CustomProviderModel {
  const reasoningReplayField = inferredReasoningReplayField(model);
  const replayRequired = isDeepSeekV4Model(model);
  return {
    ...model,
    id: model.id?.trim() ? model.id.trim() : modelRowIdFromApiModel(model.apiModel),
    name: model.name?.trim() ? model.name : model.apiModel,
    contextLength: Number.isFinite(model.contextLength) && model.contextLength > 0
      ? model.contextLength
      : DEFAULT_CATALOG_CONTEXT_LENGTH,
    remoteCompactionMode: (apiFormat === "openai_responses"
      && (model.codexTransport === "http" || model.codexTransport === "websocket"))
      || model.remoteCompactionMode === "codex_v2"
      ? "codex_v2"
      : "disabled",
    codexTransport: apiFormat === "openai_responses"
      && (model.codexTransport === "http" || model.codexTransport === "websocket")
      ? model.codexTransport
      : null,
    supportsToolLazyLoading: model.supportsToolLazyLoading === true,
    supportedReasoningEfforts: normalizeReasoningEfforts(model.supportedReasoningEfforts),
    reasoningParamFormat: model.reasoningParamFormat ?? defaultReasoningParamFormat(apiFormat),
    replayReasoningContent: replayRequired
      ? true
      : typeof model.replayReasoningContent === "boolean"
        ? model.replayReasoningContent
        : defaultReplayReasoningContent(apiFormat, model),
    reasoningReplayField,
    serverTools: normalizeServerTools(model.serverTools),
    supportsVision: model.supportsVision !== false,
  };
}

export function normalizeCustomProvider(provider: CustomProvider): CustomProvider {
  const prefixCacheTtlSeconds = Number(provider.prefixCacheTtlSeconds);
  return {
    ...provider,
    prefixCacheTtlSeconds: Number.isFinite(prefixCacheTtlSeconds)
      ? Math.max(0, Math.round(prefixCacheTtlSeconds))
      : DEFAULT_PROVIDER_PREFIX_CACHE_TTL_SECONDS,
    models: (provider.models ?? []).map((model) =>
      normalizeProviderModel(model, provider.apiFormat),
    ),
  };
}

/** Legacy single-model endpoint (e.g. Claude Code token import) mapped onto a provider. */
function endpointToProvider(ep: CustomEndpoint): CustomProvider {
  return normalizeCustomProvider({
    id: ep.id,
    name: ep.name,
    endpoint: ep.endpoint,
    apiFormat: ep.apiFormat,
    apiKey: ep.apiKey,
    catalogId: null,
    prefixCacheTtlSeconds: DEFAULT_PROVIDER_PREFIX_CACHE_TTL_SECONDS,
    models: [{
      id: modelRowIdFromApiModel(ep.apiModel),
      apiModel: ep.apiModel,
      name: ep.apiModel,
      contextLength: ep.contextLength,
      remoteCompactionMode: "disabled",
      supportsToolLazyLoading: ep.supportsToolLazyLoading === true,
      supportedReasoningEfforts: ep.supportedReasoningEfforts,
      reasoningParamFormat: ep.reasoningParamFormat ?? null,
      replayReasoningContent: ep.replayReasoningContent,
      reasoningReplayField: null,
      serverTools: normalizeServerTools(ep.serverTools),
      supportsVision: ep.supportsVision !== false,
    }],
  });
}

export const useCustomProviderSettingsStore = defineStore("customProviderSettings", () => {
  const customProviders = ref<CustomProvider[]>([]);
  const editingCustomProvider = ref<CustomProvider | null>(null);
  const isAddingCustomProvider = ref(false);
  const customProviderSaving = ref(false);
  const testStatus = ref<EndpointTestStatus>("idle");
  const testResult = ref("");
  const modelCatalog = ref<ModelCatalogResponse | null>(null);
  const modelCatalogLoading = ref(false);
  const modelCatalogRefreshing = ref(false);
  let loaded = false;
  let mutationQueue: Promise<void> = Promise.resolve();
  let pendingMutations = 0;

  function applyLoaded(providers: CustomProvider[]): CustomProvider[] {
    const normalized = providers.map(normalizeCustomProvider);
    customProviders.value = normalized;
    setWarmup("settings:customProviders", normalized);
    return normalized;
  }

  async function reloadAfterMutation(): Promise<CustomProvider[]> {
    const latest = applyLoaded(await getCustomProviders());
    useModelStore().applyCustomProviders(latest);
    return latest;
  }

  function enqueueMutation(task: () => Promise<void>): Promise<void> {
    pendingMutations += 1;
    customProviderSaving.value = true;
    const run = mutationQueue
      .catch(() => undefined)
      .then(task)
      .finally(() => {
        pendingMutations = Math.max(0, pendingMutations - 1);
        if (pendingMutations === 0) {
          customProviderSaving.value = false;
        }
      });
    mutationQueue = run;
    return run;
  }

  async function loadCustomProviders(): Promise<void> {
    try {
      applyLoaded(await getCustomProviders());
      loaded = true;
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.custom.loadFailed", err.message), {
        code: err.code,
        operation: "loadCustomProviders",
      });
    }
  }

  /** First use in this window: the startup warm-up result, else a fresh load. */
  async function ensureLoaded(): Promise<void> {
    if (loaded) return;
    const cached = getWarmup<CustomProvider[]>("settings:customProviders");
    if (cached) {
      customProviders.value = cached.map(normalizeCustomProvider);
      loaded = true;
      return;
    }
    await loadCustomProviders();
  }

  /** Saves a Claude Code proxy endpoint imported from its settings file. */
  async function saveImportedEndpoint(endpoint: CustomEndpoint): Promise<void> {
    const provider = endpointToProvider(endpoint);
    await enqueueMutation(async () => {
      const list = [...customProviders.value];
      const idx = list.findIndex((existing) => existing.id === provider.id);
      if (idx >= 0) {
        list[idx] = provider;
      } else {
        list.push(provider);
      }

      await saveCustomProviders(list);
      await reloadAfterMutation();
    });
  }

  async function loadModelCatalog(force = false): Promise<void> {
    if (modelCatalog.value && !force) return;
    modelCatalogLoading.value = true;
    try {
      modelCatalog.value = await getModelCatalog();
    } catch (e) {
      console.warn("[settings] get_model_catalog:", e);
    } finally {
      modelCatalogLoading.value = false;
    }
  }

  async function refreshCatalog(): Promise<void> {
    modelCatalogRefreshing.value = true;
    try {
      modelCatalog.value = await refreshModelCatalog();
      useSettingsFeedbackStore().flashSuccess(t("settings.custom.catalogRefreshed"));
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.custom.catalogRefreshFailed", err.message), {
        code: err.code,
        operation: "refreshModelCatalog",
      });
    } finally {
      modelCatalogRefreshing.value = false;
    }
  }

  function startAddCustomProvider(): void {
    editingCustomProvider.value = newCustomProvider();
    isAddingCustomProvider.value = true;
    testStatus.value = "idle";
    testResult.value = "";
  }

  function startEditCustomProvider(provider: CustomProvider): void {
    editingCustomProvider.value = normalizeCustomProvider(
      JSON.parse(JSON.stringify(provider)) as CustomProvider,
    );
    isAddingCustomProvider.value = false;
    testStatus.value = "idle";
    testResult.value = "";
  }

  function cancelEditCustomProvider(): void {
    editingCustomProvider.value = null;
    isAddingCustomProvider.value = false;
  }

  async function saveCustomProvider(): Promise<void> {
    if (!editingCustomProvider.value) return;
    const feedback = useSettingsFeedbackStore();
    const provider = normalizeCustomProvider(editingCustomProvider.value);
    if (!provider.name.trim()) { feedback.setError(t("settings.custom.nameRequired")); return; }
    if (!provider.endpoint.trim()) { feedback.setError(t("settings.custom.endpointRequired")); return; }
    const models = provider.models.filter((model) => model.apiModel.trim());
    if (models.length === 0) { feedback.setError(t("settings.custom.apiModelRequired")); return; }
    provider.models = models;
    feedback.clearError();

    await enqueueMutation(async () => {
      const list = [...customProviders.value];
      const idx = list.findIndex((p) => p.id === provider.id);
      if (idx >= 0) {
        list[idx] = provider;
      } else {
        list.push(provider);
      }

      try {
        await saveCustomProviders(list);
        await reloadAfterMutation();
        editingCustomProvider.value = null;
        isAddingCustomProvider.value = false;
        feedback.flashSuccess(t("settings.custom.saved"));
      } catch (e) {
        const err = normalizeAppError(e);
        useNotificationStore().addNotice("error", t("settings.custom.saveFailed", err.message), {
          code: err.code,
          operation: "saveCustomProvider",
        });
      }
    });
  }

  async function deleteCustomProvider(id: string): Promise<void> {
    await enqueueMutation(async () => {
      const list = customProviders.value.filter((p) => p.id !== id);
      try {
        await saveCustomProviders(list);
        await reloadAfterMutation();
        if (editingCustomProvider.value?.id === id) {
          editingCustomProvider.value = null;
          isAddingCustomProvider.value = false;
        }
        useSettingsFeedbackStore().flashSuccess(t("settings.custom.deleted"));
      } catch (e) {
        const err = normalizeAppError(e);
        useNotificationStore().addNotice("error", t("settings.custom.saveFailed", err.message), {
          code: err.code,
          operation: "deleteCustomProvider",
        });
      }
    });
  }

  /** Connectivity test for the provider being edited, probing the given model
   *  row (or the first row with an api model) via the legacy test payload. */
  async function testCustomProvider(modelRowId?: string): Promise<void> {
    if (!editingCustomProvider.value) return;
    const provider = normalizeCustomProvider(editingCustomProvider.value);
    const model = (modelRowId
      ? provider.models.find((m) => m.id === modelRowId)
      : undefined) ?? provider.models.find((m) => m.apiModel.trim());
    if (!model || !provider.endpoint.trim()) {
      testStatus.value = "error";
      testResult.value = t("settings.custom.testMissingFields");
      return;
    }
    testStatus.value = "testing";
    testResult.value = "";
    try {
      const reply = await testCustomEndpoint({
        id: provider.id,
        name: provider.name,
        apiModel: model.apiModel,
        endpoint: provider.endpoint,
        apiFormat: provider.apiFormat,
        apiKey: provider.apiKey,
        contextLength: model.contextLength,
        supportedReasoningEfforts: model.supportedReasoningEfforts,
        reasoningParamFormat: model.reasoningParamFormat
          ?? defaultReasoningParamFormat(provider.apiFormat),
        replayReasoningContent: model.replayReasoningContent === true,
        serverTools: model.serverTools,
        supportsToolLazyLoading: model.supportsToolLazyLoading === true,
        supportsVision: model.supportsVision,
      }, model.codexTransport ?? null);
      testStatus.value = customEndpointTestStatusForReply(reply);
      testResult.value = reply;
    } catch (e) {
      testStatus.value = "error";
      testResult.value = normalizeCustomEndpointTestErrorMessage(e);
    }
  }

  /** After "reset all settings": forget local state; the next use reloads. */
  function reset(): void {
    customProviders.value = [];
    editingCustomProvider.value = null;
    isAddingCustomProvider.value = false;
    customProviderSaving.value = false;
    testStatus.value = "idle";
    testResult.value = "";
    loaded = false;
  }

  return {
    customProviders,
    editingCustomProvider,
    isAddingCustomProvider,
    customProviderSaving,
    testStatus,
    testResult,
    modelCatalog,
    modelCatalogLoading,
    modelCatalogRefreshing,
    ensureLoaded,
    loadCustomProviders,
    saveImportedEndpoint,
    loadModelCatalog,
    refreshCatalog,
    startAddCustomProvider,
    startEditCustomProvider,
    cancelEditCustomProvider,
    saveCustomProvider,
    deleteCustomProvider,
    testCustomProvider,
    reset,
  };
});
