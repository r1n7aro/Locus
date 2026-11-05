<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, useId, watch, type CSSProperties } from "vue";
import { ChevronDown, ListTodo, Square, Timer, X } from "lucide";
import { t } from "../../i18n";
import {
  cancelBackgroundTask,
  listBackgroundTasks,
  subscribeBackgroundTasksChanged,
  type BackgroundTaskInfo,
  type BackgroundTasksChangedEvent,
} from "../../services/asyncTasks";
import type { AppTaskInfo } from "../../services/appTasks";
import { normalizeAppError } from "../../services/errors";
import { useSurfaceHidden } from "../../composables/surfaceVisibility";
import type { AsyncTaskStatus, AsyncTaskUpdatedEvent } from "../../types";
import LucideIcon from "../icons/LucideIcon.vue";
import BaseButton from "../ui/BaseButton.vue";
import { takeAppTaskFailures, useAppTasks } from "./appTaskActivity";
import {
  isTerminalTaskStatus,
  rememberTaskDescriptions,
  takeUnseenFailures,
  unseenFailureCount,
  useBackgroundTaskUpdates,
  type BackgroundTaskFailure,
} from "./backgroundTaskActivity";

const PANEL_WIDTH = 360;

const props = defineProps<{ sessionId: string | null }>();
// Process-level tasks belong to no session and are shown in every surface.
const { tasks: appTasks, failureCount: appFailureCount } = useAppTasks();
const open = ref(false);
const root = ref<HTMLElement | null>(null);
const panel = ref<HTMLElement | null>(null);
const panelId = useId();
const panelStyle = ref<CSSProperties>({});
const placement = ref<"above" | "below">("above");
const tasks = ref<BackgroundTaskInfo[]>([]);
const loading = ref(false);
const loadError = ref("");
const actionError = ref("");
const cancelling = ref(new Set<string>());
// Task output stays collapsed until the row is clicked; reset on every opening.
const expanded = ref(new Set<string>());
const now = ref(Date.now());
const error = computed(() => actionError.value || loadError.value);
// With process tasks listed, an empty session list adds nothing.
const showSessionGroup = computed(() => loading.value || tasks.value.length > 0 || !appTasks.value.length);
let generation = 0;
let mutation = 0;
// Bumped by every pushed snapshot; a list response that raced one is older.
let pushSeq = 0;
let clock: ReturnType<typeof setInterval> | undefined;
let releasePushes: (() => void) | undefined;
let unmounted = false;

// Trigger and panel state: seeded by one list call, then driven by pushed
// session snapshots (`background-tasks-changed`) and task update events. Lists
// happen only on seeding, opening, window focus and showing a hidden tab. Keys
// are session-local ids, which is what update events carry.
const activeStatuses = ref(new Map<string, AsyncTaskStatus>());
const shownFailures = ref<BackgroundTaskFailure[]>([]);
const activeCount = computed(() => activeStatuses.value.size + appTasks.value.length);
const failureCount = computed(() => unseenFailureCount(props.sessionId) + appFailureCount.value);
const tone = computed<"idle" | "running" | "cancelling">(() => {
  if (!activeCount.value) return "idle";
  if (appTasks.value.length) return "running";
  return [...activeStatuses.value.values()].every((status) => status === "cancelling") ? "cancelling" : "running";
});
const countLabel = computed(() => (activeCount.value > 9 ? "9+" : String(activeCount.value)));
const summary = computed(() => {
  const parts = [
    tone.value === "idle"
      ? t("chat.backgroundTasks.summary.idle")
      : t(`chat.backgroundTasks.summary.${tone.value}`, activeCount.value),
  ];
  if (failureCount.value) parts.push(t("chat.backgroundTasks.summary.failed", failureCount.value));
  return `${t("chat.backgroundTasks.title")} · ${parts.join(" · ")}`;
});
let activityGeneration = 0;
let reconcileTimer: ReturnType<typeof setTimeout> | undefined;
let reconcileDue = Number.POSITIVE_INFINITY;
let reconciling = false;
// Each in-flight list request records the updates that race it so they can be
// replayed over the (older) snapshot.
const recorders = new Set<AsyncTaskUpdatedEvent[]>();

function isActive(task: BackgroundTaskInfo) {
  return task.status === "queued" || task.status === "running" || task.status === "cancelling";
}

/** Applies one task status to the trigger state; returns true for a task not seen before. */
function applyStatus(taskId: string, status: AsyncTaskStatus) {
  const statuses = activeStatuses.value;
  if (isTerminalTaskStatus(status)) {
    statuses.delete(taskId);
    return false;
  }
  const known = statuses.has(taskId);
  if (statuses.get(taskId) !== status) statuses.set(taskId, status);
  return !known;
}

