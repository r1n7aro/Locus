<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { locale, t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import { getModelUsageStats } from "../../services/session";
import { useNotificationStore } from "../../stores/notification";
import type { ModelUsageMetrics, ModelUsageReport } from "../../types";
import BaseButton from "../ui/BaseButton.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";

type UsageRange = "7" | "30" | "all";

const notificationStore = useNotificationStore();
const range = ref<UsageRange>("30");
const report = ref<ModelUsageReport | null>(null);
const loading = ref(false);
const loadFailed = ref(false);
let loadSeq = 0;

const rangeOptions = computed(() => [
  { value: "7", label: t("settings.modelUsage.range7") },
  { value: "30", label: t("settings.modelUsage.range30") },
  { value: "all", label: t("settings.modelUsage.rangeAll") },
]);

const summaryItems = computed(() => {
  const usage = report.value?.usage;
  if (!usage) return [];
  return [
    { key: "total", label: t("settings.modelUsage.total"), value: totalTokens(usage) },
    { key: "read", label: t("settings.modelUsage.read"), value: usage.inputTokens },
    { key: "write", label: t("settings.modelUsage.write"), value: usage.outputTokens },
    { key: "cacheRead", label: t("settings.modelUsage.cacheRead"), value: usage.cacheReadTokens },
    { key: "cacheWrite", label: t("settings.modelUsage.cacheWrite"), value: usage.cacheWriteTokens },
  ];
});

const activityLabel = computed(() => {
  const usage = report.value?.usage;
  if (!usage) return "";
  return t(
    "settings.modelUsage.activity",
    formatNumber(usage.requestCount),
    formatNumber(usage.sessionCount),
  );
});

const recordedRangeLabel = computed(() => {
  const from = report.value?.recordedFrom;
  const to = report.value?.recordedTo;
  if (!from || !to) return "";
  return t("settings.modelUsage.recordedRange", formatDate(from), formatDate(to));
});

function selectedDays(): number | null {
  if (range.value === "all") return null;
  return Number(range.value);
}

function totalTokens(usage: ModelUsageMetrics): number {
  return usage.inputTokens
    + usage.outputTokens
    + usage.cacheReadTokens
    + usage.cacheWriteTokens;
}

function formatNumber(value: number): string {
  return value.toLocaleString(locale.value === "zh" ? "zh-CN" : "en-US");
}

function formatCost(value: number): string {
  const digits = value > 0 && value < 0.01 ? 4 : 2;
  return `$${value.toLocaleString("en-US", {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  })}`;
}

function formatDate(timestamp: number): string {
  return new Date(timestamp * 1000).toLocaleDateString(
    locale.value === "zh" ? "zh-CN" : "en-US",
    { year: "numeric", month: "2-digit", day: "2-digit" },
  );
}

async function loadReport() {
  const seq = ++loadSeq;
  loading.value = true;
  try {
    const next = await getModelUsageStats(selectedDays());
    if (seq !== loadSeq) return;
    report.value = next;
    loadFailed.value = false;
  } catch (error) {
    if (seq !== loadSeq) return;
    loadFailed.value = true;
    const normalized = normalizeAppError(error);
    notificationStore.addNotice(
      "error",
      t("settings.modelUsage.loadFailed", normalized.message),
      { code: normalized.code, operation: "loadModelUsageStats" },
    );
  } finally {
    if (seq === loadSeq) loading.value = false;
  }
}

watch(range, () => {
  void loadReport();
});

onMounted(() => {
  void loadReport();
});
</script>

<template>
  <div class="settings-section model-usage-settings">
    <div class="model-usage-heading">
      <div>
        <div class="section-label">{{ t("settings.modelUsage.title") }}</div>
        <p class="section-desc">{{ t("settings.modelUsage.desc") }}</p>
      </div>
      <div class="model-usage-toolbar">
        <BaseSegmented v-model="range" :options="rangeOptions" size="sm" />
        <BaseButton :disabled="loading" @click="loadReport">
          {{ loading ? t("common.loading") : t("common.refresh") }}
        </BaseButton>
      </div>
    </div>

    <div v-if="loading && !report" class="model-usage-empty">
      {{ t("common.loading") }}
    </div>
    <div v-else-if="loadFailed && !report" class="model-usage-empty">
      <span>{{ t("settings.modelUsage.loadFailedTitle") }}</span>
      <BaseButton :disabled="loading" @click="loadReport">{{ t("common.refresh") }}</BaseButton>
    </div>
    <template v-else-if="report">
      <div class="model-usage-context">
        <span>{{ activityLabel }}</span>
        <span v-if="recordedRangeLabel">{{ recordedRangeLabel }}</span>
      </div>

      <section class="model-usage-summary" :aria-label="t('settings.modelUsage.summary')">
        <div v-for="item in summaryItems" :key="item.key" class="model-usage-metric">
          <span class="model-usage-metric-label">{{ item.label }}</span>
          <strong class="model-usage-metric-value">{{ formatNumber(item.value) }}</strong>
        </div>
      </section>

      <section class="model-usage-table-panel">
        <div class="model-usage-table-header">
          <span>{{ t("settings.modelUsage.byModel") }}</span>
          <span>{{ t("settings.modelUsage.reportedCost", formatCost(report.usage.costUsd)) }}</span>
        </div>

        <div v-if="report.byModel.length === 0" class="model-usage-empty model-usage-table-empty">
          {{ t("settings.modelUsage.empty") }}
        </div>
        <div v-else class="model-usage-table-scroll">
          <table class="model-usage-table">
            <thead>
              <tr>
                <th>{{ t("settings.modelUsage.model") }}</th>
                <th class="numeric">{{ t("settings.modelUsage.calls") }}</th>
                <th class="numeric">{{ t("settings.modelUsage.total") }}</th>
                <th class="numeric">{{ t("settings.modelUsage.read") }}</th>
                <th class="numeric">{{ t("settings.modelUsage.write") }}</th>
                <th class="numeric">{{ t("settings.modelUsage.cacheRead") }}</th>
                <th class="numeric">{{ t("settings.modelUsage.cacheWrite") }}</th>
                <th class="numeric">{{ t("settings.modelUsage.cost") }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="group in report.byModel" :key="`${group.provider}:${group.modelId}`">
                <td>
                  <div class="model-usage-model-id">{{ group.modelId }}</div>
                  <div class="model-usage-provider">{{ group.provider }}</div>
                </td>
                <td class="numeric">{{ formatNumber(group.usage.requestCount) }}</td>
                <td class="numeric model-usage-total">{{ formatNumber(totalTokens(group.usage)) }}</td>
                <td class="numeric">{{ formatNumber(group.usage.inputTokens) }}</td>
                <td class="numeric">{{ formatNumber(group.usage.outputTokens) }}</td>
                <td class="numeric">{{ formatNumber(group.usage.cacheReadTokens) }}</td>
                <td class="numeric">{{ formatNumber(group.usage.cacheWriteTokens) }}</td>
                <td class="numeric">{{ formatCost(group.usage.costUsd) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </template>
  </div>
</template>

<style scoped>
.model-usage-settings {
  min-width: 0;
}

.model-usage-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 24px;
}

.model-usage-heading .section-desc {
  margin-bottom: 0;
}

.model-usage-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.model-usage-context {
  min-height: 28px;
  margin-top: 18px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  font-size: 11px;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}

.model-usage-summary {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--panel-bg);
  overflow: hidden;
}

.model-usage-metric {
  min-width: 0;
  padding: 12px 14px;
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.model-usage-metric + .model-usage-metric {
  border-left: 1px solid var(--border-color);
}

.model-usage-metric-label {
  font-size: 11px;
  color: var(--text-secondary);
}

.model-usage-metric-value {
  overflow: hidden;
  text-overflow: ellipsis;
  font-size: 18px;
  line-height: 1.2;
  font-weight: 600;
  color: var(--text-color);
  font-variant-numeric: tabular-nums;
}

.model-usage-table-panel {
  margin-top: 14px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--panel-bg);
  overflow: hidden;
}

.model-usage-table-header {
  min-height: 38px;
  padding: 0 12px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  border-bottom: 1px solid var(--border-color);
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
}

.model-usage-table-header span:last-child {
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 400;
  font-variant-numeric: tabular-nums;
}

.model-usage-table-scroll {
  overflow-x: auto;
}

.model-usage-table {
  width: 100%;
  min-width: 820px;
  border-collapse: collapse;
  table-layout: fixed;
}

.model-usage-table th,
.model-usage-table td {
  height: 40px;
  padding: 7px 10px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  text-align: left;
  font-size: 12px;
}

.model-usage-table th {
  height: 32px;
  background: color-mix(in srgb, var(--sidebar-bg) 64%, var(--panel-bg));
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 500;
  white-space: nowrap;
}

.model-usage-table th:first-child,
.model-usage-table td:first-child {
  width: 28%;
}

.model-usage-table tbody tr:last-child td {
  border-bottom: 0;
}

.model-usage-table tbody tr:hover td {
  background: var(--hover-bg);
}

.model-usage-table .numeric {
  text-align: right;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.model-usage-model-id {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-color);
  font-family: var(--font-mono-block);
  font-size: 12px;
}

.model-usage-provider {
  margin-top: 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-size: 10px;
}

.model-usage-total {
  color: var(--text-color);
  font-weight: 600;
}

.model-usage-empty {
  min-height: 220px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  color: var(--text-secondary);
  font-size: 12px;
  text-align: center;
}

.model-usage-table-empty {
  min-height: 280px;
}

@media (max-width: 920px) {
  .model-usage-heading {
    flex-direction: column;
    gap: 14px;
  }

  .model-usage-summary {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .model-usage-metric + .model-usage-metric {
    border-left: 0;
  }

  .model-usage-metric:nth-child(even) {
    border-left: 1px solid var(--border-color);
  }

  .model-usage-metric:nth-child(n + 3) {
    border-top: 1px solid var(--border-color);
  }

  .model-usage-context {
    align-items: flex-start;
    flex-direction: column;
    gap: 4px;
  }
}
</style>
