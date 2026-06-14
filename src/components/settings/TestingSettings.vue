<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { t } from "../../i18n";
import { useCopyFeedback } from "../../composables/useCopyFeedback";
import { normalizeAppError } from "../../services/errors";
import {
  cancelUnityIntegrationTests,
  runUnityIntegrationTests,
  subscribeUnityIntegrationTests,
  type TypeIndexSampleMode,
  type UnityIntegrationSuite,
  type UnityIntegrationTestEvent,
} from "../../services/integrationTests";
import type { RuntimeUnsubscribe } from "../../services/locusRuntime";
import { useNotificationStore } from "../../stores/notification";
import BaseButton from "../ui/BaseButton.vue";
import BaseCheckbox from "../ui/BaseCheckbox.vue";

type SuiteStatus = "idle" | "queued" | "running" | "passed" | "failed" | "cancelled";
type RunState = "idle" | "running" | "passed" | "failed" | "cancelled";

interface SuiteItem {
  id: UnityIntegrationSuite;
  labelKey: string;
  descKey: string;
}

interface SuiteState {
  status: SuiteStatus;
  passed: number;
  failed: number;
  detail: string;
}

const notificationStore = useNotificationStore();

const suiteItems: SuiteItem[] = [
  { id: "connect", labelKey: "settings.testing.suite.connect", descKey: "settings.testing.suite.connectDesc" },
  { id: "sidecar", labelKey: "settings.testing.suite.sidecar", descKey: "settings.testing.suite.sidecarDesc" },
  { id: "type-index", labelKey: "settings.testing.suite.typeIndex", descKey: "settings.testing.suite.typeIndexDesc" },
  { id: "state-probe", labelKey: "settings.testing.suite.stateProbe", descKey: "settings.testing.suite.stateProbeDesc" },
  { id: "native-bridge", labelKey: "settings.testing.suite.nativeBridge", descKey: "settings.testing.suite.nativeBridgeDesc" },
  { id: "hot-reload", labelKey: "settings.testing.suite.hotReload", descKey: "settings.testing.suite.hotReloadDesc" },
  { id: "execute", labelKey: "settings.testing.suite.execute", descKey: "settings.testing.suite.executeDesc" },
];

const running = ref(false);
const cancelling = ref(false);
const currentRunId = ref("");
const runSummary = ref("");
const runState = ref<RunState>("idle");
const autoFollow = ref(true);
const selectedDetailId = ref<UnityIntegrationSuite>(suiteItems[0].id);
const consoleRef = ref<HTMLElement | null>(null);

const suiteLogs = reactive<Record<UnityIntegrationSuite, string[]>>(emptySuiteLogs());
const selectedSuites = reactive<Record<UnityIntegrationSuite, boolean>>(
  Object.fromEntries(suiteItems.map((suite) => [suite.id, true])) as Record<UnityIntegrationSuite, boolean>,
);
const suiteStates = reactive<Record<UnityIntegrationSuite, SuiteState>>(
  Object.fromEntries(suiteItems.map((suite) => [suite.id, idleSuiteState()])) as Record<UnityIntegrationSuite, SuiteState>,
);

const installPlugin = ref(true);
const openUnity = ref(true);
const forceEditMode = ref(true);
const typeIndexSampleMode = ref<TypeIndexSampleMode>("sample32");
const { copied: outputCopied, copyText: copyOutputText } = useCopyFeedback();

let unsubscribeIntegrationTests: RuntimeUnsubscribe | null = null;

const selectedSuiteIds = computed(() => suiteItems
  .filter((suite) => selectedSuites[suite.id])
  .map((suite) => suite.id));

const canRun = computed(() => !running.value && selectedSuiteIds.value.length > 0);

const selectedItem = computed(() => suiteItems.find((suite) => suite.id === selectedDetailId.value) ?? suiteItems[0]);
const selectedState = computed(() => suiteStates[selectedDetailId.value]);
const selectedLogs = computed(() => suiteLogs[selectedDetailId.value]);
const selectedLogText = computed(() => selectedLogs.value.join("\n"));

