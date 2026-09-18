<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import { getUnityEditorResources, type UnityEditorResource } from "../../services/worktrees";
import { formatWorktreeBytes } from "../../utils/worktreeDiskUsage";
import BaseButton from "../ui/BaseButton.vue";

const editors = ref<UnityEditorResource[]>([]);
const loading = ref(false);
const loaded = ref(false);
const error = ref("");
let disposed = false;
let timer: ReturnType<typeof setInterval> | undefined;
async function refresh() {
  if (loading.value || disposed) return;
  loading.value = true;
  try {
    const result = await getUnityEditorResources();
    if (!disposed) { editors.value = result; error.value = ""; loaded.value = true; }
  } catch (cause) { if (!disposed) error.value = normalizeAppError(cause).message; }
  finally { if (!disposed) loading.value = false; }
}
function modeLabel(editor: UnityEditorResource) {
  return t(`settings.resources.editors.${editor.managed ? "managed" : editor.mode}`);
}
onMounted(() => {
  void refresh();
  timer = setInterval(() => { if (document.visibilityState !== "hidden") void refresh(); }, 5000);
});
onUnmounted(() => { disposed = true; if (timer) clearInterval(timer); });
</script>

<template>
  <div class="editor-resources" :aria-busy="loading">
    <div class="editor-toolbar">
      <span class="section-label">{{ t("settings.resources.editors.title") }}</span>
      <BaseButton :disabled="loading" @click="refresh">{{ t("common.refresh") }}</BaseButton>
    </div>
    <div v-if="editors.length" class="editor-table-scroll">
      <table>
        <thead><tr><th>{{ t("settings.resources.editors.project") }}</th><th>{{ t("settings.resources.editors.mode") }}</th><th>PID</th><th :title="t('settings.resources.editors.memoryDetail')">{{ t("settings.resources.editors.memory") }}</th></tr></thead>
        <tbody><tr v-for="editor in editors" :key="editor.processId">
          <td class="editor-project"><span class="editor-name">{{ editor.projectPath.replace(/\\/g, "/").split("/").filter(Boolean).pop() }}</span><span class="editor-path" :title="editor.projectPath">{{ editor.projectPath }}</span><span v-if="editor.lastError" class="editor-error">{{ editor.lastError }}</span></td>
          <td>{{ modeLabel(editor) }}</td><td>{{ editor.processId }}</td><td :title="t('settings.resources.editors.memoryDetail')">{{ editor.workingSetBytes === null ? "—" : formatWorktreeBytes(editor.workingSetBytes) }}</td>
        </tr></tbody>
      </table>
    </div>
    <p v-else-if="loaded && !error" class="editor-empty">{{ t("settings.resources.editors.empty") }}</p>
    <p v-if="error" class="editor-error" role="alert">{{ error }}</p>
  </div>
</template>

<style scoped>
.editor-resources { margin-top: 24px; }
.editor-toolbar { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
.section-label { font-size: 13px; font-weight: 600; }
.editor-table-scroll { overflow-x: auto; border: 1px solid var(--border-color); border-radius: 6px; }
table { width: 100%; border-collapse: collapse; font-size: 12px; font-variant-numeric: tabular-nums; }
th, td { padding: 8px 12px; text-align: right; white-space: nowrap; }
th { color: var(--text-secondary); font-weight: 500; background: var(--sidebar-bg); }
td { border-top: 1px solid var(--border-color); }
th:first-child, td:first-child { text-align: left; }
.editor-project { width: 100%; }
.editor-name, .editor-path { display: block; max-width: 350px; overflow: hidden; text-overflow: ellipsis; }
.editor-path { margin-top: 3px; color: var(--text-secondary); font-family: var(--font-mono-identifier); font-size: 11px; }
.editor-empty { margin: 0; font-size: 12px; color: var(--text-secondary); }
.editor-error { display: block; margin: 8px 0 0; font-size: 12px; color: var(--status-danger-fg); overflow-wrap: anywhere; white-space: normal; }
</style>
