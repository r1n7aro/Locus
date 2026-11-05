<script setup lang="ts">
// Diagnostics of the focused workspace on the Unity page: a status table of
// the editor connection and services, then the self-test suites. The
// switches themselves are global settings above; this block only reads state
// and runs checks. Rendered only while a workspace is focused.
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { ChevronDown, ChevronRight, RefreshCw } from "lucide";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import type { RuntimeUnsubscribe } from "../../services/locusRuntime";
import {
  csharpLspGetStatus,
  csharpLspRestart,
  subscribeCsharpLspStatus,
  subscribeUnitySidecarCompilerStatus,
  unityNativeBrokerGetStatus,
  unitySidecarCompilerGetStatus,
  unityStateProbeGetStatus,
  type UnityStateProbeStatus,
} from "../../services/csharpLsp";
import { getUnityBackgroundHookStatus } from "../../services/system";
import { getUnityTestToolsWorkspaceStatus, type UnityTestToolsWorkspaceStatus } from "../../services/unity";
import type {
  CsharpCompileStatus,
  CsharpLspStatus,
  UnityBackgroundHookStatus,
  UnityNativeBrokerStatus,
} from "../../types";
import { useHotReloadDebugGuard } from "../../composables/useHotReloadDebugGuard";
import { settingValue } from "../../composables/useSettingsStore";
import { useNotificationStore } from "../../stores/notification";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import LucideIcon from "../icons/LucideIcon.vue";
import BaseButton from "../ui/BaseButton.vue";
import TestingSettings from "./TestingSettings.vue";

/** Dot of a status row: working, in progress, needs attention, failed,
 *  enabled but waiting (hollow), switched off or unsupported (faded). */
type Tone = "ok" | "busy" | "warn" | "error" | "idle" | "off";

interface StatusRow {
  id: string;
  name: string;
  text: string;
  tone: Tone;
}

const notificationStore = useNotificationStore();
const workspaceContextStore = useWorkspaceContextStore();

function scopeKeyOf(scope: { checkoutId: string; expectedGeneration?: number | null } | null): string {
  return scope ? `${scope.checkoutId}:${scope.expectedGeneration ?? ""}` : "";
}
const scopeKey = computed(() => scopeKeyOf(workspaceContextStore.focusedWorkspaceRef));

/** Runs `load` for the focused workspace and drops results that arrive after
 *  the focus moved. */
async function forScope<T>(
  operation: string,
  load: (scope: NonNullable<typeof workspaceContextStore.focusedWorkspaceRef>) => Promise<T>,
  apply: (value: T) => void,
): Promise<void> {
  const scope = workspaceContextStore.focusedWorkspaceRef;
  if (!scope) return;
  const key = scopeKeyOf(scope);
  try {
    const value = await load(scope);
    if (scopeKey.value === key) apply(value);
  } catch (cause) {
    if (scopeKey.value !== key) return;
    const error = normalizeAppError(cause);
    notificationStore.addNotice("error", error.message, { code: error.code, operation, replaceOperation: true });
  }
}

// ── C# code analysis ─────────────────────────────────────────────────

const lspStatus = ref<CsharpLspStatus | null>(null);
const lspRestarting = ref(false);

const lspRow = computed<StatusRow>(() => {
  const status = lspStatus.value;
  const row = (text: string, tone: Tone): StatusRow => ({ id: "lsp", name: t("settings.codeAnalysis.master"), text, tone });
  if (!status) return row(t("common.loading"), "busy");
  if (!status.supported) return row(t("chat.status.code.unsupported"), "off");
  if (!status.enabled) return row(t("chat.status.code.off"), "off");
  switch (status.phase) {
    case "preparing":
      return row(t("chat.status.code.preparing"), "busy");
    case "downloading":
      return row(t("chat.status.code.downloading", status.downloadComponent ?? ""), "busy");
    case "generating":
      return row(t("chat.status.code.generating"), "busy");
    case "starting":
      return row(t("chat.status.code.starting"), "busy");
    case "loading":
      return row(status.loadedProjects != null && status.projectCount != null
        ? t("chat.status.code.loadingProgress", status.loadedProjects, status.projectCount)
        : t("chat.status.code.loading"), "busy");
    case "ready":
      return row(t("chat.status.code.ready"), "ok");
    case "error":
      return row(status.message || t("chat.status.code.error"), "error");
    default:
      return row(t("chat.status.code.idle"), "ok");
  }
});

const refreshLspStatus = () =>
  forScope("loadCsharpLspStatus", csharpLspGetStatus, (status) => { lspStatus.value = status; });