function idleSuiteState(): SuiteState {
  return { status: "idle", passed: 0, failed: 0, detail: "" };
}

function emptySuiteLogs(): Record<UnityIntegrationSuite, string[]> {
  const logs = {} as Record<UnityIntegrationSuite, string[]>;
  for (const suite of suiteItems) {
    logs[suite.id] = [];
  }
  return logs;
}

function resetSuiteStates(suites: UnityIntegrationSuite[]) {
  const selected = new Set(suites);
  for (const suite of suiteItems) {
    suiteStates[suite.id] = {
      status: selected.has(suite.id) ? "queued" : "idle",
      passed: 0,
      failed: 0,
      detail: "",
    };
    suiteLogs[suite.id] = [];
  }
}

function setRunStatus(state: RunState, summary: string) {
  runState.value = state;
  runSummary.value = summary;
}

function setAllSuites(value: boolean) {
  for (const suite of suiteItems) {
    selectedSuites[suite.id] = value;
  }
}

function selectDetail(id: UnityIntegrationSuite) {
  selectedDetailId.value = id;
  // A manual pick pins the detail pane so the live run no longer steals focus.
  autoFollow.value = false;
  scrollConsoleToBottom();
}

function scrollConsoleToBottom() {
  void nextTick(() => {
    const el = consoleRef.value;
    if (el) el.scrollTop = el.scrollHeight;
  });
}

async function runSelectedSuites() {
  await startRun(selectedSuiteIds.value);
}

async function runAllSuites() {
  const suites = suiteItems.map((suite) => suite.id);
  for (const suite of suiteItems) {
    selectedSuites[suite.id] = true;
  }
  await startRun(suites);
}

async function startRun(suites: UnityIntegrationSuite[]) {
  if (suites.length === 0 || running.value) return;

  resetSuiteStates(suites);
  autoFollow.value = true;
  cancelling.value = false;
  selectedDetailId.value = suites[0];
  running.value = true;
  setRunStatus("running", t("settings.testing.started"));
  try {
    const started = await runUnityIntegrationTests({
      suites,
      openUnity: openUnity.value,
      installPlugin: installPlugin.value,
      forceEditMode: forceEditMode.value,
      typeIndexSampleMode: typeIndexSampleMode.value,
      connectTimeoutMs: 60_000,
      suiteTimeoutMs: 1_200_000,
      pollMs: 500,
      noProgressTimeoutMs: 20_000,
    });
    currentRunId.value = started.runId;
  } catch (e) {
    running.value = false;
    const err = normalizeAppError(e);
    appendErrorLine(err.message);
    setRunStatus("failed", err.message);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "unityIntegrationTestRun",
      replaceOperation: true,
    });
  }
}

async function interruptRun() {
  if (!running.value || cancelling.value) return;
  cancelling.value = true;
  setRunStatus("cancelled", t("settings.testing.interrupting"));
  try {
    await cancelUnityIntegrationTests();
  } catch (e) {
    cancelling.value = false;
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "unityIntegrationTestCancel",
      replaceOperation: true,
    });
  }
}

function appendErrorLine(message: string) {
  const active = suiteItems.find((suite) => suiteStates[suite.id].status === "running");
  const target = active ? active.id : selectedDetailId.value;
  const line = `ERROR ${message}`;
  const previousLine = suiteLogs[target][suiteLogs[target].length - 1];
  if (previousLine === line) return;
  suiteLogs[target] = [...suiteLogs[target], line];
  if (target === selectedDetailId.value) scrollConsoleToBottom();
}

function markActiveSuitesCancelled() {
  for (const suite of suiteItems) {
    const state = suiteStates[suite.id];
    if (state.status === "running" || state.status === "queued") {
      suiteStates[suite.id] = { ...state, status: "cancelled" };
    }
  }
}

