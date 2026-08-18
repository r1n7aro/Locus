<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { RefreshCw, X } from "lucide";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { getSessionContextUsageReport } from "../services/session";
import type {
  KnowledgeAccessMode,
  SessionContextUsageReport,
  TokenUsage,
} from "../types";
import LucideIcon from "./icons/LucideIcon.vue";
import { calculateAverageOutputTokensPerSecond } from "./chat/tokenUsageDisplay";

const props = defineProps<{
  sessionId: string;
  modelId?: string | null;
  knowledgeMode?: KnowledgeAccessMode | null;
  tokenUsage: TokenUsage;
}>();

const emit = defineEmits<{
  close: [];
}>();

const panelRef = ref<HTMLElement | null>(null);
const report = ref<SessionContextUsageReport | null>(null);
const loading = ref(false);
const error = ref("");
let loadSequence = 0;
let refreshTimer = 0;

const numberFormatter = new Intl.NumberFormat();
const rateFormatter = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });
const timestampFormatter = new Intl.DateTimeFormat(undefined, {
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});

function formatNumber(value: number): string {
  return numberFormatter.format(Math.max(0, Math.round(value)));
}

function formatCompactTokens(value: number): string {
  const amount = Math.max(0, value);
  if (amount >= 1_000_000) return `${(amount / 1_000_000).toFixed(1)}M`;
  if (amount >= 1_000) return `${(amount / 1_000).toFixed(1)}k`;
  return Math.round(amount).toString();
}

function formatCost(value: number): string {
  if (value >= 1) return `$${value.toFixed(2)}`;
  if (value >= 0.01) return `$${value.toFixed(4)}`;
  return `$${value.toFixed(6)}`;
}

function formatTimestamp(value: number): string {
  return timestampFormatter.format(new Date(value * 1000));
}

function formatMessage(value: string): string {
  const message = value.replace(/\s+/g, " ").trim();
  return message || t("chat.contextStats.emptyMessage");
}

function formatCacheInvalidationReason(reason: string): string {
  switch (reason) {
    case "model_changed":
      return t("chat.contextStats.cacheReason.modelChanged");
    case "provider_changed":
      return t("chat.contextStats.cacheReason.providerChanged");
    case "input_growth_exceeds_context_threshold":
      return t("chat.contextStats.cacheReason.inputGrowthExceeded");
    default:
      return t("chat.contextStats.cacheReason.unknown");
  }
}

function formatOutputSpeed(usage: TokenUsage): string {
  const tokensPerSecond = calculateAverageOutputTokensPerSecond(usage);
  if (tokensPerSecond === null) {
    return t("chat.contextStats.unavailable");
  }
  return t("chat.contextStats.outputSpeedValue", rateFormatter.format(tokensPerSecond));
}

const contextPercent = computed(() => {
  const current = report.value;
  if (!current || current.contextLimit <= 0) return 0;
  return Math.min(100, (current.contextTokens / current.contextLimit) * 100);
});

const contextProgressStyle = computed(() => ({ width: `${contextPercent.value}%` }));

const inputConsumptionParts = computed(() => {
  const breakdown = report.value?.breakdown;
  if (!breakdown) return [];
  return [
    { key: "system", label: t("chat.contextStats.systemPrompt"), tokens: breakdown.systemPromptTokens },
    { key: "environment", label: t("chat.contextStats.environment"), tokens: breakdown.environmentTokens },
    { key: "rules", label: t("chat.contextStats.rules"), tokens: breakdown.rulesTokens },
    { key: "knowledge", label: t("chat.contextStats.knowledge"), tokens: breakdown.knowledgeTokens },
    { key: "runtime", label: t("chat.contextStats.runtimeInjection"), tokens: breakdown.runtimeInjectionTokens },
    { key: "conversation", label: t("chat.contextStats.conversation"), tokens: breakdown.conversationTokens },
    { key: "toolDefinitions", label: t("chat.contextStats.toolDefinitions"), tokens: breakdown.toolDefinitionTokens },
    { key: "toolResults", label: t("chat.contextStats.activeToolResults"), tokens: breakdown.activeToolResultTokens },
  ];
});

