<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { t } from "../../i18n";
import { useCopyFeedback } from "../../composables/useCopyFeedback";
import { useHotReloadDebugGuard } from "../../composables/useHotReloadDebugGuard";
import BaseButton from "../ui/BaseButton.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";
import {
  subscribeUnityHotReloadSelfTest,
  subscribeUnitySidecarCompilerStatus,
  unityHotReloadSelfTestRun,
  unityHotReloadSetEnabled,
  unitySidecarCompilerGetStatus,
  unitySidecarCompilerSetEnabled,
} from "../../services/csharpLsp";
import type { CsharpCompileStatus } from "../../types";
import type { RuntimeUnsubscribe } from "../../services/locusRuntime";
import { normalizeAppError } from "../../services/errors";
import { useNotificationStore } from "../../stores/notification";

const notificationStore = useNotificationStore();

const sidecarStatus = ref<CsharpCompileStatus | null>(null);
const sidecarReady = ref(false);
const sidecarBusy = ref(false);

let unsubscribeSidecarStatus: RuntimeUnsubscribe | null = null;

const sidecarEnabled = computed(() => sidecarStatus.value?.enabled ?? false);

const sidecarStatusLabel = computed(() => {
  const status = sidecarStatus.value;
  if (!status) return t("common.loading");
  if (!status.platformSupported) return t("settings.codeAnalysis.sidecarUnsupported");
  if (!status.serverAvailable) return t("settings.codeAnalysis.sidecarMissing");
  if (!status.enabled) return t("settings.codeAnalysis.sidecarOff");
  if (status.lastError) return t("settings.codeAnalysis.sidecarError", status.lastError);
  if (status.running) {
    return t(
      "settings.codeAnalysis.sidecarRunning",
      status.roslynVersion ?? "?",
      status.dotnetSource ?? "?",
    );
  }
  return t("settings.codeAnalysis.sidecarIdle");
});

const sidecarHasIssue = computed(() => {
  const status = sidecarStatus.value;
  return !!status && status.enabled && !!status.lastError;
});

const sidecarStatsLabel = computed(() => {
  const status = sidecarStatus.value;
  if (!status || !status.enabled) return "";
  const total = (status.sidecarCompiles ?? 0) + (status.fallbacks ?? 0);
  if (total === 0) return "";
  return t(
    "settings.codeAnalysis.sidecarStats",
    status.sidecarCompiles ?? 0,
    status.fallbacks ?? 0,
  );
});

async function refreshSidecarStatus() {
  try {
    sidecarStatus.value = await unitySidecarCompilerGetStatus();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadUnitySidecarCompilerStatus",
    });
  } finally {
    sidecarReady.value = true;
  }
}

async function toggleSidecarEnabled() {
  if (!sidecarReady.value || sidecarBusy.value) return;
  sidecarBusy.value = true;
  try {
    sidecarStatus.value = await unitySidecarCompilerSetEnabled(!sidecarEnabled.value);
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "toggleUnitySidecarCompiler",
      replaceOperation: true,
    });
    await refreshSidecarStatus();
  } finally {
    sidecarBusy.value = false;
  }
}

const hotReloadEnabled = computed(() => sidecarStatus.value?.hotReloadEnabled ?? false);
const hotReloadBusy = ref(false);

const hotReloadStatsLabel = computed(() => {
  const status = sidecarStatus.value;
  if (!status || !status.hotReloadEnabled) return "";
  const total =
    (status.hotUnappliedChanges ?? 0)
    + (status.hotPatchedCodeCount ?? 0)
    + (status.hotPatchFailures ?? 0)
    + (status.hotColdQueued ?? 0);
  if (total === 0) return "";
  return t(
    "settings.codeAnalysis.hotReloadStats",
    status.hotUnappliedChanges ?? 0,
    status.hotPatchedCodeCount ?? 0,
    status.hotColdQueued ?? 0,
  );
});

async function applyHotReloadEnabled(value: boolean) {
  hotReloadBusy.value = true;
  try {
    sidecarStatus.value = await unityHotReloadSetEnabled(value);
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "toggleUnityHotReload",
      replaceOperation: true,
    });
    await refreshSidecarStatus();
  } finally {
    hotReloadBusy.value = false;
  }
}