function handleIntegrationEvent(event: UnityIntegrationTestEvent) {
  if (currentRunId.value && event.runId !== currentRunId.value) return;
  if (!currentRunId.value) currentRunId.value = event.runId;

  const suite = suiteFromPayload(event.payload);

  switch (event.event) {
    case "unity_launch":
      setRunStatus("running", t("settings.testing.launchingUnity"));
      return;
    case "waiting_connection": {
      const secs = Math.round(numberPayload(event.payload, "elapsedMs") / 1000);
      setRunStatus("running", t("settings.testing.connecting", secs));
      return;
    }
    case "connected":
      setRunStatus("running", t("settings.testing.connected"));
      return;
    case "suite_start":
      if (suite) {
        suiteStates[suite] = { ...suiteStates[suite], status: "running" };
        if (!cancelling.value) setRunStatus("running", t("settings.testing.runningSuite", labelForSuite(suite)));
        if (autoFollow.value) {
          selectedDetailId.value = suite;
          scrollConsoleToBottom();
        }
      }
      return;
    case "suite_event":
      if (suite) {
        const line = stringPayload(event.payload, "line");
        if (line) {
          suiteLogs[suite] = [...suiteLogs[suite], line];
          if (suite === selectedDetailId.value) scrollConsoleToBottom();
        }
      }
      return;
    case "suite_result":
      if (suite) {
        const passed = numberPayload(event.payload, "passed");
        const failed = numberPayload(event.payload, "failed");
        suiteStates[suite] = {
          status: failed > 0 ? "failed" : "passed",
          passed,
          failed,
          detail: resultDetail(event.payload),
        };
      }
      return;
    case "error": {
      const message = stringPayload(event.payload, "message") || t("settings.testing.failed");
      appendErrorLine(message);
      setRunStatus("failed", message);
      running.value = false;
      cancelling.value = false;
      return;
    }
    case "cancelled":
      markActiveSuitesCancelled();
      setRunStatus("cancelled", t("settings.testing.cancelledSummary"));
      return;
    case "finished": {
      running.value = false;
      cancelling.value = false;
      if (event.payload.cancelled === true) {
        markActiveSuitesCancelled();
        setRunStatus("cancelled", t("settings.testing.cancelledSummary"));
      } else {
        const ok = event.payload.ok === true;
        setRunStatus(ok ? "passed" : "failed", ok ? t("settings.testing.finished") : t("settings.testing.failed"));
      }
      return;
    }
    default:
      return;
  }
}

function suiteFromPayload(payload: Record<string, unknown>): UnityIntegrationSuite | null {
  const value = typeof payload.suite === "string" ? payload.suite : "";
  return suiteItems.some((suite) => suite.id === value) ? value as UnityIntegrationSuite : null;
}

