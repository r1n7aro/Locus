<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { t } from "../../i18n";
import { useCopyFeedback } from "../../composables/useCopyFeedback";
import { normalizeAppError } from "../../services/errors";
import type { RuntimeUnsubscribe } from "../../services/locusRuntime";
import {
  subscribeUnityNativeBridgeSelfTest,
  unityNativeBridgeGetEnabled,
  unityNativeBridgeSelfTestRun,
  unityNativeBridgeSetEnabled,
  unityNativeBrokerGetStatus,
  subscribeUnityStateProbeSelfTest,
  unityStateProbeGetStatus,
  unityStateProbeSelfTestRun,
  unityStateProbeSetEnabled,
  type UnityStateProbeStatus,
} from "../../services/csharpLsp";
import {
  getUnityBackgroundHookStatus,
  setUnityBackgroundHookEnabled,
} from "../../services/system";
import type { UnityBackgroundHookStatus, UnityNativeBrokerStatus } from "../../types";
import { useNotificationStore } from "../../stores/notification";
import BaseButton from "../ui/BaseButton.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";

const notificationStore = useNotificationStore();

// ── native plugin bridge ─────────────────────────────────────────────

const nativeBridgeEnabled = ref(false);
const nativeBridgeReady = ref(false);
const nativeBridgeBusy = ref(false);
const nativeBrokerStatus = ref<UnityNativeBrokerStatus | null>(null);
const nativeBrokerBusy = ref(false);

const nativeTestRunning = ref(false);
const nativeTestLog = ref<string[]>([]);
const nativeTestSummary = ref("");
const nativeTestLogText = computed(() => nativeTestLog.value.join("\n"));
const { copied: nativeTestLogCopied, copyText: copyNativeTestLogText } = useCopyFeedback();

let unsubscribeNativeSelfTest: RuntimeUnsubscribe | null = null;

const nativeBridgeStatusLabel = computed(() => {
  if (!nativeBridgeReady.value) return t("common.loading");
  if (!nativeBridgeEnabled.value) return t("settings.testing.nativeBridgeOff");
  const status = nativeBrokerStatus.value;
  if (!status) return t("settings.testing.nativeBrokerMissing");
  if (status.managedState === "ready") {
    return t(
      "settings.testing.nativeBrokerReady",
      status.domainGeneration,
      status.editorStatus || "-",
    );
  }
  return t(
    "settings.testing.nativeBrokerState",
    status.managedState || "-",
    status.domainGeneration,
  );
});

const nativeBrokerRows = computed(() => {
  const status = nativeBrokerStatus.value;
  if (!status) return [];
  return [
    [t("settings.testing.nativeFieldManagedState"), status.managedState || "-"],
    [t("settings.testing.nativeFieldGeneration"), String(status.domainGeneration ?? "-")],
    [t("settings.testing.nativeFieldEditorStatus"), status.editorStatus || "-"],
    [
      t("settings.testing.nativeFieldQueue"),
      `${status.pendingRequests ?? 0} / ${status.inflightRequests ?? 0}`,
    ],
    [t("settings.testing.nativeFieldProtocol"), String(status.protocolVersion ?? "-")],
    [
      t("settings.testing.nativeFieldCapabilities"),
      status.capabilities?.join(", ") || "-",
    ],
  ];
});

async function refreshNativeBridgeStatus() {
  nativeBrokerBusy.value = true;
  try {
    nativeBridgeEnabled.value = await unityNativeBridgeGetEnabled();
    nativeBrokerStatus.value = await unityNativeBrokerGetStatus();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadUnityNativeBridgeStatus",
      replaceOperation: true,
    });
  } finally {
    nativeBridgeReady.value = true;
    nativeBrokerBusy.value = false;
  }
}

