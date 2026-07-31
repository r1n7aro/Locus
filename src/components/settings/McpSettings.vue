<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { t } from "../../i18n";
import BaseButton from "../ui/BaseButton.vue";
import BaseCheckbox from "../ui/BaseCheckbox.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";
import BaseDropdown from "../ui/BaseDropdown.vue";
import {
  DEFAULT_MCP_CALL_TIMEOUT_MS,
  MCP_PRESETS,
  emptyMcpServerConfig,
  mcpImportApply,
  mcpImportScan,
  mcpServersGet,
  mcpServersRemove,
  mcpServersUpsert,
  mcpServerTest,
  mcpServerToolsInventory,
  mcpServerWireTools,
  type McpImportCandidate,
  type McpLoadMode,
  type McpPreset,
  type McpServerConfig,
  type McpServerTestResult,
  type McpToolSummary,
  type McpTransport,
} from "../../services/mcp";
import { getToolPermissions, saveToolPermissions } from "../../services/permissions";
import { normalizeAppError } from "../../services/errors";
import { useNotificationStore } from "../../stores/notification";

const notificationStore = useNotificationStore();

const servers = ref<McpServerConfig[]>([]);
const ready = ref(false);
const busy = ref(false);

// ── Inline add/edit form ────────────────────────────────────────────────
const editingId = ref<string | null>(null);
const adding = ref(false);
const formName = ref("");
const formTransport = ref<McpTransport>("stdio");
const formCommand = ref("");
const formArgsText = ref("");
const formEnvText = ref("");
const formCwd = ref("");
const formUrl = ref("");
const formHeadersText = ref("");
const formTimeoutSecs = ref(DEFAULT_MCP_CALL_TIMEOUT_MS / 1000);
const formLoadMode = ref<McpLoadMode>("lazy");
const formAutoRestart = ref(false);
const formError = ref("");
const formPresetNote = ref("");

// ── Per-tool toggles ────────────────────────────────────────────────────
// Rows merge every known source of tool names: the runtime inventory of a
// saved server, a successful form test, and names already present in the
// stored allow/deny lists (so stale entries stay editable). Saving encodes
// the switches back as a denylist; new server tools default to enabled.
const formToolRows = ref<McpToolSummary[]>([]);
const formDisabledTools = ref<Set<string>>(new Set());
const formToolsLoading = ref(false);
// Allow/deny snapshot from when the form opened; decides the initial switch
// state of tool names that arrive later (inventory fetch, test result).
let formInitialAllowlist: string[] = [];
let formInitialDenylist: string[] = [];

// ── Connection test state ───────────────────────────────────────────────
const testingId = ref<string | null>(null);
const testResults = ref<Record<string, McpServerTestResult>>({});
const formTesting = ref(false);
const formTestResult = ref<McpServerTestResult | null>(null);

// ── Import from other clients ───────────────────────────────────────────
const importOpen = ref(false);
const importScanning = ref(false);
const importApplying = ref(false);
const importCandidates = ref<McpImportCandidate[]>([]);
const importSelected = ref<Set<number>>(new Set());

// ── Per-server approval bulk action ─────────────────────────────────────
const approvalBusy = ref(false);
const approvalNote = ref("");

const formOpen = computed(() => adding.value || editingId.value !== null);

const transportOptions = computed(() => [
  { value: "stdio", label: t("settings.mcp.form.transportStdio") },
  { value: "http", label: t("settings.mcp.form.transportHttp") },
]);

const loadModeOptions = computed(() => [
  { value: "lazy", label: t("settings.mcp.form.loadModeLazy") },
  { value: "direct", label: t("settings.mcp.form.loadModeDirect") },
]);

function connectionSummary(server: McpServerConfig): string {
  if (server.transport === "http") {
    return server.url;
  }
  return [server.command, ...server.args].join(" ");
}

function parseLinesText(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

function parseKeyValueText(text: string): Record<string, string> {
  const map: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const eq = trimmed.indexOf("=");
    if (eq <= 0) continue;
    map[trimmed.slice(0, eq).trim()] = trimmed.slice(eq + 1).trim();
  }
  return map;
}

function keyValueToText(map: Record<string, string>): string {
  return Object.entries(map)
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
}