function numberPayload(payload: Record<string, unknown>, key: string): number {
  const value = payload[key];
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function stringPayload(payload: Record<string, unknown>, key: string): string {
  const value = payload[key];
  return typeof value === "string" ? value : "";
}

function resultDetail(payload: Record<string, unknown>): string {
  const checkedTargets = numberPayload(payload, "checkedTargets");
  const checkedProperties = numberPayload(payload, "checkedProperties");
  if (checkedTargets > 0 || checkedProperties > 0) {
    return t("settings.testing.typeIndexDetail", checkedTargets, checkedProperties);
  }
  const transport = stringPayload(payload, "transport");
  if (transport) return transport;
  const semanticPhase = stringPayload(payload, "semanticPhase");
  if (semanticPhase) return semanticPhase;
  return "";
}

function labelForSuite(id: UnityIntegrationSuite): string {
  const suite = suiteItems.find((item) => item.id === id);
  return suite ? t(suite.labelKey) : id;
}

function statusLabel(state: SuiteState): string {
  switch (state.status) {
    case "queued":
      return t("settings.testing.statusQueued");
    case "running":
      return t("settings.testing.statusRunning");
    case "passed":
      return t("settings.testing.statusPassed", state.passed);
    case "failed":
      return t("settings.testing.statusFailed", state.failed);
    case "cancelled":
      return t("settings.testing.statusCancelled");
    default:
      return t("settings.testing.statusIdle");
  }
}

function badgeLabel(state: SuiteState): string {
  return state.status === "idle" ? "—" : statusLabel(state);
}

function lineClass(line: string): string {
  if (line.startsWith("FAIL") || line.startsWith("ERROR")) return "output-line--fail";
  if (line.startsWith("PASS")) return "output-line--pass";
  return "";
}

async function copyOutput() {
  const copied = await copyOutputText(selectedLogText.value);
  if (copied) return;
  notificationStore.addNotice("warning", t("settings.testing.copyFailed"), {
    operation: "copyUnityIntegrationTestLog",
    replaceOperation: true,
  });
}

watch(selectedDetailId, () => scrollConsoleToBottom());

onMounted(() => {
  void subscribeUnityIntegrationTests(handleIntegrationEvent).then((unsubscribe) => {
    unsubscribeIntegrationTests = unsubscribe;
  });
});

onUnmounted(() => {
  unsubscribeIntegrationTests?.();
  unsubscribeIntegrationTests = null;
});
</script>

<template>
  <div class="settings-section">
    <div class="run-bar">
      <div class="run-status" :class="`run-status--${runState}`">
        <span v-if="running" class="run-spinner" aria-hidden="true" />
        <span class="run-status-text">{{ runSummary || t("settings.testing.ready") }}</span>
      </div>
      <div class="run-actions">
        <BaseButton
          v-if="running"
          variant="danger"
          :disabled="cancelling"
          @click="interruptRun"
        >
          {{ cancelling ? t("settings.testing.interrupting") : t("settings.testing.interrupt") }}
        </BaseButton>
        <template v-else>
          <BaseButton :disabled="!canRun" @click="runSelectedSuites">
            {{ t("settings.testing.runSelected") }}
          </BaseButton>
          <BaseButton variant="primary" @click="runAllSuites">
            {{ t("settings.testing.runAll") }}
          </BaseButton>
        </template>
      </div>
    </div>

    <div class="test-workbench">
      <div class="suite-list">
        <div class="suite-list-head">
          <span>{{ t("settings.testing.suites") }}</span>
          <div class="suite-list-tools">
            <button type="button" class="link-btn" :disabled="running" @click="setAllSuites(true)">
              {{ t("settings.testing.selectAll") }}
            </button>
            <button type="button" class="link-btn" :disabled="running" @click="setAllSuites(false)">
              {{ t("settings.testing.clearSelection") }}
            </button>
          </div>
        </div>
        <div class="suite-list-body">
          <div
            v-for="suite in suiteItems"
            :key="suite.id"
            class="suite-row"
            :class="{ selected: suite.id === selectedDetailId }"
            role="button"
            tabindex="0"
            @click="selectDetail(suite.id)"
            @keydown.enter="selectDetail(suite.id)"
            @keydown.space.prevent="selectDetail(suite.id)"
          >
            <BaseCheckbox
              v-model="selectedSuites[suite.id]"
              :disabled="running"
              :aria-label="t(suite.labelKey)"
              @click.stop
            />
            <span class="suite-name">{{ t(suite.labelKey) }}</span>
            <span class="suite-badge" :class="suiteStates[suite.id].status">
              {{ badgeLabel(suiteStates[suite.id]) }}
            </span>
          </div>
        </div>
      </div>

      <div class="suite-detail">
        <div class="detail-head">
          <div class="detail-title">
            <span class="detail-name">{{ t(selectedItem.labelKey) }}</span>
            <span class="suite-badge detail-badge" :class="selectedState.status">
              {{ statusLabel(selectedState) }}
            </span>
          </div>
          <BaseButton :disabled="!selectedLogText" @click="copyOutput">
            {{ outputCopied ? t("common.copied") : t("settings.testing.copyOutput") }}
          </BaseButton>
        </div>
        <p class="detail-desc">{{ t(selectedItem.descKey) }}</p>
        <p v-if="selectedState.detail" class="detail-meta">{{ selectedState.detail }}</p>
        <div class="detail-output-label">{{ t("settings.testing.output") }}</div>
        <div ref="consoleRef" class="detail-output" role="log">
          <template v-if="selectedLogs.length > 0">
            <div
              v-for="(line, index) in selectedLogs"
              :key="index"
              class="output-line"
              :class="lineClass(line)"
            >
              {{ line }}
            </div>
          </template>
          <div v-else class="output-empty">{{ t("settings.testing.outputEmpty") }}</div>
        </div>
      </div>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.testing.options") }}</div>
    <div class="test-card">
      <label class="option-row">
        <BaseCheckbox v-model="installPlugin" :disabled="running" :aria-label="t('settings.testing.installPlugin')" />
        <span class="option-text">{{ t("settings.testing.installPlugin") }}</span>
      </label>
      <label class="option-row">
        <BaseCheckbox v-model="openUnity" :disabled="running" :aria-label="t('settings.testing.openUnity')" />
        <span class="option-text">{{ t("settings.testing.openUnity") }}</span>
      </label>
      <label class="option-row">
        <BaseCheckbox v-model="forceEditMode" :disabled="running" :aria-label="t('settings.testing.forceEditMode')" />
        <span class="option-text">{{ t("settings.testing.forceEditMode") }}</span>
      </label>
      <div class="option-row option-select-row">
        <span class="option-text">{{ t("settings.testing.typeIndexSample") }}</span>
        <select v-model="typeIndexSampleMode" class="sample-select" :disabled="running">
          <option value="sample32">{{ t("settings.testing.typeIndexSample32") }}</option>
          <option value="all">{{ t("settings.testing.typeIndexSampleAll") }}</option>
        </select>
      </div>
    </div>
  </div>
</template>

<style scoped>
.test-card {
  display: flex;
  flex-direction: column;
  max-width: 980px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
  overflow: hidden;
}

/* Run bar ------------------------------------------------------------------ */
.run-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  max-width: 980px;
  margin-bottom: 12px;
  padding: 10px 14px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
}

