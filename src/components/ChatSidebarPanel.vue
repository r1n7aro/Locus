<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { t } from "../i18n";
import { useChatChangesStore } from "../stores/chatChanges";
import { acquireSelectionLock } from "../composables/useSelectionLock";
import ChatChangesPanel from "./ChatChangesPanel.vue";
import type { WorkspaceRef } from "../services/project";
import type { ChangedFile, ChatMessage, UndoConflictInfo } from "../types";
import type { UserMessageDraft } from "../composables/chatMessageDraft";

const props = withDefaults(defineProps<{
  layout?: "side" | "bottom";
  maxSideWidth?: number;
  storageScope?: string;
  workspaceRef?: WorkspaceRef | null;
  scopedSession?: boolean;
  sessionId?: string | null;
  messages?: ChatMessage[];
  isStreaming?: boolean;
  unityConnected?: boolean;
  checkUndoConflicts?: (assistantMessageId: string) => Promise<UndoConflictInfo[]>;
  checkUndoDirty?: (assistantMessageId: string) => Promise<ChangedFile[]>;
  performUndo?: (
    targetMessageId: string,
    options: { force: boolean; acceptDirty: boolean },
  ) => Promise<boolean>;
  restoreComposerDraft?: (draft: UserMessageDraft) => void | Promise<void>;
}>(), {
  layout: "side",
  storageScope: "",
});

const changesStore = useChatChangesStore();

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

const sidebarWidthStorageKey = computed(() => scopedSidebarStorageKey(STORAGE_KEY_SIDEBAR_WIDTH));
const sidebarHeightStorageKey = computed(() => scopedSidebarStorageKey(STORAGE_KEY_SIDEBAR_HEIGHT));
const effectiveMaxSideWidth = computed(() => {
  const maxWidth = props.maxSideWidth;
  if (typeof maxWidth !== "number" || !Number.isFinite(maxWidth)) {
    return MAX_SIDEBAR_WIDTH;
  }
  return Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, Math.floor(maxWidth)));
});
const effectiveSidebarWidth = computed(() =>
  clampSidebarWidth(sidebarWidth.value, effectiveMaxSideWidth.value),
);

const sidebarStyle = computed(() => {
  if (props.layout === "bottom") {
    return {
      width: "100%",
      minWidth: "0",
      height: `${sidebarHeight.value}px`,
      minHeight: `${sidebarHeight.value}px`,
    };
  }
  const width = effectiveSidebarWidth.value;
  return {
    width: `${width}px`,
    minWidth: `${width}px`,
  };
});

function scopedSidebarStorageKey(baseKey: string) {
  const scope = props.storageScope.trim();
  if (!scope) return baseKey;
  return baseKey.replace("locus:", `locus:${scope}:`);
}

function clampSidebarWidth(next: number, maxWidth = MAX_SIDEBAR_WIDTH) {
  const normalizedNext = Number.isFinite(next) ? next : DEFAULT_SIDEBAR_WIDTH;
  const normalizedMax = Number.isFinite(maxWidth) ? maxWidth : MAX_SIDEBAR_WIDTH;
  const upperBound = Math.max(
    MIN_SIDEBAR_WIDTH,
    Math.min(MAX_SIDEBAR_WIDTH, Math.floor(normalizedMax)),
  );
  return Math.max(MIN_SIDEBAR_WIDTH, Math.min(upperBound, normalizedNext));
}

function clampSidebarHeight(next: number) {
  return Math.max(MIN_SIDEBAR_HEIGHT, Math.min(MAX_SIDEBAR_HEIGHT, next));
}

function closeSidebar() {
  if (props.scopedSession) changesStore.closePanelForSession(props.sessionId);
  else changesStore.closePanel();
}

function onSidebarResizeKeydown(event: KeyboardEvent) {
  const step = event.shiftKey ? 24 : 8;
  if (props.layout === "bottom") {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    sidebarHeight.value = clampSidebarHeight(
      sidebarHeight.value + (event.key === "ArrowUp" ? step : -step),
    );
    try {
      localStorage.setItem(sidebarHeightStorageKey.value, String(Math.round(sidebarHeight.value)));
    } catch {}
    return;
  }
  if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
  event.preventDefault();
  sidebarWidth.value = clampSidebarWidth(
    sidebarWidth.value + (event.key === "ArrowLeft" ? step : -step),
    effectiveMaxSideWidth.value,
  );
  try {
    localStorage.setItem(sidebarWidthStorageKey.value, String(Math.round(sidebarWidth.value)));
  } catch {}
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
  sidebarWidth.value = clampSidebarWidth(nextWidth, effectiveMaxSideWidth.value);
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
      localStorage.setItem(sidebarHeightStorageKey.value, String(Math.round(sidebarHeight.value)));
    } else {
      localStorage.setItem(sidebarWidthStorageKey.value, String(Math.round(effectiveSidebarWidth.value)));
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
    const savedWidth = localStorage.getItem(sidebarWidthStorageKey.value);
    if (savedWidth) {
      sidebarWidth.value = clampSidebarWidth(Number(savedWidth));
    }
    const savedHeight = localStorage.getItem(sidebarHeightStorageKey.value);
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
    <div
      class="chat-sidebar-resize-handle"
      role="separator"
      :aria-orientation="layout === 'bottom' ? 'horizontal' : 'vertical'"
      :aria-label="t('chat.changes.resizePanel')"
      :aria-valuemin="layout === 'bottom' ? MIN_SIDEBAR_HEIGHT : MIN_SIDEBAR_WIDTH"
      :aria-valuemax="layout === 'bottom' ? MAX_SIDEBAR_HEIGHT : effectiveMaxSideWidth"
      :aria-valuenow="layout === 'bottom' ? Math.round(sidebarHeight) : Math.round(effectiveSidebarWidth)"
      tabindex="0"
      @mousedown="onSidebarResizeMouseDown"
      @keydown="onSidebarResizeKeydown"
    ></div>

    <aside
      class="chat-sidebar-panel changes-only"
      :style="sidebarStyle"
    >
      <button
        class="chat-sidebar-close"
        type="button"
        :title="t('common.close')"
        :aria-label="t('common.close')"
        @click="closeSidebar"
      >&times;</button>

      <ChatChangesPanel
        class="chat-sidebar-section chat-sidebar-section-changes"
        embedded
        :show-close="false"
        :workspace-ref="workspaceRef"
        :scoped-session="scopedSession"
        :session-id="sessionId"
        :messages="messages"
        :is-streaming="isStreaming"
        :unity-connected="unityConnected"
        :check-undo-conflicts="checkUndoConflicts"
        :check-undo-dirty="checkUndoDirty"
        :perform-undo="performUndo"
        :restore-composer-draft="restoreComposerDraft"
        @close="closeSidebar"
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
.chat-sidebar-resize-handle:focus-visible,
.chat-sidebar-shell.dragging-sidebar .chat-sidebar-resize-handle {
  background: var(--text-secondary);
}

.chat-sidebar-resize-handle:focus-visible {
  outline: 1px solid var(--accent-color);
  outline-offset: -1px;
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
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
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

:deep(.changes-panel.embedded) {
  flex: 1;
  min-height: 0;
}

:deep(.changes-panel.embedded .panel-header) {
  padding-right: 48px;
}
</style>
