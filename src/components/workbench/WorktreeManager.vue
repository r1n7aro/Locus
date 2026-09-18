<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import BaseButton from "../ui/BaseButton.vue";
import BaseCheckbox from "../ui/BaseCheckbox.vue";
import WorktreeCreationBudget from "./WorktreeCreationBudget.vue";
import WorktreePlanProgress from "./WorktreePlanProgress.vue";
import type { WorktreePlanProgress as PlanProgress } from "../../services/worktrees";
import { formatWorktreeBytes } from "../../utils/worktreeDiskUsage";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import { openWorkspace } from "../../services/project";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import { createWorktree, listManagedWorktrees, importWorktree, removeManagedWorktree, acquireUnityProjectSlot, releaseUnityProjectSlot, getWorktreePoolUsage, planWorktreeCreation, type WorktreePoolUsage, type WorktreeCreationPlan, type ManagedWorktree } from "../../services/worktrees";

const props = defineProps<{ sourceRoot: string }>();
const emit = defineEmits<{ close: [] }>();
const workspaces = useWorkspaceContextStore();
const dialog = ref<HTMLDialogElement>();
const items = ref<ManagedWorktree[]>([]);
const busy = ref(false);
const planning = ref(false);
const progress = ref<PlanProgress | null>(null);
const disabled = computed(() => busy.value || planning.value);
let planId = 0;
const error = ref("");
const creating = ref(false);
const branch = ref("codex/");
const startRef = ref("HEAD");
const destination = ref("");
const includeDirty = ref(false);
const poolMode = ref(false);
const maxSlots = ref(4);
const usage = ref<WorktreePoolUsage | null>(null);
const pendingCreation = ref<{ plan: WorktreeCreationPlan; run: (allowNewProject: boolean) => Promise<void> } | null>(null);
let registeredSource: string | null = null;
const formValid = computed(() => destination.value.trim().length > 0
  && (includeDirty.value && !poolMode.value || startRef.value.trim().length > 0)
  && (poolMode.value
    ? Number.isSafeInteger(maxSlots.value) && maxSlots.value > 0
    : branch.value.trim().length > 0));
onMounted(() => { dialog.value?.showModal(); void perform(refresh); });

async function ensureSource() {
  if (registeredSource === props.sourceRoot) return;
  await openWorkspace(props.sourceRoot);
  registeredSource = props.sourceRoot;
}
async function refresh() {
  await ensureSource();
  items.value = await listManagedWorktrees(props.sourceRoot);
  usage.value = await getWorktreePoolUsage(props.sourceRoot);
}
async function perform(action: () => Promise<unknown>) {
  if (busy.value) return;
  busy.value = true; error.value = "";
  try { await action(); } catch (cause) { error.value = normalizeAppError(cause).message; }
  finally { busy.value = false; }
}
async function created(item: ManagedWorktree) {
  await workspaces.openAndFocus(item.root);
  await refresh(); creating.value = false; pendingCreation.value = null;
}
async function create() {
  if (disabled.value || !formValid.value) return;
  const id = ++planId;
  planning.value = true; error.value = "";
  progress.value = { phase: "checking", files: 0, totalFiles: null, bytes: 0 };
  try {
    await ensureSource();
    if (id !== planId) return;
    const sourceRoot = props.sourceRoot;
    const pool = poolMode.value;
    const target = destination.value.trim();
    const name = branch.value.trim();
    const dirty = !pool && includeDirty.value;
    const capacity = maxSlots.value;
    const plan = await planWorktreeCreation({ sourceRoot, directory: target, poolMode: pool,
      startRef: dirty ? "HEAD" : startRef.value.trim(), includeDirty: dirty, maxSlots: capacity }, value => {
        if (id === planId) progress.value = value;
      });
    if (id !== planId) return;
    planning.value = false; progress.value = null;
    const run = async (allowNewProject: boolean) => {
      if (pool) {
        const acquired = await acquireUnityProjectSlot({ sourceRoot, poolRoot: target, commit: plan.startOid, branch: name || null, maxSlots: capacity }, allowNewProject);
        await created(acquired.worktree);
      } else {
        await created(await createWorktree({ sourceRoot, destination: target, branch: name, startRef: plan.startOid, includeDirty: dirty }));
      }
    };
    if (plan.requiresNewProject || plan.atCapacity) pendingCreation.value = { plan, run };
    else await perform(() => run(false));
  } catch (cause) {
    if (id === planId) error.value = normalizeAppError(cause).message;
  } finally {
    if (id === planId) { planning.value = false; progress.value = null; }
  }
}
function cancelPlanning() { planId++; planning.value = false; progress.value = null; }
onUnmounted(cancelPlanning);
async function confirmCreation() {
  const pending = pendingCreation.value;
  if (!pending || pending.plan.atCapacity) return;
  await perform(() => pending.run(true));
}
async function importExisting() {
  await perform(async () => {
    const targetRoot = await open({ directory: true, multiple: false });
    if (typeof targetRoot !== "string") return;
    await ensureSource();
    await created(await importWorktree(props.sourceRoot, targetRoot));
  });
}
async function remove(item: ManagedWorktree) {
  if (!canRemove(item)) return;
  const selected = { ...item };
  const sourceRoot = props.sourceRoot;
  await perform(async () => {
    if (!await confirm(t("worktrees.removeConfirm"), { title: t("worktrees.remove"), kind: "warning" })) return;
    await removeManagedWorktree(sourceRoot, selected);
    await refresh();
  });
}
async function release(item: ManagedWorktree) {
  if (!canRelease(item)) return;
  await perform(async () => { await releaseUnityProjectSlot(props.sourceRoot, item); await refresh(); });
}
function rootKey(path: string) {
  const normalized = path.replace(/\\/g, "/").replace(/\/+$/, "");
  return /^[a-z]:/i.test(normalized) || normalized.startsWith("//") ? normalized.toLowerCase() : normalized;
}
function canRemove(item: ManagedWorktree) {
  return item.managed && !item.dirty && !item.assignmentId
    && (item.lifecycle === "active" || item.lifecycle === "available")
    && rootKey(item.root) !== rootKey(props.sourceRoot)
    && rootKey(item.root) !== rootKey(workspaces.focusedRoot || "");
}
function canRelease(item: ManagedWorktree) {
  return item.poolSlot && !!item.assignmentId && !item.dirty && item.lifecycle === "active"
    && rootKey(item.root) !== rootKey(props.sourceRoot)
    && rootKey(item.root) !== rootKey(workspaces.focusedRoot || "");
}
function showCreateForm(pool: boolean) {
  pendingCreation.value = null;
  creating.value = pool || poolMode.value || !creating.value;
  poolMode.value = pool;
  if (!pool && includeDirty.value) startRef.value = "HEAD";
}
function close() { if (!busy.value) { cancelPlanning(); emit("close"); } }
</script>