function buildFormConfig(): McpServerConfig {
  const base = editingId.value
    ? servers.value.find((s) => s.id === editingId.value) ?? emptyMcpServerConfig()
    : emptyMcpServerConfig();
  // With tool rows on screen the switches carry the whole intent: disabled
  // rows become the denylist and the allowlist clears, so tools the server
  // adds later start enabled. Without rows (never connected, nothing
  // configured) the stored lists pass through untouched.
  const toolGovernance = formToolRows.value.length > 0
    ? {
        toolAllowlist: [],
        toolDenylist: formToolRows.value
          .filter((row) => formDisabledTools.value.has(row.name))
          .map((row) => row.name),
      }
    : {
        toolAllowlist: base.toolAllowlist ?? [],
        toolDenylist: base.toolDenylist ?? [],
      };
  return {
    ...base,
    id: editingId.value ?? "",
    name: formName.value.trim(),
    transport: formTransport.value,
    command: formCommand.value.trim(),
    args: parseLinesText(formArgsText.value),
    env: parseKeyValueText(formEnvText.value),
    cwd: formCwd.value.trim(),
    url: formUrl.value.trim(),
    headers: parseKeyValueText(formHeadersText.value),
    callTimeoutMs: Math.round(Math.max(1, formTimeoutSecs.value) * 1000),
    loadMode: formLoadMode.value,
    autoRestart: formTransport.value === "stdio" && formAutoRestart.value,
    ...toolGovernance,
  };
}

function resetForm() {
  formName.value = "";
  formTransport.value = "stdio";
  formCommand.value = "";
  formArgsText.value = "";
  formEnvText.value = "";
  formCwd.value = "";
  formUrl.value = "";
  formHeadersText.value = "";
  formTimeoutSecs.value = DEFAULT_MCP_CALL_TIMEOUT_MS / 1000;
  formLoadMode.value = "lazy";
  formAutoRestart.value = false;
  formToolRows.value = [];
  formDisabledTools.value = new Set();
  formToolsLoading.value = false;
  formInitialAllowlist = [];
  formInitialDenylist = [];
  formError.value = "";
  formPresetNote.value = "";
  formTestResult.value = null;
  approvalNote.value = "";
}

function openAddForm() {
  adding.value = true;
  editingId.value = null;
  resetForm();
}

function openEditForm(server: McpServerConfig) {
  adding.value = false;
  editingId.value = server.id;
  resetForm();
  formName.value = server.name;
  formTransport.value = server.transport;
  formCommand.value = server.command;
  formArgsText.value = server.args.join("\n");
  formEnvText.value = keyValueToText(server.env);
  formCwd.value = server.cwd;
  formUrl.value = server.url;
  formHeadersText.value = keyValueToText(server.headers);
  formTimeoutSecs.value = Math.round(server.callTimeoutMs / 1000);
  formLoadMode.value = server.loadMode ?? "lazy";
  formAutoRestart.value = server.autoRestart ?? false;
  formInitialAllowlist = [...(server.toolAllowlist ?? [])];
  formInitialDenylist = [...(server.toolDenylist ?? [])];
  // Names already governed by the stored lists render immediately (without
  // descriptions); the runtime inventory fills in the rest asynchronously.
  mergeFormToolRows(
    [...formInitialAllowlist, ...formInitialDenylist]
      .map((name) => name.trim())
      .filter((name) => name.length > 0)
      .map((name) => ({ name, description: "" })),
  );
  void loadServerToolInventory(server.id);
}

/// Mirrors the backend's `McpServerConfig::tool_exposed`: deny wins, then a
/// non-empty allowlist restricts, otherwise everything is exposed.
function toolExposedByInitialLists(name: string): boolean {
  if (formInitialDenylist.some((t) => t.trim() === name)) return false;
  const allow = formInitialAllowlist.map((t) => t.trim()).filter((t) => t.length > 0);
  return allow.length === 0 || allow.includes(name);
}

/// Adds unseen tool names as rows (switch state derived from the stored
/// lists) and backfills descriptions on rows seeded without one. Rows the
/// user already toggled keep their state.
function mergeFormToolRows(tools: McpToolSummary[]) {
  const rows = formToolRows.value;
  const known = new Map(rows.map((row) => [row.name, row]));
  const disabled = new Set(formDisabledTools.value);
  for (const tool of tools) {
    const existing = known.get(tool.name);
    if (existing) {
      if (!existing.description && tool.description) {
        existing.description = tool.description;
      }
      continue;
    }
    const row = { name: tool.name, description: tool.description };
    rows.push(row);
    known.set(tool.name, row);
    if (!toolExposedByInitialLists(tool.name)) {
      disabled.add(tool.name);
    }
  }
  formDisabledTools.value = disabled;
}

