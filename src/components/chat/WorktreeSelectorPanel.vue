<script setup lang="ts">
import { computed, nextTick, onUnmounted, ref, watch } from "vue";
import { Check, GitBranch, LockKeyhole, Plus, Search } from "lucide";
import { t } from "../../i18n";
import type { WorkspaceRef } from "../../services/project";
import { normalizeAppError } from "../../services/errors";
import { listWorktreeBranches, planWorktreeSelection, type ManagedWorktree, type WorktreeBranchOption, type WorktreeCreationPlan, type SelectWorktreeRequest, type WorktreePlanProgress as PlanProgress } from "../../services/worktrees";
import LucideIcon from "../icons/LucideIcon.vue";
import BaseButton from "../ui/BaseButton.vue";
import BaseCheckbox from "../ui/BaseCheckbox.vue";
import WorktreeCreationBudget from "../workbench/WorktreeCreationBudget.vue";
import WorktreePlanProgress from "../workbench/WorktreePlanProgress.vue";
import { dismissWorktreeCreation, startWorktreeCreation, worktreeCreationJob, worktreeSelectionError } from "./worktreeCreationJobs";
import { rememberWorktreeRepository } from "./worktreeRepositories";

const props = defineProps<{
  workspaceRef: WorkspaceRef;
  locked?: boolean;
  selectWorktree: (item: ManagedWorktree) => Promise<void>;
  /** Composer that adopts a created worktree; defaults to the source checkout. */
  ownerKey?: string;
}>();
const emit = defineEmits<{ busy: [value: boolean]; selected: [] }>();
const options = ref<WorktreeBranchOption[]>([]);
const query = ref("");
const branch = ref("");
const includeDirty = ref(true);
const creating = ref(false);
const loading = ref(false);
// Creation itself runs as a background job, so only the budget review holds the popup open.
const busy = computed(() => pendingProject.value !== null);
const owner = computed(() => props.ownerKey ?? `checkout:${props.workspaceRef.checkoutId}`);
const job = computed(() => worktreeCreationJob(owner.value));
const running = computed(() => job.value?.status === "running");
const displayedError = computed(() => error.value || (job.value?.status === "failed" ? job.value.error : ""));
const planning = ref(false);
const progress = ref<PlanProgress | null>(null);
const remoteSource = ref<string | null>(null);
const error = ref("");
const branchInput = ref<HTMLInputElement>();
const pendingProject = ref<{ request: SelectWorktreeRequest; plan: WorktreeCreationPlan } | null>(null);
const SEARCH_THRESHOLD = 8;
let requestId = 0;
let operationId = 0;
let disposed = false;
const showSearch = computed(() => options.value.length > SEARCH_THRESHOLD);
const filtered = computed(() => options.value.filter((item) =>
  !showSearch.value || (item.branch ?? item.headOid).toLocaleLowerCase().includes(query.value.trim().toLocaleLowerCase()),
));
const branchExists = computed(() => options.value.some((item) => !item.remote && item.branch === branch.value.trim()));
const valid = computed(() => !!branch.value.trim() && !branchExists.value);
const blocked = computed(() => !!props.locked || planning.value || loading.value || running.value);
const disabled = computed(() => blocked.value || busy.value);

function cancelPlanning() {
  if (!planning.value) return;
  operationId++;
  planning.value = false;
  progress.value = null;
}

async function load() {
  cancelPlanning();
  const id = ++requestId;
  loading.value = true; error.value = ""; options.value = []; creating.value = false; query.value = ""; pendingProject.value = null;
  try {
    const items = await listWorktreeBranches({ ...props.workspaceRef });
    if (!disposed && id === requestId) {
      rememberWorktreeRepository(props.workspaceRef.checkoutId, items.length > 0);
      // Pool acquisition leaves a private staging ref after binding an existing
      // branch. Keep attached checkouts, but hide these unused internal refs.
      options.value = items.filter(item => item.root || !/^locus\/pool\/unity-[a-f0-9]{32}$/.test(item.branch ?? ""));
    }
  } catch (cause) {
    if (!disposed && id === requestId) error.value = normalizeAppError(cause).message;
  } finally {
    if (!disposed && id === requestId) loading.value = false;
  }
}

