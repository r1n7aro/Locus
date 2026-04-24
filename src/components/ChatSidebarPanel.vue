<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { t } from "../i18n";
import type { TodoItem } from "../types";
import { useChatStore } from "../stores/chat";
import { useChatChangesStore } from "../stores/chatChanges";
import { acquireSelectionLock } from "../composables/useSelectionLock";
import TodoPanel from "./TodoPanel.vue";
import ChatChangesPanel from "./ChatChangesPanel.vue";

const props = withDefaults(defineProps<{
  todos: TodoItem[];
  isStreaming: boolean;
  todoWriteVersion: number;
  celebrationEnabled: boolean;
  layout?: "side" | "bottom";
}>(), {
  layout: "side",
});

const chatStore = useChatStore();
const changesStore = useChatChangesStore();

const showTodoSection = computed(() => chatStore.showTodoPanel);
const showChangesSection = computed(() => changesStore.currentPanelVisible);
const hasBothSections = computed(() => showTodoSection.value && showChangesSection.value);

const STORAGE_KEY_SIDEBAR_WIDTH = "locus:chatSidebarWidth";
const STORAGE_KEY_SIDEBAR_HEIGHT = "locus:chatSidebarHeight";
const DEFAULT_SIDEBAR_WIDTH = 280;
const DEFAULT_SIDEBAR_HEIGHT = 260;
const MIN_SIDEBAR_WIDTH = 240;
const MAX_SIDEBAR_WIDTH = 520;
const MIN_SIDEBAR_HEIGHT = 180;
const MAX_SIDEBAR_HEIGHT = 460;

const shellRef = ref<HTMLElement | null>(null);
const sidebarWidth = ref(DEFAULT_SIDEBAR_WIDTH);
const sidebarHeight = ref(DEFAULT_SIDEBAR_HEIGHT);
const isDraggingSidebar = ref(false);
let releaseSidebarSelectionLock: (() => void) | null = null;

const sidebarStyle = computed(() => {
  if (props.layout === "bottom") {
    return {
      width: "100%",
      minWidth: "0",
      height: `${sidebarHeight.value}px`,
      minHeight: `${sidebarHeight.value}px`,
    };
  }
  return {
    width: `${sidebarWidth.value}px`,
    minWidth: `${sidebarWidth.value}px`,
  };
});

function clampSidebarWidth(next: number) {
  return Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, next));
}

function clampSidebarHeight(next: number) {
  return Math.max(MIN_SIDEBAR_HEIGHT, Math.min(MAX_SIDEBAR_HEIGHT, next));
}

function closeSidebar() {
  chatStore.closeTodoPanel();
  changesStore.closePanel();
}

function onSidebarResizeMouseDown(event: MouseEvent) {
  event.preventDefault();
  isDraggingSidebar.value = true;
  releaseSidebarSelectionLock?.();
  releaseSidebarSelectionLock = acquireSelectionLock();
  document.addEventListener("mousemove", onSidebarResizeMouseMove);
  document.addEventListener("mouseup", onSidebarResizeMouseUp);
  document.body.style.cursor = props.layout === "bottom" ? "row-resize" : "col-resize";
}

function onSidebarResizeMouseMove(event: MouseEvent) {
  if (!isDraggingSidebar.value || !shellRef.value) return;
  const rect = shellRef.value.getBoundingClientRect();
  if (props.layout === "bottom") {
    sidebarHeight.value = clampSidebarHeight(rect.bottom - event.clientY);
    return;
  }
  const nextWidth = rect.right - event.clientX;
  sidebarWidth.value = clampSidebarWidth(nextWidth);
}

function stopSidebarResize(persist: boolean) {
  if (!isDraggingSidebar.value && !releaseSidebarSelectionLock) return;
  isDraggingSidebar.value = false;
  document.removeEventListener("mousemove", onSidebarResizeMouseMove);
  document.removeEventListener("mouseup", onSidebarResizeMouseUp);
  document.body.style.cursor = "";
  releaseSidebarSelectionLock?.();
  releaseSidebarSelectionLock = null;
  if (!persist) return;
  try {
    if (props.layout === "bottom") {
      localStorage.setItem(STORAGE_KEY_SIDEBAR_HEIGHT, String(Math.round(sidebarHeight.value)));
    } else {
      localStorage.setItem(STORAGE_KEY_SIDEBAR_WIDTH, String(Math.round(sidebarWidth.value)));
    }
  } catch {
    // ignore persistence failures
  }
}

function onSidebarResizeMouseUp() {
  stopSidebarResize(true);
}

function onWindowResize() {
  sidebarWidth.value = clampSidebarWidth(sidebarWidth.value);
  sidebarHeight.value = clampSidebarHeight(sidebarHeight.value);
}

