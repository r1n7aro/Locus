<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../i18n";
import type {
  EmbeddingConfig,
  EmbeddingLocalModelCatalog,
  EmbeddingModelPreset,
  EmbeddingStatus,
  KnowledgeGeneralConfig,
  KnowledgeRetrievalOverview,
  KnowledgeSearchMatchKind,
  LexicalRebuildStatus,
} from "../../types";
import BaseButton from "../ui/BaseButton.vue";
import BaseDropdown from "../ui/BaseDropdown.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";

const props = defineProps<{
  overview: KnowledgeRetrievalOverview | null;
  generalConfig: KnowledgeGeneralConfig | null;
  embeddingConfig: EmbeddingConfig | null;
  embeddingLocalModelCatalog: EmbeddingLocalModelCatalog | null;
  embeddingStatus: EmbeddingStatus | null;
  lexicalRebuildStatus: LexicalRebuildStatus | null;
  searchMode: KnowledgeSearchMatchKind | null;
  searchLatencyMs: number | null;
  recentQueryTokens: number | null;
  loading: boolean;
  pending: boolean;
}>();

const emit = defineEmits<{
  (e: "toggleLexical", value: boolean): void;
  (e: "toggleSemantic", value: boolean): void;
  (e: "setDevicePolicy", value: string): void;
  (e: "setDownloadSource", value: string): void;
  (e: "rebuildLexical"): void;
  (e: "rebuildSemantic"): void;
  (e: "refresh"): void;
  (e: "selectLocalModelOption", value: string): void;
  (e: "downloadLocalModel", value?: string): void;
}>();

const downloadPopoverOpen = ref(false);
const downloadMode = ref<"preset" | "custom">("preset");
const downloadPresetId = ref("");
const downloadModelInput = ref("");

