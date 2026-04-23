
<script setup lang="ts">
import { computed, watch, ref, onBeforeUnmount } from "vue";
import type { TodoItem } from "../types";
import { t } from "../i18n";

const props = defineProps<{
  todos: TodoItem[];
  isStreaming: boolean;
  todoWriteVersion: number;
  celebrationEnabled: boolean;
  embedded?: boolean;
  showClose?: boolean;
}>();

const emit = defineEmits<{
  close: [];
}>();

const closing = ref(false);
const AUTO_CLOSE_DELAY_MS = 3200;
const CELEBRATION_RESET_DELAY_MS = 400;
function clearCloseTimer() {
  if (closeTimer) {
    clearTimeout(closeTimer);
    closeTimer = null;
  }
}

function doClose() {
  if (closing.value) return;
  clearCloseTimer();
  closing.value = true;
  // Wait for the CSS transition to finish before actually removing
}

function onTransitionEnd(e: TransitionEvent) {
  const closeProperty = props.embedded ? "max-height" : "width";
  if (closing.value && e.propertyName === closeProperty) {
    emit('close');
  }
}

const priorityLabel: Record<string, string> = {
  high: "H",
  medium: "M",
  low: "L",
};

const completedCount = computed(() => props.todos.filter(item => item.status === "completed" || item.status === "cancelled").length);

const allCompleted = computed(() => {
  return props.todos.length > 0 &&
    props.todos.every(item => item.status === "completed" || item.status === "cancelled");
});
const emptyText = computed(() => t("todo.emptyCurrent"));

const showCelebration = ref(false);
let closeTimer: ReturnType<typeof setTimeout> | null = null;

// Trigger celebration as soon as the latest todowrite marks everything done.
watch(() => props.todoWriteVersion, (version, previousVersion) => {
  if (version === previousVersion) return;
  if (!props.celebrationEnabled) {
    cancelCelebration();
    return;
  }
  if (allCompleted.value) {
    triggerCelebration();
  } else {
    cancelCelebration();
  }
});

// Keep the old end-of-stream behavior as a fallback for sessions that finish
// without another todowrite after the final completion update.
watch(() => props.isStreaming, (streaming, wasStreaming) => {
  if (!props.celebrationEnabled) {
    cancelCelebration();
    return;
  }
  if (wasStreaming && !streaming && allCompleted.value && !showCelebration.value) {
    triggerCelebration();
  }
});

// Also trigger if all tasks become completed while not streaming (e.g. loaded session)
watch([allCompleted, () => props.celebrationEnabled], ([done, enabled]) => {
  if (!enabled || !done) {
    cancelCelebration();
    return;
  }
  if (!props.isStreaming && !showCelebration.value) {
    triggerCelebration();
  }
});

function triggerCelebration() {
  if (closing.value) return;
  clearCloseTimer();
  showCelebration.value = true;
  closeTimer = setTimeout(() => {
    doClose();
    setTimeout(() => { showCelebration.value = false; }, CELEBRATION_RESET_DELAY_MS);
  }, AUTO_CLOSE_DELAY_MS);
}

function cancelCelebration() {
  if (closing.value) return;
  clearCloseTimer();
  showCelebration.value = false;
}

onBeforeUnmount(() => {
  clearCloseTimer();
});
</script>

