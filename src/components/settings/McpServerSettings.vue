<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { t } from "../../i18n";
import { useCopyFeedback } from "../../composables/useCopyFeedback";
import { normalizeAppError } from "../../services/errors";
import type { RuntimeUnsubscribe } from "../../services/locusRuntime";
import {
  claudeCodeCommand,
  genericJsonSnippet,
  mcpServerGetState,
  mcpServerIntegrationApply,
  mcpServerIntegrationRemove,
  mcpServerIntegrations,
  mcpServerRegenerateToken,
  mcpServerToolInventory,
  mcpServerUpdateSettings,
  subscribeMcpServerStatus,
  type McpExposedToolInfo,
  type McpIntegrationStatus,
  type McpServerStateView,
  type McpServerStatus,
} from "../../services/mcpServer";
import { useNotificationStore } from "../../stores/notification";
import BaseButton from "../ui/BaseButton.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";

const notificationStore = useNotificationStore();

const state = ref<McpServerStateView | null>(null);
const tools = ref<McpExposedToolInfo[]>([]);
const integrations = ref<McpIntegrationStatus[]>([]);
const ready = ref(false);
const busy = ref(false);
const integrationBusyId = ref<string | null>(null);

const portDraft = ref("");
const timeoutDraft = ref("");

const { copied: tokenCopied, copyText: copyTokenText } = useCopyFeedback();
const { copied: commandCopied, copyText: copyCommandText } = useCopyFeedback();
const { copied: jsonCopied, copyText: copyJsonText } = useCopyFeedback();

/// Two-click confirm for token regeneration (arming resets after 4s).
const regenerateArmed = ref(false);
let regenerateArmTimer: ReturnType<typeof setTimeout> | null = null;

let unsubscribeStatus: RuntimeUnsubscribe | null = null;

const settings = computed(() => state.value?.settings ?? null);
const status = computed(() => state.value?.status ?? null);

const statusLabel = computed(() => {
  if (!ready.value) return t("common.loading");
  const current = status.value;
  if (current?.running) {
    const url = state.value?.endpointUrl ?? "";
    const base = t("settings.mcpServer.statusRunning", url);
    if (current.activeSessions > 0) {
      return `${base} · ${t("settings.mcpServer.statusSessions", current.activeSessions)}`;
    }
    return base;
  }
  if (current?.lastError) return current.lastError;
  return t("settings.mcpServer.statusStopped");
});

const maskedToken = computed(() => {
  const token = settings.value?.token ?? "";
  if (token.length <= 8) return token ? "••••••••" : "";
  return `${token.slice(0, 8)}••••••••`;
});

const claudeCommandSnippet = computed(() =>
  state.value
    ? claudeCodeCommand(state.value.endpointUrl, state.value.settings.token)
    : "",
);
const jsonSnippet = computed(() =>
  state.value
    ? genericJsonSnippet(state.value.endpointUrl, state.value.settings.token)
    : "",
);

function applyState(next: McpServerStateView) {
  state.value = next;
  portDraft.value = String(next.settings.port);
  timeoutDraft.value = String(Math.round(next.settings.callTimeoutMs / 1000));
}

function notifyError(e: unknown, operation: string) {
  const err = normalizeAppError(e);
  notificationStore.addNotice("error", err.message, {
    code: err.code,
    operation,
    replaceOperation: true,
  });
}

async function refreshAll() {
  try {
    applyState(await mcpServerGetState());
    tools.value = await mcpServerToolInventory();
    integrations.value = await mcpServerIntegrations();
  } catch (e) {
    notifyError(e, "loadMcpServerState");
  } finally {
    ready.value = true;
  }
}

async function pushSettings(input: {
  enabled?: boolean;
  port?: number;
  disabledTools?: string[];
  callTimeoutMs?: number;
}) {
  const current = settings.value;
  if (!current || busy.value) return;
  busy.value = true;
  try {
    applyState(
      await mcpServerUpdateSettings({
        enabled: input.enabled ?? current.enabled,
        port: input.port ?? current.port,
        disabledTools: input.disabledTools ?? current.disabledTools,
        callTimeoutMs: input.callTimeoutMs ?? current.callTimeoutMs,
      }),
    );
    // Endpoint identity may have changed; refresh integration states.
    integrations.value = await mcpServerIntegrations();
  } catch (e) {
    notifyError(e, "updateMcpServerSettings");
    await refreshAll();
  } finally {
    busy.value = false;
  }
}