async function select(name: string, createBranch = false) {
  if (disabled.value || pendingProject.value || (createBranch && !valid.value)) return;
  dismissWorktreeCreation(owner.value);
  const id = requestId;
  const operation = ++operationId;
  const request = { workspaceRef: { ...props.workspaceRef }, branch: name, createBranch, includeDirty: createBranch && !remoteSource.value && includeDirty.value,
    ...(createBranch && remoteSource.value ? { startRef: `refs/remotes/${remoteSource.value}` } : {}) };
  planning.value = true; error.value = "";
  progress.value = { phase: "checking", files: 0, totalFiles: null, bytes: 0 };
  try {
    const plan = await planWorktreeSelection(request, value => {
      if (!disposed && operation === operationId && id === requestId) progress.value = value;
    });
    if (disposed || operation !== operationId || id !== requestId || props.locked) return;
    planning.value = false; progress.value = null;
    if (plan.requiresNewProject || plan.atCapacity) pendingProject.value = { request, plan };
    else startCreation(request, plan, false);
  } catch (cause) {
    if (!disposed && operation === operationId && id === requestId) error.value = worktreeSelectionError(cause);
  } finally {
    if (operation === operationId) { planning.value = false; progress.value = null; }
  }
}

/** Hands the reviewed creation to a background job; closing the popup no longer cancels it. */
function startCreation(request: SelectWorktreeRequest, plan: WorktreeCreationPlan, allowNewProject: boolean) {
  creating.value = false;
  void startWorktreeCreation(owner.value, request, plan, allowNewProject, props.selectWorktree);
}

function confirmNewProject() {
  if (blocked.value || !pendingProject.value || pendingProject.value.plan.atCapacity) return;
  const { request, plan } = pendingProject.value;
  pendingProject.value = null; error.value = "";
  startCreation(request, plan, true);
}

async function beginCreate(remote?: string) {
  if (disabled.value) return;
  dismissWorktreeCreation(owner.value);
  remoteSource.value = remote ?? null;
  creating.value = true; branch.value = remote ? remote.slice(remote.indexOf('/') + 1) : query.value.trim() || "codex/"; error.value = "";
  await nextTick(); branchInput.value?.focus();
}
watch(() => [props.workspaceRef.checkoutId, props.workspaceRef.expectedGeneration, props.workspaceRef.expectedMaterializationEpoch], load, { immediate: true });
watch(busy, (value) => emit("busy", value), { flush: "sync" });
// Whichever panel instance is open when the job adopts its worktree closes the popup.
watch(job, (current, previous) => { if (previous?.status === "running" && !current && !disposed) emit("selected"); });
watch(() => props.locked, (locked) => { if (locked) { if (planning.value) cancelPlanning(); creating.value = false; pendingProject.value = null; } });
onUnmounted(() => { disposed = true; requestId++; emit("busy", false); });
</script>

<template>
  <section class="worktree-selector-panel" :aria-label="t('worktrees.title')" :aria-busy="busy || planning || loading || running">
    <div class="worktree-heading">
      <span>Worktree</span>
      <LucideIcon v-if="locked" :icon="LockKeyhole" :size="12" :title="t('worktrees.selector.locked')" :aria-label="t('worktrees.selector.locked')" />
    </div>
    <label v-if="showSearch" class="worktree-search">
      <LucideIcon :icon="Search" :size="13" />
      <input v-model="query" type="search" :placeholder="t('worktrees.selector.search')" :aria-label="t('worktrees.selector.search')" :disabled="busy" spellcheck="false" />
    </label>
    <div class="worktree-options">
      <span v-if="loading" class="worktree-empty">{{ t('common.loading') }}</span>
      <button
        v-for="item in filtered" :key="`${item.remote ? 'remote' : 'local'}:${item.branch ?? item.root ?? item.headOid}`" type="button"
        class="worktree-option" :class="{ active: item.current }" :aria-pressed="item.current"
        :disabled="disabled || !!pendingProject || item.unavailable || !item.branch"
        :title="locked ? t('worktrees.selector.locked') : item.unavailable ? t('worktrees.selector.unavailable') : item.root ?? item.branch ?? ''"
        @click="item.current ? emit('selected') : item.branch && (item.remote ? beginCreate(item.branch) : select(item.branch))"
      >
        <LucideIcon :icon="GitBranch" :size="14" />
        <span class="worktree-option-text">
          <span class="worktree-name">{{ item.branch || item.headOid.slice(0, 8) }}</span>
          <span v-if="item.dirty" class="worktree-meta">{{ t('worktrees.dirty') }}</span>
        </span>
        <LucideIcon v-if="item.current" :icon="Check" :size="14" />
      </button>
      <span v-if="!loading && !filtered.length && !error" class="worktree-empty">{{ t('worktrees.selector.empty') }}</span>
    </div>
    <p v-if="displayedError" class="worktree-error" role="alert">{{ displayedError }}</p>
    <WorktreePlanProgress v-if="progress" :progress="progress" :cancellable="planning" @cancel="cancelPlanning" />
    <WorktreePlanProgress v-else-if="job && running" :progress="job.progress" :started-at="job.startedAt" :branch="job.branch" background />
    <WorktreeCreationBudget v-if="pendingProject && !locked" :plan="pendingProject.plan" @confirm="confirmNewProject" @cancel="pendingProject = null" />
    <form v-else-if="creating && !locked && !planning && !running" class="worktree-create" @submit.prevent="select(branch.trim(), true)" @keydown.esc.stop="creating = false">
      <span v-if="remoteSource" class="worktree-source" :title="remoteSource">{{ t('worktrees.start') }} · {{ remoteSource }}</span>
      <label class="worktree-field">{{ t('worktrees.branch') }}<input ref="branchInput" v-model="branch" required :disabled="busy" :aria-invalid="branchExists" spellcheck="false" /></label>
      <span v-if="branchExists" class="worktree-error">{{ t('worktrees.selector.branchExists') }}</span>
      <label v-if="!remoteSource" class="worktree-dirty"><BaseCheckbox v-model="includeDirty" :disabled="busy" :aria-label="t('worktrees.includeDirty')" />{{ t('worktrees.includeDirty') }}</label>
      <div class="worktree-actions">
        <BaseButton type="submit" :disabled="disabled || !valid">{{ t('worktrees.create') }}</BaseButton>
        <BaseButton @click="creating = false">{{ t('common.cancel') }}</BaseButton>
      </div>
    </form>
    <button v-else-if="!planning && !busy && !running" type="button" class="worktree-option worktree-create-action" :disabled="disabled || !options.length" :title="locked ? t('worktrees.selector.locked') : undefined" @click="beginCreate()">
      <LucideIcon :icon="Plus" :size="14" /><span>{{ t('worktrees.selector.create') }}</span>
    </button>
  </section>