function applySnapshot(sessionId: string, result: BackgroundTaskInfo[], log: AsyncTaskUpdatedEvent[]) {
  const active = result.filter((task) => task.sessionId === sessionId && isActive(task));
  activeStatuses.value = new Map(active.map((task) => [task.localId, task.status]));
  for (const update of log) applyStatus(update.taskId, update.status);
  rememberTaskDescriptions(sessionId, active);
}

function clearReconcile() {
  clearTimeout(reconcileTimer);
  reconcileTimer = undefined;
  reconcileDue = Number.POSITIVE_INFINITY;
}

function scheduleReconcile(delay: number) {
  const sessionId = props.sessionId;
  // While the panel is open its pushed rows keep the trigger in sync.
  if (!sessionId || open.value) return;
  const due = Date.now() + delay;
  if (reconcileTimer !== undefined && reconcileDue <= due) return;
  clearReconcile();
  reconcileDue = due;
  const currentGeneration = activityGeneration;
  reconcileTimer = setTimeout(() => {
    reconcileTimer = undefined;
    reconcileDue = Number.POSITIVE_INFINITY;
    // Every mounted workbench tab has an indicator; hidden ones check when shown
    // instead of each querying on every window focus or reconcile interval.
    if (surfaceHidden.value) {
      reconcileWhenShown = true;
      return;
    }
    void reconcile(currentGeneration, sessionId);
  }, delay);
}

const surfaceHidden = useSurfaceHidden();
let reconcileWhenShown = false;
watch(surfaceHidden, (hidden) => {
  if (hidden || !reconcileWhenShown) return;
  reconcileWhenShown = false;
  scheduleReconcile(0);
});

async function reconcile(currentGeneration: number, sessionId: string) {
  if (reconciling || currentGeneration !== activityGeneration) return;
  reconciling = true;
  const currentPush = pushSeq;
  const log: AsyncTaskUpdatedEvent[] = [];
  recorders.add(log);
  try {
    const result = await listBackgroundTasks(sessionId);
    if (currentGeneration === activityGeneration && currentPush === pushSeq) applySnapshot(sessionId, result, log);
  } catch {
    // Keep the last known state; the next push, focus or showing retries.
  } finally {
    recorders.delete(log);
    if (currentGeneration === activityGeneration) reconciling = false;
  }
}

/** Applies a pushed snapshot of the session's live rows to trigger and panel. */
function applyPushedTasks(event: BackgroundTasksChangedEvent) {
  const sessionId = props.sessionId;
  if (!sessionId || event.sessionId !== sessionId) return;
  pushSeq += 1;
  const active = event.tasks.filter((task) => task.sessionId === sessionId && isActive(task));
  activeStatuses.value = new Map(active.map((task) => [task.localId, task.status]));
  rememberTaskDescriptions(sessionId, active);
  if (!open.value) return;
  // Supersedes an in-flight panel load as well.
  mutation += 1;
  tasks.value = active;
  loading.value = false;
  loadError.value = "";
  now.value = Date.now();
}

function resetActivity() {
  activityGeneration += 1;
  clearReconcile();
  reconciling = false;
  recorders.clear();
  activeStatuses.value = new Map();
  shownFailures.value = [];
}

function appTaskTitle(task: AppTaskInfo) {
  return task.kind === "worktree" ? t("chat.backgroundTasks.app.worktree", task.title) : task.title;
}

function appTaskPhase(task: AppTaskInfo) {
  return task.kind === "worktree" && task.progress
    ? t(`worktrees.progress.${task.progress.phase}`)
    : t("chat.backgroundTasks.status.running");
}

function appTaskPercent(task: AppTaskInfo): number | null {
  const progress = task.progress;
  return progress?.total ? Math.min(100, Math.floor((progress.completed / progress.total) * 100)) : null;
}

function appFailure(task: AppTaskInfo): BackgroundTaskFailure {
  const error = task.error ?? "";
  return {
    taskId: task.taskId,
    toolName: t("chat.backgroundTasks.appGroup"),
    description: appTaskTitle(task),
    detail: error.includes("WORKTREE_NEW_PROJECT_REQUIRED") ? t("worktrees.budget.retry") : error,
    failedAt: Date.now(),
    source: "app",
  };
}

function showFailures(sessionId: string | null) {
  const failures = [
    ...(sessionId ? takeUnseenFailures(sessionId) : []),
    ...takeAppTaskFailures().map(appFailure),
  ];
  if (!failures.length) return;
  const ids = new Set(failures.map((failure) => failure.taskId));
  shownFailures.value = [...shownFailures.value.filter((failure) => !ids.has(failure.taskId)), ...failures];
}