<template>
  <dialog ref="dialog" class="worktree-dialog" aria-labelledby="worktree-dialog-title" @cancel.prevent="close">
    <header>
      <strong id="worktree-dialog-title">{{ t("worktrees.title") }}</strong>
      <BaseButton :disabled="busy" @click="close">{{ t("common.close") }}</BaseButton>
    </header>
    <div class="worktree-toolbar">
      <BaseButton :disabled="disabled" @click="showCreateForm(false)">{{ t("worktrees.create") }}</BaseButton>
      <BaseButton :disabled="disabled" @click="importExisting">{{ t("worktrees.import") }}</BaseButton>
      <BaseButton :disabled="disabled" @click="showCreateForm(true)">{{ t("worktrees.acquire") }}</BaseButton>
      <BaseButton :disabled="disabled" @click="perform(refresh)">{{ t("common.refresh") }}</BaseButton>
    </div>
    <WorktreePlanProgress v-if="progress" :progress="progress" :cancellable="planning" @cancel="cancelPlanning" />
    <WorktreeCreationBudget v-if="pendingCreation" class="manager-budget" :plan="pendingCreation.plan" :busy="busy" @confirm="confirmCreation" @cancel="pendingCreation = null" />
    <form v-else-if="creating && !planning" class="worktree-form" @submit.prevent="create">
      <label>{{ t("worktrees.branch") }}<input v-model="branch" :required="!poolMode" :disabled="disabled" /></label>
      <label>{{ t("worktrees.start") }}<input v-model="startRef" required :disabled="disabled || (!poolMode && includeDirty)" /></label>
      <label class="wide">{{ t(poolMode ? "worktrees.poolRoot" : "worktrees.destination") }}<input v-model="destination" required :disabled="disabled" spellcheck="false" /></label>
      <label v-if="poolMode">{{ t("worktrees.maxSlots") }}<input v-model.number="maxSlots" type="number" min="1" step="1" required :disabled="disabled" /></label>
      <label v-else class="include-dirty"><BaseCheckbox v-model="includeDirty" :aria-label="t('worktrees.includeDirty')" :disabled="disabled" @update:model-value="startRef = 'HEAD'" />{{ t("worktrees.includeDirty") }}</label>
      <div class="form-actions"><BaseButton type="submit" :disabled="disabled || !formValid">{{ t(poolMode ? "worktrees.acquire" : "worktrees.create") }}</BaseButton><BaseButton :disabled="disabled" @click="creating = false">{{ t("common.cancel") }}</BaseButton></div>
    </form>
    <p v-if="error" class="worktree-error" role="alert">{{ error }}</p>
    <div v-if="usage" class="worktree-usage">{{ t('worktrees.pool.summary', usage.items.length, usage.availableProjects, formatWorktreeBytes(usage.totalBytes)) }}</div>
    <div class="worktree-list" :aria-busy="busy">
      <div v-for="item in items" :key="item.checkoutId" class="worktree-row">
        <button class="worktree-target" :disabled="disabled || item.lifecycle !== 'active'" @click="perform(() => workspaces.openAndFocus(item.root))">
          <span>{{ item.branch?.replace('refs/heads/', '') || item.headOid.slice(0, 10) }}</span>
          <span class="path">{{ item.root }}</span>
        </button>
        <span class="worktree-state">{{ item.dirty ? t("worktrees.dirty") : t(`worktrees.state.${item.lifecycle}`) }}</span>
        <span v-if="usage?.items.some(entry => entry.worktree.checkoutId === item.checkoutId)" class="worktree-size">{{ formatWorktreeBytes(usage.items.find(entry => entry.worktree.checkoutId === item.checkoutId)!.sizeBytes) }}</span>
        <BaseButton v-if="item.poolSlot && item.assignmentId" :disabled="disabled || !canRelease(item)" @click="release(item)">{{ t("worktrees.release") }}</BaseButton>
        <BaseButton v-if="item.managed && item.lifecycle !== 'removed'" :disabled="disabled || !canRemove(item)" @click="remove(item)">{{ t("worktrees.remove") }}</BaseButton>
      </div>
      <span v-if="!items.length && !busy" class="empty">{{ t("worktrees.empty") }}</span>
    </div>
  </dialog>