// Release-first: enabling no longer blocks on Code Optimization. Hot reload
// works in Release (methods Unity inlines converge via recompile); we surface
// the editor's optimization only as an optional "switch to Debug" hint.
const {
  isRelease: hotReloadIsRelease,
  switching: hotReloadSwitching,
  switchError: hotReloadSwitchError,
  refreshOptimization: refreshHotReloadOptimization,
  enableHotReload: hotReloadEnable,
  switchToDebug: hotReloadSwitchToDebug,
} = useHotReloadDebugGuard(() => applyHotReloadEnabled(true));

async function toggleHotReloadEnabled() {
  if (!sidecarReady.value || hotReloadBusy.value) return;
  if (hotReloadEnabled.value) {
    await applyHotReloadEnabled(false);
  } else {
    await hotReloadEnable();
  }
}

// ── hot reload self-test ─────────────────────────────────────────────

const selfTestRunning = ref(false);
const selfTestLog = ref<string[]>([]);
const selfTestSummary = ref("");
const selfTestLogText = computed(() => selfTestLog.value.join("\n"));
const { copied: selfTestLogCopied, copyText: copySelfTestLogText } = useCopyFeedback();
let unsubscribeSelfTest: RuntimeUnsubscribe | null = null;

async function runHotReloadSelfTest() {
  if (selfTestRunning.value) return;
  selfTestRunning.value = true;
  selfTestLog.value = [];
  selfTestSummary.value = "";
  try {
    await unityHotReloadSelfTestRun();
  } catch (e) {
    const err = normalizeAppError(e);
    selfTestRunning.value = false;
    selfTestLog.value = [...selfTestLog.value, err.message];
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "unityHotReloadSelfTest",
      replaceOperation: true,
    });
  }
}

async function copyHotReloadSelfTestLog() {
  const copied = await copySelfTestLogText(selfTestLogText.value);
  if (copied) return;
  notificationStore.addNotice("warning", t("settings.codeAnalysis.hotReloadSelfTestCopyFailed"), {
    operation: "copyHotReloadSelfTestLog",
    replaceOperation: true,
  });
}

onMounted(() => {
  void refreshSidecarStatus();
  void refreshHotReloadOptimization();
  void subscribeUnitySidecarCompilerStatus((payload) => {
    sidecarStatus.value = payload;
    sidecarReady.value = true;
  }).then((unsubscribe) => {
    unsubscribeSidecarStatus = unsubscribe;
  });
  void subscribeUnityHotReloadSelfTest((payload) => {
    selfTestRunning.value = payload.running && !payload.finished;
    if (payload.line) {
      selfTestLog.value = [...selfTestLog.value, payload.line];
    }
    if (payload.finished) {
      selfTestSummary.value = t(
        "settings.codeAnalysis.hotReloadSelfTestSummary",
        payload.passed,
        payload.failed,
      );
    }
  }).then((unsubscribe) => {
    unsubscribeSelfTest = unsubscribe;
  });
});