const tokenMetrics = computed(() => {
  const usage = report.value?.usage;
  if (!usage) return [];
  return [
    { key: "input", label: t("chat.contextStats.input"), value: formatNumber(usage.totalInputTokens) },
    { key: "output", label: t("chat.contextStats.output"), value: formatNumber(usage.totalOutputTokens) },
    { key: "cacheRead", label: t("chat.contextStats.cacheRead"), value: formatNumber(usage.totalCacheReadTokens) },
    { key: "cacheWrite", label: t("chat.contextStats.cacheWrite"), value: formatNumber(usage.totalCacheWriteTokens) },
    {
      key: "outputSpeed",
      label: t("chat.contextStats.outputSpeed"),
      value: formatOutputSpeed(usage),
      title: t("chat.contextStats.outputSpeedHelp"),
    },
  ];
});

const toolResultSummary = computed(() => {
  const tools = report.value?.tools ?? [];
  return {
    calls: tools.reduce((total, tool) => total + tool.callCount, 0),
    tokens: tools.reduce((total, tool) => total + tool.resultTokens, 0),
  };
});

async function loadReport(silent = false) {
  const sessionId = props.sessionId.trim();
  if (!sessionId) return;
  const sequence = ++loadSequence;
  if (!silent || !report.value) loading.value = true;
  error.value = "";
  try {
    const next = await getSessionContextUsageReport(
      sessionId,
      props.modelId,
      props.knowledgeMode,
    );
    if (sequence !== loadSequence) return;
    report.value = next;
  } catch (cause) {
    if (sequence !== loadSequence) return;
    error.value = normalizeAppError(cause).message;
  } finally {
    if (sequence === loadSequence) loading.value = false;
  }
}

function scheduleRefresh() {
  window.clearTimeout(refreshTimer);
  refreshTimer = window.setTimeout(() => {
    void loadReport(true);
  }, 240);
}

function closeOverlay() {
  emit("close");
}

function handleKeydown(event: KeyboardEvent) {
  if (event.key !== "Escape") return;
  event.preventDefault();
  event.stopImmediatePropagation();
  closeOverlay();
}

watch(
  () => [
    props.sessionId,
    props.modelId,
    props.knowledgeMode,
  ],
  () => {
    report.value = null;
    void loadReport();
  },
);

watch(
  () => [
    props.tokenUsage.totalInputTokens,
    props.tokenUsage.totalOutputTokens,
    props.tokenUsage.totalCacheReadTokens,
    props.tokenUsage.totalCacheWriteTokens,
    props.tokenUsage.timedOutputTokens,
    props.tokenUsage.modelActiveDurationMs,
    props.tokenUsage.contextTokens,
    props.tokenUsage.contextLimit,
  ],
  scheduleRefresh,
);

onMounted(async () => {
  window.addEventListener("keydown", handleKeydown, true);
  await loadReport();
  await nextTick();
  panelRef.value?.focus();
});

onUnmounted(() => {
  window.removeEventListener("keydown", handleKeydown, true);
  window.clearTimeout(refreshTimer);
  loadSequence += 1;
});
</script>