useBackgroundTaskUpdates((update) => {
  const sessionId = props.sessionId;
  if (!sessionId || update.sessionId !== sessionId) return;
  for (const log of recorders) log.push(update);
  applyStatus(update.taskId, update.status);
  if (update.status === "failed" && open.value) showFailures(sessionId);
  // The pushed snapshot that follows carries the description and progress.
});

function handleWindowFocus() {
  scheduleReconcile(0);
}

function isCancelling(task: BackgroundTaskInfo) {
  return cancelling.value.has(task.taskId) || task.status === "cancelling";
}

// Background Claude subagents ("Agent") and Locus subagents ("subagent"). A
// stopped Claude subagent stays stopped in its conversation.
const AGENT_TASK_TOOLS = new Set(["Agent", "subagent"]);
const stoppableAgents = computed(() => tasks.value.filter((task) =>
  AGENT_TASK_TOOLS.has(task.toolName) && isActive(task) && !isCancelling(task)));

function stopAllAgents() {
  for (const task of stoppableAgents.value) void stopTask(task);
}

function isExpanded(task: BackgroundTaskInfo) {
  return expanded.value.has(task.taskId);
}

function toggleDetail(task: BackgroundTaskInfo) {
  if (!expanded.value.delete(task.taskId)) expanded.value.add(task.taskId);
}

function detailId(task: BackgroundTaskInfo) {
  return `${panelId}-detail-${task.taskId}`;
}

function rowTone(task: BackgroundTaskInfo) {
  if (isCancelling(task)) return "cancelling";
  return task.status === "queued" ? "queued" : "running";
}

function elapsed(task: BackgroundTaskInfo) {
  return elapsedSince(task.startedAt ?? task.createdAt);
}

function elapsedSince(startedAt: number) {
  const seconds = Math.max(0, Math.floor((now.value - startedAt) / 1000));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return hours
    ? `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`
    : `${minutes}:${String(seconds % 60).padStart(2, "0")}`;
}

function progress(task: BackgroundTaskInfo) {
  if (isCancelling(task)) return "";
  const value = task.progress?.trim() ?? "";
  return /^(queued|running|completed|failed|cancelled)$/i.test(value) ? "" : value;
}

function positionPanel() {
  if (!open.value || !root.value) return;
  const rect = root.value.getBoundingClientRect();
  const margin = 8;
  const gap = 6;
  const width = Math.min(PANEL_WIDTH, window.innerWidth - margin * 2);
  const above = rect.top - gap - margin;
  const below = window.innerHeight - rect.bottom - gap - margin;
  const placeAbove = above >= below;
  placement.value = placeAbove ? "above" : "below";
  panelStyle.value = {
    width: `${Math.max(0, width)}px`,
    left: `${Math.max(margin, Math.min(rect.left, window.innerWidth - width - margin))}px`,
    maxHeight: `${Math.max(0, Math.min(420, placeAbove ? above : below))}px`,
    ...(placeAbove
      ? { bottom: `${window.innerHeight - rect.top + gap}px` }
      : { top: `${rect.bottom + gap}px` }),
  };
}

function close(restoreFocus = false) {
  open.value = false;
  if (restoreFocus) root.value?.querySelector("button")?.focus();
}

function handleOutside(event: Event) {
  const target = event.target;
  if (!(target instanceof Node) || root.value?.contains(target) || panel.value?.contains(target)) return;
  close();
}

function handleKeydown(event: KeyboardEvent) {
  if (open.value && event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    close(true);
  }
}

async function refresh(currentGeneration: number, sessionId: string) {
  const currentMutation = mutation;
  const currentActivity = activityGeneration;
  const log: AsyncTaskUpdatedEvent[] = [];
  recorders.add(log);
  try {
    const result = await listBackgroundTasks(sessionId);
    if (currentMutation !== mutation) return;
    if (currentActivity === activityGeneration) applySnapshot(sessionId, result, log);
    if (currentGeneration !== generation) return;
    tasks.value = result.filter((task) => task.sessionId === sessionId && isActive(task));
    loadError.value = "";
  } catch (cause) {
    if (currentGeneration === generation && currentMutation === mutation) {
      loadError.value = normalizeAppError(cause).message;
    }
  } finally {
    recorders.delete(log);
    if (currentGeneration === generation) {
      loading.value = false;
      now.value = Date.now();
    }
  }
}