function formatBytes(bytes: number | null | undefined): string {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value >= 100 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

function formatPercent(value: number | null | undefined): string {
  if (!value) return "0%";
  return `${Math.round(value * 100)}%`;
}

function retrievalModeLabel(mode: KnowledgeSearchMatchKind): string {
  switch (mode) {
    case "hybrid":
    case "grepHybrid":
      return t("knowledge.dashboard.knowledge.searchModeHybrid");
    case "semantic":
      return t("knowledge.dashboard.knowledge.searchModeSemantic");
    case "grep":
      return t("knowledge.dashboard.knowledge.searchModeGrep");
    default:
      return t("knowledge.dashboard.knowledge.searchModeLexical");
  }
}

function searchModeLabel(mode: KnowledgeSearchMatchKind | null): string {
  return mode ? retrievalModeLabel(mode) : "—";
}

function lexicalStageLabel(stage: string | null | undefined): string {
  switch (stage) {
    case "preparing":
      return t("knowledge.dashboard.knowledge.stagePreparing");
    case "cleaning":
      return t("knowledge.dashboard.knowledge.stageCleaning");
    case "indexing":
      return t("knowledge.dashboard.knowledge.stageIndexing");
    case "committing":
      return t("knowledge.dashboard.knowledge.stageCommitting");
    case "completed":
      return t("knowledge.dashboard.knowledge.stageCompleted");
    case "downloading_model":
      return t("settings.knowledge.stage.downloadingModel");
    case "cancelling":
      return t("settings.knowledge.stage.cancelling");
    case "cancelled":
      return t("settings.knowledge.stage.cancelled");
    case "initializing_runtime":
      return t("settings.knowledge.stage.initializingRuntime");
    case "ready":
      return t("settings.knowledge.stage.ready");
    case "error":
      return t("settings.knowledge.stage.error");
    default:
      return stage || t("knowledge.dashboard.knowledge.stageIdle");
  }
}

function semanticStageLabelForStage(stage: string | null | undefined): string {
  switch (stage) {
    case "committing":
      return t("knowledge.dashboard.knowledge.stagePersistingEmbeddings");
    default:
      return lexicalStageLabel(stage);
  }
}

interface DownloadPresetMeta {
  capability: string;
  parameters: string;
  parameterCount: number | null;
  contextWindow: string;
  dimensions: string;
  license: string;
  burden: "low" | "medium" | "high" | "veryHigh";
  minGpuBytes: number;
  recommendedGpuBytes: number;
  minRamBytes: number;
  recommendedRamBytes: number;
}

function gibibytes(value: number): number {
  return value * 1024 * 1024 * 1024;
}

function downloadPresetMeta(modelId: string): DownloadPresetMeta | null {
  switch (modelId) {
    case "Qwen/Qwen3-Embedding-4B":
      return {
        capability: t("knowledge.retrieval.preset.qwen4.summary"),
        parameters: "4B",
        parameterCount: 4_000_000_000,
        contextWindow: "32K",
        dimensions: "32-2560",
        license: "Apache-2.0",
        burden: "high",
        minGpuBytes: gibibytes(10),
        recommendedGpuBytes: gibibytes(14),
        minRamBytes: gibibytes(16),
        recommendedRamBytes: gibibytes(24),
      };
    case "Qwen/Qwen3-Embedding-0.6B":
      return {
        capability: t("knowledge.retrieval.preset.qwen06.summary"),
        parameters: "0.6B",
        parameterCount: 600_000_000,
        contextWindow: "32K",
        dimensions: "32-1024",
        license: "Apache-2.0",
        burden: "medium",
        minGpuBytes: gibibytes(2),
        recommendedGpuBytes: gibibytes(4),
        minRamBytes: gibibytes(8),
        recommendedRamBytes: gibibytes(12),
      };
    case "Alibaba-NLP/gte-multilingual-base":
      return {
        capability: t("knowledge.retrieval.preset.gteMultiBase.summary"),
        parameters: "305M",
        parameterCount: 305_000_000,
        contextWindow: t("knowledge.retrieval.presetContextTokens", 8192),
        dimensions: "768",
        license: "Apache-2.0",
        burden: "low",
        minGpuBytes: gibibytes(2),
        recommendedGpuBytes: gibibytes(3),
        minRamBytes: gibibytes(6),
        recommendedRamBytes: gibibytes(8),
      };
    case "jinaai/jina-embeddings-v5-text-small-retrieval":
      return {
        capability: t("knowledge.retrieval.preset.jinaSmall.summary"),
        parameters: "677M",
        parameterCount: 677_000_000,
        contextWindow: "32K",
        dimensions: "32-1024",
        license: "CC BY-NC 4.0",
        burden: "medium",
        minGpuBytes: gibibytes(4),
        recommendedGpuBytes: gibibytes(6),
        minRamBytes: gibibytes(8),
        recommendedRamBytes: gibibytes(12),
      };
    case "jinaai/jina-embeddings-v5-text-nano-retrieval":
      return {
        capability: t("knowledge.retrieval.preset.jinaNano.summary"),
        parameters: "239M",
        parameterCount: 239_000_000,
        contextWindow: t("knowledge.retrieval.presetContextTokens", 8192),
        dimensions: "768",
        license: "CC BY-NC 4.0",
        burden: "low",
        minGpuBytes: gibibytes(1),
        recommendedGpuBytes: gibibytes(2),
        minRamBytes: gibibytes(4),
        recommendedRamBytes: gibibytes(6),
      };
    case "Qwen/Qwen3-Embedding-8B":
      return {
        capability: t("knowledge.retrieval.preset.qwen8.summary"),
        parameters: "8B",
        parameterCount: 8_000_000_000,
        contextWindow: "32K",
        dimensions: "32-4096",
        license: "Apache-2.0",
        burden: "veryHigh",
        minGpuBytes: gibibytes(18),
        recommendedGpuBytes: gibibytes(24),
        minRamBytes: gibibytes(32),
        recommendedRamBytes: gibibytes(48),
      };
    case "BAAI/bge-m3":
      return {
        capability: t("knowledge.retrieval.preset.bgeM3.summary"),
        parameters: "—",
        parameterCount: null,
        contextWindow: t("knowledge.retrieval.presetContextTokens", 8192),
        dimensions: "1024",
        license: "MIT",
        burden: "medium",
        minGpuBytes: gibibytes(4),
        recommendedGpuBytes: gibibytes(6),
        minRamBytes: gibibytes(8),
        recommendedRamBytes: gibibytes(12),
      };
    default:
      return null;
  }
}

const semanticReady = computed(() =>
  !!props.generalConfig?.semanticSearchEnabled
  && !!props.embeddingConfig?.enabled
  && !!props.embeddingStatus?.ready,
);

const lexicalEnabled = computed(() =>
  !!props.generalConfig?.enabled && !!props.generalConfig?.lexicalSearchEnabled,
);

const semanticEnabled = computed(() =>
  !!props.generalConfig?.enabled && !!props.generalConfig?.semanticSearchEnabled,
);

function runtimeRouteToDevicePolicy(route: string | null | undefined): string | null {
  const normalizedRoute = route?.trim().toLowerCase() || "";
  if (normalizedRoute === "directml") return "gpu_directml";
  if (normalizedRoute === "cuda") return "gpu_cuda";
  if (normalizedRoute === "cpu" || normalizedRoute.startsWith("cpu")) return "cpu_fastembed";
  return null;
}

function devicePolicyLabel(policy: string | null | undefined): string {
  switch (policy) {
    case "gpu_directml":
      return t("settings.knowledge.deviceGpuDirectml");
    case "gpu_cuda":
      return t("settings.knowledge.deviceGpuCuda");
    default:
      return t("settings.knowledge.deviceCpuFastembed");
  }
}

function currentDeviceLabel(
  deviceName: string | null | undefined,
  route: string | null | undefined,
): string {
  const explicitName = deviceName?.trim();
  if (explicitName) return explicitName;
  const normalizedRoute = route?.trim().toLowerCase() || "";
  if (
    normalizedRoute === "directml"
    || normalizedRoute === "cuda"
    || normalizedRoute.startsWith("gpu")
  ) {
    return t("knowledge.retrieval.deviceGpu");
  }
  if (normalizedRoute === "cpu" || normalizedRoute.startsWith("cpu")) {
    return t("knowledge.retrieval.deviceCpu");
  }
  return "-";
}

function runtimeBurdenLabel(level: DownloadPresetMeta["burden"]): string {
  switch (level) {
    case "low":
      return t("knowledge.retrieval.presetBurdenLow");
    case "medium":
      return t("knowledge.retrieval.presetBurdenMedium");
    case "high":
      return t("knowledge.retrieval.presetBurdenHigh");
    default:
      return t("knowledge.retrieval.presetBurdenVeryHigh");
  }
}

function formatEstimateRange(minBytes: number, recommendedBytes: number): string {
  if (!minBytes && !recommendedBytes) return "—";
  if (!minBytes || minBytes >= recommendedBytes) return `${formatBytes(recommendedBytes || minBytes)}`;
  return `${formatBytes(minBytes)} - ${formatBytes(recommendedBytes)}`;
}

function compareDownloadPresetSize(left: EmbeddingModelPreset, right: EmbeddingModelPreset): number {
  const leftSize = downloadPresetMeta(left.id)?.parameterCount ?? Number.POSITIVE_INFINITY;
  const rightSize = downloadPresetMeta(right.id)?.parameterCount ?? Number.POSITIVE_INFINITY;
  if (leftSize !== rightSize) {
    return leftSize - rightSize;
  }
  return left.label.localeCompare(right.label);
}

const availableSearchMode = computed(() => {
  if (lexicalEnabled.value && semanticEnabled.value) {
    return retrievalModeLabel("hybrid");
  }
  if (semanticEnabled.value) {
    return retrievalModeLabel("semantic");
  }
  if (lexicalEnabled.value) {
    return retrievalModeLabel("lexical");
  }
  if (props.generalConfig?.enabled) {
    return retrievalModeLabel("grep");
  }
  return t("knowledge.dashboard.knowledge.disabled");
});

const semanticDetail = computed(() => {
  if (props.embeddingStatus?.error) return props.embeddingStatus.error;
  if (props.overview?.semantic.error) return props.overview.semantic.error;
  if (runtimeFallbackMessage.value) return runtimeFallbackMessage.value;
  if (props.embeddingStatus?.detail) return props.embeddingStatus.detail;
  if (semanticReady.value) return t("knowledge.retrieval.semanticReady");
  return t("knowledge.retrieval.semanticIdle");
});

const semanticRuntimeFailed = computed(() =>
  !!props.embeddingStatus?.error
  || !!props.overview?.semantic.error
  || !!runtimeFallbackMessage.value
  || props.embeddingStatus?.stage === "error"
  || props.overview?.semantic.stage === "error",
);

const lexicalProgressLabel = computed(() => {
  if (props.lexicalRebuildStatus?.error) return props.lexicalRebuildStatus.error;
  if (
    props.lexicalRebuildStatus?.running
    && typeof props.lexicalRebuildStatus.progress === "number"
  ) {
      return `${formatPercent(props.lexicalRebuildStatus.progress)} · ${lexicalStageLabel(props.lexicalRebuildStatus.stage)}`;
  }
  if (
    props.lexicalRebuildStatus?.running
    && props.lexicalRebuildStatus.processedDocs != null
    && props.lexicalRebuildStatus.totalDocs != null
  ) {
    return `${props.lexicalRebuildStatus.processedDocs} / ${props.lexicalRebuildStatus.totalDocs}`;
  }
  if (props.lexicalRebuildStatus?.detail) return props.lexicalRebuildStatus.detail;
  return lexicalStageLabel(props.lexicalRebuildStatus?.stage);
});

const semanticProgressLabel = computed(() => {
  if (props.embeddingStatus?.indexProgress != null) {
    const percent = formatPercent(props.embeddingStatus.indexProgress);
    if (
      props.embeddingStatus.processedDocs != null
      && props.embeddingStatus.totalDocs != null
    ) {
      return `${percent} · ${props.embeddingStatus.processedDocs} / ${props.embeddingStatus.totalDocs}`;
    }
    return percent;
  }
  if (
    props.embeddingStatus?.processedDocs != null
    && props.embeddingStatus?.totalDocs != null
  ) {
    return `${props.embeddingStatus.processedDocs} / ${props.embeddingStatus.totalDocs}`;
  }
  if (props.lexicalRebuildStatus?.running) {
    return t("knowledge.dashboard.knowledge.rebuilding");
  }
  return t("knowledge.dashboard.knowledge.stageIdle");
});

const semanticStageLabel = computed(() =>
  semanticStageLabelForStage(props.embeddingStatus?.stage || props.overview?.semantic.stage),
);

const semanticProgressRatio = computed(() => {
  if (typeof props.embeddingStatus?.indexProgress === "number") {
    return Math.min(1, Math.max(0, props.embeddingStatus.indexProgress));
  }
  if (
    props.embeddingStatus?.processedDocs != null
    && props.embeddingStatus?.totalDocs != null
    && props.embeddingStatus.totalDocs > 0
  ) {
    return Math.min(
      1,
      Math.max(
        0,
        props.embeddingStatus.processedDocs / props.embeddingStatus.totalDocs,
      ),
    );
  }
  return null;
});

const semanticProcessedDocsLabel = computed(() => {
  if (
    props.embeddingStatus?.processedDocs == null
    || props.embeddingStatus?.totalDocs == null
  ) {
    return "—";
  }
  return `${props.embeddingStatus.processedDocs} / ${props.embeddingStatus.totalDocs}`;
});

const semanticFailedDocsLabel = computed(() => {
  const failedDocs = props.embeddingStatus?.failedDocs ?? 0;
  const totalDocs =
    props.embeddingStatus?.totalDocs ?? props.embeddingStatus?.processedDocs;
  if (!failedDocs) return "0";
  return totalDocs != null && totalDocs > 0
    ? `${failedDocs} / ${totalDocs}`
    : `${failedDocs}`;
});

const semanticLastFailureLabel = computed(() => {
  const parts = [
    props.embeddingStatus?.lastFailedFile?.trim() || "",
    props.embeddingStatus?.lastFailure?.trim() || "",
  ].filter(Boolean);
  return parts.join(" · ");
});

const showSemanticProgressPanel = computed(() =>
  !!props.embeddingStatus?.activating
  || props.embeddingStatus?.stage === "indexing"
  || props.embeddingStatus?.stage === "error"
  || (props.embeddingStatus?.failedDocs ?? 0) > 0,
);

const remoteEndpointLabel = computed(() =>
  props.embeddingConfig?.remoteEndpoint?.trim() || "—",
);

const remoteModelLabel = computed(() =>
  props.embeddingConfig?.remoteModel?.trim() || "—",
);

const availableLocalModels = computed(() =>
  props.embeddingLocalModelCatalog?.availableModels ?? [],
);

const isWindowsDesktop = computed(() =>
  typeof navigator !== "undefined" && /windows/i.test(navigator.userAgent),
);

const normalizedDevicePolicy = computed(() => {
  const rawValue = props.embeddingConfig?.devicePolicy?.trim() || "";
  if (rawValue === "gpu_directml" || rawValue === "gpu_cuda") return rawValue;
  return "cpu_fastembed";
});

const runtimeDevicePolicy = computed(() => {
  if (!props.embeddingStatus?.ready) return null;
  return runtimeRouteToDevicePolicy(props.overview?.semantic.deviceRoute);
});

const displayedDevicePolicy = computed(() =>
  runtimeDevicePolicy.value || normalizedDevicePolicy.value,
);

const runtimeFallbackMessage = computed(() => {
  if (props.embeddingConfig?.embeddingMode === "remote") return "";
  if (!runtimeDevicePolicy.value) return "";
  if (runtimeDevicePolicy.value === normalizedDevicePolicy.value) return "";
  return t(
    "knowledge.retrieval.runtimeFallback",
    devicePolicyLabel(normalizedDevicePolicy.value),
    devicePolicyLabel(runtimeDevicePolicy.value),
  );
});

const devicePolicyOptions = computed(() => ([
  {
    value: "cpu_fastembed",
    label: t("settings.knowledge.deviceCpuFastembed"),
    disabled: props.pending || !props.embeddingConfig,
  },
  {
    value: "gpu_directml",
    label: t("settings.knowledge.deviceGpuDirectml"),
    hint: isWindowsDesktop.value
      ? t("settings.knowledge.deviceGpuDirectmlHint")
      : t("settings.knowledge.deviceBackendUnavailable"),
    disabled: props.pending || !props.embeddingConfig || !isWindowsDesktop.value,
  },
  {
    value: "gpu_cuda",
    label: t("settings.knowledge.deviceGpuCuda"),
    hint: t("settings.knowledge.deviceBackendUnavailable"),
    disabled: true,
  },
]));

const activeLocalModelOptions = computed(() => {
  return availableLocalModels.value.map((model) => ({
    value: model.localModelPath ? `directory:${model.localModelPath}` : `preset:${model.modelId}`,
    label: model.label,
  }));
});

const activeLocalModelValue = computed(() => {
  if (props.embeddingConfig?.localModelPath?.trim()) {
    const localModelPath = props.embeddingConfig.localModelPath.trim();
    return availableLocalModels.value.some((model) => model.localModelPath === localModelPath)
      ? `directory:${localModelPath}`
      : "";
  }
  const localModelId = props.embeddingConfig?.localModel?.trim() || "";
  if (!localModelId) return "";
  return availableLocalModels.value.some(
    (model) => !model.localModelPath && model.modelId === localModelId,
  )
    ? `preset:${localModelId}`
    : "";
});

const activeLocalModelSelectedLabel = computed(() => {
  const selected = activeLocalModelOptions.value.find(
    (option) => option.value === activeLocalModelValue.value,
  );
  return selected?.label || "";
});

const sortedDownloadPresets = computed(() =>
  [...(props.embeddingLocalModelCatalog?.presets ?? [])].sort(compareDownloadPresetSize),
);

const currentLocalDeviceLabel = computed(() => {
  return currentDeviceLabel(
    props.overview?.semantic.deviceName,
    props.overview?.semantic.deviceRoute,
  );
});

const downloadPresetOptions = computed(() =>
  sortedDownloadPresets.value.map((preset) => {
    const meta = downloadPresetMeta(preset.id);
    const status = preset.downloaded
      ? t("settings.knowledge.modelDownloaded")
      : t("knowledge.retrieval.downloadPresetHint");
    return {
      value: preset.id,
      label: preset.label,
      costLine: meta
        ? t("knowledge.retrieval.presetListCostLine", runtimeBurdenLabel(meta.burden))
        : "",
      capabilityLine: meta
        ? t("knowledge.retrieval.presetListCapabilityLine", meta.capability)
        : "",
      footerLine: meta
        ? `${meta.parameters} · ${meta.contextWindow} · ${meta.license} · ${status}`
        : status,
    };
  }),
);

const downloadModeOptions = computed(() => ([
  { value: "preset", label: t("knowledge.retrieval.downloadModePreset") },
  { value: "custom", label: t("knowledge.retrieval.downloadModeCustom") },
]));

const normalizedDownloadSource = computed(() => {
  const rawValue = props.embeddingConfig?.localModelDownloadSource?.trim() || "";
  return rawValue.toLowerCase().replace(/_/g, "-") === "hf-mirror"
    ? "hf-mirror"
    : "official";
});

const downloadSourceOptions = computed(() => ([
  {
    value: "official",
    label: t("knowledge.retrieval.downloadSourceOfficial"),
    disabled: props.pending || !props.embeddingConfig,
  },
  {
    value: "hf-mirror",
    label: t("knowledge.retrieval.downloadSourceMirror"),
    disabled: props.pending || !props.embeddingConfig,
  },
]));

const downloadSourceHint = computed(() =>
  normalizedDownloadSource.value === "hf-mirror"
    ? t("knowledge.retrieval.downloadSourceMirrorHint")
    : t("knowledge.retrieval.downloadSourceOfficialHint"),
);

const selectedDownloadPreset = computed(() =>
  sortedDownloadPresets.value.find((preset) => preset.id === downloadPresetId.value) ?? null,
);

const selectedDownloadPresetMeta = computed(() =>
  selectedDownloadPreset.value ? downloadPresetMeta(selectedDownloadPreset.value.id) : null,
);

const selectedDownloadPresetVramEstimate = computed(() => {
  const meta = selectedDownloadPresetMeta.value;
  if (!meta) return "—";
  return formatEstimateRange(meta.minGpuBytes, meta.recommendedGpuBytes);
});

const selectedDownloadPresetRamEstimate = computed(() => {
  const meta = selectedDownloadPresetMeta.value;
  if (!meta) return "—";
  return formatEstimateRange(meta.minRamBytes, meta.recommendedRamBytes);
});

const selectedDownloadPresetBurden = computed(() => {
  const meta = selectedDownloadPresetMeta.value;
  return meta ? runtimeBurdenLabel(meta.burden) : "—";
});

const downloadPresetStatus = computed(() => {
  if (!selectedDownloadPreset.value) return "";
  return selectedDownloadPreset.value.downloaded
    ? t("settings.knowledge.modelDownloaded")
    : t("knowledge.retrieval.downloadPresetHint");
});

const downloadCustomStatus = computed(() => {
  const inputValue = downloadModelInput.value.trim();
  if (!inputValue) return t("knowledge.retrieval.downloadCustomHint");

  const localModel = availableLocalModels.value.find((item) =>
    item.modelId === inputValue || item.label === inputValue,
  );
  if (localModel) return t("settings.knowledge.modelDownloaded");

  const preset = props.embeddingLocalModelCatalog?.presets.find(
    (item) => item.id === inputValue,
  );
  if (preset?.downloaded) return t("settings.knowledge.modelDownloaded");

  return t("knowledge.retrieval.downloadCustomHint");
});

const downloadTargetModelId = computed(() =>
  downloadMode.value === "preset"
    ? downloadPresetId.value.trim()
    : downloadModelInput.value.trim(),
);

function syncDownloadPresetId(forceInput = false) {
  const options = sortedDownloadPresets.value;
  const currentModel = props.embeddingConfig?.localModel?.trim() || "";
  const nextPresetId = options.some((preset) => preset.id === currentModel)
    ? currentModel
    : options.some((preset) => preset.id === downloadPresetId.value)
      ? downloadPresetId.value
      : options[0]?.id ?? "";

  downloadPresetId.value = nextPresetId;
  if (forceInput) {
    downloadMode.value = currentModel && !options.some((preset) => preset.id === currentModel)
      ? "custom"
      : "preset";
  }
  if (
    (forceInput || !downloadModelInput.value.trim())
    && currentModel
    && !options.some((preset) => preset.id === currentModel)
  ) {
    downloadModelInput.value = currentModel;
  }
}

function openDownloadPopover() {
  if (props.pending) return;
  syncDownloadPresetId(true);
  downloadPopoverOpen.value = true;
}

function closeDownloadPopover() {
  downloadPopoverOpen.value = false;
}

function handleActiveLocalModelUpdate(value: string) {
  emit("selectLocalModelOption", value);
}

function handleDevicePolicyUpdate(value: string) {
  if (!value) return;
  if (value === normalizedDevicePolicy.value && value === displayedDevicePolicy.value) return;
  emit("setDevicePolicy", value);
}

function handleDownloadPresetSelection(value: string) {
  downloadPresetId.value = value;
}

function handleDownloadModeUpdate(value: string) {
  downloadMode.value = value === "custom" ? "custom" : "preset";
}

function handleDownloadSourceUpdate(value: string) {
  const nextValue = value === "hf-mirror" ? "hf-mirror" : "official";
  if (nextValue === normalizedDownloadSource.value) return;
  emit("setDownloadSource", nextValue);
}

function handleDownloadPreset() {
  const modelId = downloadTargetModelId.value;
  if (!modelId) return;
  emit("downloadLocalModel", modelId);
  closeDownloadPopover();
}

watch(
  () => [
    props.embeddingConfig?.localModel,
    props.embeddingLocalModelCatalog?.presets.length ?? 0,
  ],
  () => {
    syncDownloadPresetId();
  },
  { immediate: true },
);

const managedDirectoryPath = computed(() =>
  props.embeddingLocalModelCatalog?.managedDirectory || "—",
);
</script>

<template>
  <div class="retrieval-panel">
    <div class="retrieval-header">
      <div class="retrieval-header-main">
        <div class="retrieval-title">{{ t("knowledge.retrieval.title") }}</div>
        <div class="retrieval-subtitle">{{ t("knowledge.retrieval.subtitle") }}</div>
      </div>
      <div class="retrieval-actions">
        <BaseButton :disabled="pending" @click="emit('refresh')">
          {{ t("common.refresh") }}
        </BaseButton>
      </div>
    </div>

    <div v-if="loading && !overview" class="retrieval-loading">{{ t("common.loading") }}</div>

    <div class="retrieval-summary-grid">
      <section class="retrieval-card">
        <div class="card-title">{{ t("knowledge.retrieval.queryCard") }}</div>
        <div class="card-subtitle">{{ t("knowledge.retrieval.queryHint") }}</div>
        <div class="hero-line">
          <span class="hero-value">{{ searchLatencyMs ?? "—" }}</span>
          <span class="hero-label">{{ t("knowledge.retrieval.lastSearchLatency") }}</span>
        </div>
        <div class="metric-list">
          <div class="metric-row">
            <span>{{ t("knowledge.retrieval.lastSearchMode") }}</span>
            <span>{{ searchModeLabel(searchMode) }}</span>
          </div>
          <div class="metric-row">
            <span>{{ t("knowledge.retrieval.currentPath") }}</span>
            <span>{{ availableSearchMode }}</span>
          </div>
          <div class="metric-row">
            <span>{{ t("knowledge.dashboard.knowledge.recentQueryTokens") }}</span>
            <span>{{ recentQueryTokens ?? 0 }}</span>
          </div>
        </div>
      </section>

      <section class="retrieval-card">
        <div class="card-title">{{ t("knowledge.retrieval.performanceCard") }}</div>
        <div class="card-subtitle">{{ t("knowledge.retrieval.performanceHint") }}</div>
        <div class="hero-line compact">
          <span class="hero-value">{{ formatBytes(overview?.performance.totalBytes) }}</span>
          <span class="hero-label">{{ t("knowledge.retrieval.totalStorage") }}</span>
        </div>
        <div class="metric-list">
          <div class="metric-row">
            <span>{{ t("knowledge.dashboard.knowledge.lexicalIndexSize") }}</span>
            <span>{{ formatBytes(overview?.performance.lexicalIndexBytes) }}</span>
          </div>
          <div class="metric-row">
            <span>{{ t("knowledge.dashboard.knowledge.databaseSize") }}</span>
            <span>{{ formatBytes(overview?.performance.dbBytes) }}</span>
          </div>
          <div class="metric-row">
            <span>{{ t("knowledge.dashboard.knowledge.modelMemory") }}</span>
            <span>{{ formatBytes(overview?.performance.localModelBytes) }}</span>
          </div>
          <div class="metric-row">
            <span>{{ t("knowledge.retrieval.gpuMemory") }}</span>
            <span>{{ formatBytes(overview?.performance.gpuMemoryBytes) }}</span>
          </div>
        </div>
      </section>
    </div>

    <div class="retrieval-settings-grid">
      <section class="retrieval-card settings-card">
        <div class="settings-header">
          <div class="settings-head">
            <div class="control-title">{{ t("knowledge.retrieval.lexicalTitle") }}</div>
            <div class="control-meta">{{ t("knowledge.retrieval.lexicalHint") }}</div>
          </div>
          <BaseSwitch
            :model-value="lexicalEnabled"
            :disabled="pending || !generalConfig"
            :aria-label="t('knowledge.retrieval.lexicalTitle')"
            @update:model-value="emit('toggleLexical', $event)"
          />
        </div>

        <div class="settings-body">
          <div class="hero-line">
            <span class="hero-value">{{ overview?.fullText.indexedItemCount ?? 0 }}</span>
            <span class="hero-label">{{ t("knowledge.dashboard.knowledge.docsUnit") }}</span>
          </div>
          <div class="metric-list">
            <div class="metric-row">
              <span>{{ t("knowledge.dashboard.knowledge.coverage") }}</span>
              <span>
                {{ overview?.fullText.indexedItemCount ?? 0 }} / {{ overview?.fullText.indexableItemCount ?? 0 }}
              </span>
            </div>
            <div class="metric-row">
              <span>{{ t("knowledge.dashboard.knowledge.chunkCount") }}</span>
              <span>{{ overview?.fullText.chunkCount ?? 0 }}</span>
            </div>
            <div class="metric-row">
              <span>{{ t("knowledge.retrieval.pendingDocs") }}</span>
              <span>{{ overview?.fullText.pendingItemCount ?? 0 }}</span>
            </div>
            <div class="metric-row">
              <span>{{ t("knowledge.dashboard.knowledge.lastBuild") }}</span>
              <span>{{ overview?.fullText.lastBuildAt || "—" }}</span>
            </div>
            <div class="metric-row">
              <span>{{ t("knowledge.dashboard.knowledge.rebuildProgress") }}</span>
              <span>{{ lexicalProgressLabel }}</span>
            </div>
          </div>
        </div>

        <div class="settings-footer">
          <BaseButton :disabled="pending || !lexicalEnabled" @click="emit('rebuildLexical')">
            {{ t("knowledge.dashboard.knowledge.rebuildIndex") }}
          </BaseButton>
        </div>
      </section>

      <section
        class="retrieval-card settings-card"
        :class="{ 'is-error': semanticRuntimeFailed }"
      >
        <div class="settings-header">
          <div class="settings-head">
            <div class="control-title">{{ t("knowledge.retrieval.semanticTitle") }}</div>
            <div class="control-meta" :class="{ 'is-error': semanticRuntimeFailed }">
              {{ semanticDetail }}
            </div>
          </div>
          <BaseSwitch
            :model-value="semanticEnabled"
            :disabled="pending || !generalConfig"
            :aria-label="t('knowledge.retrieval.semanticTitle')"
            @update:model-value="emit('toggleSemantic', $event)"
          />
        </div>

        <div class="settings-body">
          <div class="hero-line">
            <span class="hero-value">{{ formatPercent(overview?.semantic.coverageRatio) }}</span>
            <span class="hero-label">{{ t("knowledge.dashboard.knowledge.coverage") }}</span>
          </div>
          <div class="metric-list">
            <div
              v-if="embeddingConfig?.embeddingMode !== 'remote'"
              class="metric-row metric-row-control"
            >
              <span>{{ t("settings.knowledge.devicePolicy") }}</span>
              <div class="metric-row-input">
                <BaseSegmented
                  size="sm"
                  :model-value="displayedDevicePolicy"
                  :options="devicePolicyOptions"
                  @update:model-value="handleDevicePolicyUpdate"
                />
              </div>
            </div>
            <div
              v-if="embeddingConfig?.embeddingMode !== 'remote'"
              class="metric-row metric-row-control"
            >
              <span>{{ t("knowledge.retrieval.localModel") }}</span>
              <div class="metric-row-input">
                <BaseDropdown
                  size="sm"
                  :model-value="activeLocalModelValue"
                  :options="activeLocalModelOptions"
                  :selected-label="activeLocalModelSelectedLabel"
                  :disabled="pending || !activeLocalModelOptions.length"
                  :placeholder="t('knowledge.retrieval.localModelEmpty')"
                  :aria-label="t('knowledge.retrieval.localModel')"
                  @update:model-value="handleActiveLocalModelUpdate"
                />
              </div>
            </div>
            <div v-if="embeddingConfig?.embeddingMode !== 'remote'" class="metric-row">
              <span>{{ t("knowledge.retrieval.currentDevice") }}</span>
              <span>{{ currentLocalDeviceLabel }}</span>
            </div>
            <div class="metric-row">
              <span>{{ t("knowledge.dashboard.knowledge.rebuildProgress") }}</span>
              <span>{{ semanticProgressLabel }}</span>
            </div>
            <div class="metric-row">
              <span>{{ t("knowledge.dashboard.knowledge.chunkCount") }}</span>
              <span>{{ overview?.semantic.embeddedChunkCount ?? 0 }}</span>
            </div>
            <div class="metric-row">
              <span>{{ t("knowledge.retrieval.pendingDocs") }}</span>
              <span>{{ overview?.semantic.pendingItemCount ?? 0 }}</span>
            </div>
            <div class="metric-row" :class="{ 'metric-row-status-error': semanticRuntimeFailed }">
              <span>{{ t("knowledge.dashboard.knowledge.rebuildStage") }}</span>
              <span>{{ semanticStageLabel }}</span>
            </div>
            <div class="metric-row">
              <span>{{ t("knowledge.retrieval.modelLabel") }}</span>
              <span class="truncate">{{ overview?.semantic.model || "—" }}</span>
            </div>
          </div>

          <div v-if="embeddingConfig?.embeddingMode === 'remote'" class="config-grid">
            <div class="config-divider"></div>
            <div class="config-row">
              <span class="config-label">{{ t("settings.knowledge.remoteEndpoint") }}</span>
              <span class="config-value truncate">{{ remoteEndpointLabel }}</span>
            </div>
            <div class="config-row">
              <span class="config-label">{{ t("settings.knowledge.remoteModel") }}</span>
              <span class="config-value truncate">{{ remoteModelLabel }}</span>
            </div>
            <div class="config-note">
              {{ t("knowledge.retrieval.remoteSummary") }}
            </div>
          </div>

          <div v-if="showSemanticProgressPanel" class="semantic-progress-panel">
            <div class="semantic-progress-head">
              <span class="semantic-progress-stage">{{ semanticStageLabel }}</span>
              <span class="semantic-progress-value">{{ semanticProgressLabel }}</span>
            </div>
            <div
              class="semantic-progress-track"
              :class="{ indeterminate: semanticProgressRatio == null }"
              aria-hidden="true"
            >
              <div
                class="semantic-progress-fill"
                :class="{ indeterminate: semanticProgressRatio == null }"
                :style="semanticProgressRatio != null
                  ? { width: `${Math.round(semanticProgressRatio * 100)}%` }
                  : undefined"
              />
            </div>
            <div class="semantic-progress-list">
              <div
                v-if="embeddingStatus?.processedDocs != null && embeddingStatus?.totalDocs != null"
                class="semantic-progress-row"
              >
                <span>{{ t("knowledge.retrieval.processedDocs") }}</span>
                <span>{{ semanticProcessedDocsLabel }}</span>
              </div>
              <div v-if="embeddingStatus?.currentFile" class="semantic-progress-row">
                <span>{{ t("settings.knowledge.currentFile") }}</span>
                <span class="truncate">{{ embeddingStatus.currentFile }}</span>
              </div>
              <div
                v-if="(embeddingStatus?.failedDocs ?? 0) > 0"
                class="semantic-progress-row semantic-progress-row-error"
              >
                <span>{{ t("knowledge.retrieval.failedDocs") }}</span>
                <span>{{ semanticFailedDocsLabel }}</span>
              </div>
            </div>
            <div v-if="semanticLastFailureLabel" class="semantic-progress-log">
              <div class="semantic-progress-log-label">
                {{ t("knowledge.retrieval.recentFailure") }}
              </div>
              <div class="semantic-progress-log-value">
                {{ semanticLastFailureLabel }}
              </div>
            </div>
          </div>

        </div>

        <div
          v-if="semanticEnabled || embeddingConfig?.embeddingMode !== 'remote'"
          class="settings-footer"
          :class="{ 'settings-footer-between': embeddingConfig?.embeddingMode !== 'remote' }"
        >
          <div v-if="embeddingConfig?.embeddingMode !== 'remote'" class="settings-footer-start">
            <BaseButton :disabled="pending" @click="openDownloadPopover">
              {{ t("knowledge.retrieval.downloadModelButton") }}
            </BaseButton>
          </div>
          <div v-if="semanticEnabled" class="settings-footer-end">
            <BaseButton :disabled="pending" @click="emit('rebuildSemantic')">
              {{ t("knowledge.dashboard.knowledge.rebuildIndex") }}
            </BaseButton>
          </div>
        </div>
      </section>
    </div>

    <Teleport to="body">
      <div v-if="downloadPopoverOpen" class="download-modal-overlay">
        <div class="download-modal">
          <div class="download-modal-header">
            <div class="download-modal-header-copy">
              <div class="download-popover-title">{{ t("knowledge.retrieval.downloadPopoverTitle") }}</div>
              <div class="download-popover-subtitle">{{ t("knowledge.retrieval.downloadPopoverHint") }}</div>
            </div>
            <button
              type="button"
              class="download-modal-close"
              :aria-label="t('common.close')"
              @click="closeDownloadPopover"
            >
              &times;
            </button>
          </div>

          <div class="download-modal-body">
            <div class="download-popover-section download-toolbar">
              <div class="download-toolbar-row">
                <div class="download-toolbar-group">
                  <div class="download-popover-label compact">
                    {{ t("knowledge.retrieval.downloadMode") }}
                  </div>
                  <BaseSegmented
                    size="sm"
                    :model-value="downloadMode"
                    :options="downloadModeOptions"
                    @update:model-value="handleDownloadModeUpdate"
                  />
                </div>
                <div class="download-toolbar-group">
                  <div class="download-popover-label compact">
                    {{ t("knowledge.retrieval.downloadSource") }}
                  </div>
                  <BaseSegmented
                    size="sm"
                    :model-value="normalizedDownloadSource"
                    :options="downloadSourceOptions"
                    @update:model-value="handleDownloadSourceUpdate"
                  />
                  <div class="download-toolbar-note">{{ downloadSourceHint }}</div>
                </div>
              </div>
            </div>

            <div
              v-if="downloadMode === 'preset'"
              class="download-workspace"
            >
              <section class="download-pane download-pane-list">
                <div class="download-preset-list" :aria-label="t('knowledge.retrieval.huggingFaceModel')">
                  <button
                    v-for="option in downloadPresetOptions"
                    :key="option.value"
                    type="button"
                    class="download-preset-item"
                    :class="{ active: option.value === downloadPresetId }"
                    :disabled="pending"
                    @click="handleDownloadPresetSelection(option.value)"
                  >
                    <span class="download-preset-name">{{ option.label }}</span>
                    <span v-if="option.costLine" class="download-preset-meta">{{ option.costLine }}</span>
                    <span v-if="option.capabilityLine" class="download-preset-summary">{{ option.capabilityLine }}</span>
                    <span class="download-preset-hint">{{ option.footerLine }}</span>
                  </button>
                </div>
              </section>

              <section class="download-pane download-pane-detail">
                <div class="download-pane-header">
                  <div class="download-detail-group">
                    <div class="download-popover-label compact">{{ t("knowledge.retrieval.huggingFaceModelId") }}</div>
                    <div class="download-detail-value is-mono">{{ selectedDownloadPreset?.id || "—" }}</div>
                  </div>
                </div>

                <div class="download-pane-body">
                  <div v-if="selectedDownloadPresetMeta" class="download-overview-grid">
                    <div class="download-overview-card">
                      <div class="download-popover-label compact">{{ t("knowledge.retrieval.presetCostOverview") }}</div>
                      <div class="download-detail-value strong">{{ selectedDownloadPresetBurden }}</div>
                      <div class="download-popover-note">
                        {{ t("knowledge.retrieval.presetMemoryEstimateLine", selectedDownloadPresetVramEstimate, selectedDownloadPresetRamEstimate) }}
                      </div>
                    </div>
                    <div class="download-overview-card">
                      <div class="download-popover-label compact">{{ t("knowledge.retrieval.presetCapabilityScope") }}</div>
                      <div class="download-popover-note download-capability-copy">
                        {{ selectedDownloadPresetMeta.capability }}
                      </div>
                    </div>
                  </div>

                  <div v-if="selectedDownloadPresetMeta" class="download-detail-grid">
                    <div class="download-detail-group">
                      <div class="download-popover-label compact">{{ t("knowledge.retrieval.presetParameters") }}</div>
                      <div class="download-detail-value">{{ selectedDownloadPresetMeta.parameters }}</div>
                    </div>
                    <div class="download-detail-group">
                      <div class="download-popover-label compact">{{ t("knowledge.retrieval.presetContextWindow") }}</div>
                      <div class="download-detail-value">{{ selectedDownloadPresetMeta.contextWindow }}</div>
                    </div>
                    <div class="download-detail-group">
                      <div class="download-popover-label compact">{{ t("knowledge.retrieval.presetDimensions") }}</div>
                      <div class="download-detail-value">{{ selectedDownloadPresetMeta.dimensions }}</div>
                    </div>
                    <div class="download-detail-group">
                      <div class="download-popover-label compact">{{ t("knowledge.retrieval.presetLicense") }}</div>
                      <div class="download-detail-value">{{ selectedDownloadPresetMeta.license }}</div>
                    </div>
                  </div>

                  <div class="download-detail-group">
                    <div class="download-popover-label compact">{{ t("knowledge.retrieval.downloadWindowStatus") }}</div>
                    <div class="download-popover-note">{{ downloadPresetStatus }}</div>
                  </div>

                  <div v-if="selectedDownloadPresetMeta" class="download-popover-note download-estimate-note">
                    {{ t("knowledge.retrieval.presetEstimateHint") }}
                  </div>
                </div>
              </section>
            </div>

            <div v-else class="download-custom-panel">
              <div class="download-pane download-pane-detail">
                <div class="download-pane-body">
                  <div class="download-input-group">
                    <span class="download-popover-label compact">{{ t("knowledge.retrieval.huggingFaceModelId") }}</span>
                    <input
                      v-model="downloadModelInput"
                      class="download-model-input"
                      :placeholder="t('knowledge.retrieval.huggingFaceModelIdPlaceholder')"
                      :disabled="pending"
                    >
                  </div>
                  <div class="download-popover-note">{{ downloadCustomStatus }}</div>
                </div>
              </div>
            </div>
          </div>

          <div class="download-modal-footer">
            <div class="download-modal-footer-copy">
              <div class="download-popover-label">{{ t("knowledge.retrieval.managedDirectory") }}</div>
              <div class="download-directory-note">{{ managedDirectoryPath }}</div>
            </div>
            <BaseButton
              :disabled="pending || !downloadTargetModelId"
              @click="handleDownloadPreset"
            >
              {{ t("knowledge.retrieval.downloadLocalModel") }}
            </BaseButton>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.retrieval-panel {
  flex: 1;
  padding: 16px 20px 20px;
  overflow: auto;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--bg-color) 6%);
}