<template>
  <div class="context-stats-backdrop" @click.self="closeOverlay">
    <main
      ref="panelRef"
      class="context-stats-window"
      role="dialog"
      aria-modal="true"
      :aria-label="t('chat.contextStats.windowTitle')"
      tabindex="-1"
    >
    <header class="context-stats-titlebar">
      <div class="context-stats-title">
        <span class="context-stats-title-main">{{ t("chat.contextStats.windowTitle") }}</span>
        <span v-if="report?.sessionTitle" class="context-stats-session" :title="report.sessionTitle">
          {{ report.sessionTitle }}
        </span>
      </div>
      <div class="context-stats-window-actions">
        <button
          type="button"
          class="context-stats-icon-button"
          :title="t('common.refresh')"
          :aria-label="t('common.refresh')"
          :disabled="loading"
          @click="loadReport()"
        >
          <LucideIcon :icon="RefreshCw" :size="14" :class="{ spinning: loading }" />
        </button>
        <button
          type="button"
          class="context-stats-icon-button"
          :title="t('app.win.close')"
          :aria-label="t('app.win.close')"
          @click="closeOverlay"
        >
          <LucideIcon :icon="X" :size="14" />
        </button>
      </div>
    </header>

    <div class="context-stats-body">
      <div v-if="error && !report" class="context-stats-state">
        <span>{{ t("chat.contextStats.loadFailed") }}</span>
        <span class="context-stats-state-detail">{{ error }}</span>
      </div>
      <div v-else-if="loading && !report" class="context-stats-state">
        {{ t("common.loading") }}
      </div>

      <template v-else-if="report">
        <section class="context-overview" :aria-label="t('chat.contextStats.contextUsage')">
          <div class="context-section-heading">
            <div>
              <h1>{{ t("chat.contextStats.contextUsage") }}</h1>
              <div class="context-section-meta">{{ report.modelId }} · {{ report.agentId }}</div>
            </div>
            <div class="context-overview-value">
              <strong>{{ formatCompactTokens(report.contextTokens) }} / {{ formatCompactTokens(report.contextLimit) }}</strong>
              <span>{{ contextPercent.toFixed(1) }}%</span>
            </div>
          </div>
          <div class="context-progress-track" aria-hidden="true">
            <span class="context-progress-fill" :style="contextProgressStyle"></span>
          </div>
        </section>

        <section class="context-token-section" :aria-label="t('chat.contextStats.tokenUsage')">
          <div class="context-section-heading context-token-heading">
            <h2>{{ t("chat.contextStats.tokenUsage") }}</h2>
            <span v-if="report.usage.pricedRounds > 0" class="context-section-meta">
              {{ t("chat.contextStats.reportedCost", formatCost(report.usage.totalCostUsd)) }}
            </span>
          </div>
          <div class="context-token-metrics">
            <div
              v-for="metric in tokenMetrics"
              :key="metric.key"
              class="context-token-metric"
              :title="metric.title"
            >
              <span>{{ metric.label }}</span>
              <strong>{{ metric.value }}</strong>
            </div>
          </div>
        </section>

        <section class="context-cache-section" :aria-label="t('chat.contextStats.cacheInvalidations')">
          <div class="context-section-heading">
            <h2>{{ t("chat.contextStats.cacheInvalidations") }}</h2>
            <span class="context-section-meta">
              {{ t("chat.contextStats.cacheInvalidationSummary", report.cacheInvalidations.length) }}
            </span>
          </div>
          <div v-if="report.cacheInvalidations.length === 0" class="context-tools-empty">
            {{ t("chat.contextStats.noCacheInvalidations") }}
          </div>
          <div v-else class="context-tools-table-wrap context-cache-table-wrap">
            <table class="context-tools-table context-cache-table">
              <thead>
                <tr>
                  <th>{{ t("chat.contextStats.invalidatedAt") }}</th>
                  <th>{{ t("chat.contextStats.recentMessage") }}</th>
                  <th>{{ t("chat.contextStats.cacheReason") }}</th>
                  <th class="numeric">{{ t("chat.contextStats.cacheBaseline") }}</th>
                  <th class="numeric">{{ t("chat.contextStats.cacheExcessInput") }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="event in report.cacheInvalidations" :key="event.messageId">
                  <td class="context-cache-time">{{ formatTimestamp(event.occurredAt) }}</td>
                  <td
                    class="context-cache-message"
                    :title="`${event.modelId}\n${event.message}`"
                  >
                    {{ formatMessage(event.message) }}
                  </td>
                  <td>{{ formatCacheInvalidationReason(event.reason) }}</td>
                  <td class="numeric">{{ formatNumber(event.baselineTokens) }}</td>
                  <td class="numeric">{{ formatNumber(event.excessInputTokens) }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>

        <section class="context-tools-section" :aria-label="t('chat.contextStats.tools')">
          <div class="context-section-heading">
            <h2>{{ t("chat.contextStats.tools") }}</h2>
            <span class="context-section-meta">
              {{ t(
                "chat.contextStats.toolSummary",
                toolResultSummary.calls,
                formatNumber(toolResultSummary.tokens),
              ) }}
            </span>
          </div>
          <div v-if="report.tools.length === 0" class="context-tools-empty">
            {{ t("chat.contextStats.noToolResults") }}
          </div>
          <div v-else class="context-tools-table-wrap">
            <table class="context-tools-table">
              <thead>
                <tr>
                  <th>{{ t("chat.contextStats.tool") }}</th>
                  <th class="numeric">{{ t("chat.contextStats.calls") }}</th>
                  <th class="numeric">{{ t("chat.contextStats.resultTokens") }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="tool in report.tools" :key="tool.name">
                  <td class="context-tool-name">{{ tool.name }}</td>
                  <td class="numeric context-tool-calls">{{ formatNumber(tool.callCount) }}</td>
                  <td class="numeric">{{ formatNumber(tool.resultTokens) }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>

        <section class="context-breakdown-section" :aria-label="t('chat.contextStats.inputBreakdown')">
          <div class="context-section-heading">
            <h2>{{ t("chat.contextStats.inputBreakdown") }}</h2>
            <span class="context-section-meta">{{ t("chat.contextStats.estimated") }}</span>
          </div>
          <div class="context-breakdown-list">
            <div
              v-for="part in inputConsumptionParts"
              :key="part.key"
              class="context-breakdown-row"
            >
              <span class="context-breakdown-label">{{ part.label }}</span>
              <strong class="context-breakdown-value">{{ formatNumber(part.tokens) }}</strong>
            </div>
          </div>
        </section>
      </template>
    </div>

    <footer v-if="report" class="context-stats-footer">
      {{ t("chat.contextStats.consumptionNote") }}
    </footer>
    </main>
  </div>
</template>

<style scoped>
.context-stats-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1200;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background: color-mix(in srgb, var(--bg-color) 54%, transparent);
}

.context-stats-window {
  width: min(920px, calc(100vw - 48px));
  height: min(720px, calc(100vh - 48px));
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
  color: var(--text-color);
  border: 1px solid var(--border-strong);
  border-radius: 10px;
  box-shadow: 0 18px 36px color-mix(in srgb, var(--bg-color) 64%, transparent);
  outline: none;
}

.context-stats-titlebar {
  min-height: 38px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 8px 0 14px;
  background: var(--sidebar-bg);
  border-bottom: 1px solid var(--border-color);
}

.context-stats-title {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.context-stats-title-main {
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 600;
}

.context-stats-session {
  min-width: 0;
  overflow: hidden;
  color: var(--text-secondary);
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.context-stats-window-actions {
  display: flex;
  align-items: center;
}

.context-stats-icon-button {
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.context-stats-icon-button:hover,
.context-stats-icon-button:focus-visible {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
  outline: none;
}

.context-stats-icon-button:disabled {
  opacity: 0.55;
}

.context-stats-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 18px 20px 22px;
}

.context-stats-state {
  min-height: 180px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  color: var(--text-secondary);
  font-size: 13px;
}

.context-stats-state-detail {
  max-width: 560px;
  color: var(--status-error-fg);
  text-align: center;
}

.context-overview,
.context-token-section,
.context-breakdown-section,
.context-cache-section,
.context-tools-section {
  max-width: 980px;
  margin: 0 auto;
}

.context-token-section,
.context-breakdown-section,
.context-cache-section,
.context-tools-section {
  margin-top: 20px;
}

.context-section-heading {
  min-height: 26px;
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 8px;
}

.context-section-heading h1,
.context-section-heading h2 {
  margin: 0;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
  line-height: 1.5;
}

.context-section-meta {
  overflow: hidden;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  line-height: 1.5;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.context-overview-value {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-family: var(--font-mono-identifier);
}

.context-overview-value strong {
  font-size: 15px;
  font-weight: 600;
}

.context-overview-value span {
  color: var(--text-secondary);
  font-size: 11px;
}

.context-progress-track {
  height: 5px;
  display: block;
  overflow: hidden;
  border-radius: 3px;
  background: var(--hover-bg);
}

.context-progress-fill {
  height: 100%;
  display: block;
  border-radius: inherit;
  background: var(--accent-color);
  transition: width 0.18s ease;
}

.context-token-heading {
  align-items: baseline;
}

.context-token-metrics {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--sidebar-bg);
}

.context-token-metric {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 5px;
  padding: 10px 12px;
}

.context-token-metric + .context-token-metric {
  border-left: 1px solid var(--border-color);
}

.context-token-metric span {
  color: var(--text-secondary);
  font-size: 11px;
}

.context-token-metric strong {
  overflow: hidden;
  font-family: var(--font-mono-identifier);
  font-size: 13px;
  font-weight: 600;
  text-overflow: ellipsis;
}

.context-breakdown-list,
.context-tools-table-wrap,
.context-tools-empty {
  overflow: hidden;
  border: 1px solid var(--border-color);
  border-radius: 8px;
}

.context-breakdown-row {
  min-height: 34px;
  display: grid;
  grid-template-columns: minmax(120px, 1fr) 120px;
  align-items: center;
  gap: 12px;
  padding: 0 12px;
  font-size: 12px;
}

.context-breakdown-row + .context-breakdown-row {
  border-top: 1px solid var(--border-color);
}

.context-breakdown-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.context-breakdown-value {
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  font-weight: 500;
  text-align: right;
}

.context-tools-table-wrap {
  max-height: 300px;
  overflow: auto;
}

.context-tools-table {
  width: 100%;
  border-collapse: collapse;
  table-layout: fixed;
  font-size: 12px;
}

.context-tools-table th,
.context-tools-table td {
  height: 32px;
  padding: 0 12px;
  border-bottom: 1px solid var(--border-color);
  text-align: left;
}

.context-tools-table th {
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--sidebar-bg);
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 500;
}

.context-tools-table th:nth-child(2),
.context-tools-table td:nth-child(2) {
  width: 80px;
}

.context-tools-table th:nth-child(3),
.context-tools-table td:nth-child(3) {
  width: 140px;
}

.context-tools-table tbody tr:last-child td {
  border-bottom: 0;
}

.context-tools-table tbody tr:hover td {
  background: var(--hover-bg);
}

.context-tools-table .numeric {
  font-family: var(--font-mono-identifier);
  text-align: right;
}

.context-tool-name {
  overflow: hidden;
  font-family: var(--font-mono-identifier);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.context-tool-calls {
  color: var(--text-secondary);
}

.context-cache-table-wrap {
  max-height: 240px;
}

.context-cache-table th:first-child,
.context-cache-table td:first-child {
  width: 154px;
}

.context-cache-table th:nth-child(2),
.context-cache-table td:nth-child(2) {
  width: auto;
}

.context-cache-table th:nth-child(3),
.context-cache-table td:nth-child(3),
.context-cache-table th:nth-child(4),
.context-cache-table td:nth-child(4) {
  width: 110px;
}

.context-cache-time {
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  white-space: nowrap;
}

.context-cache-message {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.context-tools-empty {
  padding: 24px 12px;
  color: var(--text-secondary);
  font-size: 12px;
  text-align: center;
}

.context-stats-footer {
  flex-shrink: 0;
  padding: 8px 14px;
  border-top: 1px solid var(--border-color);
  background: var(--sidebar-bg);
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1.4;
}

.spinning {
  animation: context-stats-spin 0.8s linear infinite;
}

@keyframes context-stats-spin {
  to { transform: rotate(360deg); }
}

@media (max-width: 720px) {
  .context-token-metrics {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .context-token-metric:nth-child(odd) {
    border-left: 0;
  }

  .context-token-metric:nth-child(n + 3) {
    border-top: 1px solid var(--border-color);
  }

  .context-breakdown-row {
    grid-template-columns: minmax(110px, 1fr) 90px;
  }
}
</style>
