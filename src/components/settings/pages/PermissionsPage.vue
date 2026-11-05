<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from "vue";
import { t } from "../../../i18n";
import { normalizeAppError } from "../../../services/errors";
import {
  readBool,
  setSettingValue,
  settingValue,
  useSetting,
} from "../../../composables/useSettingsStore";
import { useChatStore } from "../../../stores/chat";
import { useNotificationStore } from "../../../stores/notification";
import ToolPermissions from "../ToolPermissions.vue";

type ToolMode = "auto" | "ask";

interface PermissionItem {
  name: string;
  label: string;
  desc: string;
  defaultMode: ToolMode;
}

const TOOLS_KEY = "permissions.tools";

/** Tools listed with their default approval; the label is the tool name. */
const TOOL_DEFAULTS: [name: string, defaultMode: ToolMode][] = [
  ["read", "auto"],
  ["grep", "auto"],
  ["list", "auto"],
  ["subagent", "ask"],
  ["todowrite", "auto"],
  ["ask_user_question", "auto"],
  ["write", "ask"],
  ["edit", "ask"],
  ["bash", "ask"],
  ["web_fetch", "ask"],
  ["unity_set_play_mode", "auto"],
  ["unity_execute", "ask"],
  ["unity_run_states", "ask"],
  ["unity_test_list", "auto"],
  ["unity_test_run", "ask"],
  ["unity_recompile", "auto"],
  ["unity_hot_reload", "auto"],
  ["unity_ref_search", "auto"],
  ["code_find_references", "auto"],
  ["code_goto_definition", "auto"],
  ["code_symbol_search", "auto"],
  ["code_diagnostics", "auto"],
  ["code_hover", "auto"],
  ["unity_code_usages", "auto"],
  ["unity_asset_search", "auto"],
  ["unity_yaml_search", "auto"],
  ["unity_yaml_read", "auto"],
  ["knowledge_query", "auto"],
  ["skill_list", "auto"],
];

/** Approval behaviors: [key, i18n stem under settings.perms.behavior, default]. */
const BEHAVIOR_DEFAULTS: [name: string, labelKey: string, defaultMode: ToolMode][] = [
  ["behavior.local_dangerous_commands", "localDangerousCommands", "ask"],
  ["behavior.unity_editor_status_change", "unityEditorStatusChange", "auto"],
  ["behavior.knowledge_governance", "knowledgeGovernance", "auto"],
  ["behavior.settings_change", "settingsChange", "auto"],
  ["behavior.sensitive_settings_change", "sensitiveSettingsChange", "ask"],
];

const chatStore = useChatStore();

const toolList = computed<PermissionItem[]>(() =>
  TOOL_DEFAULTS.map(([name, defaultMode]) => ({
    name,
    label: name,
    desc: t(`tool.desc.${name}`),
    defaultMode,
  })));

const behaviorList = computed<PermissionItem[]>(() =>
  BEHAVIOR_DEFAULTS.map(([name, stem, defaultMode]) => ({
    name,
    label: t(`settings.perms.behavior.${stem}`),
    desc: t(`settings.perms.behavior.${stem}Desc`),
    defaultMode,
  })));

/** Saved overrides; `task` is the old name of `subagent`. */
const toolPermissions = computed<Record<string, ToolMode>>(() => {
  const raw = settingValue(TOOLS_KEY);
  const normalized: Record<string, ToolMode> = {};
  if (raw && typeof raw === "object" && !Array.isArray(raw)) {
    for (const [name, mode] of Object.entries(raw as Record<string, unknown>)) {
      normalized[name] = mode === "ask" ? "ask" : "auto";
    }
  }
  if (normalized.subagent === undefined && normalized.task !== undefined) {
    normalized.subagent = normalized.task;
  }
  delete normalized.task;
  return normalized;
});

function modeOf(item: PermissionItem): ToolMode {
  return toolPermissions.value[item.name] ?? item.defaultMode;
}

const permSaveMsg = ref("");
let permSaveTimer: ReturnType<typeof setTimeout> | null = null;

function flashSaved() {
  permSaveMsg.value = t("settings.perms.saved");
  if (permSaveTimer) clearTimeout(permSaveTimer);
  permSaveTimer = setTimeout(() => {
    permSaveMsg.value = "";
    permSaveTimer = null;
  }, 2000);
}

onBeforeUnmount(() => {
  if (permSaveTimer) clearTimeout(permSaveTimer);
});

/** Saves every listed item's effective mode, as the old permissions file did. */
async function setPermission(name: string, mode: ToolMode) {
  const items = [...toolList.value, ...behaviorList.value];
  const item = items.find((entry) => entry.name === name);
  if (!item || modeOf(item) === mode) return;
  const next: Record<string, ToolMode> = {};
  for (const entry of items) next[entry.name] = entry.name === name ? mode : modeOf(entry);
  try {
    await setSettingValue(TOOLS_KEY, next);
    flashSaved();
  } catch (e) {
    const err = normalizeAppError(e);
    useNotificationStore().addNotice("error", t("settings.perms.saveFailed", err.message), {
      code: err.code,
      operation: "saveToolPermissions",
    });
  }
}

const {
  value: fileBoundaryEnabled,
  ready: fileBoundaryReady,
  busy: fileBoundaryBusy,
  set: saveFileBoundary,
} = useSetting("permissions.fileWorkspaceBoundary", readBool(false));

async function setFileWorkspaceBoundary(value: boolean) {
  if (!fileBoundaryReady.value || fileBoundaryBusy.value || fileBoundaryEnabled.value === value) return;
  try {
    await saveFileBoundary(value);
    flashSaved();
  } catch (e) {
    const err = normalizeAppError(e);
    useNotificationStore().addNotice("error", t("settings.perms.fileBoundarySaveFailed", err.message), {
      code: err.code,
      operation: "setFileToolWorkspaceBoundary",
    });
  }
}
</script>

<template>
  <ToolPermissions
    :tool-permission-mode="chatStore.toolPermissionMode"
    :tool-list="toolList"
    :behavior-list="behaviorList"
    :tool-permissions="toolPermissions"
    :file-workspace-boundary-enabled="fileBoundaryEnabled"
    :file-workspace-boundary-ready="fileBoundaryReady"
    :file-workspace-boundary-busy="fileBoundaryBusy"
    :perm-save-msg="permSaveMsg"
    @set-global-permission-mode="chatStore.setToolPermissionMode"
    @set-permission="setPermission"
    @set-file-workspace-boundary="setFileWorkspaceBoundary"
  />
</template>