async function stopTask(task: BackgroundTaskInfo) {
  const sessionId = props.sessionId;
  if (!sessionId || task.sessionId !== sessionId || isCancelling(task) || !isActive(task)) return;
  const currentGeneration = generation;
  const currentActivity = activityGeneration;
  mutation += 1;
  cancelling.value.add(task.taskId);
  actionError.value = "";
  try {
    const updated = await cancelBackgroundTask(sessionId, task.taskId);
    if (currentActivity === activityGeneration) applyStatus(updated.localId, updated.status);
    if (currentGeneration !== generation) return;
    mutation += 1;
    tasks.value = tasks.value
      .map((item) => item.taskId === updated.taskId ? updated : item)
      .filter(isActive);
  } catch (cause) {
    if (currentGeneration === generation) actionError.value = normalizeAppError(cause).message;
  } finally {
    if (currentGeneration === generation) cancelling.value.delete(task.taskId);
  }
}

watch(() => props.sessionId, (sessionId, previous) => {
  if (sessionId !== previous) close();
  resetActivity();
  if (sessionId) void reconcile(activityGeneration, sessionId);
}, { immediate: true, flush: "sync" });
watch(open, (value) => {
  generation += 1;
  clearInterval(clock);
  tasks.value = [];
  cancelling.value.clear();
  expanded.value.clear();
  loadError.value = "";
  actionError.value = "";
  loading.value = false;
  if (!value) {
    shownFailures.value = [];
    return;
  }
  clearReconcile();
  showFailures(props.sessionId);
  // Rows change with pushed snapshots; elapsed times tick with this clock.
  now.value = Date.now();
  clock = setInterval(() => { now.value = Date.now(); }, 1000);
  if (props.sessionId) {
    loading.value = true;
    void refresh(generation, props.sessionId);
  }
}, { flush: "sync" });
watch(appFailureCount, (count) => { if (count && open.value) showFailures(props.sessionId); });

watch(open, (value) => {
  if (!value) return;
  positionPanel();
  panel.value?.focus();
}, { flush: "post" });

onMounted(() => {
  document.addEventListener("pointerdown", handleOutside);
  document.addEventListener("focusin", handleOutside);
  document.addEventListener("keydown", handleKeydown);
  window.addEventListener("resize", positionPanel);
  window.addEventListener("scroll", positionPanel, true);
  window.addEventListener("focus", handleWindowFocus);
  void subscribeBackgroundTasksChanged(applyPushedTasks)
    .then((release) => { if (unmounted) release(); else releasePushes = release; })
    .catch(() => {
      // Seeding, opening and focus still list the tasks.
    });
});

onUnmounted(() => {
  unmounted = true;
  releasePushes?.();
  releasePushes = undefined;
  generation += 1;
  activityGeneration += 1;
  clearInterval(clock);
  clearReconcile();
  recorders.clear();
  window.removeEventListener("focus", handleWindowFocus);
  document.removeEventListener("pointerdown", handleOutside);
  document.removeEventListener("focusin", handleOutside);
  document.removeEventListener("keydown", handleKeydown);
  window.removeEventListener("resize", positionPanel);
  window.removeEventListener("scroll", positionPanel, true);
});
</script>

