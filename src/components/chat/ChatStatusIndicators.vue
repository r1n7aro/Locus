<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { BookOpen, Box, CodeXml, Database, type IconNode } from "lucide";
import { t } from "../../i18n";
import { listAgentInjectedItems } from "../../services/agent";
import {
  csharpLspGetStatus,
  csharpLspRestart,
  csharpLspSetEnabled,
  subscribeCsharpLspStatus,
} from "../../services/csharpLsp";
import { normalizeAppError } from "../../services/errors";
import type { RuntimeUnsubscribe } from "../../services/locusRuntime";
import {
  knowledgeGetEmbeddingStatus,
  knowledgeGetLexicalRebuildStatus,
  knowledgeGetOverview,
} from "../../services/knowledge";
import type {
  AssetDbScanEvent,
  CsharpLspStatus,
  EmbeddingStatus,
  InjectedPromptItem,
  KnowledgeAccessMode,
  KnowledgeRetrievalOverview,
  LexicalRebuildStatus,
  ScanStats,
  UnityConnectionStatus,
  UnityEditorProcessState,
} from "../../types";
import BaseButton from "../ui/BaseButton.vue";
import BaseSegmented, { type SegmentedOption } from "../ui/BaseSegmented.vue";
import { estimateKnowledgeContextCostTokens } from "./knowledgeContextCost";

type StatusId = "assetDb" | "unity" | "knowledge" | "code";
type StatusTone = "success" | "danger" | "accent" | "muted";
type StatusIcon = "database" | "unity" | "knowledge" | "code";
type UnityPluginNotice = "missing" | "outdated";
type UnityLaunchState = "idle" | "starting" | "waitingConnection";

interface StatusDetailRow {
  label: string;
  value: string;
  mono?: boolean;
}

interface StatusItem {
  id: StatusId;
  icon: StatusIcon;
  title: string;
  summary: string;
  inlineLabel: string;
  tone: StatusTone;
  rows: StatusDetailRow[];
  modeOptions?: SegmentedOption[];
  actionLabel?: string;
  actionTitle?: string;
  actionDisabled?: boolean;
  actionVariant?: "neutral" | "primary" | "danger";
}

const STATUS_ICONS: Record<StatusIcon, IconNode> = {
  database: Database,
  unity: Box,
  knowledge: BookOpen,
  code: CodeXml,
};

const props = defineProps<{
  unityConnected?: boolean;
  unityPluginStatus?: UnityPluginNotice | null;
  unityPluginInstalling?: boolean;
  unityLaunching?: boolean;
  unityLaunchState?: UnityLaunchState;
  unityConnectionStatus?: UnityConnectionStatus | null;
  unityRecompiling?: boolean;
  workingDir?: string;
  isUnityProject?: boolean;
  scanPhase?: AssetDbScanEvent | null;
  lastScanStats?: ScanStats | null;
  knowledgeAccessMode?: KnowledgeAccessMode;
  selectedAgentId?: string;
}>();

const emit = defineEmits<{
  startScan: [];
  installPlugin: [];
  launchUnityProject: [];
  updateKnowledgeAccessMode: [mode: KnowledgeAccessMode];
}>();

const activePopover = ref<StatusId | null>(null);
const csharpLsp = ref<CsharpLspStatus | null>(null);
const csharpLspPending = ref(false);
let csharpLspUnsubscribe: RuntimeUnsubscribe | null = null;
let csharpLspDisposed = false;
const knowledgeOverview = ref<KnowledgeRetrievalOverview | null>(null);
const lexicalRebuildStatus = ref<LexicalRebuildStatus | null>(null);
const embeddingStatus = ref<EmbeddingStatus | null>(null);
const injectedItems = ref<InjectedPromptItem[]>([]);
const knowledgeStatusLoading = ref(false);
const knowledgeRetrievalError = ref("");
const knowledgeContextError = ref("");
let knowledgeStatusSeq = 0;

function isAssetDbRunningPhase(phase: AssetDbScanEvent | null | undefined): boolean {
  return phase != null
    && phase.phase !== "done"
    && phase.phase !== "reconcileDone"
    && phase.phase !== "error";
}

const isScanning = computed(() => {
  return isAssetDbRunningPhase(props.scanPhase);
});

const scanError = computed(() => {
  const p = props.scanPhase;
  return p != null && p.phase === "error" ? p.error : null;
});

const scanLabel = computed(() => {
  const p = props.scanPhase;
  if (!p) return "";
  switch (p.phase) {
    case "dirScan": return t("chat.assetDb.scanning.dirScan");
    case "metaParse": return t("chat.assetDb.scanning.metaParse", p.completed, p.total);
    case "yamlParse": return t("chat.assetDb.scanning.yamlParse", p.completed, p.total);
    case "dbWrite": return t("chat.assetDb.scanning.dbWrite");
    case "reconcile": return reconcileScanLabel(p);
    case "reconcileDone": return "";
    case "done": return "";
    case "error": return t("chat.assetDb.scanning.error", p.error.message);
  }
});

const scanSummary = computed(() => {
  const s = props.lastScanStats;
  if (!s) return "";
  return t("chat.assetDb.summary", s.nodesAdded, s.edgesAdded);
});

const unityWorkingDir = computed(() => props.workingDir?.trim() ?? "");

function stripExtendedPathPrefix(path: string) {
  return path.startsWith("\\\\?\\") ? path.slice(4) : path;
}

function unityPipeNameForWorkingDir(workingDir: string) {
  const normalized = stripExtendedPathPrefix(workingDir).trim();
  if (!normalized) return "";
  const sanitized = normalized.replace(/[\\/: ]/g, "_");
  return `\\\\.\\pipe\\locus_unity_${sanitized}`;
}

