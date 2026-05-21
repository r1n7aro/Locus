<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import { useNotificationStore } from "../../stores/notification";
import { openFileExternal } from "../../services/unity";
import { assetRiskReport } from "../../services/asset";
import BaseButton from "../ui/BaseButton.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";
import type {
  AssetDbScanEvent,
  AssetDbOverview,
  AssetRiskEntry,
  AssetRiskKind,
  WatcherTuning,
} from "../../types";

const props = defineProps<{
  overview: AssetDbOverview | null;
  loading: boolean;
  tuning: WatcherTuning | null;
  tuningSaving: boolean;
}>();

const emit = defineEmits<{
  (e: "rescan"): void;
  (e: "updateTuning", debounceMs: number, workerCount: number): void;
}>();

interface LiveScanProgress {
  stageLabel: string;
  summaryLabel: string;
  heroValue: string;
  heroLabel: string;
  progressRatio: number | null;
  completed: number | null;
  total: number | null;
  indeterminate: boolean;
  queued?: number | null;
  failed?: number | null;
}

const notificationStore = useNotificationStore();

// Stepless intensity slider — backend tuning is debounce in [0, 1000] ms,
// where lower means more aggressive scanning. The UI inverts that mapping so
// moving the thumb to the right always means "stronger".
const INTENSITY_MIN_MS = 0;
const INTENSITY_MAX_MS = 1000;
const INTENSITY_STEP_MS = 10;

const draftDebounceMs = ref<number | null>(null);

function clampDebounceMs(ms: number) {
  return Math.min(INTENSITY_MAX_MS, Math.max(INTENSITY_MIN_MS, ms));
}

function debounceMsToSliderValue(ms: number) {
  return INTENSITY_MAX_MS - clampDebounceMs(ms);
}

function sliderValueToDebounceMs(value: number) {
  return INTENSITY_MAX_MS - clampDebounceMs(value);
}

const effectiveDebounceMs = computed(() => {
  if (draftDebounceMs.value != null) return draftDebounceMs.value;
  return props.tuning?.debounceMs ?? 100;
});

const intensitySliderValue = computed(() =>
  debounceMsToSliderValue(effectiveDebounceMs.value),
);

const intensityLabel = computed(() => {
  const ms = effectiveDebounceMs.value;
  if (ms <= 30) return t("asset.db.intensity.turbo");
  if (ms >= 600) return t("asset.db.intensity.eco");
  return t("asset.db.intensity.normal");
});

const intensityTitle = computed(() =>
  `${intensityLabel.value} · ${effectiveDebounceMs.value} ms`,
);

function onIntensityInput(e: Event) {
  const target = e.target as HTMLInputElement;
  draftDebounceMs.value = sliderValueToDebounceMs(Number(target.value));
}

function commitIntensity() {
  if (draftDebounceMs.value == null || !props.tuning) return;
  const next = draftDebounceMs.value;
  draftDebounceMs.value = null;
  if (next === props.tuning.debounceMs) return;
  emit("updateTuning", next, props.tuning.workerCount);
}

const workerOptions = computed(() => {
  const max = props.tuning?.maxWorkerCount ?? 8;
  // Pick sensible discrete steps that include 1, 2, 4, max.
  const candidates = [1, 2, 4, max];
  const out: number[] = [];
  for (const c of candidates) {
    if (c >= 1 && c <= max && !out.includes(c)) out.push(c);
  }
  return out.sort((a, b) => a - b);
});
const workerSegmentOptions = computed(() =>
  workerOptions.value.map((value) => ({ value: String(value), label: String(value) })),
);
const workerSegmentValue = computed(() =>
  String(props.tuning?.workerCount ?? workerOptions.value[0] ?? 1),
);

function setWorkerCount(n: number) {
  if (!props.tuning) return;
  if (props.tuning.workerCount === n) return;
  emit("updateTuning", props.tuning.debounceMs, n);
}

function fmtNum(n: number): string {
  return n.toLocaleString();
}