async function restartLsp() {
  if (lspRestarting.value || !lspStatus.value?.enabled) return;
  lspRestarting.value = true;
  await forScope("restartCsharpLsp", csharpLspRestart, (status) => { lspStatus.value = status; });
  lspRestarting.value = false;
}

// ── compile service and hot reload ───────────────────────────────────

const compileStatus = ref<CsharpCompileStatus | null>(null);

const compileRow = computed<StatusRow>(() => {
  const status = compileStatus.value;
  const row = (text: string, tone: Tone): StatusRow => ({ id: "compile", name: t("settings.unity.compileService"), text, tone });
  if (!status) return row(t("common.loading"), "busy");
  if (!status.platformSupported) return row(t("settings.codeAnalysis.sidecarUnsupported"), "off");
  if (!status.serverAvailable) return row(t("settings.codeAnalysis.sidecarMissing"), "warn");
  if (!status.enabled) return row(t("settings.codeAnalysis.sidecarOff"), "off");
  const stats = (status.sidecarCompiles ?? 0) + (status.fallbacks ?? 0) > 0
    ? ` · ${t("settings.codeAnalysis.sidecarStats", status.sidecarCompiles ?? 0, status.fallbacks ?? 0)}`
    : "";
  if (status.lastError) return row(t("settings.codeAnalysis.sidecarError", status.lastError), "error");
  if (status.running) {
    return row(t("settings.codeAnalysis.sidecarRunning", status.roslynVersion ?? "?", status.dotnetSource ?? "?") + stats, "ok");
  }
  return row(t("settings.codeAnalysis.sidecarIdle") + stats, "ok");
});

const hotReloadEnabled = computed(() => compileStatus.value?.hotReloadEnabled ?? false);

const hotReloadRow = computed<StatusRow>(() => {
  const status = compileStatus.value;
  const pending = (status?.hotUnappliedChanges ?? 0)
    + (status?.hotPatchedCodeCount ?? 0)
    + (status?.hotPatchFailures ?? 0)
    + (status?.hotColdQueued ?? 0);
  const text = pending > 0
    ? t(
      "settings.codeAnalysis.hotReloadStats",
      status?.hotUnappliedChanges ?? 0,
      status?.hotPatchedCodeCount ?? 0,
      status?.hotColdQueued ?? 0,
    )
    : t("settings.unity.hotReloadIdle");
  return {
    id: "hotReload",
    name: t("settings.codeAnalysis.hotReloadLabel"),
    text,
    tone: (status?.hotPatchFailures ?? 0) > 0 ? "warn" : "ok",
  };
});

const refreshCompileStatus = () =>
  forScope("loadUnitySidecarCompilerStatus", unitySidecarCompilerGetStatus, (status) => {
    compileStatus.value = status;
  });

// Hot reload works in Release; the editor's optimization level is only a hint.
const {
  isRelease: hotReloadIsRelease,
  switching: hotReloadSwitching,
  switchError: hotReloadSwitchError,
  refreshOptimization: refreshHotReloadOptimization,
  switchToDebug: hotReloadSwitchToDebug,
} = useHotReloadDebugGuard(() => workspaceContextStore.focusedWorkspaceRef, async () => {});

// ── native plugin bridge ─────────────────────────────────────────────

const nativeBridgeEnabled = computed(() => settingValue("unity.connection.nativeBridge") !== false);
const nativeBrokerStatus = ref<UnityNativeBrokerStatus | null>(null);
const nativeBrokerLoaded = ref(false);
const nativeDetailsOpen = ref(false);

const nativeRow = computed<StatusRow>(() => {
  const row = (text: string, tone: Tone): StatusRow => ({
    id: "native",
    name: t("settings.item.unity.connection.nativeBridge.label"),
    text,
    tone,
  });
  if (!nativeBridgeEnabled.value) return row(t("settings.testing.nativeBridgeOff"), "off");
  if (!nativeBrokerLoaded.value) return row(t("common.loading"), "busy");
  const status = nativeBrokerStatus.value;
  if (!status) return row(t("settings.testing.nativeBrokerMissing"), "warn");
  if (status.managedState === "ready") {
    return row(t("settings.testing.nativeBrokerReady", status.domainGeneration, status.editorStatus || "-"), "ok");
  }
  return row(t("settings.testing.nativeBrokerState", status.managedState || "-", status.domainGeneration), "busy");
});