.retrieval-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 14px;
}

.retrieval-header-main {
  min-width: 0;
}

.retrieval-title {
  font-size: 18px;
  line-height: 1.2;
  font-weight: 600;
  color: var(--text-color);
}

.retrieval-subtitle {
  margin-top: 4px;
  max-width: 760px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.retrieval-actions,
.settings-footer {
  display: flex;
  align-items: center;
  gap: 8px;
}

.retrieval-summary-grid,
.retrieval-settings-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.retrieval-settings-grid {
  margin-top: 12px;
}

.retrieval-card {
  min-width: 0;
  padding: 14px 16px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: var(--panel-bg);
}

.retrieval-card.is-error {
  border-color: color-mix(in srgb, var(--status-danger-border) 76%, var(--border-color) 24%);
  background: color-mix(in srgb, var(--status-danger-bg) 10%, var(--panel-bg) 90%);
}

.settings-card {
  display: flex;
  flex-direction: column;
}

.settings-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}

.settings-head,
.settings-body {
  min-width: 0;
}

.settings-body {
  margin-top: 12px;
  flex: 1;
}

.settings-footer {
  justify-content: flex-end;
  margin-top: 12px;
  padding-top: 12px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
}

.settings-footer-between {
  justify-content: space-between;
}

.settings-footer-start,
.settings-footer-end {
  display: flex;
  align-items: center;
  gap: 8px;
}