async function toggleNativeBridgeEnabled() {
  if (!nativeBridgeReady.value || nativeBridgeBusy.value) return;
  nativeBridgeBusy.value = true;
  try {
    nativeBridgeEnabled.value = await unityNativeBridgeSetEnabled(!nativeBridgeEnabled.value);
    nativeBrokerStatus.value = await unityNativeBrokerGetStatus();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "toggleUnityNativeBridge",
      replaceOperation: true,
    });
    await refreshNativeBridgeStatus();
  } finally {
    nativeBridgeBusy.value = false;
  }
}

async function runNativeBridgeSelfTest() {
  if (nativeTestRunning.value) return;
  nativeTestRunning.value = true;
  nativeTestLog.value = [];
  nativeTestSummary.value = "";
  try {
    await unityNativeBridgeSelfTestRun();
  } catch (e) {
    const err = normalizeAppError(e);
    nativeTestRunning.value = false;
    nativeTestLog.value = [...nativeTestLog.value, err.message];
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "unityNativeBridgeSelfTest",
      replaceOperation: true,
    });
  }
}

async function copyNativeBridgeSelfTestLog() {
  const copied = await copyNativeTestLogText(nativeTestLogText.value);
  if (copied) return;
  notificationStore.addNotice("warning", t("settings.testing.nativeSelfTestCopyFailed"), {
    operation: "copyUnityNativeBridgeSelfTestLog",
    replaceOperation: true,
  });
}

// ── native state probe ───────────────────────────────────────────────

const stateProbeStatus = ref<UnityStateProbeStatus | null>(null);
const stateProbeReady = ref(false);
const stateProbeBusy = ref(false);

const stateProbeEnabled = computed(() => stateProbeStatus.value?.enabled ?? false);

const stateProbeStatusLabel = computed(() => {
  const status = stateProbeStatus.value;
  if (!status) return t("common.loading");
  if (!status.supported) return t("settings.codeAnalysis.stateProbeUnsupported");
  if (!status.enabled) return t("settings.codeAnalysis.stateProbeOff");
  switch (status.tier) {
    case "passive":
      return t("settings.codeAnalysis.stateProbeTierPassive");
    case "stack":
      return t("settings.codeAnalysis.stateProbeTierStack", status.reloadSymbols);
    case "cpu_only":
      return t("settings.codeAnalysis.stateProbeTierCpuOnly");
    case "inference":
      return t("settings.codeAnalysis.stateProbeTierInference");
    default:
      return t("settings.codeAnalysis.stateProbeTierInactive");
  }
});

async function refreshStateProbeStatus() {
  try {
    stateProbeStatus.value = await unityStateProbeGetStatus();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadUnityStateProbeStatus",
    });
  } finally {
    stateProbeReady.value = true;
  }
}

async function toggleStateProbeEnabled() {
  if (!stateProbeReady.value || stateProbeBusy.value) return;
  stateProbeBusy.value = true;
  try {
    stateProbeStatus.value = await unityStateProbeSetEnabled(!stateProbeEnabled.value);
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "toggleUnityStateProbe",
      replaceOperation: true,
    });
    await refreshStateProbeStatus();
  } finally {
    stateProbeBusy.value = false;
  }
}

const probeTestRunning = ref(false);
const probeTestLog = ref<string[]>([]);
const probeTestSummary = ref("");
const probeTestLogText = computed(() => probeTestLog.value.join("\n"));
const { copied: probeTestLogCopied, copyText: copyProbeTestLogText } = useCopyFeedback();
let unsubscribeProbeSelfTest: RuntimeUnsubscribe | null = null;

async function runStateProbeSelfTest() {
  if (probeTestRunning.value) return;
  probeTestRunning.value = true;
  probeTestLog.value = [];
  probeTestSummary.value = "";
  try {
    await unityStateProbeSelfTestRun();
  } catch (e) {
    const err = normalizeAppError(e);
    probeTestRunning.value = false;
    probeTestLog.value = [...probeTestLog.value, err.message];
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "unityStateProbeSelfTest",
      replaceOperation: true,
    });
  }
}