const nativeBrokerFields = computed(() => {
  const status = nativeBrokerStatus.value;
  if (!status) return [];
  return [
    [t("settings.testing.nativeFieldManagedState"), status.managedState || "-"],
    [t("settings.testing.nativeFieldGeneration"), String(status.domainGeneration ?? "-")],
    [t("settings.testing.nativeFieldEditorStatus"), status.editorStatus || "-"],
    [t("settings.testing.nativeFieldQueue"), `${status.pendingRequests ?? 0} / ${status.inflightRequests ?? 0}`],
    [t("settings.testing.nativeFieldProtocol"), String(status.protocolVersion ?? "-")],
    [t("settings.testing.nativeFieldCapabilities"), status.capabilities?.join(", ") || "-"],
  ];
});

const refreshNativeBridgeStatus = () =>
  forScope("loadUnityNativeBridgeStatus", unityNativeBrokerGetStatus, (status) => {
    nativeBrokerStatus.value = status;
    nativeBrokerLoaded.value = true;
  });

// ── state probe and background hook ──────────────────────────────────

const stateProbeStatus = ref<UnityStateProbeStatus | null>(null);

const stateProbeRow = computed<StatusRow>(() => {
  const status = stateProbeStatus.value;
  const row = (text: string, tone: Tone): StatusRow => ({
    id: "stateProbe",
    name: t("settings.item.unity.connection.stateProbe.label"),
    text,
    tone,
  });
  if (!status) return row(t("common.loading"), "busy");
  if (!status.supported) return row(t("settings.codeAnalysis.stateProbeUnsupported"), "off");
  if (!status.enabled) return row(t("settings.codeAnalysis.stateProbeOff"), "off");
  switch (status.tier) {
    case "passive":
      return row(t("settings.codeAnalysis.stateProbeTierPassive"), "ok");
    case "stack":
      return row(t("settings.codeAnalysis.stateProbeTierStack", status.reloadSymbols), "ok");
    case "cpu_only":
      return row(t("settings.codeAnalysis.stateProbeTierCpuOnly"), "ok");
    case "inference":
      return row(t("settings.codeAnalysis.stateProbeTierInference"), "warn");
    default:
      return row(t("settings.codeAnalysis.stateProbeTierInactive"), "idle");
  }
});

const refreshStateProbeStatus = () =>
  forScope("loadUnityStateProbeStatus", unityStateProbeGetStatus, (status) => {
    stateProbeStatus.value = status;
  });

const backgroundHookStatus = ref<UnityBackgroundHookStatus | null>(null);

const backgroundHookRow = computed<StatusRow>(() => {
  const status = backgroundHookStatus.value;
  const row = (text: string, tone: Tone): StatusRow => ({
    id: "backgroundHook",
    name: t("settings.item.unity.connection.backgroundHook.label"),
    text,
    tone,
  });
  if (!status) return row(t("common.loading"), "busy");
  if (!status.supported) return row(t("settings.codeAnalysis.backgroundHookUnsupported"), "off");
  if (!status.enabled) return row(t("settings.codeAnalysis.backgroundHookOff"), "off");
  switch (status.state) {
    case "patched":
      return row(t("settings.codeAnalysis.backgroundHookPatched"), "ok");
    case "inactive":
      return row(t("settings.codeAnalysis.backgroundHookInactive"), "idle");
    case "failed":
      return row(t("settings.codeAnalysis.backgroundHookFailed", status.error ?? ""), "error");
    case "unsupported":
      return row(t("settings.codeAnalysis.backgroundHookUnsupported"), "off");
    default:
      return row(t("settings.codeAnalysis.backgroundHookOff"), "off");
  }
});

const refreshBackgroundHookStatus = () =>
  forScope("loadUnityBackgroundHookStatus", getUnityBackgroundHookStatus, (status) => {
    backgroundHookStatus.value = status;
  });

// ── Unity Test Framework package ─────────────────────────────────────

const testPackageStatus = ref<UnityTestToolsWorkspaceStatus | null>(null);

const testPackageRow = computed<StatusRow>(() => {
  const status = testPackageStatus.value;
  const row = (text: string, tone: Tone): StatusRow => ({ id: "testPackage", name: t("settings.unity.testPackage"), text, tone });
  if (!status) return row(t("common.loading"), "busy");
  if (!status.packageInstalled) return row(t("settings.unityConnection.unityTestPackageMissing"), "off");
  if (!status.packageSupported) {
    return row(t("settings.unityConnection.unityTestPackageUnsupported", status.packageVersion ?? "unknown"), "warn");
  }
  return row(t("settings.unity.testPackageReady", status.packageVersion ?? ""), "ok");
});