.control-title,
.card-title,
.download-popover-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.card-subtitle,
.control-meta,
.download-popover-subtitle,
.download-popover-note {
  margin-top: 4px;
  font-size: 12px;
  line-height: 1.55;
  color: var(--text-secondary);
}

.control-meta.is-error {
  color: color-mix(in srgb, var(--status-danger-fg) 82%, var(--text-secondary) 18%);
}

.hero-line {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin-top: 10px;
  margin-bottom: 12px;
}

.hero-line.compact {
  margin-bottom: 10px;
}

.hero-value {
  font-size: 26px;
  line-height: 1;
  font-weight: 700;
  color: var(--text-color);
}

.hero-label {
  font-size: 12px;
  color: var(--text-secondary);
}

.metric-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.metric-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  font-size: 12px;
  color: var(--text-secondary);
}

.metric-row span:last-child {
  color: var(--text-color);
  font-weight: 600;
}

.metric-row-status-error span:last-child {
  color: color-mix(in srgb, var(--status-danger-fg) 90%, var(--text-color) 10%);
}

.metric-row-control {
  align-items: flex-start;
}

.metric-row-input {
  width: min(260px, 100%);
  min-width: 0;
}

.metric-row-input :deep(.base-dropdown) {
  width: 100%;
}

.semantic-progress-panel {
  margin-top: 14px;
  padding: 12px;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 74%, var(--sidebar-bg) 26%);
}