function setFormToolEnabled(name: string, enabled: boolean) {
  const disabled = new Set(formDisabledTools.value);
  if (enabled) {
    disabled.delete(name);
  } else {
    disabled.add(name);
  }
  formDisabledTools.value = disabled;
}

async function loadServerToolInventory(serverId: string) {
  formToolsLoading.value = true;
  try {
    const tools = await mcpServerToolsInventory(serverId);
    // The user may have switched to another server (or closed the form)
    // while the request was in flight.
    if (editingId.value !== serverId) return;
    mergeFormToolRows(tools);
  } catch {
    // Lazy servers that never connected simply have no inventory yet; the
    // empty-state hint points at the Test button.
  } finally {
    if (editingId.value === serverId) {
      formToolsLoading.value = false;
    }
  }
}

function closeForm() {
  adding.value = false;
  editingId.value = null;
  formError.value = "";
  formPresetNote.value = "";
  formTestResult.value = null;
}

function applyPreset(preset: McpPreset) {
  const config = preset.config;
  if (config.name !== undefined && !formName.value.trim()) formName.value = config.name;
  if (config.transport !== undefined) formTransport.value = config.transport;
  if (config.command !== undefined) formCommand.value = config.command;
  if (config.args !== undefined) formArgsText.value = config.args.join("\n");
  if (config.env !== undefined) formEnvText.value = keyValueToText(config.env);
  if (config.url !== undefined) formUrl.value = config.url;
  if (config.headers !== undefined) formHeadersText.value = keyValueToText(config.headers);
  if (config.callTimeoutMs !== undefined) {
    formTimeoutSecs.value = Math.round(config.callTimeoutMs / 1000);
  }
  formPresetNote.value = t(preset.noteKey);
}

async function refresh() {
  try {
    servers.value = await mcpServersGet();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadMcpServers",
    });
  } finally {
    ready.value = true;
  }
}

async function saveForm() {
  if (busy.value) return;
  formError.value = "";
  const config = buildFormConfig();
  if (!config.name) {
    formError.value = t("settings.mcp.form.nameRequired");
    return;
  }
  if (config.transport === "stdio" && !config.command) {
    formError.value = t("settings.mcp.form.commandRequired");
    return;
  }
  if (config.transport === "http" && !config.url) {
    formError.value = t("settings.mcp.form.urlRequired");
    return;
  }
  busy.value = true;
  try {
    servers.value = await mcpServersUpsert(config);
    closeForm();
  } catch (e) {
    formError.value = normalizeAppError(e).message;
  } finally {
    busy.value = false;
  }
}

async function removeServer(server: McpServerConfig) {
  if (busy.value) return;
  busy.value = true;
  try {
    servers.value = await mcpServersRemove(server.id);
    delete testResults.value[server.id];
    if (editingId.value === server.id) closeForm();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "removeMcpServer",
      replaceOperation: true,
    });
  } finally {
    busy.value = false;
  }
}

async function toggleEnabled(server: McpServerConfig) {
  if (busy.value) return;
  busy.value = true;
  try {
    servers.value = await mcpServersUpsert({ ...server, enabled: !server.enabled });
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "toggleMcpServer",
      replaceOperation: true,
    });
    await refresh();
  } finally {
    busy.value = false;
  }
}

async function testServer(server: McpServerConfig) {
  if (testingId.value) return;
  testingId.value = server.id;
  delete testResults.value[server.id];
  try {
    testResults.value = {
      ...testResults.value,
      [server.id]: await mcpServerTest(server),
    };
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "testMcpServer",
      replaceOperation: true,
    });
  } finally {
    testingId.value = null;
  }
}

async function testForm() {
  if (formTesting.value) return;
  formError.value = "";
  const config = buildFormConfig();
  if (config.transport === "stdio" && !config.command) {
    formError.value = t("settings.mcp.form.commandRequired");
    return;
  }
  if (config.transport === "http" && !config.url) {
    formError.value = t("settings.mcp.form.urlRequired");
    return;
  }
  formTesting.value = true;
  formTestResult.value = null;
  try {
    const result = await mcpServerTest(config);
    formTestResult.value = result;
    if (result.ok) {
      mergeFormToolRows(result.tools);
    }
  } catch (e) {
    formError.value = normalizeAppError(e).message;
  } finally {
    formTesting.value = false;
  }
}