const refreshTestPackageStatus = () =>
  forScope("loadUnityTestToolsWorkspaceStatus", getUnityTestToolsWorkspaceStatus, (status) => {
    testPackageStatus.value = status;
  });

// ── status table ─────────────────────────────────────────────────────

const rows = computed<StatusRow[]>(() => [
  lspRow.value,
  compileRow.value,
  ...(hotReloadEnabled.value ? [hotReloadRow.value] : []),
  nativeRow.value,
  stateProbeRow.value,
  backgroundHookRow.value,
  testPackageRow.value,
]);

const refreshing = ref(false);

async function refreshAll() {
  if (refreshing.value) return;
  refreshing.value = true;
  await Promise.allSettled([
    refreshLspStatus(),
    refreshCompileStatus(),
    refreshNativeBridgeStatus(),
    refreshStateProbeStatus(),
    refreshBackgroundHookStatus(),
    refreshTestPackageStatus(),
    refreshHotReloadOptimization(),
  ]);
  refreshing.value = false;
}

// ── lifecycle ────────────────────────────────────────────────────────

let subscriptions: RuntimeUnsubscribe[] = [];
let bindVersion = 0;
let disposed = false;

function unsubscribeAll() {
  for (const unsubscribe of subscriptions) unsubscribe();
  subscriptions = [];
}

async function bind() {
  const version = ++bindVersion;
  unsubscribeAll();
  lspStatus.value = null;
  compileStatus.value = null;
  nativeBrokerStatus.value = null;
  nativeBrokerLoaded.value = false;
  nativeDetailsOpen.value = false;
  stateProbeStatus.value = null;
  backgroundHookStatus.value = null;
  testPackageStatus.value = null;
  const scope = workspaceContextStore.focusedWorkspaceRef;
  if (!scope || disposed) return;
  const key = scopeKeyOf(scope);
  const current = () => !disposed && version === bindVersion && scopeKey.value === key;

  void refreshAll();
  const results = await Promise.allSettled([
    subscribeCsharpLspStatus(scope, (payload) => {
      if (current()) lspStatus.value = payload;
    }),
    subscribeUnitySidecarCompilerStatus(scope, (payload) => {
      if (current()) compileStatus.value = payload;
    }),
  ]);
  const unsubscribes = results
    .filter((result): result is PromiseFulfilledResult<RuntimeUnsubscribe> => result.status === "fulfilled")
    .map((result) => result.value);
  if (current()) subscriptions = unsubscribes;
  else for (const unsubscribe of unsubscribes) unsubscribe();
}

onMounted(() => {
  disposed = false;
  void bind();
});

watch(scopeKey, () => void bind());

// A switch on the page (or the agent) changed: reread what it affects.
watch(() => settingValue("unity.codeAnalysis.csharpLsp"), () => void refreshLspStatus());
watch(
  () => [settingValue("unity.compile.sidecar"), settingValue("unity.compile.nonPublicAccess"), settingValue("unity.hotReload.enabled")],
  () => {
    void refreshCompileStatus();
    void refreshHotReloadOptimization();
  },
);
watch(() => settingValue("unity.connection.nativeBridge"), () => void refreshNativeBridgeStatus());
watch(() => settingValue("unity.connection.stateProbe"), () => void refreshStateProbeStatus());
watch(() => settingValue("unity.connection.backgroundHook"), () => void refreshBackgroundHookStatus());

onUnmounted(() => {
  disposed = true;
  bindVersion += 1;
  unsubscribeAll();
});
</script>