.semantic-progress-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  font-size: 12px;
}

.semantic-progress-stage {
  color: var(--text-color);
  font-weight: 600;
}

.semantic-progress-value {
  color: var(--text-secondary);
  text-align: right;
}

.semantic-progress-track {
  position: relative;
  margin-top: 10px;
  height: 6px;
  overflow: hidden;
  border-radius: 999px;
  background: color-mix(in srgb, var(--sidebar-bg) 76%, var(--border-color) 24%);
}

.semantic-progress-fill {
  height: 100%;
  border-radius: inherit;
  background: color-mix(in srgb, var(--accent-color) 72%, white 28%);
  transition: width 0.18s ease;
}

.semantic-progress-track.indeterminate .semantic-progress-fill.indeterminate {
  width: 34%;
  animation: semantic-progress-indeterminate 1.25s ease-in-out infinite;
}

.semantic-progress-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 12px;
}

.semantic-progress-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  font-size: 12px;
  color: var(--text-secondary);
}

.semantic-progress-row span:last-child {
  color: var(--text-color);
  font-weight: 600;
  text-align: right;
}

.semantic-progress-row-error span:last-child {
  color: color-mix(in srgb, var(--status-danger-fg) 90%, var(--text-color) 10%);
}

.semantic-progress-log {
  margin-top: 12px;
  padding-top: 12px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
}