async function copyStateProbeSelfTestLog() {
  const copied = await copyProbeTestLogText(probeTestLogText.value);
  if (copied) return;
  notificationStore.addNotice("warning", t("settings.codeAnalysis.stateProbeSelfTestCopyFailed"), {
    operation: "copyStateProbeSelfTestLog",
    replaceOperation: true,
  });
}

// ── background hook ──────────────────────────────────────────────────

const backgroundHookStatus = ref<UnityBackgroundHookStatus | null>(null);
const backgroundHookReady = ref(false);
const backgroundHookBusy = ref(false);

const backgroundHookEnabled = computed(() => backgroundHookStatus.value?.enabled ?? false);

const backgroundHookStatusLabel = computed(() => {
  const status = backgroundHookStatus.value;
  if (!status) return t("common.loading");
  if (!status.supported) return t("settings.codeAnalysis.backgroundHookUnsupported");
  if (!status.enabled) return t("settings.codeAnalysis.backgroundHookOff");
  switch (status.state) {
    case "patched":
      return t("settings.codeAnalysis.backgroundHookPatched");
    case "inactive":
      return t("settings.codeAnalysis.backgroundHookInactive");
    case "failed":
      return t("settings.codeAnalysis.backgroundHookFailed", status.error ?? "");
    case "unsupported":
      return t("settings.codeAnalysis.backgroundHookUnsupported");
    default:
      return t("settings.codeAnalysis.backgroundHookOff");
  }
});

async function refreshBackgroundHookStatus() {
  try {
    backgroundHookStatus.value = await getUnityBackgroundHookStatus();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadUnityBackgroundHookStatus",
    });
  } finally {
    backgroundHookReady.value = true;
  }
}

async function toggleBackgroundHookEnabled() {
  if (!backgroundHookReady.value || backgroundHookBusy.value) return;
  backgroundHookBusy.value = true;
  try {
    backgroundHookStatus.value = await setUnityBackgroundHookEnabled(!backgroundHookEnabled.value);
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "toggleUnityBackgroundHook",
      replaceOperation: true,
    });
    await refreshBackgroundHookStatus();
  } finally {
    backgroundHookBusy.value = false;
  }
}

onMounted(() => {
  void refreshNativeBridgeStatus();
  void subscribeUnityNativeBridgeSelfTest((payload) => {
    nativeTestRunning.value = payload.running && !payload.finished;
    if (payload.line) {
      nativeTestLog.value = [...nativeTestLog.value, payload.line];
    }
    if (payload.finished) {
      nativeTestSummary.value = t(
        "settings.testing.nativeSelfTestSummary",
        payload.passed,
        payload.failed,
      );
      void refreshNativeBridgeStatus();
    }
  }).then((unsubscribe) => {
    unsubscribeNativeSelfTest = unsubscribe;
  });

  void refreshStateProbeStatus();
  void subscribeUnityStateProbeSelfTest((payload) => {
    probeTestRunning.value = payload.running && !payload.finished;
    if (payload.line) {
      probeTestLog.value = [...probeTestLog.value, payload.line];
    }
    if (payload.finished) {
      probeTestSummary.value = t(
        "settings.codeAnalysis.stateProbeSelfTestSummary",
        payload.passed,
        payload.failed,
      );
      void refreshStateProbeStatus();
    }
  }).then((unsubscribe) => {
    unsubscribeProbeSelfTest = unsubscribe;
  });

  void refreshBackgroundHookStatus();
});