.run-status {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  color: var(--text-secondary);
  font-size: 12px;
}

.run-status-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.run-status--running { color: var(--accent-color); }
.run-status--passed { color: var(--status-good-fg); }
.run-status--failed { color: var(--status-danger-fg); }
.run-status--cancelled { color: var(--text-secondary); }

.run-spinner {
  flex-shrink: 0;
  width: 12px;
  height: 12px;
  border: 2px solid color-mix(in srgb, var(--accent-color) 35%, transparent);
  border-top-color: var(--accent-color);
  border-radius: 50%;
  animation: run-spin 0.7s linear infinite;
}

@keyframes run-spin {
  to { transform: rotate(360deg); }
}

.run-actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}

/* Workbench: list + detail ------------------------------------------------- */
.test-workbench {
  display: grid;
  grid-template-columns: minmax(220px, 290px) 1fr;
  max-width: 980px;
  min-height: 380px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
  overflow: hidden;
}

.suite-list {
  display: flex;
  flex-direction: column;
  min-width: 0;
  border-right: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 70%, var(--sidebar-bg) 30%);
}

.suite-list-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 10px 12px;
  border-bottom: 1px solid var(--border-color);
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.4px;
}

.suite-list-tools {
  display: flex;
  gap: 4px;
}

.link-btn {
  padding: 2px 4px;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: 10.5px;
  font-weight: 600;
  text-transform: none;
  letter-spacing: 0;
  cursor: pointer;
  border-radius: 4px;
  box-shadow: none;
}

.link-btn:hover:not(:disabled) {
  color: var(--accent-color);
  background: var(--hover-bg);
}

.link-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.suite-list-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow-y: auto;
  padding: 6px;
  gap: 2px;
}