function toggleEnabled() {
  void pushSettings({ enabled: !(settings.value?.enabled ?? false) });
}

function commitPort() {
  const current = settings.value;
  if (!current) return;
  const parsed = Number.parseInt(portDraft.value, 10);
  if (!Number.isFinite(parsed) || parsed < 1024 || parsed > 65535) {
    portDraft.value = String(current.port);
    return;
  }
  if (parsed === current.port) return;
  void pushSettings({ port: parsed });
}

function commitTimeout() {
  const current = settings.value;
  if (!current) return;
  const parsed = Number.parseInt(timeoutDraft.value, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    timeoutDraft.value = String(Math.round(current.callTimeoutMs / 1000));
    return;
  }
  const ms = parsed * 1000;
  if (ms === current.callTimeoutMs) return;
  void pushSettings({ callTimeoutMs: ms });
}

function toggleTool(tool: McpExposedToolInfo) {
  const current = settings.value;
  if (!current || !tool.available) return;
  const disabled = new Set(current.disabledTools);
  if (disabled.has(tool.name)) {
    disabled.delete(tool.name);
  } else {
    disabled.add(tool.name);
  }
  tools.value = tools.value.map((row) =>
    row.name === tool.name ? { ...row, enabled: !disabled.has(tool.name) ? true : false } : row,
  );
  void pushSettings({ disabledTools: [...disabled] }).then(async () => {
    tools.value = await mcpServerToolInventory().catch(() => tools.value);
  });
}

async function copyToken() {
  const token = settings.value?.token ?? "";
  if (!token) return;
  const copied = await copyTokenText(token);
  if (!copied) {
    notificationStore.addNotice("warning", t("settings.mcpServer.copyFailed"), {
      operation: "copyMcpServerToken",
      replaceOperation: true,
    });
  }
}

function requestRegenerate() {
  if (!regenerateArmed.value) {
    regenerateArmed.value = true;
    if (regenerateArmTimer) clearTimeout(regenerateArmTimer);
    regenerateArmTimer = setTimeout(() => {
      regenerateArmed.value = false;
    }, 4000);
    return;
  }
  regenerateArmed.value = false;
  if (regenerateArmTimer) {
    clearTimeout(regenerateArmTimer);
    regenerateArmTimer = null;
  }
  void (async () => {
    busy.value = true;
    try {
      applyState(await mcpServerRegenerateToken());
      integrations.value = await mcpServerIntegrations();
    } catch (e) {
      notifyError(e, "regenerateMcpServerToken");
    } finally {
      busy.value = false;
    }
  })();
}

function integrationStateLabel(row: McpIntegrationStatus): string {
  switch (row.state) {
    case "current":
      return t("settings.mcpServer.integrationCurrent");
    case "stale":
      return t("settings.mcpServer.integrationStale");
    default:
      return t("settings.mcpServer.integrationAbsent");
  }
}

function integrationActionLabel(row: McpIntegrationStatus): string {
  switch (row.state) {
    case "current":
      return t("settings.mcpServer.integrationRemove");
    case "stale":
      return t("settings.mcpServer.integrationUpdate");
    default:
      return t("settings.mcpServer.integrationApply");
  }
}

async function runIntegrationAction(row: McpIntegrationStatus) {
  if (integrationBusyId.value) return;
  integrationBusyId.value = row.id;
  try {
    const next =
      row.state === "current"
        ? await mcpServerIntegrationRemove(row.id)
        : await mcpServerIntegrationApply(row.id);
    integrations.value = integrations.value.map((entry) =>
      entry.id === next.id ? next : entry,
    );
  } catch (e) {
    notifyError(e, "mcpServerIntegration");
  } finally {
    integrationBusyId.value = null;
  }
}

async function copySnippet(kind: "command" | "json") {
  const text = kind === "command" ? claudeCommandSnippet.value : jsonSnippet.value;
  if (!text) return;
  const copy = kind === "command" ? copyCommandText : copyJsonText;
  const copied = await copy(text);
  if (!copied) {
    notificationStore.addNotice("warning", t("settings.mcpServer.copyFailed"), {
      operation: "copyMcpServerSnippet",
      replaceOperation: true,
    });
  }
}