<template>
  <div ref="root" class="background-tasks-indicator">
    <BaseButton
      class="background-tasks-icon"
      :class="[`tone-${tone}`, { active: open }]"
      :aria-label="summary"
      aria-haspopup="dialog"
      :aria-expanded="open"
      :aria-controls="open ? panelId : undefined"
      @click="open = !open"
    >
      <LucideIcon class="background-tasks-glyph" :icon="ListTodo" :size="14" />
      <span v-if="activeCount" class="background-tasks-count" aria-hidden="true">{{ countLabel }}</span>
      <span v-if="failureCount" class="background-tasks-failure-dot" aria-hidden="true" />
      <span class="background-tasks-hint" aria-hidden="true">{{ summary }}</span>
    </BaseButton>
    <Teleport to="body">
      <section
        v-if="open"
        :id="panelId"
        ref="panel"
        class="background-tasks-panel"
        :class="`is-${placement}`"
        :style="panelStyle"
        role="dialog"
        :aria-label="t('chat.backgroundTasks.title')"
        tabindex="-1"
      >
        <header class="background-tasks-header">
          <LucideIcon class="background-tasks-header-icon" :icon="ListTodo" :size="14" />
          <span class="background-tasks-title">{{ t('chat.backgroundTasks.title') }}</span>
          <span v-if="tasks.length + appTasks.length" class="background-tasks-header-count">{{ tasks.length + appTasks.length }}</span>
          <BaseButton
            v-if="stoppableAgents.length > 1"
            class="background-tasks-stop-all"
            :aria-label="t('chat.backgroundTasks.stopAllAgents', stoppableAgents.length)"
            @click="stopAllAgents"
          >
            <LucideIcon :icon="Square" :size="10" />
            <span>{{ t('chat.backgroundTasks.stopAllAgents', stoppableAgents.length) }}</span>
          </BaseButton>
          <BaseButton class="background-tasks-close" :aria-label="t('common.close')" @click="close(true)">
            <LucideIcon :icon="X" :size="14" />
          </BaseButton>
        </header>
        <div class="background-tasks-body" :aria-busy="loading">
          <p v-if="error" class="background-tasks-error" role="alert">{{ error }}</p>
          <section v-if="shownFailures.length" class="background-tasks-group">
            <h3 class="background-tasks-group-label is-failed">
              {{ t('chat.backgroundTasks.failedGroup', shownFailures.length) }}
            </h3>
            <ul class="background-tasks-list">
              <li
                v-for="failure in shownFailures"
                :key="failure.taskId"
                class="background-task-item background-task-failure is-failed"
              >
                <span class="background-task-dot" aria-hidden="true" />
                <div class="background-task-summary">
                  <span class="background-task-name" :title="failure.description || failure.toolName">
                    {{ failure.description || failure.toolName }}
                  </span>
                  <span class="background-task-meta">
                    <span class="background-task-status">{{ t('chat.backgroundTasks.status.failed') }}</span>
                    <span v-if="failure.description" class="background-task-tool">{{ failure.toolName }}</span>
                    <span v-if="failure.source !== 'app'" class="background-task-id" :title="`${failure.toolName} · ${failure.taskId}`">{{ failure.taskId }}</span>
                  </span>
                </div>
                <p v-if="failure.detail" class="background-task-detail is-clamped" :title="failure.detail">
                  <span class="background-task-detail-text">{{ failure.detail }}</span>
                </p>
              </li>
            </ul>
          </section>
          <section v-if="showSessionGroup" class="background-tasks-group">
            <h3 v-if="tasks.length && (shownFailures.length || appTasks.length)" class="background-tasks-group-label">
              {{ t('chat.backgroundTasks.activeGroup') }}
            </h3>
            <p v-if="loading" class="background-tasks-empty is-loading">{{ t('common.loading') }}</p>
            <p v-else-if="!tasks.length && !loadError" class="background-tasks-empty">{{ t('chat.backgroundTasks.empty') }}</p>
            <ul v-else-if="tasks.length" class="background-tasks-list">
              <li
                v-for="task in tasks"
                :key="task.taskId"
                class="background-task-item background-task-row"
                :class="`is-${rowTone(task)}`"
              >
                <span class="background-task-dot" aria-hidden="true" />
                <button
                  type="button"
                  class="background-task-summary"
                  :class="{ 'is-expandable': progress(task), 'is-expanded': isExpanded(task) }"
                  :disabled="!progress(task)"
                  :aria-expanded="progress(task) ? isExpanded(task) : undefined"
                  :aria-controls="progress(task) && isExpanded(task) ? detailId(task) : undefined"
                  @click="toggleDetail(task)"
                >
                  <span class="background-task-name" :title="task.description || task.toolName">
                    {{ task.description || task.toolName }}
                  </span>
                  <span class="background-task-meta">
                    <span class="background-task-status">
                      {{ t(`chat.backgroundTasks.status.${isCancelling(task) ? 'cancelling' : task.status}`) }}
                    </span>
                    <span class="background-task-time" :title="t('chat.backgroundTasks.elapsed')">
                      <LucideIcon class="background-task-time-icon" :icon="Timer" :size="11" />
                      {{ elapsed(task) }}
                    </span>
                    <span v-if="task.description" class="background-task-tool">{{ task.toolName }}</span>
                    <span class="background-task-id" :title="`${task.toolName} · ${task.localId}`">{{ task.localId }}</span>
                    <LucideIcon v-if="progress(task)" class="background-task-chevron" :icon="ChevronDown" :size="12" />
                  </span>
                </button>
                <BaseButton
                  class="background-task-stop"
                  :disabled="isCancelling(task)"
                  :aria-label="t('chat.backgroundTasks.stopTask', task.description || task.localId)"
                  @click="stopTask(task)"
                >
                  <LucideIcon :icon="Square" :size="10" />
                  <span>{{ t('chat.backgroundTasks.stop') }}</span>
                </BaseButton>
                <p
                  v-if="progress(task) && isExpanded(task)"
                  :id="detailId(task)"
                  class="background-task-detail is-scrollable"
                >
                  <span class="background-task-detail-text">{{ progress(task) }}</span>
                </p>
              </li>
            </ul>
          </section>
          <section v-if="appTasks.length" class="background-tasks-group">
            <h3 class="background-tasks-group-label">{{ t('chat.backgroundTasks.appGroup') }}</h3>
            <ul class="background-tasks-list">
              <li
                v-for="task in appTasks"
                :key="task.taskId"
                class="background-task-item background-task-app is-running"
              >
                <span class="background-task-dot" aria-hidden="true" />
                <div class="background-task-summary">
                  <span class="background-task-name" :title="task.detail ? `${appTaskTitle(task)} · ${task.detail}` : appTaskTitle(task)">
                    {{ appTaskTitle(task) }}
                  </span>
                  <span class="background-task-meta">
                    <span class="background-task-status">{{ appTaskPhase(task) }}</span>
                    <span class="background-task-time" :title="t('chat.backgroundTasks.elapsed')">
                      <LucideIcon class="background-task-time-icon" :icon="Timer" :size="11" />
                      {{ elapsedSince(task.startedAt) }}
                    </span>
                    <span v-if="appTaskPercent(task) !== null" class="background-task-percent">
                      {{ appTaskPercent(task) }}% · {{ task.progress!.completed.toLocaleString() }}/{{ task.progress!.total!.toLocaleString() }}
                    </span>
                  </span>
                </div>
                <progress
                  class="background-task-progress"
                  :value="appTaskPercent(task) ?? undefined"
                  max="100"
                  :aria-label="appTaskPhase(task)"
                />
              </li>
            </ul>
          </section>
        </div>
      </section>
    </Teleport>
  </div>