onUnmounted(() => {
  unsubscribeSidecarStatus?.();
  unsubscribeSidecarStatus = null;
  unsubscribeSelfTest?.();
  unsubscribeSelfTest = null;
});
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.tab.hotReload") }}</div>
    <p class="section-desc">{{ t("settings.codeAnalysis.sidecarDesc") }}</p>
    <div class="tool-card">
      <div class="tool-row master-row">
        <div class="tool-info">
          <span class="tool-name">{{ t("settings.codeAnalysis.sidecarLabel") }}</span>
          <span class="tool-desc" :class="{ 'status-error': sidecarHasIssue }">
            {{ sidecarStatusLabel }}
          </span>
          <span v-if="sidecarStatsLabel" class="tool-desc">{{ sidecarStatsLabel }}</span>
        </div>
        <div class="master-actions">
          <BaseSwitch
            v-if="sidecarReady"
            :model-value="sidecarEnabled"
            :disabled="
              sidecarBusy ||
              !(sidecarStatus?.platformSupported ?? true) ||
              !(sidecarStatus?.serverAvailable ?? true)
            "
            :aria-label="t('settings.codeAnalysis.sidecarLabel')"
            @update:model-value="toggleSidecarEnabled"
          />
          <span v-else class="switch-placeholder" aria-hidden="true" />
        </div>
      </div>
      <div class="tool-row">
        <div class="tool-info">
          <span class="tool-name">{{ t("settings.codeAnalysis.hotReloadLabel") }}</span>
          <span class="tool-desc">{{ t("settings.codeAnalysis.hotReloadDesc") }}</span>
          <span v-if="hotReloadStatsLabel" class="tool-desc">{{ hotReloadStatsLabel }}</span>
          <span v-if="!sidecarEnabled" class="tool-desc tool-dep">
            {{ t("settings.codeAnalysis.hotReloadDepSidecar") }}
          </span>
          <template v-if="hotReloadEnabled && hotReloadIsRelease">
            <span class="tool-desc tool-dep">
              {{ t("settings.codeAnalysis.hotReloadReleaseHint") }}
            </span>
            <div class="hotreload-switch-row">
              <BaseButton :disabled="hotReloadSwitching" @click="hotReloadSwitchToDebug">
                {{
                  hotReloadSwitching
                    ? t("settings.codeAnalysis.hotReloadSwitching")
                    : t("settings.codeAnalysis.hotReloadSwitchToDebug")
                }}
              </BaseButton>
              <span v-if="hotReloadSwitchError" class="tool-desc status-error">
                {{ hotReloadSwitchError }}
              </span>
            </div>
          </template>
        </div>
        <div class="master-actions">
          <BaseSwitch
            v-if="sidecarReady"
            :model-value="hotReloadEnabled"
            :disabled="hotReloadBusy || !sidecarEnabled"
            :aria-label="t('settings.codeAnalysis.hotReloadLabel')"
            @update:model-value="toggleHotReloadEnabled"
          />
          <span v-else class="switch-placeholder" aria-hidden="true" />
        </div>
      </div>
      <div v-if="hotReloadEnabled" class="tool-row">
        <div class="tool-info">
          <span class="tool-name">{{ t("settings.codeAnalysis.hotReloadSelfTestLabel") }}</span>
          <span class="tool-desc">{{ t("settings.codeAnalysis.hotReloadSelfTestDesc") }}</span>
          <span v-if="selfTestSummary" class="tool-desc">{{ selfTestSummary }}</span>
        </div>
        <div class="master-actions">
          <BaseButton :disabled="selfTestRunning || !sidecarEnabled" @click="runHotReloadSelfTest">
            {{
              selfTestRunning
                ? t("settings.codeAnalysis.hotReloadSelfTestRunning")
                : t("settings.codeAnalysis.hotReloadSelfTestRun")
            }}
          </BaseButton>
        </div>
      </div>
      <div v-if="selfTestLog.length > 0" class="selftest-log-panel">
        <div class="selftest-log-header">
          <span class="selftest-log-title">
            {{ t("settings.codeAnalysis.hotReloadSelfTestLog") }}
          </span>
          <BaseButton :disabled="!selfTestLogText" @click="copyHotReloadSelfTestLog">
            {{
              selfTestLogCopied
                ? t("common.copied")
                : t("settings.codeAnalysis.hotReloadSelfTestCopy")
            }}
          </BaseButton>
        </div>
        <div class="selftest-log" role="log">
          <div v-for="(line, index) in selfTestLog" :key="index" class="selftest-log-line">
            {{ line }}
          </div>
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
.tool-row + .tool-row {
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
  font-family: var(--font-mono-identifier);
  color: var(--text-color);
}
.tool-desc {
  font-size: 11.5px;
  color: var(--text-secondary);
  line-height: 1.45;
}
.tool-dep {
  color: var(--status-warning-fg, var(--text-secondary));
}
.hotreload-switch-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 4px;
}
.status-error {
  color: var(--status-danger-fg);
}
.master-row .tool-name {
  font-family: inherit;
  font-size: 13px;
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