function formatBytes(bytes: number | null | undefined): string {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value >= 100 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

function fmtCompact(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "k";
  return n.toString();
}

function fmtDateTime(ms?: number): string {
  /*
  if (!ms) return "—";
  */
  if (!ms) return "--";
  const d = new Date(ms);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) {
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  return d.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function fmtDuration(ms?: number): string {
  /*
  if (ms == null) return "—";
  */
  if (ms == null) return "--";
  if (ms < 1000) return `${ms} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  const m = Math.floor(ms / 60_000);
  const s = Math.floor((ms % 60_000) / 1000);
  return `${m}m ${s}s`;
}

function clampProgress(value: number): number {
  return Math.max(0, Math.min(1, value));
}

function fmtPercent(value: number): string {
  return `${Math.round(clampProgress(value) * 100)}%`;
}

function fmtStageStep(step: number): string {
  return `${step} / 4`;
}

function isFiniteCount(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function reconcileStageLabel(stage: string | null | undefined): string {
  switch (stage) {
    case "scanning": return t("chat.status.assetDb.reconcileStage.scanning");
    case "discovering": return t("chat.status.assetDb.reconcileStage.discovering");
    case "processing": return t("chat.status.assetDb.reconcileStage.processing");
    default: return stage || t("asset.db.scanPhase.reconcile");
  }
}

function buildLiveScanProgress(phase: AssetDbScanEvent): LiveScanProgress | null {
  switch (phase.phase) {
    case "dirScan":
      return {
        stageLabel: t("asset.db.scanPhase.dirScan"),
        summaryLabel: t("chat.assetDb.scanning.dirScan"),
        heroValue: fmtStageStep(1),
        heroLabel: t("asset.db.scanPhase.label"),
        progressRatio: null,
        completed: null,
        total: null,
        indeterminate: true,
      };
    case "metaParse": {
      const progressRatio = phase.total > 0 ? clampProgress(phase.completed / phase.total) : 1;
      return {
        stageLabel: t("asset.db.scanPhase.metaParse"),
        summaryLabel: t("asset.db.scanProgressCount", fmtNum(phase.completed), fmtNum(phase.total)),
        heroValue: fmtPercent(progressRatio),
        heroLabel: t("asset.db.scanProgress"),
        progressRatio,
        completed: phase.completed,
        total: phase.total,
        indeterminate: false,
      };
    }
    case "yamlParse": {
      const progressRatio = phase.total > 0 ? clampProgress(phase.completed / phase.total) : 1;
      return {
        stageLabel: t("asset.db.scanPhase.yamlParse"),
        summaryLabel: t("asset.db.scanProgressCount", fmtNum(phase.completed), fmtNum(phase.total)),
        heroValue: fmtPercent(progressRatio),
        heroLabel: t("asset.db.scanProgress"),
        progressRatio,
        completed: phase.completed,
        total: phase.total,
        indeterminate: false,
      };
    }
    case "dbWrite":
      return {
        stageLabel: t("asset.db.scanPhase.dbWrite"),
        summaryLabel: t("chat.assetDb.scanning.dbWrite"),
        heroValue: fmtStageStep(4),
        heroLabel: t("asset.db.scanPhase.label"),
        progressRatio: null,
        completed: null,
        total: null,
        indeterminate: true,
      };
    case "reconcile":
    {
      const hasProgress = isFiniteCount(phase.completed)
        && isFiniteCount(phase.total)
        && phase.total > 0;
      const progressRatio = hasProgress
        ? clampProgress((phase.completed ?? 0) / (phase.total ?? 1))
        : null;
      const summaryLabel = hasProgress
        ? t("asset.db.scanProgressCount", fmtNum(phase.completed ?? 0), fmtNum(phase.total ?? 0))
        : phase.stage === "discovering" && isFiniteCount(phase.queued)
          ? t("chat.assetDb.scanning.reconcile.discovering", fmtNum(phase.queued))
          : t("chat.assetDb.scanning.reconcile");
      return {
        stageLabel: reconcileStageLabel(phase.stage),
        summaryLabel,
        heroValue: progressRatio != null ? fmtPercent(progressRatio) : fmtStageStep(4),
        heroLabel: progressRatio != null ? t("asset.db.scanProgress") : t("asset.db.scanPhase.label"),
        progressRatio,
        completed: isFiniteCount(phase.completed) ? phase.completed : null,
        total: isFiniteCount(phase.total) ? phase.total : null,
        indeterminate: progressRatio == null,
        queued: isFiniteCount(phase.queued) ? phase.queued : null,
        failed: isFiniteCount(phase.failed) ? phase.failed : null,
      };
    }
    default:
      return null;
  }
}

const statusKey = computed(() => props.overview?.status ?? "none");
const scanError = computed(() =>
  props.overview?.currentScanPhase?.phase === "error"
    ? props.overview.currentScanPhase.error
    : null,
);
const rescanRequired = computed(() =>
  scanError.value?.code?.startsWith("ref_graph.rescan_required") ?? false,
);
const statusLabel = computed(() => {
  if (statusKey.value === "error" && rescanRequired.value) {
    return t("asset.db.status.rescanRequired");
  }
  return t(`asset.db.status.${statusKey.value}`);
});

const statusTone = computed<"good" | "warn" | "danger" | "muted">(() => {
  switch (statusKey.value) {
    case "indexed": return "good";
    case "scanning": return "warn";
    case "error": return rescanRequired.value ? "warn" : "danger";
    default: return "muted";
  }
});

const scanActionLabel = computed(() => {
  switch (statusKey.value) {
    case "indexed":
      return t("asset.db.action.rescan");
    case "scanning":
      return t("asset.db.status.scanning");
    case "error":
      return rescanRequired.value
        ? t("asset.db.action.rescan")
        : t("asset.db.action.retry");
    default:
      return t("asset.db.action.scanNow");
  }
});

const issueBanner = computed(() => {
  if (!scanError.value) return null;
  return {
    tone: rescanRequired.value ? "warn" : "danger",
    title: rescanRequired.value
      ? t("asset.db.issue.rescanTitle")
      : t("asset.db.issue.errorTitle"),
    message: rescanRequired.value
      ? t("asset.db.issue.rescanBody")
      : scanError.value.message,
  };
});

const watcherTone = computed<"good" | "muted">(() =>
  props.overview?.watcherRunning ? "good" : "muted",
);

const lastScanStats = computed(() => props.overview?.lastScanStats ?? null);

const isScanning = computed(() => statusKey.value === "scanning");

const liveScanProgress = computed(() => {
  const phase = props.overview?.currentScanPhase;
  if (!phase || phase.phase === "done" || phase.phase === "reconcileDone" || phase.phase === "error") return null;
  return buildLiveScanProgress(phase);
});

// Caption shown next to the status badge in the scan-status card.
// - never scanned & not indexed -> "Never scanned"
// - indexed but no in-process scan history -> "Indexed from disk"
// - has lastScanAt → formatted timestamp
const lastCompletedScanCaption = computed(() => {
  const o = props.overview;
  /*
  if (!o) return "—";
  */
  if (!o) return "--";
  if (o.lastScanAt) return fmtDateTime(o.lastScanAt);
  if (rescanRequired.value) return t("asset.db.status.rescanRequired");
  if (statusKey.value === "indexed") return t("asset.db.indexedFromDisk");
  return t("asset.db.never");
});

const scanCaption = computed(() =>
  liveScanProgress.value?.stageLabel ?? lastCompletedScanCaption.value,
);

// Hero value for the scan-status card. Shows duration when we have one,
// otherwise a placeholder so the card does not repeat the status caption.
const lastCompletedScanDuration = computed(() => {
  const o = props.overview;
  if (o?.lastScanDurationMs != null) return fmtDuration(o.lastScanDurationMs);
  /*
  return "—";
  */
  return "--";
});

const scanHeroLabel = computed(() =>
  liveScanProgress.value?.heroValue ?? lastCompletedScanDuration.value,
);

const scanHeroMetricLabel = computed(() =>
  liveScanProgress.value?.heroLabel ?? t("asset.db.scanDuration"),
);

const totalNodes = computed(() => props.overview?.nodes ?? 0);
const totalEdges = computed(() => props.overview?.edges ?? 0);
const dbBytes = computed(() => props.overview?.dbBytes ?? 0);
const assetBytes = computed(() => props.overview?.assetBytes ?? 0);

const queueLen = computed(() => props.overview?.watcherQueueLen ?? 0);
const currentFile = computed(() => props.overview?.watcherCurrentFile ?? null);
const currentFileName = computed(() => {
  const f = currentFile.value;
  if (!f) return "";
  const idx = f.lastIndexOf("/");
  return idx >= 0 ? f.slice(idx + 1) : f;
});
const currentFileDir = computed(() => {
  const f = currentFile.value;
  if (!f) return "";
  const idx = f.lastIndexOf("/");
  return idx >= 0 ? f.slice(0, idx + 1) : "";
});

const assetRisks = computed(() => props.overview?.assetRisks ?? []);
const hasAssetRisks = computed(() => assetRisks.value.length > 0);
const openingRiskKind = ref<AssetRiskKind | null>(null);

function riskLabelKey(kind: AssetRiskKind): string {
  switch (kind) {
    case "brokenReferences":
      return "asset.db.risk.brokenReferences";
    case "missingScripts":
      return "asset.db.risk.missingScripts";
    case "parseFailures":
      return "asset.db.risk.parseFailures";
    case "duplicateGuids":
      return "asset.db.risk.duplicateGuids";
  }
}

function riskCountLabel(entry: AssetRiskEntry): string {
  return t("asset.db.riskCount", fmtCompact(entry.count));
}

async function openRiskDetail(kind: AssetRiskKind) {
  try {
    openingRiskKind.value = kind;
    const reportPath = await assetRiskReport(kind);
    await openFileExternal(reportPath);
  } catch (error: any) {
    notificationStore.addNotice(
      "error",
      error?.message ?? t("asset.db.riskOpenFailed"),
      {
        code: error?.code,
        operation: "open_asset_risk_report",
      },
    );
  } finally {
    if (openingRiskKind.value === kind) {
      openingRiskKind.value = null;
    }
  }
}
</script>

<template>
  <div class="asset-overview">
    <div v-if="loading && !overview" class="overview-loading">{{ t("asset.preview.loading") }}</div>

    <template v-else-if="overview">
      <div v-if="issueBanner" class="overview-alert" :class="issueBanner.tone">
        <div class="overview-alert-title">{{ issueBanner.title }}</div>
        <div class="overview-alert-body">{{ issueBanner.message }}</div>
      </div>

      <div class="overview-grid overview-grid-summary">
        <!-- ── Card 1: Index Scale ────────────────────── -->
        <section class="overview-card">
          <div class="card-title">{{ t("asset.db.cardScale") }}</div>
          <div class="mini-grid mini-grid-two scale-stat-grid">
            <div class="mini-stat scale-stat">
              <span class="scale-stat-value">{{ fmtNum(totalNodes) }}</span>
              <span class="scale-stat-label">{{ t("asset.db.assetCount") }}</span>
            </div>
            <div class="mini-stat scale-stat">
              <span class="scale-stat-value">{{ fmtNum(totalEdges) }}</span>
              <span class="scale-stat-label">{{ t("asset.db.referenceCount") }}</span>
            </div>
            <div class="mini-stat scale-stat">
              <span class="scale-stat-value">{{ formatBytes(dbBytes) }}</span>
              <span class="scale-stat-label">{{ t("asset.db.dbSize") }}</span>
            </div>
            <div class="mini-stat scale-stat">
              <span class="scale-stat-value">{{ formatBytes(assetBytes) }}</span>
              <span class="scale-stat-label">{{ t("asset.db.assetSize") }}</span>
            </div>
          </div>
          <div class="detail-list scale-detail-list">
            <div class="detail-row">
              <span class="detail-key">{{ t("asset.db.lastScan") }}</span>
              <span class="detail-value">{{ scanCaption }}</span>
            </div>
          </div>
        </section>

        <!-- ── Card 2: Scan Status ────────────────────── -->
        <section class="overview-card">
          <div class="card-title">{{ t("asset.db.cardScan") }}</div>
          <div class="card-status-row">
            <span class="status-badge" :class="statusTone">{{ statusLabel }}</span>
            <span class="status-caption">{{ scanCaption }}</span>
          </div>
          <div class="card-hero compact">
            <span class="hero-value">{{ scanHeroLabel }}</span>
            <span class="hero-label">{{ scanHeroMetricLabel }}</span>
          </div>
          <div v-if="liveScanProgress" class="scan-progress-block">
            <div class="scan-progress-row">
              <span class="scan-progress-stage">{{ liveScanProgress.stageLabel }}</span>
              <span class="scan-progress-summary">{{ liveScanProgress.summaryLabel }}</span>
            </div>
            <div class="scan-progress-track" :class="{ indeterminate: liveScanProgress.indeterminate }">
              <div
                class="scan-progress-fill"
                :class="{ indeterminate: liveScanProgress.indeterminate }"
                :style="liveScanProgress.progressRatio != null
                  ? { width: `${Math.round(liveScanProgress.progressRatio * 100)}%` }
                  : undefined"
              />
            </div>
          </div>
          <div class="detail-list">
            <template v-if="liveScanProgress">
              <div v-if="liveScanProgress.completed != null" class="detail-row">
                <span class="detail-key">{{ t("asset.db.scanPhase.completed") }}</span>
                <span class="detail-value">{{ fmtNum(liveScanProgress.completed) }}</span>
              </div>
              <div v-if="liveScanProgress.total != null" class="detail-row">
                <span class="detail-key">{{ t("asset.db.scanPhase.total") }}</span>
                <span class="detail-value">{{ fmtNum(liveScanProgress.total) }}</span>
              </div>
              <div v-if="liveScanProgress.queued != null" class="detail-row">
                <span class="detail-key">{{ t("chat.status.assetDb.queued") }}</span>
                <span class="detail-value">{{ fmtNum(liveScanProgress.queued) }}</span>
              </div>
              <div v-if="liveScanProgress.failed != null && liveScanProgress.failed > 0" class="detail-row">
                <span class="detail-key">{{ t("chat.status.assetDb.failed") }}</span>
                <span class="detail-value warn">{{ fmtNum(liveScanProgress.failed) }}</span>
              </div>
              <div class="detail-row">
                <span class="detail-key">{{ t("asset.db.lastScan") }}</span>
                <span class="detail-value dim">{{ lastCompletedScanCaption }}</span>
              </div>
              <div class="detail-row">
                <span class="detail-key">{{ t("asset.db.scanDuration") }}</span>
                <span class="detail-value">{{ lastCompletedScanDuration }}</span>
              </div>
            </template>
            <div v-if="lastScanStats && !liveScanProgress" class="detail-row">
              <span class="detail-key">{{ t("asset.db.dirsScanned") }}</span>
              <span class="detail-value">{{ fmtNum(lastScanStats.dirsScanned) }}</span>
            </div>
            <div v-if="lastScanStats && !liveScanProgress" class="detail-row">
              <span class="detail-key">{{ t("asset.db.yamlAssets") }}</span>
              <span class="detail-value">{{ fmtNum(lastScanStats.yamlAssetsFound) }}</span>
            </div>
            <div v-if="lastScanStats && !liveScanProgress" class="detail-row">
              <span class="detail-key">{{ t("asset.db.nodesAdded") }}</span>
              <span class="detail-value">{{ fmtNum(lastScanStats.nodesAdded) }}</span>
            </div>
            <div v-if="lastScanStats && !liveScanProgress" class="detail-row">
              <span class="detail-key">{{ t("asset.db.edgesAdded") }}</span>
              <span class="detail-value">{{ fmtNum(lastScanStats.edgesAdded) }}</span>
            </div>
            <div v-if="lastScanStats && !liveScanProgress" class="detail-row">
              <span class="detail-key">{{ t("asset.db.parseFailures") }}</span>
              <span class="detail-value" :class="{ warn: lastScanStats.parseFailures > 0 }">
                {{ fmtNum(lastScanStats.parseFailures) }}
              </span>
            </div>
            <div v-if="!lastScanStats && !liveScanProgress" class="detail-row">
              <span class="detail-key">{{ t("asset.db.lastScan") }}</span>
              <span class="detail-value dim">
                {{ statusKey === "indexed"
                    ? t("asset.db.indexedNoScan")
                    : rescanRequired
                      ? t("asset.db.status.rescanRequired")
                      : t("asset.db.never") }}
              </span>
            </div>
          </div>
          <div class="card-actions">
            <BaseButton class="card-btn card-btn-rescan" size="md" block :disabled="isScanning" @click="emit('rescan')">
              {{ scanActionLabel }}
            </BaseButton>
          </div>
        </section>

        <!-- ── Card 3: Watcher ────────────────────────── -->
        <section class="overview-card">
          <div class="card-title">{{ t("asset.db.cardWatcher") }}</div>
          <div class="card-status-row">
            <span class="status-badge" :class="watcherTone">
              {{ overview.watcherRunning ? t("asset.db.watcher.running") : t("asset.db.watcher.stopped") }}
            </span>
            <span class="status-caption">
              {{ overview.watcherRunning
                  ? (queueLen > 0 || currentFile
                    ? t("asset.db.watcherPending", queueLen)
                    : t("asset.db.watcherIdleNow"))
                  : t("asset.db.watcher") }}
            </span>
          </div>
          <div class="card-hero compact">
            <span class="hero-value">{{ fmtNum(queueLen) }}</span>
            <span class="hero-label">{{ t("asset.db.watcherQueue") }}</span>
          </div>
          <div class="watcher-current">
            <div class="watcher-current-label">{{ t("asset.db.watcherCurrent") }}</div>
            <div
              v-if="currentFile"
              class="watcher-current-value"
              :title="currentFile"
            >
              <span class="watcher-pulse"></span>
              <span class="watcher-current-name">{{ currentFileName }}</span>
              <span v-if="currentFileDir" class="watcher-current-dir">{{ currentFileDir }}</span>
            </div>
            <div v-else class="watcher-current-value idle">
              {{ t("asset.db.watcherIdleNow") }}
            </div>
          </div>
          <div v-if="tuning" class="watcher-tuning">
            <div class="tuning-row">
              <span class="tuning-label">{{ t("asset.db.intensity") }}</span>
              <div class="tuning-control">
                <input
                  type="range"
                  class="intensity-slider"
                  :min="INTENSITY_MIN_MS"
                  :max="INTENSITY_MAX_MS"
                  :step="INTENSITY_STEP_MS"
                  :value="intensitySliderValue"
                  :disabled="tuningSaving"
                  :title="intensityTitle"
                  @input="onIntensityInput"
                  @change="commitIntensity"
                />
                <div class="tuning-scale" aria-hidden="true">
                  <span>{{ t("asset.db.intensity.eco") }}</span>
                  <span>{{ t("asset.db.intensity.turbo") }}</span>
                </div>
              </div>
              <span class="tuning-readout-val">
                {{ intensityLabel }}
                <span class="tuning-readout-sep">·</span>
                {{ effectiveDebounceMs }}<span class="tuning-readout-unit">ms</span>
              </span>
            </div>
            <div class="tuning-row">
              <span class="tuning-label">{{ t("asset.db.workerCount") }}</span>
              <BaseSegmented
                class="seg-group"
                size="sm"
                :model-value="workerSegmentValue"
                :options="workerSegmentOptions"
                @update:model-value="setWorkerCount(Number($event))"
              />
            </div>
          </div>

          <div class="card-note">
            {{ overview.watcherRunning ? t("asset.db.watcherActiveHint") : t("asset.db.watcherIdleHint") }}
          </div>
        </section>

        <section class="overview-card slim asset-risk-card">
          <div class="card-title-row">
            <span class="card-title">{{ t("asset.db.cardAssetRisk") }}</span>
            <span class="status-badge" :class="hasAssetRisks ? 'warn' : 'good'">
              {{ hasAssetRisks ? t("asset.db.guidRiskDetected") : t("asset.db.guidRiskHealthy") }}
            </span>
          </div>

          <div v-if="hasAssetRisks" class="risk-list">
            <button
              v-for="risk in assetRisks"
              :key="risk.kind"
              type="button"
              class="risk-row"
              :disabled="openingRiskKind === risk.kind"
              @click="openRiskDetail(risk.kind)"
            >
              <span class="risk-row-main">
                <span class="risk-row-name">{{ t(riskLabelKey(risk.kind)) }}</span>
                <span class="risk-row-value">{{ riskCountLabel(risk) }}</span>
              </span>
              <span class="risk-row-meta">{{ t("asset.db.riskOpenDetail") }}</span>
            </button>
          </div>

          <div v-else class="risk-empty">
            {{ t("asset.db.riskHealthyBody") }}
          </div>
        </section>
      </div>
    </template>

    <div v-else class="overview-loading">{{ t("asset.db.never") }}</div>
  </div>
</template>

<style scoped>
.asset-overview {
  flex: 1;
  padding: 16px 20px 20px;
  overflow: auto;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--bg-color) 6%);
}

.overview-loading {
  min-height: 220px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 13px;
}

.overview-alert {
  margin-bottom: 12px;
  padding: 11px 12px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--input-bg);
}

.overview-alert.warn {
  border-color: var(--status-warn-border);
  background: var(--status-warn-bg);
}

.overview-alert.danger {
  border-color: var(--status-danger-border);
  background: var(--status-danger-bg);
}

.overview-alert-title {
  margin-bottom: 4px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.overview-alert.warn .overview-alert-title {
  color: var(--status-warn-fg);
}

.overview-alert.danger .overview-alert-title {
  color: var(--status-danger-fg);
}

.overview-alert-body {
  font-size: 12px;
  line-height: 1.55;
  color: var(--text-secondary);
}

.overview-grid {
  display: grid;
  gap: 12px;
}

.overview-grid-summary {
  grid-template-columns: repeat(3, minmax(0, 1fr));
  margin-bottom: 12px;
}

.overview-card {
  min-width: 0;
  padding: 14px 16px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: var(--panel-bg);
  display: flex;
  flex-direction: column;
}
.overview-card.slim {
  min-height: 0;
}
.asset-risk-card {
  width: auto;
}

.card-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
  margin-bottom: 12px;
}

.card-title-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 12px;
}
.card-title-row .card-title {
  margin-bottom: 0;
}
.card-title-meta {
  font-size: 11px;
  color: var(--text-secondary);
}

.card-hero {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin-bottom: 14px;
}
.card-hero.compact {
  margin-bottom: 12px;
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

.mini-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px;
  margin-bottom: 12px;
}
.mini-grid-two {
  grid-template-columns: repeat(2, minmax(0, 1fr));
}
.scale-stat-grid {
  margin-bottom: 10px;
}
.mini-stat {
  padding: 10px 10px 9px;
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 74%, var(--input-bg) 26%);
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  min-width: 0;
}
.scale-stat {
  padding: 12px 12px 11px;
}
.scale-stat-value {
  display: block;
  font-size: 20px;
  line-height: 1.1;
  font-weight: 700;
  color: var(--text-color);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.scale-stat-label {
  display: block;
  margin-top: 5px;
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.mini-label {
  display: block;
  font-size: 11px;
  color: var(--text-secondary);
  margin-bottom: 4px;
  font-family: var(--font-mono-identifier);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.mini-value {
  font-size: 17px;
  font-weight: 700;
  color: var(--text-color);
}
.mini-value.warn {
  color: var(--status-danger-fg);
}

.card-status-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 10px;
}

.status-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 22px;
  padding: 0 10px;
  border-radius: var(--radius-badge);
  font-size: 11px;
  font-weight: 600;
  border: 1px solid transparent;
}
.status-badge.good {
  color: var(--status-good-fg);
  background: var(--status-good-bg);
  border-color: var(--status-good-border);
}
.status-badge.warn {
  color: var(--status-warn-fg);
  background: var(--status-warn-bg);
  border-color: var(--status-warn-border);
}
.status-badge.danger {
  color: var(--status-danger-fg);
  background: var(--status-danger-bg);
  border-color: var(--status-danger-border);
}
.status-badge.muted {
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--panel-bg) 70%, var(--hover-bg) 30%);
  border-color: var(--border-color);
}

.status-caption {
  min-width: 0;
  font-size: 11px;
  color: var(--text-secondary);
  text-align: right;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.scan-progress-block {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-bottom: 12px;
  padding: 10px 12px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 76%, var(--input-bg) 24%);
}

.scan-progress-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 10px;
}

.scan-progress-stage {
  min-width: 0;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.scan-progress-summary {
  flex-shrink: 0;
  font-size: 11px;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
  font-family: var(--font-mono-identifier);
}

.scan-progress-track {
  position: relative;
  height: 6px;
  border-radius: 999px;
  overflow: hidden;
  background: color-mix(in srgb, var(--border-color) 82%, transparent);
}

.scan-progress-fill {
  height: 100%;
  border-radius: inherit;
  background: color-mix(in srgb, var(--accent-color) 72%, var(--hover-bg) 28%);
  transition: width 0.16s ease;
}

.scan-progress-track.indeterminate .scan-progress-fill.indeterminate {
  width: 38%;
  animation: assetScanProgressSlide 1.35s ease-in-out infinite;
}

@keyframes assetScanProgressSlide {
  0% { transform: translateX(-100%); }
  100% { transform: translateX(320%); }
}

.detail-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.scale-detail-list {
  margin-top: auto;
}
.detail-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.risk-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.risk-row {
  width: 100%;
  padding: 8px 10px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 76%, var(--input-bg) 24%);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  text-align: left;
  cursor: pointer;
  color: var(--text-color);
  font: inherit;
  transition: border-color 0.15s ease, background 0.15s ease;
}
.risk-row:hover,
.risk-row:focus-visible {
  border-color: var(--border-strong);
  background: var(--hover-bg);
}
.risk-row:disabled {
  cursor: progress;
  opacity: 0.7;
}
.risk-row-main {
  min-width: 0;
  display: flex;
  align-items: baseline;
  gap: 10px;
}
.risk-row-name {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}
.risk-row-value {
  font-size: 12px;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}
.risk-row-meta {
  flex-shrink: 0;
  font-size: 11px;
  color: var(--text-secondary);
}
.risk-empty {
  min-height: 88px;
  display: flex;
  align-items: center;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.6;
}
.risk-row-name,
.risk-row-value {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.detail-key {
  font-size: 12px;
  color: var(--text-secondary);
}
.detail-value {
  text-align: right;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  font-variant-numeric: tabular-nums;
}
.detail-value.warn {
  color: var(--status-danger-fg);
}
.detail-value.dim {
  color: var(--text-secondary);
  font-weight: 500;
  font-style: italic;
}
/* ── Watcher tuning controls ── */
.watcher-tuning {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-bottom: 8px;
  padding: 8px 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
}
.tuning-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.tuning-control {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.tuning-label {
  font-size: 11px;
  color: var(--text-secondary);
  flex-shrink: 0;
}
.tuning-readout-val {
  font-size: 11px;
  color: var(--text-color);
  font-variant-numeric: tabular-nums;
  font-family: var(--font-mono-identifier);
  flex-shrink: 0;
  min-width: 88px;
  text-align: right;
}
.tuning-readout-sep {
  margin: 0 4px;
  color: var(--text-secondary);
}
.tuning-readout-unit {
  margin-left: 2px;
  font-size: 10px;
  color: var(--text-secondary);
}
.tuning-scale {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  font-size: 10px;
  color: var(--text-secondary);
  line-height: 1;
}

/* ── Stepless intensity slider (inline) ── */
.intensity-slider {
  -webkit-appearance: none;
  appearance: none;
  flex: 1;
  min-width: 0;
  height: 4px;
  border-radius: 999px;
  background: linear-gradient(
    to right,
    var(--hover-bg),
    color-mix(in srgb, var(--accent-color) 16%, var(--hover-bg) 84%)
  );
  outline: none;
  cursor: pointer;
  margin: 0;
}
.intensity-slider:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.intensity-slider::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: var(--accent-color);
  border: 2px solid var(--bg-color);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
  cursor: pointer;
  transition: transform 0.1s;
}
.intensity-slider::-webkit-slider-thumb:hover {
  transform: scale(1.15);
}
.intensity-slider::-moz-range-thumb {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: var(--accent-color);
  border: 2px solid var(--bg-color);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
  cursor: pointer;
}
.seg-group {
  flex-shrink: 0;
}
.tuning-hint {
  font-size: 10px;
  color: var(--text-secondary);
  line-height: 1.4;
}

.card-desc {
  font-size: 12px;
  line-height: 1.55;
  color: var(--text-secondary);
  margin-bottom: 10px;
}

.watcher-current {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-bottom: 12px;
}
.watcher-current-label {
  font-size: 11px;
  color: var(--text-secondary);
}
.watcher-current-value {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 10px;
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 70%, var(--input-bg) 30%);
  font-size: 12px;
  color: var(--text-color);
  min-width: 0;
}
.watcher-current-value.idle {
  color: var(--text-secondary);
  font-style: italic;
}
.watcher-current-name {
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  font-weight: 400;
  white-space: nowrap;
  flex-shrink: 0;
}
.watcher-current-dir {
  flex: 1;
  min-width: 0;
  color: var(--text-secondary);
  opacity: 0.45;
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.watcher-pulse {
  flex-shrink: 0;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--status-good-fg);
}
.card-note {
  margin-top: auto;
  padding: 10px 12px;
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 74%, var(--input-bg) 26%);
  font-size: 11px;
  line-height: 1.55;
  color: var(--text-secondary);
}

.card-actions {
  display: flex;
  margin-top: auto;
  padding-top: 12px;
}
.card-btn {
  min-width: 0;
}

@media (max-width: 1280px) {
  .overview-grid-summary {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 720px) {
  .asset-overview {
    padding: 14px;
  }
  .overview-grid-summary {
    grid-template-columns: minmax(0, 1fr);
  }
  .mini-grid {
    grid-template-columns: minmax(0, 1fr);
  }
  .scan-progress-row {
    flex-direction: column;
    align-items: flex-start;
  }
  .scan-progress-summary {
    flex-shrink: 1;
  }
}
</style>