</template>

<style scoped>
/* Spacing comes from the status icon row's gap, like its sibling icons. */
.background-tasks-indicator {
  position: relative;
  display: inline-flex;
  align-items: center;
}

/* Trigger: matches the neighbouring status icons in the composer row. */
.background-tasks-icon.base-button {
  position: relative;
  width: 24px;
  height: 24px;
  min-height: 24px;
  padding: 0;
  border-color: transparent;
  border-radius: 5px;
  color: var(--text-secondary);
  transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease;
}

.background-tasks-icon.base-button:hover:not(:disabled),
.background-tasks-icon.base-button.active,
.background-tasks-icon.base-button:focus-visible {
  background: var(--hover-bg);
  border-color: color-mix(in srgb, currentColor 22%, transparent);
}

/* Ready reads as healthy like the neighbouring status icons; only work in
   progress switches to the accent colour. */
.background-tasks-icon.base-button.tone-idle,
.background-tasks-icon.base-button.tone-idle:hover:not(:disabled) {
  color: var(--status-good-fg);
}

.background-tasks-icon.base-button.tone-running,
.background-tasks-icon.base-button.tone-running:hover:not(:disabled) {
  color: var(--accent-color);
}

.background-tasks-icon.base-button.tone-cancelling,
.background-tasks-icon.base-button.tone-cancelling:hover:not(:disabled) {
  color: var(--status-warn-fg);
}

.background-tasks-icon.tone-running .background-tasks-glyph {
  animation: background-tasks-breathe 1.35s ease-in-out infinite;
  transform-origin: center;
}

.background-tasks-count {
  position: absolute;
  right: -3px;
  bottom: -2px;
  min-width: 12px;
  height: 12px;
  padding: 0 3px;
  box-sizing: border-box;
  border-radius: 6px;
  background: var(--accent-color);
  color: var(--panel-bg);
  font-size: 9px;
  font-weight: 700;
  line-height: 12px;
  text-align: center;
  font-variant-numeric: tabular-nums;
  pointer-events: none;
}

.background-tasks-icon.tone-cancelling .background-tasks-count {
  background: var(--status-warn-fg);
}

.background-tasks-failure-dot {
  position: absolute;
  top: 0;
  right: 0;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--status-danger-fg);
  pointer-events: none;
}

.background-tasks-hint {
  position: absolute;
  left: 50%;
  bottom: calc(100% + 6px);
  z-index: 35;
  max-width: 220px;
  padding: 4px 7px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  background: var(--surface-elevated, var(--panel-bg));
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.16);
  color: var(--text-color);
  overflow: hidden;
  font-size: 11px;
  font-weight: 400;
  line-height: 1.3;
  text-overflow: ellipsis;
  white-space: nowrap;
  pointer-events: none;
  opacity: 0;
  transform: translate(-50%, 3px);
  transition: opacity 0.1s ease, transform 0.1s ease;
}