function testSummary(result: McpServerTestResult): string {
  const name = result.serverName || "?";
  const version = result.serverVersion ? ` ${result.serverVersion}` : "";
  const protocol = result.protocolVersion || "?";
  return t("settings.mcp.test.summary", `${name}${version}`, protocol, result.tools.length, Math.round(result.elapsedMs / 100) / 10);
}

// ── Import ──────────────────────────────────────────────────────────────

async function openImport() {
  importOpen.value = true;
  importScanning.value = true;
  importCandidates.value = [];
  importSelected.value = new Set();
  try {
    importCandidates.value = await mcpImportScan();
    const selected = new Set<number>();
    importCandidates.value.forEach((candidate, index) => {
      if (!candidate.duplicateOf) selected.add(index);
    });
    importSelected.value = selected;
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "scanMcpImports",
    });
  } finally {
    importScanning.value = false;
  }
}

function toggleImportSelection(index: number) {
  const selected = new Set(importSelected.value);
  if (selected.has(index)) {
    selected.delete(index);
  } else {
    selected.add(index);
  }
  importSelected.value = selected;
}

function importSourceLabel(candidate: McpImportCandidate): string {
  switch (candidate.source) {
    case "claude_desktop":
      return "Claude Desktop";
    case "claude_code":
      return "Claude Code";
    case "cursor":
      return "Cursor";
    default:
      return candidate.source;
  }
}

async function applyImport() {
  if (importApplying.value) return;
  const picked = importCandidates.value.filter((_, index) => importSelected.value.has(index));
  if (picked.length === 0) {
    importOpen.value = false;
    return;
  }
  importApplying.value = true;
  try {
    servers.value = await mcpImportApply(picked.map((c) => c.server));
    importOpen.value = false;
    notificationStore.addNotice("info", t("settings.mcp.import.done", picked.length), {
      operation: "applyMcpImports",
      replaceOperation: true,
    });
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "applyMcpImports",
      replaceOperation: true,
    });
  } finally {
    importApplying.value = false;
  }
}

// ── Approval bulk action ────────────────────────────────────────────────

async function setServerApproval(mode: "auto" | "ask") {
  if (approvalBusy.value || !editingId.value) return;
  approvalBusy.value = true;
  approvalNote.value = "";
  try {
    const tools = await mcpServerWireTools(editingId.value);
    if (tools.length === 0) {
      approvalNote.value = t("settings.mcp.approval.noTools");
      return;
    }
    const permissions = await getToolPermissions();
    for (const tool of tools) {
      if (mode === "auto") {
        permissions[tool] = "auto";
      } else {
        delete permissions[tool];
      }
    }
    await saveToolPermissions(permissions);
    approvalNote.value =
      mode === "auto"
        ? t("settings.mcp.approval.setAutoDone", tools.length)
        : t("settings.mcp.approval.setAskDone", tools.length);
  } catch (e) {
    approvalNote.value = normalizeAppError(e).message;
  } finally {
    approvalBusy.value = false;
  }
}

