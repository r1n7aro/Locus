<script setup lang="ts">
import { onMounted, ref } from "vue";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import { getWorkspaceResourceLimits, setWorkspaceResourceLimits, type WorkspaceResourceLimits } from "../../services/worktrees";
import UnityEditorResources from "./UnityEditorResources.vue";

const limits = ref<WorkspaceResourceLimits | null>(null);
const busy = ref(false);
const error = ref("");
const fields = ["maxRunningSessions", "maxUnityEditors", "maxRunningWorkspaceServices", "maxWatchedWorkspaces", "maxLspProcesses", "maxConcurrentServiceStarts", "maxConcurrentCompileJobs", "serviceIdleTimeoutSecs"] as const;
function displayValue(field: typeof fields[number]) {
  const value = limits.value?.[field] ?? 1;
  return field === "serviceIdleTimeoutSecs" ? Math.ceil(value / 60) : value;
}
onMounted(async () => {
  try { limits.value = (await getWorkspaceResourceLimits()).limits; }
  catch (cause) { error.value = normalizeAppError(cause).message; }
});
async function update(field: typeof fields[number], event: Event) {
  const input = event.target as HTMLInputElement;
  if (!limits.value || busy.value) return;
  const value = Number(input.value);
  const storedValue = field === "serviceIdleTimeoutSecs" ? value * 60 : value;
  if (!Number.isSafeInteger(storedValue) || !Number.isSafeInteger(value) || value < 1) { input.value = String(displayValue(field)); return; }
  busy.value = true; error.value = "";
  try { limits.value = (await setWorkspaceResourceLimits({ ...limits.value, [field]: storedValue })).limits; }
  catch (cause) { error.value = normalizeAppError(cause).message; input.value = String(displayValue(field)); }
  finally { busy.value = false; }
}
</script>

<template>
  <section class="workspace-resources" :aria-busy="busy || !limits">
    <div class="section-label">{{ t("settings.resources.title") }}</div>
    <div v-if="limits" class="resource-grid">
      <label v-for="field in fields" :key="field">
        <span>{{ t(`settings.resources.${field}`) }}</span>
        <input type="number" min="1" step="1" :value="displayValue(field)" :disabled="busy"
          @wheel="($event.target as HTMLInputElement).blur()" @change="update(field, $event)" />
      </label>
    </div>
    <p v-if="error" class="resource-error" role="alert">{{ error }}</p>
    <UnityEditorResources />
  </section>
</template>

<style scoped>
.workspace-resources { margin-bottom: 28px; }
.section-label { margin-bottom: 12px; font-size: 13px; font-weight: 600; color: var(--text-color); }
.resource-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 12px 24px; }
label { display: flex; align-items: center; justify-content: space-between; gap: 12px; color: var(--text-secondary); font-size: 13px; }
input { width: 68px; padding: 5px 8px; border: 1px solid var(--border-color); border-radius: 6px; background: var(--input-bg, var(--bg-color)); color: var(--text-color); font-family: var(--font-ui); }
input:focus-visible { outline: 1px solid var(--accent-color); }
.resource-error { color: var(--status-danger-fg); font-size: 12px; }
</style>
