<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { Box, Database, type IconNode } from "lucide";
import { t } from "../../i18n";
import type { AssetDbScanEvent, ScanStats } from "../../types";
import BaseButton from "../ui/BaseButton.vue";

type StatusId = "assetDb" | "unity";
type StatusTone = "success" | "danger" | "accent" | "muted";
type StatusIcon = "database" | "unity";
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
  actionLabel?: string;
  actionTitle?: string;
  actionDisabled?: boolean;
  actionVariant?: "neutral" | "primary" | "danger";
}

const STATUS_ICONS: Record<StatusIcon, IconNode> = {
  database: Database,
  unity: Box,
};

const props = defineProps<{
  unityConnected?: boolean;
  unityPluginStatus?: UnityPluginNotice | null;
  unityPluginInstalling?: boolean;
  unityLaunching?: boolean;
  unityLaunchState?: UnityLaunchState;
  workingDir?: string;
  isUnityProject?: boolean;
  scanPhase?: AssetDbScanEvent | null;
  lastScanStats?: ScanStats | null;
}>();

const emit = defineEmits<{
  startScan: [];
  installPlugin: [];
  launchUnityProject: [];
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
  props.unityConnected ? unityPipeNameForWorkingDir(unityWorkingDir.value) : "",
);

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

const unitySummary = computed(() => {
  if (unityPluginLabel.value) return unityPluginLabel.value;
  if (effectiveUnityLaunchState.value === "starting") return t("chat.unity.launching");
  if (effectiveUnityLaunchState.value === "waitingConnection") return t("chat.unity.waitingConnection");
  return props.unityConnected ? t("chat.unity.connected") : t("chat.unity.disconnected");
});

const unityTone = computed<StatusTone>(() =>
  props.unityPluginStatus
    ? "danger"
    : props.unityConnected
      ? "success"
      : effectiveUnityLaunchState.value !== "idle"
        ? "accent"
        : "danger",
);

const unityCanLaunch = computed(() =>
  !!props.isUnityProject && !props.unityConnected && !props.unityPluginStatus,
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
  if (!phase || (phase.phase !== "metaParse" && phase.phase !== "yamlParse")) return null;
  return {
    label: t("chat.status.assetDb.progress"),
    value: `${phase.completed} / ${phase.total}`,
  };
}

const assetRows = computed<StatusDetailRow[]>(() => {
  const rows: StatusDetailRow[] = [];

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

const unityRows = computed<StatusDetailRow[]>(() => {
  const rows: StatusDetailRow[] = [];
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
      : effectiveUnityLaunchState.value !== "idle" || !props.isUnityProject,
    actionVariant: props.unityPluginStatus ? "neutral" : "primary",
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
}

function closePopover() {
  activePopover.value = null;
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
        :class="[
          `tone-${item.tone}`,
          {
            active: activePopover === item.id,
            'is-scanning': item.id === 'assetDb' && isScanning,
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