</template>

<style scoped>
.worktree-dialog { width: min(820px, 90vw); max-height: 80vh; padding: 0; border: 1px solid var(--border-color); border-radius: 8px; background: var(--panel-bg, var(--bg-color)); color: var(--text-color); font-family: var(--font-ui); }
.worktree-dialog::backdrop { background: color-mix(in srgb, var(--bg-color) 65%, transparent); }
header { display: flex; align-items: center; justify-content: space-between; padding: 12px 16px; border-bottom: 1px solid var(--border-color); font-size: 14px; }
.worktree-toolbar { display: flex; gap: 8px; padding: 10px 16px; border-bottom: 1px solid var(--border-color); }
.manager-budget { padding: 16px; max-width: 480px; }
.worktree-usage { padding: 8px 16px; border-bottom: 1px solid var(--border-color); font-size: 12px; color: var(--text-secondary); }
.worktree-size { font-size: 11px; color: var(--text-secondary); font-variant-numeric: tabular-nums; }
.worktree-form { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; padding: 16px; border-bottom: 1px solid var(--border-color); }
label { display: flex; flex-direction: column; gap: 6px; color: var(--text-secondary); font-size: 12px; }
.wide, .form-actions { grid-column: 1 / -1; }
input { min-width: 0; padding: 6px 8px; border: 1px solid var(--border-color); border-radius: 6px; background: var(--input-bg, var(--bg-color)); color: var(--text-color); font-family: var(--font-mono-identifier); }
.include-dirty { flex-direction: row; align-items: center; }
.form-actions { display: flex; gap: 8px; }
.worktree-list { padding: 4px 0; max-height: 40vh; overflow: auto; }
.worktree-row { display: flex; align-items: center; gap: 10px; padding: 8px 16px; border-bottom: 1px solid var(--border-color); }
.worktree-target { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: start; gap: 4px; border: 0; padding: 4px 0; background: transparent; color: var(--text-color); text-align: left; cursor: pointer; font: inherit; font-size: 13px; }
.worktree-target:hover:not(:disabled) { color: var(--accent-color); }
.path { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font-mono-identifier); font-size: 11px; color: var(--text-secondary); }
.worktree-state, .empty { color: var(--text-secondary); font-size: 12px; }
.empty { display: block; padding: 20px 16px; }
.worktree-error { margin: 12px 16px; color: var(--status-danger-fg); font-size: 12px; white-space: pre-wrap; }
button:focus-visible, input:focus-visible { outline: 1px solid var(--accent-color); outline-offset: 2px; }
</style>
