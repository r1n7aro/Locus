<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { t } from "../../i18n";
import BaseSwitch from "../ui/BaseSwitch.vue";
import {
  codeAnalysisToolsGetConfig,
  codeAnalysisToolsSetConfig,
  csharpLspGetStatus,
  csharpLspRestart,
  csharpLspSetEnabled,
  subscribeCsharpLspStatus,
  unitySidecarCompilerGetStatus,
  unitySidecarCompilerSetEnabled,
} from "../../services/csharpLsp";
import type { CodeAnalysisToolsConfig, CsharpCompileStatus, CsharpLspStatus } from "../../types";
import { defaultCodeAnalysisToolsConfig } from "../../types";
import type { RuntimeUnsubscribe } from "../../services/locusRuntime";
import { normalizeAppError } from "../../services/errors";
import { useNotificationStore } from "../../stores/notification";

const notificationStore = useNotificationStore();

const lspStatus = ref<CsharpLspStatus | null>(null);
const lspReady = ref(false);
const lspBusy = ref(false);
const restartBusy = ref(false);

const codeTools = ref<CodeAnalysisToolsConfig>(defaultCodeAnalysisToolsConfig());
const codeToolsReady = ref(false);
const codeToolsBusy = ref(false);

const sidecarStatus = ref<CsharpCompileStatus | null>(null);
const sidecarReady = ref(false);
const sidecarBusy = ref(false);

let unsubscribeStatus: RuntimeUnsubscribe | null = null;

const lspEnabled = computed(() => lspStatus.value?.enabled ?? false);

const lspStatusLabel = computed(() => {
  const status = lspStatus.value;
  if (!status) return t("common.loading");
  if (!status.supported) return t("chat.status.code.unsupported");
  if (!status.enabled) return t("chat.status.code.off");
  switch (status.phase) {
    case "preparing":
      return t("chat.status.code.preparing");
    case "downloading":
      return t("chat.status.code.downloading", status.downloadComponent ?? "");
    case "starting":
      return t("chat.status.code.starting");
    case "loading":
      if (status.loadedProjects != null && status.projectCount != null) {
        return t(
          "chat.status.code.loadingProgress",
          status.loadedProjects,
          status.projectCount,
        );
      }
      return t("chat.status.code.loading");
    case "ready":
      return t("chat.status.code.ready");
    case "error":
      return status.message || t("chat.status.code.error");
    default:
      return t("chat.status.code.idle");
  }
});

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

type CodeToolKey = keyof CodeAnalysisToolsConfig;

interface CodeToolItem {
  key: CodeToolKey;
  label: string;
  desc: string;
}

const lspToolItems = computed<CodeToolItem[]>(() => [
  { key: "codeSymbolSearch", label: "code_symbol_search", desc: t("settings.codeAnalysis.tool.codeSymbolSearch") },
  { key: "codeGotoDefinition", label: "code_goto_definition", desc: t("settings.codeAnalysis.tool.codeGotoDefinition") },
  { key: "codeFindReferences", label: "code_find_references", desc: t("settings.codeAnalysis.tool.codeFindReferences") },
  { key: "codeDiagnostics", label: "code_diagnostics", desc: t("settings.codeAnalysis.tool.codeDiagnostics") },
  { key: "codeHover", label: "code_hover", desc: t("settings.codeAnalysis.tool.codeHover") },
  { key: "unityAnalyzers", label: t("settings.codeAnalysis.tool.unityAnalyzersLabel"), desc: t("settings.codeAnalysis.tool.unityAnalyzers") },
]);

const assetToolItems = computed<CodeToolItem[]>(() => [
  { key: "unityCodeUsages", label: "unity_code_usages", desc: t("settings.codeAnalysis.tool.unityCodeUsages") },
]);

async function refreshLspStatus() {
  try {
    lspStatus.value = await csharpLspGetStatus();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadCsharpLspStatus",
    });
  } finally {
    lspReady.value = true;
  }
}

async function toggleLspEnabled() {
  if (!lspReady.value || lspBusy.value) return;
  lspBusy.value = true;
  try {
    lspStatus.value = await csharpLspSetEnabled(!lspEnabled.value);
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "toggleCsharpLsp",
      replaceOperation: true,
    });
    await refreshLspStatus();
  } finally {
    lspBusy.value = false;
  }
}

async function restartLsp() {
  if (restartBusy.value || !lspEnabled.value) return;
  restartBusy.value = true;
  try {
    lspStatus.value = await csharpLspRestart();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "restartCsharpLsp",
      replaceOperation: true,
    });
  } finally {
    restartBusy.value = false;
  }
}