const unityPipeName = computed(() =>
  props.unityConnectionStatus?.pipeName || unityPipeNameForWorkingDir(unityWorkingDir.value),
);

const unityEditorStatus = computed(() =>
  props.unityConnectionStatus?.editorStatus || (props.unityConnected ? "editing" : "disconnected"),
);

const unityEditorProcessState = computed<UnityEditorProcessState>(() =>
  props.unityConnectionStatus?.editorProcessState ?? (props.unityConnected ? "running" : "unknown"),
);

function unityEditorStatusLabel(status: string) {
  const normalized = status || "disconnected";
  const key = `chat.toolConfirm.unityStatus.status.${normalized}`;
  const label = t(key);
  return label === key ? normalized : label;
}

function unityEditorProcessStateLabel(status: string | null | undefined) {
  const normalized = status || "unknown";
  const key = `chat.status.unity.processState.${normalized}`;
  const label = t(key);
  return label === key ? normalized : label;
}

function unityBackgroundHookLabel(status: UnityConnectionStatus["backgroundHook"] | null | undefined) {
  const normalized = status?.state || "inactive";
  const key = `chat.status.unity.backgroundHook.${normalized}`;
  const label = t(key);
  return label === key ? normalized : label;
}

function formatTimestamp(ms: number | null | undefined) {
  if (!Number.isFinite(ms ?? Number.NaN) || !ms) return "";
  return new Date(ms).toLocaleTimeString();
}

const countFormatter = new Intl.NumberFormat("zh-CN");

function formatCount(value: number): string {
  return countFormatter.format(Math.max(0, Math.round(value)));
}

function formatPercent(value: number | null | undefined): string {
  if (typeof value !== "number" || !Number.isFinite(value)) return "0%";
  return `${Math.round(Math.min(1, Math.max(0, value)) * 100)}%`;
}