.semantic-progress-log-label {
  font-size: 11px;
  color: var(--text-secondary);
}

.semantic-progress-log-value {
  margin-top: 4px;
  font-size: 12px;
  line-height: 1.55;
  color: color-mix(in srgb, var(--status-danger-fg) 88%, var(--text-color) 12%);
  word-break: break-word;
}

.config-divider {
  margin: 14px 0 12px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
}

.config-grid,
.download-popover-section {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.download-toolbar {
  gap: 10px;
  padding-bottom: 12px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
}

.download-toolbar-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.download-toolbar-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.download-toolbar-note {
  max-width: 280px;
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-secondary);
}

.download-workspace,
.download-custom-panel {
  flex: 1;
  min-height: 0;
  margin-top: 12px;
}

.download-workspace {
  display: grid;
  grid-template-columns: minmax(260px, 320px) minmax(0, 1fr);
  gap: 14px;
}

.download-pane {
  min-width: 0;
  min-height: 0;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--sidebar-bg) 18%);
}

.download-pane-list,
.download-pane-detail {
  display: flex;
  flex-direction: column;
}

.download-pane-header,
.download-pane-body {
  min-width: 0;
}

.download-pane-header {
  padding: 12px 14px 0;
}

.download-pane-body {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 12px 14px 14px;
  min-height: 0;
  overflow: auto;
}