onMounted(() => {
  void refresh();
});
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.mcp.title") }}</div>
    <p class="section-desc">{{ t("settings.mcp.desc") }}</p>

    <div v-if="ready && servers.length === 0 && !formOpen" class="mcp-empty">
      {{ t("settings.mcp.empty") }}
    </div>

    <div v-if="servers.length > 0" class="tool-card">
      <template v-for="server in servers" :key="server.id">
        <div class="tool-row">
          <div class="tool-info">
            <span class="tool-name">
              {{ server.name }}
              <span class="mcp-transport-tag">{{ server.transport }}</span>
            </span>
            <span class="tool-desc mcp-command">{{ connectionSummary(server) }}</span>
          </div>
          <div class="mcp-row-actions">
            <BaseButton
              :disabled="testingId !== null || busy"
              @click="testServer(server)"
            >
              {{ testingId === server.id ? t("settings.mcp.testing") : t("settings.mcp.test") }}
            </BaseButton>
            <BaseButton :disabled="busy" @click="openEditForm(server)">
              {{ t("common.edit") }}
            </BaseButton>
            <BaseButton :disabled="busy" @click="removeServer(server)">
              {{ t("common.delete") }}
            </BaseButton>
            <BaseSwitch
              :model-value="server.enabled"
              :disabled="busy"
              :aria-label="server.name"
              @update:model-value="toggleEnabled(server)"
            />
          </div>
        </div>
        <div
          v-if="testResults[server.id]"
          class="mcp-test-result"
          :class="{ 'is-error': !testResults[server.id].ok }"
        >
          <template v-if="testResults[server.id].ok">
            <div class="mcp-test-summary">{{ testSummary(testResults[server.id]) }}</div>
            <div v-if="testResults[server.id].tools.length" class="mcp-tool-chips">
              <span
                v-for="tool in testResults[server.id].tools"
                :key="tool.name"
                class="mcp-tool-chip"
                :title="tool.description"
              >
                {{ tool.name }}
              </span>
            </div>
          </template>
          <pre v-else class="mcp-test-error">{{ testResults[server.id].error }}</pre>
        </div>
      </template>
    </div>

    <div v-if="!formOpen" class="mcp-cta-row">
      <button class="mcp-add-cta" :disabled="!ready" @click="openAddForm">
        + {{ t("settings.mcp.add") }}
      </button>
      <button class="mcp-add-cta mcp-import-cta" :disabled="!ready" @click="openImport">
        {{ t("settings.mcp.import.open") }}
      </button>
    </div>

    <!-- Import from Claude Desktop / Claude Code / Cursor -->
    <div v-if="importOpen" class="tool-card mcp-form">
      <div class="mcp-form-title">{{ t("settings.mcp.import.title") }}</div>
      <p class="mcp-field-hint">{{ t("settings.mcp.import.hint") }}</p>
      <div v-if="importScanning" class="mcp-test-summary">
        {{ t("settings.mcp.import.scanning") }}
      </div>
      <div v-else-if="importCandidates.length === 0" class="mcp-test-summary">
        {{ t("settings.mcp.import.none") }}
      </div>
      <div v-else class="mcp-import-list">
        <label
          v-for="(candidate, index) in importCandidates"
          :key="index"
          class="mcp-import-row"
          :class="{ 'is-duplicate': candidate.duplicateOf }"
        >
          <input
            type="checkbox"
            :checked="importSelected.has(index)"
            @change="toggleImportSelection(index)"
          />
          <span class="mcp-import-info">
            <span class="mcp-import-name">
              {{ candidate.server.name }}
              <span class="mcp-transport-tag">{{ candidate.server.transport }}</span>
              <span class="mcp-import-source">{{ importSourceLabel(candidate) }}</span>
              <span v-if="candidate.duplicateOf" class="mcp-import-duplicate">
                {{ t("settings.mcp.import.duplicate", candidate.duplicateOf) }}
              </span>
            </span>
            <span class="tool-desc mcp-command">{{ connectionSummary(candidate.server) }}</span>
          </span>
        </label>
      </div>
      <div class="mcp-form-actions">
        <span class="mcp-form-spacer" />
        <BaseButton :disabled="importApplying" @click="importOpen = false">
          {{ t("common.cancel") }}
        </BaseButton>
        <BaseButton
          variant="primary"
          :disabled="importApplying || importScanning || importSelected.size === 0"
          @click="applyImport"
        >
          {{ t("settings.mcp.import.apply", importSelected.size) }}
        </BaseButton>
      </div>
    </div>

    <div v-if="formOpen" class="tool-card mcp-form">
      <div class="mcp-form-title">
        {{ editingId ? t("settings.mcp.form.editTitle") : t("settings.mcp.form.addTitle") }}
      </div>

      <div v-if="adding" class="mcp-preset-row">
        <span class="mcp-field-label">{{ t("settings.mcp.form.presets") }}</span>
        <button
          v-for="preset in MCP_PRESETS"
          :key="preset.id"
          class="mcp-preset-chip"
          type="button"
          @click="applyPreset(preset)"
        >
          {{ preset.label }}
        </button>
      </div>
      <p v-if="formPresetNote" class="mcp-preset-note">{{ formPresetNote }}</p>

      <div class="mcp-field-pair">
        <label class="mcp-field">
          <span class="mcp-field-label">{{ t("settings.mcp.form.name") }}</span>
          <input v-model="formName" class="mcp-input" :placeholder="t('settings.mcp.form.namePlaceholder')" />
        </label>
        <div class="mcp-field mcp-field-transport">
          <span class="mcp-field-label">{{ t("settings.mcp.form.transport") }}</span>
          <BaseDropdown
            v-model="formTransport"
            :options="transportOptions"
            :aria-label="t('settings.mcp.form.transport')"
            menu-align="start"
          />
        </div>
      </div>

      <template v-if="formTransport === 'stdio'">
        <label class="mcp-field">
          <span class="mcp-field-label">{{ t("settings.mcp.form.command") }}</span>
          <input v-model="formCommand" class="mcp-input" placeholder="uvx" spellcheck="false" />
        </label>
        <label class="mcp-field">
          <span class="mcp-field-label">{{ t("settings.mcp.form.args") }}</span>
          <textarea
            v-model="formArgsText"
            class="mcp-input mcp-textarea"
            rows="2"
            placeholder="blender-mcp"
            spellcheck="false"
          />
          <span class="mcp-field-hint">{{ t("settings.mcp.form.argsHint") }}</span>
        </label>
        <label class="mcp-field">
          <span class="mcp-field-label">{{ t("settings.mcp.form.env") }}</span>
          <textarea
            v-model="formEnvText"
            class="mcp-input mcp-textarea"
            rows="2"
            placeholder="API_KEY=${MY_API_KEY}"
            spellcheck="false"
          />
          <span class="mcp-field-hint">{{ t("settings.mcp.form.envHint") }}</span>
        </label>
        <label class="mcp-field">
          <span class="mcp-field-label">{{ t("settings.mcp.form.cwd") }}</span>
          <input v-model="formCwd" class="mcp-input" spellcheck="false" />
        </label>
      </template>

      <template v-else>
        <label class="mcp-field">
          <span class="mcp-field-label">{{ t("settings.mcp.form.url") }}</span>
          <input
            v-model="formUrl"
            class="mcp-input"
            placeholder="http://127.0.0.1:3845/mcp"
            spellcheck="false"
          />
          <span class="mcp-field-hint">{{ t("settings.mcp.form.urlHint") }}</span>
        </label>
        <label class="mcp-field">
          <span class="mcp-field-label">{{ t("settings.mcp.form.headers") }}</span>
          <textarea
            v-model="formHeadersText"
            class="mcp-input mcp-textarea"
            rows="2"
            placeholder="Authorization=Bearer ${MY_TOKEN}"
            spellcheck="false"
          />
          <span class="mcp-field-hint">{{ t("settings.mcp.form.headersHint") }}</span>
        </label>
      </template>

      <div class="mcp-field-pair">
        <label class="mcp-field mcp-field-timeout">
          <span class="mcp-field-label">{{ t("settings.mcp.form.timeout") }}</span>
          <input v-model.number="formTimeoutSecs" class="mcp-input" type="number" min="1" max="3600" />
        </label>
        <div class="mcp-field mcp-field-loadmode">
          <span class="mcp-field-label">{{ t("settings.mcp.form.loadMode") }}</span>
          <BaseDropdown
            v-model="formLoadMode"
            :options="loadModeOptions"
            :aria-label="t('settings.mcp.form.loadMode')"
            menu-align="start"
          />
        </div>
        <div v-if="formTransport === 'stdio'" class="mcp-field mcp-field-autorestart">
          <span class="mcp-field-label">{{ t("settings.mcp.form.autoRestart") }}</span>
          <BaseSwitch
            v-model="formAutoRestart"
            :aria-label="t('settings.mcp.form.autoRestart')"
          />
        </div>
      </div>
      <span v-if="formTransport === 'stdio' && formAutoRestart" class="mcp-field-hint">
        {{ t("settings.mcp.form.autoRestartHint") }}
      </span>

      <div class="mcp-field">
        <span class="mcp-field-label">{{ t("settings.mcp.form.tools") }}</span>
        <div v-if="formToolRows.length === 0" class="mcp-tools-empty">
          {{ formToolsLoading ? t("settings.mcp.form.toolsLoading") : t("settings.mcp.form.toolsEmpty") }}
        </div>
        <template v-else>
          <div class="mcp-tool-list">
            <div v-for="row in formToolRows" :key="row.name" class="mcp-tool-toggle-row">
              <BaseCheckbox
                :model-value="!formDisabledTools.has(row.name)"
                :aria-label="row.name"
                @update:model-value="setFormToolEnabled(row.name, $event)"
              />
              <span
                class="mcp-tool-toggle-name"
                :class="{ 'is-off': formDisabledTools.has(row.name) }"
              >{{ row.name }}</span>
              <span
                v-if="row.description"
                class="mcp-tool-toggle-desc"
                :title="row.description"
              >{{ row.description }}</span>
            </div>
          </div>
          <span class="mcp-field-hint">{{ t("settings.mcp.form.toolsHint") }}</span>
        </template>
      </div>

      <div v-if="editingId" class="mcp-approval-block">
        <span class="mcp-field-label">{{ t("settings.mcp.approval.title") }}</span>
        <div class="mcp-approval-actions">
          <BaseButton :disabled="approvalBusy" @click="setServerApproval('auto')">
            {{ t("settings.mcp.approval.setAuto") }}
          </BaseButton>
          <BaseButton :disabled="approvalBusy" @click="setServerApproval('ask')">
            {{ t("settings.mcp.approval.setAsk") }}
          </BaseButton>
        </div>
        <span class="mcp-field-hint">{{ t("settings.mcp.approval.hint") }}</span>
        <span v-if="approvalNote" class="mcp-approval-note">{{ approvalNote }}</span>
      </div>

      <p v-if="formError" class="mcp-form-error">{{ formError }}</p>
      <div
        v-if="formTestResult"
        class="mcp-test-result mcp-form-test"
        :class="{ 'is-error': !formTestResult.ok }"
      >
        <!-- Discovered tools land in the toggle list above, so the test
             block only reports the connection outcome. -->
        <div v-if="formTestResult.ok" class="mcp-test-summary">{{ testSummary(formTestResult) }}</div>
        <pre v-else class="mcp-test-error">{{ formTestResult.error }}</pre>
      </div>

      <div class="mcp-form-actions">
        <BaseButton :disabled="formTesting" @click="testForm">
          {{ formTesting ? t("settings.mcp.testing") : t("settings.mcp.test") }}
        </BaseButton>
        <span class="mcp-form-spacer" />
        <BaseButton :disabled="busy" @click="closeForm">
          {{ t("common.cancel") }}
        </BaseButton>
        <BaseButton variant="primary" :disabled="busy" @click="saveForm">
          {{ t("common.save") }}
        </BaseButton>
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
}
.tool-row + .tool-row,
.mcp-test-result + .tool-row {
  border-top: 1px solid var(--border-color);
}
.tool-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.tool-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}
.tool-desc {
  font-size: 11.5px;
  color: var(--text-secondary);
  line-height: 1.45;
}
.mcp-transport-tag {
  margin-left: 6px;
  padding: 1px 6px;
  border: 1px solid var(--border-color);
  border-radius: 999px;
  font-size: 9.5px;
  font-weight: 500;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
.mcp-command {
  font-family: var(--font-mono-identifier);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.mcp-row-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.mcp-empty {
  max-width: 760px;
  padding: 18px 16px;
  border: 1px dashed var(--border-color);
  border-radius: 10px;
  font-size: 12px;
  color: var(--text-secondary);
}
.mcp-cta-row {
  display: flex;
  gap: 10px;
  max-width: 760px;
}
.mcp-add-cta {
  margin-top: 10px;
  flex: 1;
  padding: 9px 0;
  border: 1px dashed var(--border-color);
  border-radius: 10px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 12.5px;
  cursor: pointer;
  transition: color 0.12s, border-color 0.12s;
}
.mcp-import-cta {
  flex: 0 0 auto;
  padding: 9px 18px;
}
.mcp-add-cta:hover:not(:disabled) {
  color: var(--text-color);
  border-color: var(--border-strong);
}
.mcp-test-result {
  padding: 8px 16px 12px;
  border-top: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 92%, var(--sidebar-bg) 8%);
}
.mcp-test-summary {
  font-size: 11.5px;
  color: var(--text-secondary);
}
.mcp-test-error {
  margin: 0;
  font-size: 11px;
  font-family: var(--font-mono-identifier);
  color: var(--status-danger-fg);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 180px;
  overflow-y: auto;
}
.mcp-tool-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
  margin-top: 7px;
}
.mcp-tool-chip {
  padding: 2px 8px;
  border: 1px solid var(--border-color);
  border-radius: 999px;
  font-size: 10.5px;
  font-family: var(--font-mono-identifier);
  color: var(--text-secondary);
  background: var(--panel-bg);
}
.mcp-tool-list {
  display: flex;
  flex-direction: column;
  max-height: 280px;
  overflow-y: auto;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--panel-bg);
}
.mcp-tool-toggle-row {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 6px 10px;
  min-width: 0;
}
.mcp-tool-toggle-row + .mcp-tool-toggle-row {
  border-top: 1px solid var(--border-color);
}
.mcp-tool-toggle-name {
  flex-shrink: 0;
  font-size: 11.5px;
  font-family: var(--font-mono-identifier);
  color: var(--text-color);
}
.mcp-tool-toggle-name.is-off {
  color: var(--text-secondary);
  text-decoration: line-through;
  opacity: 0.7;
}
.mcp-tool-toggle-desc {
  flex: 1;
  min-width: 0;
  font-size: 10.5px;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.mcp-tools-empty {
  padding: 12px 10px;
  border: 1px dashed var(--border-color);
  border-radius: 8px;
  font-size: 11px;
  color: var(--text-secondary);
}
.mcp-form {
  margin-top: 10px;
  padding: 14px 16px;
  gap: 10px;
}
.mcp-form-title {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-color);
}
.mcp-preset-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}
.mcp-preset-chip {
  padding: 3px 10px;
  border: 1px solid var(--border-color);
  border-radius: 999px;
  background: var(--panel-bg);
  color: var(--text-secondary);
  font-size: 11px;
  cursor: pointer;
  transition: color 0.12s, border-color 0.12s;
}
.mcp-preset-chip:hover {
  color: var(--text-color);
  border-color: var(--border-strong);
}
.mcp-preset-note {
  margin: 0;
  font-size: 11px;
  color: var(--text-secondary);
  padding: 6px 9px;
  border: 1px dashed var(--border-color);
  border-radius: 7px;
}
.mcp-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
  flex: 1;
}
.mcp-field-label {
  font-size: 11.5px;
  color: var(--text-secondary);
}
.mcp-field-hint {
  font-size: 10.5px;
  color: var(--text-secondary);
  opacity: 0.8;
}
.mcp-field-pair {
  display: flex;
  gap: 12px;
}
.mcp-field-timeout {
  max-width: 140px;
  flex: 0 0 140px;
}
.mcp-field-transport {
  max-width: 180px;
  flex: 0 0 180px;
}
.mcp-field-loadmode {
  max-width: 200px;
  flex: 0 0 200px;
}
.mcp-field-autorestart {
  max-width: 160px;
  flex: 0 0 160px;
  align-items: flex-start;
}
.mcp-field-autorestart :deep(.base-switch) {
  margin-top: 4px;
}
.mcp-input {
  padding: 6px 9px;
  border: 1px solid var(--border-color);
  border-radius: 7px;
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 12px;
  font-family: var(--font-mono-identifier);
  outline: none;
}
.mcp-input:focus {
  border-color: var(--border-strong);
}
.mcp-textarea {
  resize: vertical;
  min-height: 30px;
}
.mcp-form-error {
  margin: 0;
  font-size: 11.5px;
  color: var(--status-danger-fg);
}
.mcp-form-test {
  border: 1px solid var(--border-color);
  border-radius: 8px;
}
.mcp-form-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}
.mcp-form-spacer {
  flex: 1;
}
.mcp-import-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 320px;
  overflow-y: auto;
}
.mcp-import-row {
  display: flex;
  align-items: flex-start;
  gap: 9px;
  padding: 7px 9px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  cursor: pointer;
}
.mcp-import-row.is-duplicate {
  opacity: 0.65;
}
.mcp-import-row input[type="checkbox"] {
  margin-top: 2px;
}
.mcp-import-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.mcp-import-name {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}
.mcp-import-source {
  margin-left: 6px;
  font-size: 10px;
  font-weight: 400;
  color: var(--text-secondary);
}
.mcp-import-duplicate {
  margin-left: 6px;
  font-size: 10px;
  font-weight: 400;
  color: var(--status-warning-fg, #b58a00);
}
.mcp-approval-block {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 9px 11px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
}
.mcp-approval-actions {
  display: flex;
  gap: 8px;
}
.mcp-approval-note {
  font-size: 11px;
  color: var(--text-secondary);
}
</style>