.suite-row {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  padding: 8px 9px;
  border: 1px solid transparent;
  border-radius: 7px;
  background: transparent;
  text-align: left;
  cursor: pointer;
  box-shadow: none;
  transition: background 0.12s ease, border-color 0.12s ease;
}

.suite-row:hover {
  background: var(--hover-bg, rgba(128, 128, 128, 0.08));
}

.suite-row.selected {
  background: color-mix(in srgb, var(--accent-color) 12%, transparent);
  border-color: var(--accent-border);
}

.suite-row:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent-color) 50%, transparent);
  outline-offset: -2px;
}

.suite-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-color);
  font-size: 12.5px;
  font-weight: 600;
}

.suite-badge {
  flex-shrink: 0;
  padding: 1px 7px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 14%, transparent);
  color: var(--text-secondary);
  font-size: 10.5px;
  font-weight: 600;
  white-space: nowrap;
}

.suite-badge.queued {
  color: var(--text-secondary);
}

.suite-badge.running {
  background: color-mix(in srgb, var(--accent-color) 18%, transparent);
  color: var(--accent-color);
}

.suite-badge.passed {
  background: color-mix(in srgb, var(--status-good-fg) 16%, transparent);
  color: var(--status-good-fg);
}

.suite-badge.failed {
  background: color-mix(in srgb, var(--status-danger-fg) 16%, transparent);
  color: var(--status-danger-fg);
}

.suite-badge.cancelled {
  background: color-mix(in srgb, var(--text-secondary) 16%, transparent);
  color: var(--text-secondary);
}

.suite-badge.idle {
  background: transparent;
}

/* Detail pane -------------------------------------------------------------- */
.suite-detail {
  display: flex;
  flex-direction: column;
  min-width: 0;
  padding: 14px 16px;
  gap: 8px;
}

.detail-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.detail-title {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}

.detail-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-color);
  font-size: 14px;
  font-weight: 600;
}

.detail-badge {
  font-size: 11px;
  padding: 2px 9px;
}

.detail-desc {
  margin: 0;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.5;
}

.detail-meta {
  margin: 0;
  color: var(--text-secondary);
  font-size: 11.5px;
  font-family: var(--font-mono-identifier);
}

.detail-output-label {
  margin-top: 2px;
  color: var(--text-secondary);
  font-size: 10.5px;
  font-weight: 600;
  letter-spacing: 0.4px;
  text-transform: uppercase;
}

.detail-output {
  flex: 1;
  min-height: 180px;
  max-height: 460px;
  overflow-y: auto;
  padding: 10px 11px;
  border: 1px solid var(--border-color);
  border-radius: 7px;
  background: color-mix(in srgb, var(--input-bg) 90%, var(--panel-bg) 10%);
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11.5px;
  line-height: 1.55;
}

.output-line {
  white-space: pre-wrap;
  word-break: break-word;
}

.output-line--pass {
  color: var(--status-good-fg);
}

.output-line--fail {
  color: var(--status-danger-fg);
}

.output-empty {
  color: var(--text-tertiary, var(--text-secondary));
  opacity: 0.7;
  font-style: italic;
}

/* Options ------------------------------------------------------------------ */
.option-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 11px 14px;
  cursor: default;
  transition: background 0.12s;
}

.option-row + .option-row {
  border-top: 1px solid var(--border-color);
}

.option-row:hover {
  background: var(--hover-bg, rgba(128, 128, 128, 0.08));
}

.option-text {
  color: var(--text-color);
  font-size: 12.5px;
  font-weight: 600;
}

.option-select-row {
  justify-content: space-between;
}

.sample-select {
  width: 150px;
  height: 28px;
  padding: 0 9px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 12px;
  outline: none;
}

.sample-select:focus {
  border-color: var(--accent-border);
}

@media (max-width: 720px) {
  .test-workbench {
    grid-template-columns: 1fr;
  }

  .suite-list {
    border-right: none;
    border-bottom: 1px solid var(--border-color);
  }
}
</style>