onUnmounted(() => {
  unsubscribeNativeSelfTest?.();
  unsubscribeNativeSelfTest = null;
  unsubscribeProbeSelfTest?.();
  unsubscribeProbeSelfTest = null;
});
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.testing.nativeBridge") }}</div>
    <p class="section-desc">{{ t("settings.testing.nativeBridgeDesc") }}</p>
    <div class="tool-card">
      <div class="tool-row master-row">
        <div class="tool-info">
          <span class="tool-name">{{ t("settings.testing.nativeBridgeLabel") }}</span>
          <span class="tool-desc">{{ nativeBridgeStatusLabel }}</span>
        </div>
        <div class="master-actions">
          <BaseButton :disabled="nativeBrokerBusy" @click="refreshNativeBridgeStatus">
            {{ t("common.refresh") }}
          </BaseButton>
          <BaseSwitch
            v-if="nativeBridgeReady"
            :model-value="nativeBridgeEnabled"
            :disabled="nativeBridgeBusy"
            :aria-label="t('settings.testing.nativeBridgeLabel')"
            @update:model-value="toggleNativeBridgeEnabled"
          />
          <span v-else class="switch-placeholder" aria-hidden="true" />
        </div>
      </div>

      <div v-if="nativeBrokerRows.length > 0" class="status-grid">
        <div v-for="[label, value] in nativeBrokerRows" :key="label" class="status-row">
          <span class="status-label">{{ label }}</span>
          <span class="status-value">{{ value }}</span>
        </div>
      </div>

      <div class="tool-row">
        <div class="tool-info">
          <span class="tool-name">{{ t("settings.testing.nativeSelfTestLabel") }}</span>
          <span class="tool-desc">{{ t("settings.testing.nativeSelfTestDesc") }}</span>
          <span v-if="nativeTestSummary" class="tool-desc">{{ nativeTestSummary }}</span>
        </div>
        <div class="master-actions">
          <BaseButton
            :disabled="nativeTestRunning || !nativeBridgeEnabled"
            @click="runNativeBridgeSelfTest"
          >
            {{
              nativeTestRunning
                ? t("settings.testing.nativeSelfTestRunning")
                : t("settings.testing.nativeSelfTestRun")
            }}
          </BaseButton>
        </div>
      </div>

      <div v-if="nativeTestLog.length > 0" class="selftest-log-panel">
        <div class="selftest-log-header">
          <span class="selftest-log-title">
            {{ t("settings.testing.nativeSelfTestLog") }}
          </span>
          <BaseButton :disabled="!nativeTestLogText" @click="copyNativeBridgeSelfTestLog">
            {{
              nativeTestLogCopied
                ? t("common.copied")
                : t("settings.testing.nativeSelfTestCopy")
            }}
          </BaseButton>
        </div>
        <div class="selftest-log" role="log">
          <div v-for="(line, index) in nativeTestLog" :key="index" class="selftest-log-line">
            {{ line }}
          </div>
        </div>
      </div>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.codeAnalysis.stateProbe") }}</div>
    <p class="section-desc">{{ t("settings.codeAnalysis.stateProbeDesc") }}</p>
    <div class="tool-card">
      <div class="tool-row master-row">
        <div class="tool-info">
          <span class="tool-name">{{ t("settings.codeAnalysis.stateProbeLabel") }}</span>
          <span class="tool-desc">{{ stateProbeStatusLabel }}</span>
        </div>
        <div class="master-actions">
          <BaseSwitch
            v-if="stateProbeReady"
            :model-value="stateProbeEnabled"
            :disabled="stateProbeBusy || !(stateProbeStatus?.supported ?? true)"
            :aria-label="t('settings.codeAnalysis.stateProbeLabel')"
            @update:model-value="toggleStateProbeEnabled"
          />
          <span v-else class="switch-placeholder" aria-hidden="true" />
        </div>
      </div>
      <div v-if="stateProbeEnabled" class="tool-row">
        <div class="tool-info">
          <span class="tool-name">{{ t("settings.codeAnalysis.stateProbeSelfTestLabel") }}</span>
          <span class="tool-desc">{{ t("settings.codeAnalysis.stateProbeSelfTestDesc") }}</span>
          <span v-if="probeTestSummary" class="tool-desc">{{ probeTestSummary }}</span>
        </div>
        <div class="master-actions">
          <BaseButton :disabled="probeTestRunning" @click="runStateProbeSelfTest">
            {{
              probeTestRunning
                ? t("settings.codeAnalysis.stateProbeSelfTestRunning")
                : t("settings.codeAnalysis.stateProbeSelfTestRun")
            }}
          </BaseButton>
        </div>
      </div>
      <div v-if="probeTestLog.length > 0" class="selftest-log-panel">
        <div class="selftest-log-header">
          <span class="selftest-log-title">
            {{ t("settings.codeAnalysis.stateProbeSelfTestLog") }}
          </span>
          <BaseButton :disabled="!probeTestLogText" @click="copyStateProbeSelfTestLog">
            {{
              probeTestLogCopied
                ? t("common.copied")
                : t("settings.codeAnalysis.stateProbeSelfTestCopy")
            }}
          </BaseButton>
        </div>
        <div class="selftest-log" role="log">
          <div v-for="(line, index) in probeTestLog" :key="index" class="selftest-log-line">
            {{ line }}
          </div>
        </div>
      </div>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.codeAnalysis.backgroundHook") }}</div>
    <p class="section-desc">{{ t("settings.codeAnalysis.backgroundHookDesc") }}</p>
    <div class="tool-card">
      <div class="tool-row master-row">
        <div class="tool-info">
          <span class="tool-name">{{ t("settings.codeAnalysis.backgroundHookLabel") }}</span>
          <span class="tool-desc">{{ backgroundHookStatusLabel }}</span>
        </div>
        <div class="master-actions">
          <BaseSwitch
            v-if="backgroundHookReady"
            :model-value="backgroundHookEnabled"
            :disabled="backgroundHookBusy || !(backgroundHookStatus?.supported ?? true)"
            :aria-label="t('settings.codeAnalysis.backgroundHookLabel')"
            @update:model-value="toggleBackgroundHookEnabled"
          />
          <span v-else class="switch-placeholder" aria-hidden="true" />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.tool-card {
  display: flex;
  flex-direction: column;
  max-width: 760px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
  overflow: hidden;
}
.tool-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 11px 16px;
  transition: background 0.12s;
}
.tool-row + .tool-row,
.status-grid + .tool-row {
  border-top: 1px solid var(--border-color);
}
.tool-row:hover {
  background: var(--hover-bg, rgba(128, 128, 128, 0.08));
}
.tool-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.tool-name {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-color);
}
.tool-desc {
  font-size: 11.5px;
  color: var(--text-secondary);
  line-height: 1.45;
}
.master-actions {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-shrink: 0;
}
.switch-placeholder {
  flex-shrink: 0;
  width: 34px;
  height: 18px;
  border: 1px solid color-mix(in srgb, var(--border-strong) 82%, var(--text-secondary) 18%);
  border-radius: 6px;
  background: color-mix(in srgb, var(--input-bg) 76%, var(--hover-bg) 24%);
  opacity: 0.55;
}
.status-grid {
  display: grid;
  grid-template-columns: minmax(140px, 180px) minmax(0, 1fr);
  gap: 0;
  padding: 8px 16px 10px;
  border-top: 1px solid var(--border-color);
}
.status-row {
  display: contents;
}
.status-label,
.status-value {
  min-width: 0;
  padding: 4px 0;
  font-size: 11.5px;
  line-height: 1.45;
}
.status-label {
  color: var(--text-secondary);
}
.status-value {
  overflow-wrap: anywhere;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
}
.selftest-log-panel {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0 12px 12px;
  padding-top: 10px;
  border-top: 1px solid var(--border-color);
}
.selftest-log-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.selftest-log-title {
  min-width: 0;
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.3px;
  text-transform: uppercase;
}
.selftest-log {
  padding: 9px 10px;
  max-height: 240px;
  overflow-y: auto;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--input-bg) 90%, var(--panel-bg) 10%);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  line-height: 1.55;
  color: var(--text-secondary);
}
.selftest-log-line {
  white-space: pre-wrap;
  word-break: break-word;
}
.selftest-log-line:first-letter {
  text-transform: none;
}
</style>