async function refreshCodeTools() {
  try {
    codeTools.value = await codeAnalysisToolsGetConfig();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadCodeAnalysisTools",
    });
  } finally {
    codeToolsReady.value = true;
  }
}

async function toggleCodeTool(key: CodeToolKey) {
  if (!codeToolsReady.value || codeToolsBusy.value) return;
  codeToolsBusy.value = true;
  const next = { ...codeTools.value, [key]: !codeTools.value[key] };
  codeTools.value = next;
  try {
    codeTools.value = await codeAnalysisToolsSetConfig(next);
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "saveCodeAnalysisTools",
      replaceOperation: true,
    });
    await refreshCodeTools();
  } finally {
    codeToolsBusy.value = false;
  }
}

onMounted(() => {
  void refreshLspStatus();
  void refreshCodeTools();
  void refreshSidecarStatus();
  void subscribeCsharpLspStatus((payload) => {
    lspStatus.value = payload;
  }).then((unsubscribe) => {
    unsubscribeStatus = unsubscribe;
  });
});

onUnmounted(() => {
  unsubscribeStatus?.();
  unsubscribeStatus = null;
});
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.codeAnalysis.master") }}</div>
    <p class="section-desc">{{ t("settings.codeAnalysis.masterDesc") }}</p>
    <div class="tool-card">
      <div class="tool-row master-row">
        <div class="tool-info">
          <span class="tool-name">{{ t("chat.status.code.title") }}</span>
          <span class="tool-desc" :class="{ 'status-error': lspStatus?.phase === 'error' }">
            {{ lspStatusLabel }}
          </span>
        </div>
        <div class="master-actions">
          <button
            v-if="lspEnabled"
            class="action-btn"
            :disabled="restartBusy"
            :title="t('chat.status.code.restartTitle')"
            @click="restartLsp"
          >
            {{ t("chat.status.code.restart") }}
          </button>
          <BaseSwitch
            v-if="lspReady"
            :model-value="lspEnabled"
            :disabled="lspBusy || !(lspStatus?.supported ?? true)"
            :aria-label="t('chat.status.code.title')"
            @update:model-value="toggleLspEnabled"
          />
          <span v-else class="switch-placeholder" aria-hidden="true" />
        </div>
      </div>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.codeAnalysis.sidecar") }}</div>
    <p class="section-desc">{{ t("settings.codeAnalysis.sidecarDesc") }}</p>
    <div class="tool-card">
      <div class="tool-row master-row">
        <div class="tool-info">
          <span class="tool-name">{{ t("settings.codeAnalysis.sidecarLabel") }}</span>
          <span class="tool-desc" :class="{ 'status-error': sidecarHasIssue }">
            {{ sidecarStatusLabel }}
          </span>
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
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.codeAnalysis.lspTools") }}</div>
    <p class="section-desc">{{ t("settings.codeAnalysis.lspToolsDesc") }}</p>
    <div class="tool-card" :class="{ 'card-dimmed': !lspEnabled }" :aria-busy="!codeToolsReady">
      <div v-for="item in lspToolItems" :key="item.key" class="tool-row">
        <div class="tool-info">
          <span class="tool-name">{{ item.label }}</span>
          <span class="tool-desc">{{ item.desc }}</span>
        </div>
        <BaseSwitch
          v-if="codeToolsReady"
          :model-value="codeTools[item.key]"
          :disabled="codeToolsBusy"
          :aria-label="item.label"
          @update:model-value="toggleCodeTool(item.key)"
        />
        <span v-else class="switch-placeholder" aria-hidden="true" />
      </div>
    </div>
    <p v-if="!lspEnabled" class="section-desc tools-note">
      {{ t("settings.codeAnalysis.lspToolsOffNote") }}
    </p>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.codeAnalysis.assetTools") }}</div>
    <p class="section-desc">{{ t("settings.codeAnalysis.assetToolsDesc") }}</p>
    <div class="tool-card" :aria-busy="!codeToolsReady">
      <div v-for="item in assetToolItems" :key="item.key" class="tool-row">
        <div class="tool-info">
          <span class="tool-name">{{ item.label }}</span>
          <span class="tool-desc">{{ item.desc }}</span>
        </div>
        <BaseSwitch
          v-if="codeToolsReady"
          :model-value="codeTools[item.key]"
          :disabled="codeToolsBusy"
          :aria-label="item.label"
          @update:model-value="toggleCodeTool(item.key)"
        />
        <span v-else class="switch-placeholder" aria-hidden="true" />
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
.card-dimmed {
  opacity: 0.65;
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
.tools-note {
  margin-top: 10px;
}
</style>