.background-tasks-icon:not(.active):hover .background-tasks-hint,
.background-tasks-icon:not(.active):focus-visible .background-tasks-hint {
  opacity: 1;
  transform: translate(-50%, 0);
}

/* Panel: same surface, radius and shadow as the status popovers. */
.background-tasks-panel {
  position: fixed;
  z-index: 1200;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  box-sizing: border-box;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--surface-elevated, var(--panel-bg));
  box-shadow: 0 12px 28px rgba(0, 0, 0, 0.18);
  color: var(--text-color);
  font-size: 12px;
  outline: none;
  animation: background-tasks-enter-above 0.12s ease;
}

.background-tasks-panel.is-below {
  animation-name: background-tasks-enter-below;
}

.background-tasks-header {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
  padding: 6px 6px 6px 12px;
  border-bottom: 1px solid var(--border-color);
}

.background-tasks-header-icon {
  flex: 0 0 auto;
  color: var(--text-secondary);
}

.background-tasks-title {
  font-weight: 600;
  line-height: 1.35;
}

.background-tasks-header-count {
  min-width: 16px;
  height: 16px;
  padding: 0 5px;
  box-sizing: border-box;
  border-radius: 8px;
  background: var(--accent-soft);
  color: var(--accent-color);
  font-size: 10.5px;
  font-weight: 600;
  line-height: 16px;
  text-align: center;
  font-variant-numeric: tabular-nums;
}

.background-tasks-close.base-button {
  width: 24px;
  height: 24px;
  min-height: 24px;
  margin-left: auto;
  padding: 0;
  border-color: transparent;
  border-radius: 5px;
}

.background-tasks-stop-all.base-button {
  gap: 4px;
  min-height: 22px;
  height: 22px;
  margin-left: auto;
  padding: 0 7px;
  border-color: var(--border-color);
  color: var(--text-secondary);
  font-size: 11px;
}

.background-tasks-stop-all.base-button:hover:not(:disabled) {
  border-color: var(--status-danger-border);
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.background-tasks-stop-all.base-button + .background-tasks-close.base-button {
  margin-left: 0;
}

.background-tasks-body {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-height: 0;
  padding: 4px;
  overflow: auto;
}

.background-tasks-group + .background-tasks-group {
  padding-top: 4px;
  border-top: 1px solid var(--border-color);
}

.background-tasks-group-label {
  margin: 0;
  padding: 6px 8px 2px;
  color: var(--text-secondary);
  font-size: 10.5px;
  font-weight: 600;
  line-height: 1.35;
}

.background-tasks-group-label.is-failed {
  color: var(--status-danger-fg);
}

.background-tasks-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin: 0;
  padding: 0;
  list-style: none;
}

/* Reading order: what the task is (title + its action), how it is going
   (status · elapsed · tool · id), then the output once asked for. Everything
   below the title shares the title's left edge and the action's right edge. */
.background-task-item {
  display: grid;
  grid-template-columns: 7px minmax(0, 1fr) auto;
  grid-template-areas:
    "dot summary stop"
    ". detail detail";
  align-items: start;
  column-gap: 8px;
  padding: 8px;
  border-radius: 6px;
  transition: background 0.12s ease;
}

.background-task-failure,
.background-task-app {
  grid-template-areas:
    "dot summary summary"
    ". detail detail";
}

.background-task-percent {
  flex: 0 0 auto;
  font-variant-numeric: tabular-nums;
}

.background-task-progress {
  grid-area: detail;
  width: 100%;
  height: 3px;
  margin-top: 6px;
  accent-color: var(--accent-color);
}

.background-task-item:hover {
  background: var(--hover-bg);
}

/* Centred on the title line (12px × 1.4). */
.background-task-dot {
  grid-area: dot;
  width: 7px;
  height: 7px;
  margin-top: 5px;
  box-sizing: border-box;
  border-radius: 50%;
  background: currentColor;
  color: var(--accent-color);
}

.background-task-item.is-running .background-task-dot {
  animation: background-tasks-pulse 1.2s ease-in-out infinite;
}

.background-task-item.is-queued .background-task-dot {
  border: 1.5px solid var(--text-secondary);
  background: transparent;
}

.background-task-item.is-cancelling .background-task-dot {
  color: var(--status-warn-fg);
}

.background-task-item.is-failed .background-task-dot {
  color: var(--status-danger-fg);
}

/* Title and meta form one click target that reveals the output. */
.background-task-summary {
  grid-area: summary;
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
  margin: 0;
  padding: 0;
  border: 0;
  border-radius: 3px;
  background: none;
  color: inherit;
  font: inherit;
  text-align: left;
}