onMounted(() => {
  try {
    const savedWidth = localStorage.getItem(STORAGE_KEY_SIDEBAR_WIDTH);
    if (savedWidth) {
      sidebarWidth.value = clampSidebarWidth(Number(savedWidth));
    }
    const savedHeight = localStorage.getItem(STORAGE_KEY_SIDEBAR_HEIGHT);
    if (savedHeight) {
      sidebarHeight.value = clampSidebarHeight(Number(savedHeight));
    }
  } catch {
    // ignore persistence failures
  }
  sidebarWidth.value = clampSidebarWidth(sidebarWidth.value);
  sidebarHeight.value = clampSidebarHeight(sidebarHeight.value);
  window.addEventListener("resize", onWindowResize);
});

onUnmounted(() => {
  window.removeEventListener("resize", onWindowResize);
  stopSidebarResize(false);
});
</script>

<template>
  <div
    ref="shellRef"
    class="chat-sidebar-shell"
    :class="[
      `layout-${layout}`,
      { 'dragging-sidebar': isDraggingSidebar },
    ]"
  >
    <div class="chat-sidebar-resize-handle" @mousedown="onSidebarResizeMouseDown"></div>

    <aside
      class="chat-sidebar-panel"
      :class="{
        'has-both-sections': hasBothSections,
        'todo-only': showTodoSection && !showChangesSection,
        'changes-only': showChangesSection && !showTodoSection,
      }"
      :style="sidebarStyle"
    >
      <button class="chat-sidebar-close" :title="t('todo.close')" @click="closeSidebar">&times;</button>

      <TodoPanel
        v-if="showTodoSection"
        class="chat-sidebar-section chat-sidebar-section-todo"
        :todos="props.todos"
        :is-streaming="props.isStreaming"
        :todo-write-version="props.todoWriteVersion"
        :celebration-enabled="props.celebrationEnabled"
        embedded
        :show-close="false"
        @close="chatStore.closeTodoPanel()"
      />

      <div v-if="hasBothSections" class="chat-sidebar-divider"></div>

      <ChatChangesPanel
        v-if="showChangesSection"
        class="chat-sidebar-section chat-sidebar-section-changes"
        embedded
        :show-close="false"
        @close="changesStore.closePanel()"
      />
    </aside>
  </div>
</template>

<style scoped>
.chat-sidebar-shell {
  display: flex;
  height: 100%;
  min-height: 0;
  flex-shrink: 0;
}

.chat-sidebar-shell.layout-bottom {
  width: 100%;
  height: auto;
  min-width: 0;
  flex-direction: column;
}

.chat-sidebar-resize-handle {
  width: 3px;
  flex-shrink: 0;
  cursor: col-resize;
  background: var(--border-color);
  transition: background 0.15s ease;
}

.chat-sidebar-shell.layout-bottom .chat-sidebar-resize-handle {
  width: 100%;
  height: 3px;
  cursor: row-resize;
}

.chat-sidebar-resize-handle:hover,
.chat-sidebar-shell.dragging-sidebar .chat-sidebar-resize-handle {
  background: var(--text-secondary);
}

.chat-sidebar-panel {
  width: 280px;
  min-width: 280px;
  height: 100%;
  min-height: 0;
  background: var(--msg-assistant-bg);
  display: flex;
  flex-direction: column;
  position: relative;
  overflow: hidden;
  flex-shrink: 0;
}

.chat-sidebar-shell.layout-bottom .chat-sidebar-panel {
  width: 100%;
  min-width: 0;
  height: 260px;
  min-height: 180px;
}

.chat-sidebar-section {
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.chat-sidebar-panel.has-both-sections .chat-sidebar-section-todo {
  flex: 0 1 40%;
  min-height: 168px;
}

.chat-sidebar-panel.has-both-sections .chat-sidebar-section-changes {
  flex: 1 1 0;
  min-height: 220px;
}

.chat-sidebar-panel.todo-only .chat-sidebar-section,
.chat-sidebar-panel.changes-only .chat-sidebar-section {
  flex: 1 1 auto;
}

.chat-sidebar-divider {
  height: 1px;
  flex-shrink: 0;
  background: var(--border-color);
}

:deep(.todo-panel.embedded.chat-sidebar-section-todo.closing) {
  min-height: 0 !important;
  flex: 0 0 auto !important;
}

.chat-sidebar-close {
  position: absolute;
  top: 12px;
  right: 16px;
  z-index: 2;
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
}

.chat-sidebar-close:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

:deep(.todo-panel.embedded),
:deep(.changes-panel.embedded) {
  flex: 1;
  min-height: 0;
}

:deep(.todo-panel.embedded .panel-header),
:deep(.changes-panel.embedded .panel-header) {
  padding-right: 48px;
}
</style>