<template>
  <aside
    class="todo-panel"
    :class="{ celebrating: showCelebration, closing: closing, embedded: props.embedded }"
    @transitionend="onTransitionEnd"
  >
    <div class="panel-header">
      <span class="panel-title">{{ t("todo.title") }}</span>
      <span class="todo-count">{{ t("todo.remaining", String(props.todos.filter(item => item.status !== 'completed' && item.status !== 'cancelled').length)) }}</span>
      <button v-if="props.showClose ?? true" class="close-btn" @click="doClose()" :title="t('todo.close')">&times;</button>
    </div>

    <div class="todo-list">
      <div
        v-for="(todo, idx) in props.todos"
        :key="idx"
        class="todo-item"
        :class="[`status-${todo.status}`]"
      >
        <span class="todo-status" :title="todo.status" aria-hidden="true"></span>
        <span class="todo-content">{{ todo.content }}</span>
        <span class="todo-priority" :class="`priority-${todo.priority}`">{{ priorityLabel[todo.priority] || todo.priority }}</span>
      </div>
      <div v-if="props.todos.length === 0" class="empty-hint">{{ emptyText }}</div>
    </div>

    <!-- Celebration overlay -->
    <Transition name="celebration">
      <div v-if="showCelebration" class="celebration-overlay">
        <div class="celebration-content">
          <!-- Animated circle + checkmark -->
          <svg class="check-svg" viewBox="0 0 52 52" width="56" height="56">
            <circle class="check-circle" cx="26" cy="26" r="24" fill="none" stroke-width="2.5" />
            <path class="check-mark" fill="none" stroke-width="3" d="M14.1 27.2l7.1 7.2 16.7-16.8" />
          </svg>
          <div class="celebration-text">{{ t("todo.allDone") }}</div>
          <div class="celebration-count">{{ completedCount }}/{{ props.todos.length }}</div>
        </div>
        <!-- Sparkle particles -->
        <div class="sparkles">
          <span v-for="i in 8" :key="i" class="sparkle" :style="{ '--i': i }" />
        </div>
      </div>
    </Transition>
  </aside>
</template>

<style scoped>
.todo-panel {
  width: 280px;
  min-width: 280px;
  height: 100vh;
  background: var(--msg-assistant-bg);
  border-left: 1px solid var(--border-color);
  display: flex;
  flex-direction: column;
  position: relative;
  transition: width 0.3s ease, min-width 0.3s ease, opacity 0.25s ease;
  overflow: hidden;
}

.todo-panel.embedded {
  width: auto;
  min-width: 0;
  height: auto;
  max-height: 2000px;
  background: transparent;
  border-left: none;
  transition: max-height 0.28s ease, opacity 0.25s ease;
}

.todo-panel.closing {
  width: 0;
  min-width: 0;
  opacity: 0;
  border-left-width: 0;
}

.todo-panel.embedded.closing {
  width: auto;
  min-width: 0;
  max-height: 0;
  opacity: 0;
}

.panel-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border-color);
}

.panel-title {
  flex: 1;
  font-size: 14px;
  font-weight: 600;
  white-space: nowrap;
}

.todo-count {
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
}

.close-btn {
  width: 24px;
  height: 24px;
  border-radius: 4px;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: 16px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  box-shadow: none;
  flex-shrink: 0;
}

.close-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

/* ── Todo list ── */
.todo-list {
  flex: 1;
  overflow-y: auto;
  padding: 8px;
}

.todo-item {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 6px;
  margin-bottom: 2px;
  transition: background 0.15s;
}

.todo-item:hover {
  background: var(--hover-bg);
}

.todo-status {
  flex-shrink: 0;
  width: 14px;
  min-width: 14px;
  height: 20px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
}

.todo-status::before {
  content: "";
  width: 7px;
  height: 7px;
  border-radius: 999px;
  border: 1px solid currentColor;
  box-sizing: border-box;
}

.status-pending .todo-status {
  color: color-mix(in srgb, var(--text-secondary) 72%, transparent);
}

.status-in_progress .todo-status {
  color: var(--accent-color);
}

.status-in_progress .todo-status::before {
  background: linear-gradient(90deg, currentColor 50%, transparent 50%);
}

.status-completed .todo-status {
  color: var(--status-good-fg);
}

.status-completed .todo-status::before {
  background: currentColor;
}

.status-cancelled .todo-status {
  color: color-mix(in srgb, var(--text-secondary) 70%, transparent);
  opacity: 0.5;
}

.status-cancelled .todo-status::before {
  width: 8px;
  height: 2px;
  border: none;
  border-radius: 999px;
  background: currentColor;
}

.todo-content {
  flex: 1;
  font-size: 13px;
  line-height: 20px;
  word-break: break-word;
}

.status-completed .todo-content {
  text-decoration: line-through;
  opacity: 0.6;
}

