<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { t } from "../../i18n";
import type { AssetDbScanEvent, ScanStats } from "../../types";

type StatusId = "assetDb" | "unity";
type StatusTone = "success" | "danger" | "accent" | "muted";
type StatusIcon = "database" | "unity";

interface StatusDetailRow {
  label: string;
  value: string;
}

interface StatusItem {
  id: StatusId;
  icon: StatusIcon;
  title: string;
  summary: string;
  tone: StatusTone;
  rows: StatusDetailRow[];
  actionLabel?: string;
  actionTitle?: string;
  actionDisabled?: boolean;
}

const props = defineProps<{
  unityConnected?: boolean;
  isUnityProject?: boolean;
  scanPhase?: AssetDbScanEvent | null;
  lastScanStats?: ScanStats | null;
}>();

const emit = defineEmits<{
  startScan: [];
}>();

const activePopover = ref<StatusId | null>(null);

const isScanning = computed(() => {
  const p = props.scanPhase;
  return p != null && p.phase !== "done" && p.phase !== "error";
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
    case "done": return "";
    case "error": return t("chat.assetDb.scanning.error", p.error.message);
  }
});

const scanSummary = computed(() => {
  const s = props.lastScanStats;
  if (!s) return "";
  return t("chat.assetDb.summary", s.nodesAdded, s.edgesAdded);
});

const assetStatusLabel = computed(() => {
  if (isScanning.value) return scanLabel.value;
  if (scanError.value) return scanError.value.message;
  if (scanSummary.value) return scanSummary.value;
  return props.isUnityProject ? t("chat.assetDb.notBuilt") : t("chat.status.assetDb.noWorkspace");
});

const assetTone = computed<StatusTone>(() => {
  if (scanError.value) return "danger";
  if (isScanning.value) return "accent";
  if (scanSummary.value) return "success";
  return "muted";
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
  if (!phase || (phase.phase !== "metaParse" && phase.phase !== "yamlParse")) return null;
  return {
    label: t("chat.status.assetDb.progress"),
    value: `${phase.completed} / ${phase.total}`,
  };
}

const assetRows = computed<StatusDetailRow[]>(() => {
  const rows: StatusDetailRow[] = [
    { label: t("chat.status.detail.status"), value: assetStatusLabel.value },
  ];

  const progress = scanProgressRow(props.scanPhase);
  if (progress) rows.push(progress);

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

const unityRows = computed<StatusDetailRow[]>(() => [
  {
    label: t("chat.status.detail.status"),
    value: props.unityConnected ? t("chat.unity.connected") : t("chat.unity.disconnected"),
  },
]);

const statusItems = computed<StatusItem[]>(() => [
  {
    id: "assetDb",
    icon: "database",
    title: t("chat.status.assetDb.title"),
    summary: assetStatusLabel.value,
    tone: assetTone.value,
    rows: assetRows.value,
    actionLabel: assetActionLabel.value,
    actionTitle: assetActionTitle.value,
    actionDisabled: !props.isUnityProject || isScanning.value,
  },
  {
    id: "unity",
    icon: "unity",
    title: t("chat.status.unity.title"),
    summary: props.unityConnected ? t("chat.unity.connected") : t("chat.unity.disconnected"),
    tone: props.unityConnected ? "success" : "danger",
    rows: unityRows.value,
  },
]);

const activeItem = computed(() =>
  statusItems.value.find((item) => item.id === activePopover.value) ?? null,
);

function togglePopover(id: StatusId) {
  activePopover.value = activePopover.value === id ? null : id;
}

function closePopover() {
  activePopover.value = null;
}

function onDocumentKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") {
    closePopover();
  }
}

onMounted(() => {
  document.addEventListener("click", closePopover);
  document.addEventListener("keydown", onDocumentKeydown);
});

onUnmounted(() => {
  document.removeEventListener("click", closePopover);
  document.removeEventListener("keydown", onDocumentKeydown);
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
        :class="[`tone-${item.tone}`, { active: activePopover === item.id }]"
        :title="item.summary"
        :aria-label="item.title"
        :aria-expanded="activePopover === item.id"
        @click="togglePopover(item.id)"
      >
        <svg
          v-if="item.icon === 'database'"
          viewBox="0 0 16 16"
          width="14"
          height="14"
          fill="none"
          aria-hidden="true"
        >
          <ellipse cx="8" cy="3.5" rx="4.7" ry="2" stroke="currentColor" stroke-width="1.3" />
          <path d="M3.3 3.5v8.8c0 1.1 2.1 2.2 4.7 2.2s4.7-1.1 4.7-2.2V3.5" stroke="currentColor" stroke-width="1.3" />
          <path d="M3.3 8c0 1.1 2.1 2.2 4.7 2.2s4.7-1.1 4.7-2.2" stroke="currentColor" stroke-width="1.3" />
        </svg>
        <svg
          v-else
          viewBox="0 0 16 16"
          width="14"
          height="14"
          fill="none"
          aria-hidden="true"
        >
          <path d="M8 1.4 2.2 4.7v6.6L8 14.6l5.8-3.3V4.7L8 1.4Z" stroke="currentColor" stroke-width="1.25" stroke-linejoin="round" />
          <path d="M8 1.4v5.2m5.8-1.9L8 6.6m-5.8-1.9L8 6.6m0 8V9.4" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" />
          <path d="m2.2 11.3 3.1-1.8m8.5 1.8-3.1-1.8" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" />
        </svg>
      </button>
    </div>

    <Transition name="status-popover">
      <div
        v-if="activeItem"
        class="chat-status-popover"
        role="dialog"
        :aria-label="activeItem.title"
        @click.stop
      >
        <div class="chat-status-popover-head">
          <span class="chat-status-popover-title">{{ activeItem.title }}</span>
          <span class="chat-status-popover-summary" :class="`tone-${activeItem.tone}`">
            {{ activeItem.summary }}
          </span>
        </div>
        <dl class="chat-status-detail-list">
          <template v-for="row in activeItem.rows" :key="`${row.label}:${row.value}`">
            <dt>{{ row.label }}</dt>
            <dd>{{ row.value }}</dd>
          </template>
        </dl>
        <button
          v-if="activeItem.id === 'assetDb' && activeItem.actionLabel"
          type="button"
          class="chat-status-action ui-select-none"
          :disabled="activeItem.actionDisabled"
          :title="activeItem.actionTitle"
          @click="emit('startScan'); closePopover()"
        >
          {{ activeItem.actionLabel }}
        </button>
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
.chat-status-icon-btn.active {
  background: var(--hover-bg);
  border-color: color-mix(in srgb, currentColor 22%, transparent);
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
  align-items: flex-start;
  gap: 8px;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--border-color);
}

.chat-status-popover-title {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  font-weight: 600;
}

.chat-status-popover-summary {
  min-width: 0;
  max-width: 190px;
  font-size: 11px;
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

.chat-status-action {
  margin-top: 10px;
  min-height: 26px;
  padding: 0 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: transparent;
  color: var(--text-color);
  font-size: 12px;
  cursor: pointer;
  box-shadow: none;
}

.chat-status-action:hover:not(:disabled) {
  background: var(--hover-bg);
  border-color: var(--border-strong, var(--border-color));
}

.chat-status-action:disabled {
  cursor: not-allowed;
  opacity: 0.5;
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
</style>