.download-preset-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-height: 0;
  padding: 12px;
  overflow: auto;
}

.download-preset-item {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 10px 12px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 76%, var(--hover-bg) 24%);
  color: var(--text-color);
  text-align: left;
  cursor: pointer;
  transition: border-color 0.15s ease, background 0.15s ease, color 0.15s ease;
}

.download-preset-item:hover:not(:disabled) {
  background: var(--hover-bg);
  border-color: color-mix(in srgb, var(--text-secondary) 42%, var(--border-color));
}

.download-preset-item.active {
  border-color: var(--accent-border);
  background: color-mix(in srgb, var(--accent-soft) 70%, var(--panel-bg) 30%);
}

.download-preset-item:disabled {
  opacity: 0.52;
  cursor: not-allowed;
}

.download-preset-name {
  font-size: 12px;
  line-height: 1.5;
  font-weight: 600;
  color: inherit;
  word-break: break-word;
}

.download-preset-summary,
.download-preset-meta,
.download-preset-hint {
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-secondary);
}

.download-preset-summary {
  color: color-mix(in srgb, var(--text-color) 82%, var(--text-secondary) 18%);
}

.download-detail-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.download-overview-grid,
.download-detail-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.download-overview-card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--sidebar-bg) 62%, var(--panel-bg) 38%);
}