</template>

<style scoped>
/* Let the adjacent selector columns determine the height, including while branches load. */
.worktree-selector-panel { min-width: 0; min-height: 0; max-height: min(404px, calc(100vh - 176px)); display: flex; flex-direction: column; border-left: 1px solid var(--border-color); padding-left: 4px; color: var(--text-color); contain: size; overflow-y: auto; }
.worktree-heading { display: flex; align-items: center; justify-content: space-between; padding: 4px 12px 2px; color: var(--text-secondary); font-size: 10px; font-weight: 600; text-transform: uppercase; letter-spacing: .5px; }
.worktree-search { display: flex; align-items: center; gap: 6px; margin: 4px 8px; color: var(--text-secondary); }
input { min-width: 0; width: 100%; box-sizing: border-box; padding: 5px 7px; border: 1px solid var(--border-color); border-radius: 6px; background: var(--input-bg, var(--bg-color)); color: var(--text-color); font: inherit; font-size: 12px; }
.worktree-search input { padding: 4px 0; border-color: transparent; background: transparent; }
.worktree-options { flex: 1; min-height: 60px; overflow-y: auto; }
.worktree-option { display: flex; align-items: center; gap: 8px; width: 100%; padding: 6px 12px; border: 0; border-radius: 8px; background: transparent; color: var(--text-color); font: inherit; font-size: 13px; text-align: left; cursor: pointer; }
.worktree-option:hover:not(:disabled) { background: var(--hover-bg); }
.worktree-option.active { background: var(--active-bg); }
.worktree-option:disabled { cursor: default; color: var(--text-secondary); }
.worktree-option :deep(svg) { flex-shrink: 0; }
.worktree-option-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.worktree-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; }
.worktree-meta, .worktree-empty { color: var(--text-secondary); font-size: 11px; }
.worktree-source { color: var(--text-secondary); font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.worktree-empty { display: block; padding: 12px; }
.worktree-create-action { margin-top: 4px; border-top: 1px solid var(--border-color); border-radius: 0; font-size: 12px; }
.worktree-create { display: flex; flex-direction: column; gap: 8px; padding: 10px 8px 6px; border-top: 1px solid var(--border-color); overflow-y: auto; }
.worktree-field { display: flex; flex-direction: column; gap: 5px; color: var(--text-secondary); font-size: 12px; }
.worktree-dirty { display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--text-secondary); }
.worktree-actions { display: flex; gap: 6px; }
.worktree-error { margin: 4px 8px; color: var(--status-danger-fg); font-size: 12px; overflow-wrap: anywhere; max-height: 72px; overflow-y: auto; }
input:focus-visible, button:focus-visible { outline: 1px solid var(--accent-color); outline-offset: -1px; }
</style>