button.background-task-summary:disabled {
  cursor: default;
}

.background-task-summary.is-expandable {
  cursor: pointer;
}

.background-task-summary:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: 2px;
}

.background-task-name {
  display: block;
  min-width: 0;
  overflow: hidden;
  font-weight: 600;
  line-height: 1.4;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.background-task-meta {
  display: flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1.4;
  white-space: nowrap;
}

/* Uniform separators keep the meta line readable as one sentence. */
.background-task-meta > * + *::before {
  content: "·";
  margin-right: 5px;
  color: var(--text-tertiary);
  font-weight: 400;
}

.background-task-status {
  flex: 0 0 auto;
  font-weight: 600;
}

.background-task-time {
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  color: var(--text-color);
  font-variant-numeric: tabular-nums;
}

.background-task-time-icon {
  margin-right: 3px;
  color: var(--text-secondary);
}

.background-task-tool {
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.background-task-item.is-running .background-task-status {
  color: var(--accent-color);
}

.background-task-item.is-cancelling .background-task-status {
  color: var(--status-warn-fg);
}

.background-task-item.is-failed .background-task-status {
  color: var(--status-danger-fg);
}

.background-task-id {
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
  color: var(--text-tertiary);
  font-family: var(--font-mono-identifier);
  text-overflow: ellipsis;
}

.background-task-chevron {
  flex: 0 0 auto;
  color: var(--text-tertiary);
  transition: transform 0.12s ease, color 0.12s ease;
}

.background-task-summary.is-expandable:hover .background-task-chevron,
.background-task-summary.is-expanded .background-task-chevron {
  color: var(--text-color);
}

.background-task-summary.is-expanded .background-task-chevron {
  transform: rotate(180deg);
}

/* Output reads as a quoted console block, set apart from the labels above. */
.background-task-detail {
  grid-area: detail;
  min-width: 0;
  margin: 6px 0 0;
  padding: 4px 7px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--text-color) 5%, transparent);
  color: var(--text-secondary);
  font-family: var(--font-mono-block);
  font-size: 11px;
  line-height: 1.45;
}

.background-task-detail-text {
  display: block;
  overflow-wrap: anywhere;
  white-space: pre-wrap;
}

/* An opened command is shown in full, scrolling past eight lines. */
.background-task-detail.is-scrollable {
  max-height: calc(1.45em * 8 + 8px);
  overflow-y: auto;
  user-select: text;
}

/* Clamped on an inner box so a third line never shows through the padding. */
.background-task-detail.is-clamped .background-task-detail-text {
  display: -webkit-box;
  overflow: hidden;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  line-clamp: 2;
}

.background-task-failure .background-task-detail {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

/* Centred on the title line without adding to its height. */
.background-task-stop.base-button {
  grid-area: stop;
  gap: 4px;
  min-height: 22px;
  height: 22px;
  margin-top: -3px;
  padding: 0 7px;
  border-color: var(--border-color);
  color: var(--text-secondary);
  font-size: 11px;
}

.background-task-stop.base-button:hover:not(:disabled) {
  border-color: var(--status-danger-border);
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.background-tasks-empty {
  margin: 0;
  padding: 10px 8px;
  color: var(--text-secondary);
  line-height: 1.45;
}

/* Fast responses never flash the loading text. */
.background-tasks-empty.is-loading {
  animation: background-tasks-fade-in 0.15s ease 0.25s both;
}

.background-tasks-error {
  flex-shrink: 0;
  margin: 0;
  padding: 7px 8px;
  border: 1px solid var(--status-danger-border);
  border-radius: 6px;
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
  line-height: 1.45;
  overflow-wrap: anywhere;
}

@keyframes background-tasks-breathe {
  0%,
  100% {
    opacity: 0.72;
    transform: scale(0.96);
  }
  50% {
    opacity: 1;
    transform: scale(1.04);
  }
}

@keyframes background-tasks-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.35;
  }
}

@keyframes background-tasks-enter-above {
  from {
    opacity: 0;
    transform: translateY(4px);
  }
}

@keyframes background-tasks-enter-below {
  from {
    opacity: 0;
    transform: translateY(-4px);
  }
}

@keyframes background-tasks-fade-in {
  from {
    opacity: 0;
  }
}

@media (prefers-reduced-motion: reduce) {
  .background-tasks-icon.tone-running .background-tasks-glyph,
  .background-task-item.is-running .background-task-dot,
  .background-tasks-panel,
  .background-tasks-empty.is-loading {
    animation: none;
  }
}
</style>