.download-capability-copy {
  margin-top: 0;
  color: var(--text-color);
}

.download-estimate-note {
  padding-top: 10px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.download-detail-value {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-color);
  word-break: break-word;
}

.download-detail-value.strong {
  font-weight: 600;
}

.download-detail-value.is-mono {
  font-family: var(--font-mono-identifier);
}

.config-row {
  display: grid;
  grid-template-columns: 96px minmax(0, 1fr);
  gap: 12px;
  align-items: flex-start;
}

.config-label,
.download-popover-label {
  padding-top: 7px;
  font-size: 12px;
  color: var(--text-secondary);
}

.download-popover-label,
.download-popover-label.compact {
  padding-top: 0;
}

.config-note,
.config-value,
.download-popover-note,
.download-directory-note {
  font-size: 12px;
  line-height: 1.6;
}

.config-note,
.download-popover-note,
.download-directory-note {
  color: var(--text-secondary);
}

.config-value {
  color: var(--text-color);
}

.truncate {
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.retrieval-loading {
  margin-bottom: 12px;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--border-color) 70%, transparent);
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: flex-start;
  color: var(--text-secondary);
  font-size: 12px;
}

.retrieval-card.is-error .settings-footer,
.retrieval-card.is-error .config-divider,
.retrieval-card.is-error .download-toolbar,
.retrieval-card.is-error .download-modal-footer {
  border-color: color-mix(in srgb, var(--status-danger-border) 62%, transparent);
}

.download-modal-overlay {
  position: fixed;
  inset: 0;
  z-index: 220;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background: color-mix(in srgb, var(--bg-color) 26%, transparent);
  backdrop-filter: blur(4px);
}

.download-modal {
  width: min(880px, calc(100vw - 48px));
  max-height: min(84vh, 760px);
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: var(--sidebar-bg);
  overflow: hidden;
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.28);
}

.download-modal-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
  border-bottom: 1px solid var(--border-color);
}

.download-modal-header-copy,
.download-modal-body {
  min-width: 0;
}

.download-modal-body {
  display: flex;
  flex-direction: column;
  flex: 1;
  gap: 0;
  min-height: 0;
  padding: 14px;
  overflow: hidden;
}

.download-modal-close {
  flex-shrink: 0;
  background: none;
  border: none;
  color: var(--text-secondary);
  font-size: 18px;
  line-height: 1;
  cursor: pointer;
  padding: 0 4px;
}

.download-modal-close:hover {
  color: var(--text-color);
}

.download-modal-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  padding: 12px 14px 14px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  background: color-mix(in srgb, var(--sidebar-bg) 88%, var(--panel-bg) 12%);
}

.download-modal-footer-copy {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.download-custom-panel {
  display: flex;
  min-width: 0;
}

.download-custom-panel .download-pane {
  flex: 1;
}

.download-input-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.download-model-input {
  width: 100%;
  min-width: 0;
  height: 32px;
  padding: 0 10px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--hover-bg) 28%);
  color: var(--text-color);
  font-size: 12px;
  font-family: var(--font-mono-identifier);
  outline: none;
  transition: border-color 0.15s ease, box-shadow 0.15s ease, color 0.15s ease;
}

.download-model-input:hover:not(:disabled) {
  border-color: color-mix(in srgb, var(--text-secondary) 48%, var(--border-color));
}

.download-model-input:focus {
  border-color: var(--accent-color);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 14%, transparent);
}

.download-model-input:disabled {
  opacity: 0.52;
  cursor: not-allowed;
}

.download-model-input::placeholder {
  color: var(--text-secondary);
  opacity: 0.72;
}

.download-directory-note {
  font-size: 11px;
  line-height: 1.55;
  font-family: var(--font-mono-identifier);
  word-break: break-all;
}

@keyframes semantic-progress-indeterminate {
  0% {
    transform: translateX(-120%);
  }

  100% {
    transform: translateX(320%);
  }
}

@media (max-width: 1180px) {
  .retrieval-summary-grid,
  .retrieval-settings-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}

@media (max-width: 720px) {
  .retrieval-header {
    flex-direction: column;
    align-items: stretch;
  }

  .metric-row-control {
    flex-direction: column;
    align-items: stretch;
  }

  .metric-row-input {
    width: 100%;
  }

  .semantic-progress-head,
  .semantic-progress-row {
    flex-direction: column;
    align-items: stretch;
  }

  .semantic-progress-value,
  .semantic-progress-row span:last-child {
    text-align: left;
  }

  .config-row {
    grid-template-columns: minmax(0, 1fr);
    gap: 6px;
  }

  .config-label {
    padding-top: 0;
  }

  .truncate {
    max-width: none;
  }

  .download-modal {
    width: calc(100vw - 24px);
    max-height: calc(100vh - 24px);
  }

  .download-workspace,
  .download-overview-grid,
  .download-detail-grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .download-modal-footer {
    align-items: stretch;
    flex-direction: column;
  }
}
</style>