.status-cancelled .todo-content {
  text-decoration: line-through;
  opacity: 0.4;
}

.todo-priority {
  flex-shrink: 0;
  font-size: 10px;
  font-weight: 600;
  width: 18px;
  height: 18px;
  border-radius: 3px;
  display: flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
}

.priority-high {
  background: rgba(239, 68, 68, 0.15);
  color: #ef4444;
}

.priority-medium {
  background: rgba(234, 179, 8, 0.15);
  color: #ca8a04;
}

.priority-low {
  background: rgba(107, 114, 128, 0.1);
  color: var(--text-secondary);
}

.empty-hint {
  text-align: center;
  color: var(--text-secondary);
  font-size: 13px;
  padding: 24px 0;
}

/* ── Celebration overlay ── */
.celebration-overlay {
  position: absolute;
  inset: 0;
  background: var(--sidebar-bg);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 10;
  overflow: hidden;
}

.celebration-content {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
}

.celebration-enter-active {
  animation: celebration-in 0.4s cubic-bezier(0.34, 1.56, 0.64, 1);
}

.celebration-leave-active {
  animation: celebration-out 0.35s ease forwards;
}

@keyframes celebration-in {
  from { opacity: 0; transform: scale(0.9); }
  to { opacity: 1; transform: scale(1); }
}

@keyframes celebration-out {
  from { opacity: 1; transform: scale(1) translateX(0); }
  to { opacity: 0; transform: scale(0.95) translateX(20px); }
}

/* ── SVG checkmark animation ── */
.check-svg {
  filter: drop-shadow(0 0 8px rgba(34, 197, 94, 0.3));
}

.check-circle {
  stroke: #22c55e;
  stroke-dasharray: 151;
  stroke-dashoffset: 151;
  animation: circle-draw 0.6s 0.15s cubic-bezier(0.65, 0, 0.45, 1) forwards;
  transform-origin: center;
}

.check-mark {
  stroke: #22c55e;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-dasharray: 36;
  stroke-dashoffset: 36;
  animation: check-draw 0.35s 0.55s cubic-bezier(0.65, 0, 0.45, 1) forwards;
}

@keyframes circle-draw {
  to { stroke-dashoffset: 0; }
}

@keyframes check-draw {
  to { stroke-dashoffset: 0; }
}

.celebration-text {
  font-size: 14px;
  font-weight: 600;
  color: #22c55e;
  opacity: 0;
  animation: text-rise 0.4s 0.7s ease forwards;
}

.celebration-count {
  font-size: 12px;
  color: var(--text-secondary);
  opacity: 0;
  animation: text-rise 0.4s 0.85s ease forwards;
}

@keyframes text-rise {
  from { opacity: 0; transform: translateY(6px); }
  to { opacity: 1; transform: translateY(0); }
}

/* ── Sparkle particles ── */
.sparkles {
  position: absolute;
  top: 50%;
  left: 50%;
  pointer-events: none;
}

.sparkle {
  position: absolute;
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: #22c55e;
  opacity: 0;
  animation: sparkle-burst 0.8s calc(0.5s + var(--i) * 0.04s) ease-out forwards;
}

.sparkle:nth-child(odd) {
  background: #86efac;
  width: 3px;
  height: 3px;
}

.sparkle:nth-child(1) { --angle: 0deg; }
.sparkle:nth-child(2) { --angle: 45deg; }
.sparkle:nth-child(3) { --angle: 90deg; }
.sparkle:nth-child(4) { --angle: 135deg; }
.sparkle:nth-child(5) { --angle: 180deg; }
.sparkle:nth-child(6) { --angle: 225deg; }
.sparkle:nth-child(7) { --angle: 270deg; }
.sparkle:nth-child(8) { --angle: 315deg; }

@keyframes sparkle-burst {
  0% {
    opacity: 1;
    transform: rotate(var(--angle)) translateY(0);
  }
  70% {
    opacity: 1;
  }
  100% {
    opacity: 0;
    transform: rotate(var(--angle)) translateY(-48px);
  }
}
</style>