onMounted(() => {
  void refreshAll();
  void subscribeMcpServerStatus((payload: McpServerStatus) => {
    if (state.value) {
      state.value = { ...state.value, status: payload };
    }
  }).then((unsubscribe) => {
    unsubscribeStatus = unsubscribe;
  });
});

onUnmounted(() => {
  unsubscribeStatus?.();
  unsubscribeStatus = null;
  if (regenerateArmTimer) {
    clearTimeout(regenerateArmTimer);
    regenerateArmTimer = null;
  }
});
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.mcpServer.title") }}</div>
    <p class="section-desc">{{ t("settings.mcpServer.subtitle") }}</p>

    <div class="tool-card">
      <div class="tool-row master-row">
        <div class="tool-info">
          <span class="tool-name">{{ t("settings.mcpServer.enable") }}</span>
          <span class="tool-desc" :class="{ 'status-error': !!status?.lastError }">
            {{ statusLabel }}
          </span>
        </div>
        <div class="master-actions">
          <BaseSwitch
            v-if="ready"
            :model-value="settings?.enabled ?? false"
            :disabled="busy"
            :aria-label="t('settings.mcpServer.enable')"
            @update:model-value="toggleEnabled"
          />
          <span v-else class="switch-placeholder" aria-hidden="true" />
        </div>
      </div>

      <div class="tool-row">
        <div class="tool-info">
          <span class="tool-name">{{ t("settings.mcpServer.port") }}</span>
          <span class="tool-desc">{{ t("settings.mcpServer.portHint") }}</span>
        </div>
        <div class="master-actions">
          <input
            v-model="portDraft"
            class="num-input"
            type="number"
            min="1024"
            max="65535"
            :disabled="busy || !ready"
            :aria-label="t('settings.mcpServer.port')"
            @blur="commitPort"
            @keydown.enter="($event.target as HTMLInputElement).blur()"
          />
        </div>
      </div>

      <div class="tool-row">
        <div class="tool-info">
          <span class="tool-name">{{ t("settings.mcpServer.timeout") }}</span>
        </div>
        <div class="master-actions">
          <input
            v-model="timeoutDraft"
            class="num-input"
            type="number"
            min="10"
            :disabled="busy || !ready"
            :aria-label="t('settings.mcpServer.timeout')"
            @blur="commitTimeout"
            @keydown.enter="($event.target as HTMLInputElement).blur()"
          />
        </div>
      </div>

      <div class="tool-row">
        <div class="tool-info">
          <span class="tool-name">{{ t("settings.mcpServer.token") }}</span>
          <span class="tool-desc token-value">{{ maskedToken }}</span>
          <span v-if="regenerateArmed" class="tool-desc status-error">
            {{ t("settings.mcpServer.regenerateConfirm") }}
          </span>
        </div>
        <div class="master-actions">
          <BaseButton :disabled="!settings?.token" @click="copyToken">
            {{ tokenCopied ? t("common.copied") : t("settings.mcpServer.copyToken") }}
          </BaseButton>
          <BaseButton :disabled="busy || !ready" @click="requestRegenerate">
            {{
              regenerateArmed
                ? t("settings.mcpServer.regenerateArmed")
                : t("settings.mcpServer.regenerateToken")
            }}
          </BaseButton>
        </div>
      </div>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.mcpServer.integrations") }}</div>
    <p class="section-desc">{{ t("settings.mcpServer.integrationsDesc") }}</p>
    <div class="tool-card">
      <div v-for="row in integrations" :key="row.id" class="tool-row">
        <div class="tool-info">
          <span class="tool-name">{{ row.name }}</span>
          <span class="tool-desc">
            <template v-if="row.detected">
              {{ integrationStateLabel(row) }} · {{ row.configPath }}
            </template>
            <template v-else>{{ t("settings.mcpServer.integrationNotDetected") }}</template>
          </span>
          <span v-if="row.id === 'codex'" class="tool-desc">
            {{ t("settings.mcpServer.codexHeaderHint") }}
          </span>
        </div>
        <div class="master-actions">
          <span
            class="integration-badge"
            :class="`integration-${row.state}`"
            :data-state="row.state"
          >
            {{ integrationStateLabel(row) }}
          </span>
          <BaseButton
            :disabled="!row.detected || integrationBusyId !== null"
            @click="runIntegrationAction(row)"
          >
            {{ integrationActionLabel(row) }}
          </BaseButton>
        </div>
      </div>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.mcpServer.manualSetup") }}</div>
    <p class="section-desc">{{ t("settings.mcpServer.manualSetupDesc") }}</p>
    <div class="tool-card">
      <div class="snippet-block">
        <div class="snippet-header">
          <span class="snippet-title">{{ t("settings.mcpServer.claudeCommand") }}</span>
          <BaseButton :disabled="!claudeCommandSnippet" @click="copySnippet('command')">
            {{ commandCopied ? t("common.copied") : t("settings.mcpServer.copy") }}
          </BaseButton>
        </div>
        <pre class="snippet-body">{{ claudeCommandSnippet }}</pre>
      </div>
      <div class="snippet-block">
        <div class="snippet-header">
          <span class="snippet-title">{{ t("settings.mcpServer.genericJson") }}</span>
          <BaseButton :disabled="!jsonSnippet" @click="copySnippet('json')">
            {{ jsonCopied ? t("common.copied") : t("settings.mcpServer.copy") }}
          </BaseButton>
        </div>
        <pre class="snippet-body">{{ jsonSnippet }}</pre>
      </div>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.mcpServer.tools") }}</div>
    <p class="section-desc">{{ t("settings.mcpServer.toolsHint") }}</p>
    <div class="tool-card">
      <div
        v-for="tool in tools"
        :key="tool.name"
        class="tool-row"
        :class="{ 'tool-row-unavailable': !tool.available }"
      >
        <div class="tool-info">
          <span class="tool-name mono">{{ tool.name }}</span>
          <span class="tool-desc">{{ tool.description }}</span>
          <span v-if="!tool.available && tool.unavailableReason" class="tool-desc status-error">
            {{ t("settings.mcpServer.toolUnavailable", tool.unavailableReason) }}
          </span>
        </div>
        <div class="master-actions">
          <BaseSwitch
            :model-value="tool.enabled && tool.available"
            :disabled="busy || !tool.available"
            :aria-label="tool.name"
            @update:model-value="toggleTool(tool)"
          />
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
.snippet-block + .snippet-block {
  border-top: 1px solid var(--border-color);
}
.tool-row:hover {
  background: var(--hover-bg, rgba(128, 128, 128, 0.08));
}
.tool-row-unavailable {
  opacity: 0.55;
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
.tool-name.mono {
  font-family: var(--font-mono-identifier);
}
.tool-desc {
  font-size: 11.5px;
  color: var(--text-secondary);
  line-height: 1.45;
  overflow-wrap: anywhere;
}
.token-value {
  font-family: var(--font-mono-identifier);
}
.status-error {
  color: var(--error-color, #e5484d);
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
.num-input {
  width: 92px;
  padding: 5px 8px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 12px;
  font-family: var(--font-mono-identifier);
}
.num-input:focus {
  outline: none;
  border-color: var(--accent-color);
}
.integration-badge {
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 10.5px;
  font-weight: 600;
  letter-spacing: 0.2px;
  border: 1px solid var(--border-color);
  color: var(--text-secondary);
}
.integration-current {
  color: var(--success-color, #2f9e5f);
  border-color: color-mix(in srgb, var(--success-color, #2f9e5f) 45%, transparent);
}
.integration-stale {
  color: var(--warning-color, #d7a021);
  border-color: color-mix(in srgb, var(--warning-color, #d7a021) 45%, transparent);
}
.snippet-block {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 11px 16px;
}
.snippet-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.snippet-title {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.3px;
  text-transform: uppercase;
  color: var(--text-secondary);
}
.snippet-body {
  margin: 0;
  padding: 9px 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--input-bg) 90%, var(--panel-bg) 10%);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  line-height: 1.55;
  color: var(--text-secondary);
  white-space: pre-wrap;
  word-break: break-all;
  user-select: text;
}
</style>