<template>
  <div class="diagnostics-heading">
    <span class="diagnostics-title">{{ t("settings.unity.statusTitle") }}</span>
    <BaseButton :disabled="refreshing" @click="refreshAll">
      <LucideIcon :icon="RefreshCw" :size="12" />
      {{ t("common.refresh") }}
    </BaseButton>
  </div>

  <div class="status-table" role="list">
    <div v-for="row in rows" :key="row.id" class="status-item" role="listitem" :data-status-id="row.id">
      <div class="status-main">
        <span class="status-name">
          <span class="status-dot" :class="`tone-${row.tone}`" aria-hidden="true" />
          {{ row.name }}
        </span>
        <span class="status-text" :class="{ 'is-error': row.tone === 'error' }">{{ row.text }}</span>
        <span class="status-actions">
          <BaseButton
            v-if="row.id === 'lsp' && lspStatus?.enabled"
            :disabled="lspRestarting"
            :title="t('chat.status.code.restartTitle')"
            @click="restartLsp"
          >
            {{ t("chat.status.code.restart") }}
          </BaseButton>
          <button
            v-if="row.id === 'native' && nativeBridgeEnabled && nativeBrokerFields.length > 0"
            type="button"
            class="status-toggle"
            :aria-expanded="nativeDetailsOpen"
            @click="nativeDetailsOpen = !nativeDetailsOpen"
          >
            <LucideIcon :icon="nativeDetailsOpen ? ChevronDown : ChevronRight" :size="12" />
            {{ t("settings.unity.details") }}
          </button>
        </span>
      </div>

      <div v-if="row.id === 'hotReload' && hotReloadIsRelease" class="status-extra">
        <span class="status-note">{{ t("settings.codeAnalysis.hotReloadReleaseHint") }}</span>
        <BaseButton :disabled="hotReloadSwitching" @click="hotReloadSwitchToDebug">
          {{
            hotReloadSwitching
              ? t("settings.codeAnalysis.hotReloadSwitching")
              : t("settings.codeAnalysis.hotReloadSwitchToDebug")
          }}
        </BaseButton>
        <span v-if="hotReloadSwitchError" class="status-text is-error">{{ hotReloadSwitchError }}</span>
      </div>

      <div v-if="row.id === 'native' && nativeDetailsOpen" class="status-fields">
        <template v-for="[label, value] in nativeBrokerFields" :key="label">
          <span class="status-field-label">{{ label }}</span>
          <span class="status-field-value">{{ value }}</span>
        </template>
      </div>
    </div>
  </div>

  <div class="diagnostics-heading self-test-heading">
    <span class="diagnostics-title">{{ t("settings.unity.selfTestTitle") }}</span>
  </div>
  <p class="section-desc self-test-desc">{{ t("settings.unity.selfTestDesc") }}</p>
  <TestingSettings />
</template>

<style scoped>
.diagnostics-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  max-width: 760px;
  margin-bottom: 8px;
}
.self-test-heading {
  margin-top: 20px;
  margin-bottom: 2px;
}
.self-test-desc {
  max-width: 760px;
}
.diagnostics-title {
  color: var(--text-color);
  font-size: 12.5px;
  font-weight: 600;
}
.diagnostics-heading :deep(button) {
  display: inline-flex;
  align-items: center;
  gap: 5px;
}
.status-table {
  --status-name-width: 240px;
  display: flex;
  flex-direction: column;
  max-width: 760px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
  overflow: hidden;
}
.status-item + .status-item {
  border-top: 1px solid var(--border-color);
}
.status-main {
  display: grid;
  grid-template-columns: var(--status-name-width) minmax(0, 1fr) auto;
  align-items: center;
  gap: 12px;
  min-height: 40px;
  padding: 6px 14px;
}
.status-name {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  color: var(--text-color);
  font-size: 12.5px;
  font-weight: 500;
}
.status-dot {
  flex-shrink: 0;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--text-secondary);
}
.status-dot.tone-ok { background: var(--status-good-fg); }
.status-dot.tone-busy { background: var(--accent-color); }
.status-dot.tone-warn { background: var(--status-warning-fg, #d8a54a); }
.status-dot.tone-error { background: var(--status-danger-fg); }
.status-dot.tone-idle {
  background: transparent;
  box-shadow: inset 0 0 0 1.5px var(--text-secondary);
}
.status-dot.tone-off { background: color-mix(in srgb, var(--text-secondary) 55%, transparent); }
.status-text {
  min-width: 0;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.45;
  overflow-wrap: anywhere;
}
.status-text.is-error {
  color: var(--status-danger-fg);
}
.status-actions {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.status-toggle {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 3px 6px;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 11.5px;
  cursor: pointer;
}
.status-toggle:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}
.status-extra {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px 10px;
  padding: 0 14px 10px calc(var(--status-name-width) + 26px);
}
.status-note {
  color: var(--status-warning-fg, var(--text-secondary));
  font-size: 12px;
  line-height: 1.45;
}
.status-fields {
  display: grid;
  grid-template-columns: 150px minmax(0, 1fr);
  margin: 0 14px 10px calc(var(--status-name-width) + 26px);
  padding: 6px 10px;
  border-radius: 6px;
  background: color-mix(in srgb, var(--input-bg) 70%, transparent);
}
.status-field-label,
.status-field-value {
  min-width: 0;
  padding: 3px 0;
  font-size: 11.5px;
  line-height: 1.45;
}
.status-field-label {
  color: var(--text-secondary);
}
.status-field-value {
  overflow-wrap: anywhere;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
}
</style>