function isFiniteCount(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function formatProgressCount(completed: number, total: number): string {
  return `${formatCount(completed)} / ${formatCount(total)}`;
}

function reconcileStageLabel(stage: string | null | undefined): string {
  switch (stage) {
    case "scanning": return t("chat.status.assetDb.reconcileStage.scanning");
    case "discovering": return t("chat.status.assetDb.reconcileStage.discovering");
    case "processing": return t("chat.status.assetDb.reconcileStage.processing");
    default: return stage || t("asset.db.scanPhase.reconcile");
  }
}

function reconcileProgressRatio(phase: Extract<AssetDbScanEvent, { phase: "reconcile" }>): number | null {
  if (!isFiniteCount(phase.completed) || !isFiniteCount(phase.total) || phase.total <= 0) {
    return null;
  }
  return Math.min(1, Math.max(0, phase.completed / phase.total));
}

function reconcileProgressText(phase: Extract<AssetDbScanEvent, { phase: "reconcile" }>): string {
  if (!isFiniteCount(phase.completed) || !isFiniteCount(phase.total) || phase.total <= 0) return "";
  const ratio = reconcileProgressRatio(phase);
  const percent = ratio == null ? "" : `${formatPercent(ratio)} · `;
  return `${percent}${formatProgressCount(phase.completed, phase.total)}`;
}

function reconcileScanLabel(phase: Extract<AssetDbScanEvent, { phase: "reconcile" }>): string {
  const count = reconcileProgressText(phase);
  switch (phase.stage) {
    case "scanning":
      return count
        ? t("chat.assetDb.scanning.reconcile.scanning", count)
        : t("chat.assetDb.scanning.reconcile.scanningUnknown");
    case "discovering":
      return isFiniteCount(phase.queued)
        ? t("chat.assetDb.scanning.reconcile.discovering", formatCount(phase.queued))
        : t("chat.assetDb.scanning.reconcile.discoveringUnknown");
    case "processing":
      return count
        ? t("chat.assetDb.scanning.reconcile.processing", count)
        : t("chat.assetDb.scanning.reconcile.processingUnknown");
    default:
      return t("chat.assetDb.scanning.reconcile");
  }
}

function lexicalStageLabel(stage: string | null | undefined): string {
  switch (stage) {
    case "preparing": return t("knowledge.dashboard.knowledge.stagePreparing");
    case "cleaning": return t("knowledge.dashboard.knowledge.stageCleaning");
    case "indexing": return t("knowledge.dashboard.knowledge.stageIndexing");
    case "committing": return t("knowledge.dashboard.knowledge.stageCommitting");
    case "completed": return t("knowledge.dashboard.knowledge.stageCompleted");
    case "downloading_model": return t("settings.knowledge.stage.downloadingModel");
    case "cancelling": return t("settings.knowledge.stage.cancelling");
    case "cancelled": return t("settings.knowledge.stage.cancelled");
    case "initializing_runtime": return t("settings.knowledge.stage.initializingRuntime");
    case "ready": return t("settings.knowledge.stage.ready");
    case "error": return t("settings.knowledge.stage.error");
    default: return stage || t("knowledge.dashboard.knowledge.stageIdle");
  }
}

function semanticStageLabel(stage: string | null | undefined): string {
  if (stage === "committing") return t("knowledge.dashboard.knowledge.stagePersistingEmbeddings");
  return lexicalStageLabel(stage);
}

const unityPluginLabel = computed(() => {
  if (props.unityPluginStatus === "missing") return t("app.plugin.notInstalled");
  if (props.unityPluginStatus === "outdated") return t("app.plugin.needUpdate");
  return "";
});

const effectiveUnityLaunchState = computed<UnityLaunchState>(() => {
  if (props.unityConnected) return "idle";
  if (props.unityLaunchState && props.unityLaunchState !== "idle") {
    return props.unityLaunchState;
  }
  return props.unityLaunching ? "starting" : "idle";
});

const unityRecompileWaitingConnection = computed(() =>
  !!props.unityRecompiling
  && !props.unityConnected
  && !props.unityPluginStatus,
);

const unityRecompileProcessStable = computed(() =>
  unityRecompileWaitingConnection.value
  && unityEditorProcessState.value === "running",
);

const unitySummary = computed(() => {
  if (unityPluginLabel.value) return unityPluginLabel.value;
  if (unityRecompileWaitingConnection.value) return t("chat.unity.waitingRecompileConnection");
  if (effectiveUnityLaunchState.value === "starting") return t("chat.unity.launching");
  if (effectiveUnityLaunchState.value === "waitingConnection") return t("chat.unity.waitingConnection");
  if (!props.unityConnected && unityEditorProcessState.value === "running") {
    return t("chat.unity.runningDisconnected");
  }
  return props.unityConnected ? t("chat.unity.connected") : t("chat.unity.disconnected");
});

const unityTone = computed<StatusTone>(() =>
  props.unityPluginStatus
    ? "danger"
    : props.unityConnected || unityRecompileProcessStable.value
      ? "success"
      : unityEditorProcessState.value === "running"
        || unityRecompileWaitingConnection.value
        || effectiveUnityLaunchState.value !== "idle"
        ? "accent"
        : "danger",
);

const unityCanLaunch = computed(() =>
  !!props.isUnityProject
  && !props.unityConnected
  && !props.unityPluginStatus
  && !unityRecompileWaitingConnection.value
  && unityEditorProcessState.value !== "running",
);

const unityActionLabel = computed(() => {
  if (props.unityPluginStatus) {
    if (props.unityPluginInstalling) return t("app.plugin.installing");
    return props.unityPluginStatus === "missing"
      ? t("app.plugin.clickInstall")
      : t("app.plugin.clickUpdate");
  }
  if (!unityCanLaunch.value) return "";
  if (effectiveUnityLaunchState.value === "starting") return t("chat.status.unity.launching");
  if (effectiveUnityLaunchState.value === "waitingConnection") {
    return t("chat.status.unity.waitingConnection");
  }
  return t("chat.status.unity.launch");
});

const unityActionTitle = computed(() => {
  if (props.unityPluginStatus) return unityActionLabel.value;
  if (effectiveUnityLaunchState.value === "starting") return t("chat.status.unity.launchingTitle");
  if (effectiveUnityLaunchState.value === "waitingConnection") {
    return t("chat.status.unity.waitingConnectionTitle");
  }
  if (unityCanLaunch.value) return t("chat.status.unity.launchTitle");
  return "";
});

const assetStatusLabel = computed(() => {
  if (isScanning.value) return scanLabel.value;
  if (scanError.value) return scanError.value.message;
  if (scanSummary.value) return t("chat.assetDb.ready");
  return props.isUnityProject ? t("chat.assetDb.notBuilt") : t("chat.status.assetDb.noWorkspace");
});

const assetTone = computed<StatusTone>(() => {
  if (scanError.value) return "danger";
  if (isScanning.value) return "accent";
  if (scanSummary.value) return "success";
  return props.isUnityProject ? "danger" : "muted";
});

const assetActionLabel = computed(() => {
  if (isScanning.value) return "";
  if (scanError.value) return t("chat.assetDb.retry");
  if (scanSummary.value) return t("chat.assetDb.rescan");
  return t("chat.assetDb.scan");
});

const assetActionTitle = computed(() =>
  scanSummary.value ? t("chat.assetDb.reScanTitle") : t("chat.assetDb.buildTitle"),
);

function formatElapsed(ms: number) {
  if (!Number.isFinite(ms) || ms < 0) return "-";
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)} s`;
}

function scanProgressRow(phase: AssetDbScanEvent | null | undefined): StatusDetailRow | null {
  if (!phase) return null;
  if (phase.phase === "reconcile") {
    const value = reconcileProgressText(phase);
    return value ? { label: t("chat.status.assetDb.progress"), value } : null;
  }
  if (phase.phase !== "metaParse" && phase.phase !== "yamlParse") return null;
  return {
    label: t("chat.status.assetDb.progress"),
    value: formatProgressCount(phase.completed, phase.total),
  };
}

const assetRows = computed<StatusDetailRow[]>(() => {
  const rows: StatusDetailRow[] = [];

  const progress = scanProgressRow(props.scanPhase);
  if (progress) rows.push(progress);

  if (props.scanPhase?.phase === "reconcile") {
    const phase = props.scanPhase;
    rows.push({
      label: t("chat.status.assetDb.stage"),
      value: reconcileStageLabel(phase.stage),
    });
    rows.push({
      label: t("chat.status.assetDb.reconcileMode"),
      value: phase.verifyHashes
        ? t("chat.status.assetDb.reconcileModeHash")
        : t("chat.status.assetDb.reconcileModeMtime"),
    });
    if (isFiniteCount(phase.queued)) {
      rows.push({
        label: t("chat.status.assetDb.queued"),
        value: formatCount(phase.queued),
      });
    }
    if (isFiniteCount(phase.failed) && phase.failed > 0) {
      rows.push({
        label: t("chat.status.assetDb.failed"),
        value: formatCount(phase.failed),
      });
    }
  }

  if (scanError.value) {
    rows.push({ label: t("chat.status.detail.code"), value: scanError.value.code });
    if (scanError.value.detail) {
      rows.push({ label: t("chat.status.detail.detail"), value: scanError.value.detail });
    }
  }

  const stats = props.lastScanStats;
  if (stats) {
    rows.push(
      { label: t("chat.status.assetDb.assets"), value: String(stats.nodesAdded) },
      { label: t("chat.status.assetDb.references"), value: String(stats.edgesAdded) },
      { label: t("chat.status.assetDb.metaFiles"), value: String(stats.metaFilesFound) },
      { label: t("chat.status.assetDb.yamlAssets"), value: String(stats.yamlAssetsFound) },
      { label: t("chat.status.assetDb.parseFailures"), value: String(stats.parseFailures) },
      { label: t("chat.status.assetDb.elapsed"), value: formatElapsed(stats.elapsedMs) },
    );
  }

  return rows;
});

const unityRows = computed<StatusDetailRow[]>(() => {
  const rows: StatusDetailRow[] = [];
  const status = props.unityConnectionStatus ?? null;

  rows.push({
    label: t("chat.status.detail.status"),
    value: unityEditorStatusLabel(unityEditorStatus.value),
  });

  rows.push({
    label: t("chat.status.unity.process"),
    value: unityEditorProcessStateLabel(unityEditorProcessState.value),
  });

  if (typeof status?.editorProcessId === "number") {
    rows.push({
      label: t("chat.status.unity.processId"),
      value: String(status.editorProcessId),
    });
  }

  if (status?.editorProjectPath) {
    rows.push({
      label: t("chat.status.unity.editorProjectPath"),
      value: status.editorProjectPath,
      mono: true,
    });
  }

  if (status?.editorProcessPath) {
    rows.push({
      label: t("chat.status.unity.editorProcessPath"),
      value: status.editorProcessPath,
      mono: true,
    });
  }

  if (status?.scenePath) {
    rows.push({
      label: t("chat.status.unity.scene"),
      value: status.scenePath,
      mono: true,
    });
  }

  if (typeof status?.latencyMs === "number") {
    rows.push({
      label: t("chat.status.unity.latency"),
      value: formatElapsed(status.latencyMs),
    });
  }

  if (status?.backgroundHook) {
    rows.push({
      label: t("chat.status.unity.backgroundHook"),
      value: unityBackgroundHookLabel(status.backgroundHook),
    });
    if (status.backgroundHook.error) {
      rows.push({
        label: t("chat.status.unity.backgroundHookError"),
        value: status.backgroundHook.error,
        mono: true,
      });
    }
  }

  if (status?.checkedAtMs) {
    const checkedAt = formatTimestamp(status.checkedAtMs);
    if (checkedAt) {
      rows.push({
        label: t("chat.status.unity.checkedAt"),
        value: checkedAt,
      });
    }
  }

  if (status?.processCheckedAtMs) {
    const checkedAt = formatTimestamp(status.processCheckedAtMs);
    if (checkedAt) {
      rows.push({
        label: t("chat.status.unity.processCheckedAt"),
        value: checkedAt,
      });
    }
  }

  if (!props.unityConnected && (status?.reconnectAttempts ?? 0) > 0) {
    rows.push({
      label: t("chat.status.unity.reconnectAttempts"),
      value: String(status?.reconnectAttempts ?? 0),
    });
  }

  if (status?.lastError) {
    rows.push({
      label: t("chat.status.unity.lastError"),
      value: status.lastError,
      mono: true,
    });
  }

  if (status?.processLastError) {
    rows.push({
      label: t("chat.status.unity.processLastError"),
      value: status.processLastError,
      mono: true,
    });
  }

  if (unityPipeName.value) {
    rows.push({
      label: t("chat.status.unity.pipe"),
      value: unityPipeName.value,
      mono: true,
    });
  }
  if (unityWorkingDir.value) {
    rows.push({
      label: t("chat.status.unity.workingDir"),
      value: unityWorkingDir.value,
      mono: true,
    });
  }
  return rows;
});

const knowledgeMode = computed<KnowledgeAccessMode>(() => props.knowledgeAccessMode ?? "full");

const knowledgeHasWorkspace = computed(() => !!props.workingDir?.trim());

const knowledgeModeOptions = computed<SegmentedOption[]>(() => [
  {
    value: "disabled",
    label: t("chat.status.knowledge.mode.disabled"),
    hint: t("chat.status.knowledge.mode.disabledHint"),
  },
  {
    value: "read_only",
    label: t("chat.status.knowledge.mode.readOnly"),
    hint: t("chat.status.knowledge.mode.readOnlyHint"),
  },
  {
    value: "full",
    label: t("chat.status.knowledge.mode.full"),
    hint: t("chat.status.knowledge.mode.fullHint"),
  },
]);

const knowledgeModeSummary = computed(() => {
  if (!knowledgeHasWorkspace.value) return t("chat.status.knowledge.noWorkspace");
  if (knowledgeMode.value === "disabled") return t("chat.status.knowledge.disabled");
  if (knowledgeMode.value === "read_only") return t("chat.status.knowledge.readOnly");
  return t("chat.status.knowledge.full");
});

const knowledgeTone = computed<StatusTone>(() => {
  if (!knowledgeHasWorkspace.value || knowledgeMode.value === "disabled") return "muted";
  return knowledgeMode.value === "read_only" ? "accent" : "success";
});

const knowledgeAgentId = computed(() => props.selectedAgentId?.trim() ?? "");

const knowledgeContextEstimatedTokens = computed(() => {
  if (knowledgeMode.value === "disabled") return 0;
  return estimateKnowledgeContextCostTokens(injectedItems.value);
});

const knowledgeContextCostLabel = computed(() => {
  if (!knowledgeHasWorkspace.value) return t("chat.status.knowledge.noWorkspace");
  if (knowledgeMode.value === "disabled") return t("chat.status.knowledge.contextCostZero");
  if (knowledgeStatusLoading.value && injectedItems.value.length === 0) {
    return t("chat.status.knowledge.loading");
  }
  return t(
    "chat.status.knowledge.contextCostTokens",
    formatCount(knowledgeContextEstimatedTokens.value),
  );
});

const lexicalRetrievalLabel = computed(() => {
  if (!knowledgeHasWorkspace.value) return t("chat.status.knowledge.noWorkspace");
  if (knowledgeMode.value === "disabled") return t("chat.status.knowledge.requestOff");
  if (knowledgeStatusLoading.value && !knowledgeOverview.value && !lexicalRebuildStatus.value) {
    return t("chat.status.knowledge.loading");
  }
  if (lexicalRebuildStatus.value?.error) return lexicalRebuildStatus.value.error;
  if (lexicalRebuildStatus.value?.running) {
    const progress = typeof lexicalRebuildStatus.value.progress === "number"
      ? `${formatPercent(lexicalRebuildStatus.value.progress)} · `
      : "";
    return `${progress}${lexicalStageLabel(lexicalRebuildStatus.value.stage)}`;
  }

  const fullText = knowledgeOverview.value?.fullText;
  if (!fullText?.enabled) return t("chat.status.knowledge.off");
  if (fullText.pendingItemCount > 0 || fullText.staleItemCount > 0) {
    return t(
      "chat.status.knowledge.indexPending",
      formatCount(fullText.pendingItemCount + fullText.staleItemCount),
    );
  }
  return t(
    "chat.status.knowledge.indexReady",
    formatCount(fullText.indexedItemCount),
    formatCount(fullText.indexableItemCount),
  );
});

const semanticRetrievalLabel = computed(() => {
  if (!knowledgeHasWorkspace.value) return t("chat.status.knowledge.noWorkspace");
  if (knowledgeMode.value === "disabled") return t("chat.status.knowledge.requestOff");
  if (knowledgeStatusLoading.value && !knowledgeOverview.value && !embeddingStatus.value) {
    return t("chat.status.knowledge.loading");
  }
  if (embeddingStatus.value?.error) return embeddingStatus.value.error;
  if (knowledgeOverview.value?.semantic.error) return knowledgeOverview.value.semantic.error;
  if (embeddingStatus.value?.activating || embeddingStatus.value?.stage === "indexing") {
    if (embeddingStatus.value.indexProgress != null) {
      return `${formatPercent(embeddingStatus.value.indexProgress)} · ${semanticStageLabel(embeddingStatus.value.stage)}`;
    }
    return semanticStageLabel(embeddingStatus.value.stage);
  }

  const semantic = knowledgeOverview.value?.semantic;
  if (!semantic?.enabled) return t("chat.status.knowledge.off");
  if (!semantic.ready || !embeddingStatus.value?.ready) {
    return semanticStageLabel(embeddingStatus.value?.stage || semantic.stage);
  }
  if (semantic.pendingItemCount > 0) {
    return t("chat.status.knowledge.indexPending", formatCount(semantic.pendingItemCount));
  }
  return t("chat.status.knowledge.semanticReady", formatPercent(semantic.coverageRatio));
});

const knowledgeRows = computed<StatusDetailRow[]>(() => {
  return [
    {
      label: t("chat.status.knowledge.lexicalRetrieval"),
      value: knowledgeRetrievalError.value || lexicalRetrievalLabel.value,
    },
    {
      label: t("chat.status.knowledge.semanticRetrieval"),
      value: knowledgeRetrievalError.value || semanticRetrievalLabel.value,
    },
    {
      label: t("chat.status.knowledge.contextCost"),
      value: knowledgeContextError.value || knowledgeContextCostLabel.value,
    },
  ];
});

// Until the first status fetch succeeds, assume the platform is supported so
// the toggle stays reachable; a failing toggle then surfaces its error below.
const codeSupported = computed(() =>
  csharpLsp.value ? csharpLsp.value.supported : true,
);
const codeEnabled = computed(() => (csharpLsp.value?.enabled ?? false) && codeSupported.value);
const csharpLspActionError = ref("");
const codeTransitional = computed(() =>
  ["preparing", "downloading", "starting", "loading"].includes(csharpLsp.value?.phase ?? ""),
);

function codeDownloadText(status: CsharpLspStatus): string {
  const component = status.downloadComponent === "dotnet" ? ".NET" : "Roslyn";
  const received = status.downloadReceived ?? 0;
  const total = status.downloadTotal ?? 0;
  if (total > 0) {
    return `${component} ${Math.round((received / total) * 100)}%`;
  }
  return `${component} ${(received / 1024 / 1024).toFixed(1)} MB`;
}

const codeSummary = computed(() => {
  const status = csharpLsp.value;
  if (!status) return t("chat.status.code.off");
  if (!status.supported) return t("chat.status.code.unsupported");
  switch (status.phase) {
    case "disabled": return t("chat.status.code.off");
    case "idle": return t("chat.status.code.idle");
    case "preparing": return t("chat.status.code.preparing");
    case "downloading": return t("chat.status.code.downloading", codeDownloadText(status));
    case "starting": return t("chat.status.code.starting");
    case "loading": {
      const total = status.projectCount;
      const done = status.loadedProjects;
      if (typeof done === "number" && typeof total === "number" && total > 0) {
        return t("chat.status.code.loadingProgress", formatCount(done), formatCount(total));
      }
      return t("chat.status.code.loading");
    }
    case "ready": return t("chat.status.code.ready");
    case "error": return t("chat.status.code.error");
    default: return t("chat.status.code.off");
  }
});

const codeTone = computed<StatusTone>(() => {
  const status = csharpLsp.value;
  if (!status || !status.supported || !status.enabled) return "muted";
  switch (status.phase) {
    case "ready": return "success";
    case "error": return "danger";
    case "idle": return "muted";
    default: return "accent";
  }
});

function formatUptime(totalSecs: number): string {
  if (totalSecs < 60) return `${totalSecs}s`;
  const minutes = Math.floor(totalSecs / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

const codeRows = computed<StatusDetailRow[]>(() => {
  const rows: StatusDetailRow[] = [];
  if (csharpLspActionError.value) {
    rows.push({
      label: t("chat.status.code.rows.error"),
      value: csharpLspActionError.value,
      mono: true,
    });
  }
  const status = csharpLsp.value;
  if (!status || !status.supported || !status.enabled) return rows;
  rows.push({ label: t("chat.status.code.rows.state"), value: codeSummary.value });
  if (status.projectFile) {
    rows.push({
      label: t("chat.status.code.rows.project"),
      value: status.projectFile,
      mono: true,
    });
  }
  if (typeof status.projectCount === "number") {
    rows.push({
      label: t("chat.status.code.rows.projects"),
      value: formatCount(status.projectCount),
    });
  }
  if (typeof status.openDocuments === "number") {
    rows.push({
      label: t("chat.status.code.rows.openDocs"),
      value: formatCount(status.openDocuments),
    });
  }
  const references = status.queryReferences ?? 0;
  const definitions = status.queryDefinitions ?? 0;
  const symbols = status.querySymbols ?? 0;
  rows.push({
    label: t("chat.status.code.rows.queries"),
    value: t(
      "chat.status.code.queries.detail",
      formatCount(references + definitions + symbols),
      formatCount(references),
      formatCount(definitions),
      formatCount(symbols),
    ),
  });
  if (typeof status.uptimeSecs === "number") {
    rows.push({
      label: t("chat.status.code.rows.uptime"),
      value: formatUptime(status.uptimeSecs),
    });
  }
  rows.push({
    label: t("chat.status.code.rows.server"),
    value: status.serverVersion,
    mono: true,
  });
  if (status.dotnetSource) {
    rows.push({
      label: t("chat.status.code.rows.dotnet"),
      value: status.dotnetSource === "system"
        ? t("chat.status.code.rows.dotnet.system")
        : t("chat.status.code.rows.dotnet.managed"),
    });
  }
  if (status.phase === "error" && status.message) {
    rows.push({
      label: t("chat.status.code.rows.error"),
      value: status.message,
      mono: true,
    });
  }
  return rows;
});

const codeModeOptions = computed<SegmentedOption[]>(() => [
  {
    value: "off",
    label: t("chat.status.code.mode.off"),
    hint: t("chat.status.code.mode.offHint"),
  },
  {
    value: "on",
    label: t("chat.status.code.mode.on"),
    hint: t("chat.status.code.mode.onHint"),
  },
]);

async function refreshCsharpLspStatus() {
  try {
    csharpLsp.value = await csharpLspGetStatus();
  } catch {
    // status stays stale; the event stream will refresh it
  }
}

async function setCodeEnabled(value: boolean) {
  if (csharpLspPending.value) return;
  if (csharpLsp.value && csharpLsp.value.enabled === value) return;
  csharpLspPending.value = true;
  try {
    csharpLsp.value = await csharpLspSetEnabled(value);
    csharpLspActionError.value = "";
  } catch (error) {
    csharpLspActionError.value = normalizeAppError(error).message;
    await refreshCsharpLspStatus();
  } finally {
    csharpLspPending.value = false;
  }
}

async function restartCode() {
  if (csharpLspPending.value) return;
  csharpLspPending.value = true;
  try {
    csharpLsp.value = await csharpLspRestart();
    csharpLspActionError.value = "";
  } catch (error) {
    csharpLspActionError.value = normalizeAppError(error).message;
    await refreshCsharpLspStatus();
  } finally {
    csharpLspPending.value = false;
  }
}

const statusItems = computed<StatusItem[]>(() => [
  {
    id: "assetDb",
    icon: "database",
    title: t("chat.status.assetDb.title"),
    summary: assetStatusLabel.value,
    inlineLabel: assetStatusLabel.value,
    tone: assetTone.value,
    rows: assetRows.value,
    actionLabel: assetActionLabel.value,
    actionTitle: assetActionTitle.value,
    actionDisabled: !props.isUnityProject || isScanning.value,
    actionVariant: "neutral",
  },
  {
    id: "unity",
    icon: "unity",
    title: t("chat.status.unity.title"),
    summary: unitySummary.value,
    inlineLabel: unitySummary.value,
    tone: unityTone.value,
    rows: unityRows.value,
    actionLabel: unityActionLabel.value,
    actionTitle: unityActionTitle.value,
    actionDisabled: props.unityPluginStatus
      ? props.unityPluginInstalling
      : unityRecompileWaitingConnection.value
        || effectiveUnityLaunchState.value !== "idle"
        || !props.isUnityProject,
    actionVariant: props.unityPluginStatus ? "neutral" : "primary",
  },
  {
    id: "knowledge",
    icon: "knowledge",
    title: t("chat.status.knowledge.title"),
    summary: knowledgeModeSummary.value,
    inlineLabel: knowledgeModeSummary.value,
    tone: knowledgeTone.value,
    rows: knowledgeRows.value,
    modeOptions: knowledgeModeOptions.value,
  },
  {
    id: "code",
    icon: "code",
    title: t("chat.status.code.title"),
    summary: codeSummary.value,
    inlineLabel: codeSummary.value,
    tone: codeTone.value,
    rows: codeRows.value,
    modeOptions: codeSupported.value ? codeModeOptions.value : undefined,
    actionLabel: codeEnabled.value
      && (csharpLsp.value?.phase === "ready" || csharpLsp.value?.phase === "error")
      ? t("chat.status.code.restart")
      : "",
    actionTitle: t("chat.status.code.restartTitle"),
    actionDisabled: csharpLspPending.value,
    actionVariant: "neutral",
  },
]);

const activeItem = computed(() =>
  statusItems.value.find((item) => item.id === activePopover.value) ?? null,
);

function statusIconNode(icon: StatusIcon) {
  return STATUS_ICONS[icon];
}

function togglePopover(id: StatusId) {
  activePopover.value = activePopover.value === id ? null : id;
  if (activePopover.value === "knowledge") {
    void loadKnowledgeStatus();
  }
  if (activePopover.value === "code") {
    void refreshCsharpLspStatus();
  }
}

function segmentedModelFor(id: StatusId): string {
  if (id === "knowledge") return knowledgeMode.value;
  if (id === "code") return codeEnabled.value ? "on" : "off";
  return "";
}

function applySegmentedMode(id: StatusId, mode: string) {
  if (id === "knowledge") {
    setKnowledgeMode(mode);
  } else if (id === "code") {
    void setCodeEnabled(mode === "on");
  }
}

function closePopover() {
  activePopover.value = null;
}

function setKnowledgeMode(mode: string) {
  if (mode === "disabled" || mode === "read_only" || mode === "full") {
    emit("updateKnowledgeAccessMode", mode);
  }
}

function clearKnowledgeStatus() {
  knowledgeOverview.value = null;
  lexicalRebuildStatus.value = null;
  embeddingStatus.value = null;
  injectedItems.value = [];
  knowledgeRetrievalError.value = "";
  knowledgeContextError.value = "";
}

async function loadKnowledgeStatus() {
  const seq = ++knowledgeStatusSeq;
  if (!knowledgeHasWorkspace.value) {
    clearKnowledgeStatus();
    knowledgeStatusLoading.value = false;
    return;
  }
  if (knowledgeMode.value === "disabled") {
    clearKnowledgeStatus();
    knowledgeStatusLoading.value = false;
    return;
  }

  knowledgeStatusLoading.value = true;
  const agentId = knowledgeAgentId.value;
  const [overviewResult, lexicalResult, embeddingResult, injectedResult] =
    await Promise.allSettled([
      knowledgeGetOverview(),
      knowledgeGetLexicalRebuildStatus(),
      knowledgeGetEmbeddingStatus(),
      agentId
        ? listAgentInjectedItems(agentId, knowledgeMode.value)
        : Promise.resolve([] as InjectedPromptItem[]),
    ]);

  if (seq !== knowledgeStatusSeq) return;

  knowledgeRetrievalError.value = "";
  knowledgeContextError.value = "";

  if (overviewResult.status === "fulfilled") {
    knowledgeOverview.value = overviewResult.value;
  } else {
    knowledgeOverview.value = null;
    knowledgeRetrievalError.value = normalizeAppError(overviewResult.reason).message;
  }

  if (lexicalResult.status === "fulfilled") {
    lexicalRebuildStatus.value = lexicalResult.value;
  }

  if (embeddingResult.status === "fulfilled") {
    embeddingStatus.value = embeddingResult.value;
  }

  if (injectedResult.status === "fulfilled") {
    injectedItems.value = injectedResult.value;
  } else {
    injectedItems.value = [];
    knowledgeContextError.value = normalizeAppError(injectedResult.reason).message;
  }

  knowledgeStatusLoading.value = false;
}

function runStatusAction(item: StatusItem) {
  if (item.id === "assetDb") {
    emit("startScan");
  } else if (item.id === "unity") {
    if (props.unityPluginStatus) {
      emit("installPlugin");
    } else {
      emit("launchUnityProject");
    }
  } else if (item.id === "code") {
    void restartCode();
    return;
  }
  closePopover();
}

function onDocumentKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") {
    closePopover();
  }
}

onMounted(() => {
  document.addEventListener("click", closePopover);
  document.addEventListener("keydown", onDocumentKeydown);
  csharpLspDisposed = false;
  void refreshCsharpLspStatus();
  subscribeCsharpLspStatus((payload) => {
    csharpLsp.value = payload;
  })
    .then((unsubscribe) => {
      if (csharpLspDisposed) {
        unsubscribe();
      } else {
        csharpLspUnsubscribe = unsubscribe;
      }
    })
    .catch(() => {});
});

watch(
  () => `${props.workingDir ?? ""}::${knowledgeAgentId.value}::${knowledgeMode.value}`,
  () => {
    if (activePopover.value === "knowledge") {
      void loadKnowledgeStatus();
    }
  },
);

onUnmounted(() => {
  document.removeEventListener("click", closePopover);
  document.removeEventListener("keydown", onDocumentKeydown);
  csharpLspDisposed = true;
  csharpLspUnsubscribe?.();
  csharpLspUnsubscribe = null;
});
</script>

<template>
  <div class="chat-status-indicators" @click.stop>
    <div class="chat-status-icon-row">
      <button
        v-for="item in statusItems"
        :key="item.id"
        type="button"
        class="chat-status-icon-btn ui-select-none"
        :class="[
          `tone-${item.tone}`,
          {
            active: activePopover === item.id,
            'is-scanning': (item.id === 'assetDb' && isScanning)
              || (item.id === 'code' && codeTransitional),
          },
        ]"
        :aria-label="`${item.title}: ${item.summary}`"
        :aria-expanded="activePopover === item.id"
        @click="togglePopover(item.id)"
      >
        <svg
          class="chat-status-icon"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
          focusable="false"
        >
          <component
            :is="tag"
            v-for="([tag, attrs], idx) in statusIconNode(item.icon)"
            :key="idx"
            v-bind="attrs"
          />
        </svg>
        <span class="chat-status-icon-label">{{ item.inlineLabel }}</span>
      </button>
    </div>

    <Transition name="status-popover">
      <div
        v-if="activeItem"
        class="chat-status-popover"
        :class="{ 'has-details': activeItem.rows.length > 0 }"
        role="dialog"
        :aria-label="activeItem.title"
        @click.stop
      >
        <div class="chat-status-popover-head">
          <div class="chat-status-popover-heading">
            <span class="chat-status-popover-summary" :class="`tone-${activeItem.tone}`">
              {{ activeItem.summary }}
            </span>
          </div>
          <BaseButton
            v-if="activeItem.actionLabel"
            class="chat-status-action ui-select-none"
            size="sm"
            :variant="activeItem.actionVariant"
            :disabled="activeItem.actionDisabled"
            :title="activeItem.actionTitle"
            @click="runStatusAction(activeItem)"
          >
            {{ activeItem.actionLabel }}
          </BaseButton>
        </div>
        <BaseSegmented
          v-if="activeItem.modeOptions"
          class="chat-status-mode"
          size="sm"
          :model-value="segmentedModelFor(activeItem.id)"
          :options="activeItem.modeOptions"
          @update:model-value="applySegmentedMode(activeItem.id, $event)"
        />
        <dl v-if="activeItem.rows.length > 0" class="chat-status-detail-list">
          <template v-for="row in activeItem.rows" :key="`${row.label}:${row.value}`">
            <dt>{{ row.label }}</dt>
            <dd :class="{ 'is-mono': row.mono }">{{ row.value }}</dd>
          </template>
        </dl>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.chat-status-indicators {
  position: relative;
  display: inline-flex;
  align-items: center;
  min-width: 0;
}

.chat-status-icon-row {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.chat-status-icon-btn {
  position: relative;
  width: 24px;
  height: 24px;
  min-width: 24px;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  box-shadow: none;
  transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease;
}

.chat-status-icon-btn:hover,
.chat-status-icon-btn.active,
.chat-status-icon-btn:focus-visible {
  background: var(--hover-bg);
  border-color: color-mix(in srgb, currentColor 22%, transparent);
}

.chat-status-icon {
  width: 14px;
  height: 14px;
  flex: 0 0 auto;
  display: block;
}

.chat-status-icon-label {
  position: absolute;
  left: 50%;
  bottom: calc(100% + 6px);
  z-index: 35;
  max-width: 180px;
  padding: 4px 7px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  background: var(--surface-elevated, var(--panel-bg));
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.16);
  color: currentColor;
  pointer-events: none;
  overflow: hidden;
  font-size: 11px;
  line-height: 1.3;
  opacity: 0;
  transform: translate(-50%, 3px);
  text-overflow: ellipsis;
  white-space: nowrap;
  transition: opacity 0.1s ease, transform 0.1s ease;
}

.chat-status-icon-btn:not(.active):hover .chat-status-icon-label,
.chat-status-icon-btn:not(.active):focus-visible .chat-status-icon-label {
  opacity: 1;
  transform: translate(-50%, 0);
}

.chat-status-icon-btn.tone-success {
  color: var(--status-good-fg);
}

.chat-status-icon-btn.tone-danger {
  color: var(--status-danger-fg);
}

.chat-status-icon-btn.tone-accent {
  color: var(--accent-color);
}

.chat-status-icon-btn.is-scanning > svg {
  animation: chat-status-icon-breathe 1.35s ease-in-out infinite;
  transform-origin: center;
}

.chat-status-popover {
  position: absolute;
  left: 0;
  bottom: calc(100% + 8px);
  z-index: 30;
  width: min(320px, calc(100vw - 32px));
  padding: 10px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--surface-elevated, var(--panel-bg));
  box-shadow: 0 12px 28px rgba(0, 0, 0, 0.18);
  color: var(--text-color);
}

.chat-status-popover-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.chat-status-popover.has-details .chat-status-popover-head {
  padding-bottom: 8px;
  border-bottom: 1px solid var(--border-color);
}

.chat-status-popover-heading {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.chat-status-popover-summary {
  min-width: 0;
  font-size: 12px;
  line-height: 1.35;
  font-weight: 600;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.chat-status-popover-summary.tone-success {
  color: var(--status-good-fg);
}

.chat-status-popover-summary.tone-danger {
  color: var(--status-danger-fg);
}

.chat-status-popover-summary.tone-accent {
  color: var(--accent-color);
}

.chat-status-popover-summary.tone-muted {
  color: var(--text-secondary);
}

.chat-status-mode {
  width: 100%;
  margin-top: 10px;
}

.chat-status-mode :deep(.base-segmented-item) {
  flex: 1 1 0;
  padding: 0 8px;
}

.chat-status-detail-list {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  gap: 6px 10px;
  margin: 10px 0 0;
  font-size: 12px;
}

.chat-status-detail-list dt {
  color: var(--text-secondary);
}

.chat-status-detail-list dd {
  margin: 0;
  min-width: 0;
  color: var(--text-color);
  overflow-wrap: anywhere;
}

.chat-status-detail-list dd.is-mono {
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  line-height: 1.4;
}

.chat-status-action {
  flex: 0 0 auto;
}

.status-popover-enter-active,
.status-popover-leave-active {
  transition: opacity 0.12s ease, transform 0.12s ease;
}

.status-popover-enter-from,
.status-popover-leave-to {
  opacity: 0;
  transform: translateY(4px);
}

@keyframes chat-status-icon-breathe {
  0%,
  100% {
    opacity: 0.72;
    transform: scale(0.96);
  }
  50% {
    opacity: 1;
    transform: scale(1.04);
  }
}
</style>
