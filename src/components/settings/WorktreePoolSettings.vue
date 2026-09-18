<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { t } from "../../i18n";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import { getAllWorktreePoolUsage, type WorktreeProjectUsage } from "../../services/worktrees";
import { normalizeAppError } from "../../services/errors";
import { formatWorktreeBytes } from "../../utils/worktreeDiskUsage";
import BaseButton from "../ui/BaseButton.vue";
import WorktreeManager from "../workbench/WorktreeManager.vue";

const workspaces = useWorkspaceContextStore();
const projects = ref<WorktreeProjectUsage[]>([]);
const totals = computed(() => projects.value.reduce((sum, project) => ({
  count: sum.count + (project.usage?.items.length ?? 0),
  available: sum.available + (project.usage?.availableProjects ?? 0),
  bytes: sum.bytes + (project.usage?.totalBytes ?? 0),
}), { count: 0, available: 0, bytes: 0 }));
const incomplete = computed(() => projects.value.some(project => project.error));
const loading = ref(false);
const error = ref("");
const managerSource = ref<string | null>(null);
let revision = 0;
async function refresh() {
  const current = ++revision;
  error.value = "";
  loading.value = true;
  try {
    const result = await getAllWorktreePoolUsage();
    if (current === revision) projects.value = result;
  } catch (cause) { if (current === revision) error.value = normalizeAppError(cause).message; }
  finally { if (current === revision) loading.value = false; }
}
function closeManager() { managerSource.value = null; void refresh(); }
watch(() => workspaces.projects.map(project => [project.projectId, ...project.checkouts.map(checkout => checkout.root)].join("\0")).join("\n"), refresh, { immediate: true });
onUnmounted(() => { revision++; });
</script>

<template>
  <div class="worktree-pool-settings">
    <div class="section-label">{{ t('worktrees.pool.title') }}</div>
    <div class="pool-panel" :aria-busy="loading">
      <div class="pool-toolbar">
        <span class="pool-summary">{{ loading ? t('worktrees.pool.measuring') : t('worktrees.pool.summary', totals.count, totals.available, formatWorktreeBytes(totals.bytes)) }}</span>
        <BaseButton :disabled="loading" @click="refresh">{{ t('common.refresh') }}</BaseButton>
      </div>
      <div v-if="projects.length" class="pool-table-scroll">
        <table class="pool-table">
          <thead><tr><th>{{ t('worktrees.pool.project') }}</th><th>Worktree</th><th>{{ t('worktrees.pool.available') }}</th><th>{{ t('worktrees.pool.disk') }}</th><th><span class="sr-only">{{ t('worktrees.pool.manage') }}</span></th></tr></thead>
          <tbody><tr v-for="project in projects" :key="project.sourceRoot">
            <td class="pool-project"><span class="pool-project-name">{{ project.sourceRoot.replace(/\\/g, '/').split('/').filter(Boolean).pop() }}</span><span class="pool-project-path" :title="project.sourceRoot">{{ project.sourceRoot }}</span><span v-if="project.error" class="pool-error" role="alert">{{ project.error }}</span></td>
            <td>{{ project.usage?.items.length ?? '—' }}</td><td>{{ project.usage?.availableProjects ?? '—' }}</td><td>{{ project.usage ? formatWorktreeBytes(project.usage.totalBytes) : '—' }}</td>
            <td><BaseButton :disabled="!!project.error || loading" @click="managerSource = project.sourceRoot">{{ t('worktrees.pool.manage') }}</BaseButton></td>
          </tr></tbody>
        </table>
      </div>
      <p v-else-if="!loading && !error" class="pool-empty">{{ t('worktrees.pool.noProjects') }}</p>
    </div>
    <p v-if="incomplete" class="pool-partial">{{ t('worktrees.pool.partial') }}</p>
    <p v-if="error" class="pool-error" role="alert">{{ error }}</p>
    <Teleport to="body"><WorktreeManager v-if="managerSource" :source-root="managerSource" @close="closeManager" /></Teleport>
  </div>
</template>

<style scoped>
.worktree-pool-settings { margin-top: 24px; max-width: 760px; }
.section-label { margin-bottom: 12px; font-size: 13px; font-weight: 600; }
.pool-panel { border: 1px solid var(--border-color); border-radius: 10px; overflow: hidden; }
.pool-toolbar { display: flex; align-items: center; gap: 10px; padding: 12px 16px; background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%); }
.pool-summary { flex: 1; min-width: 0; color: var(--text-secondary); font-size: 12px; font-variant-numeric: tabular-nums; }
.pool-table-scroll { overflow-x: auto; }
.pool-table { width: 100%; border-collapse: collapse; font-size: 12px; font-variant-numeric: tabular-nums; }
.pool-table th, .pool-table td { padding: 8px 12px; border-top: 1px solid var(--border-color); text-align: right; white-space: nowrap; }
.pool-table th { color: var(--text-secondary); font-weight: 500; }
.pool-table th:first-child, .pool-table td:first-child { text-align: left; }
.pool-project { width: 100%; }
.pool-project-name, .pool-project-path { display: block; max-width: 340px; overflow: hidden; text-overflow: ellipsis; }
.pool-project-path { margin-top: 3px; color: var(--text-secondary); font-family: var(--font-mono-identifier); font-size: 11px; }
.pool-empty { padding: 0 16px 12px; margin: 0; }
.pool-empty, .pool-partial { color: var(--text-secondary); font-size: 12px; }
.pool-error { margin: 8px 0; color: var(--status-danger-fg); font-size: 12px; overflow-wrap: anywhere; }
.pool-project .pool-error { display: block; white-space: normal; }
.sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
</style>
