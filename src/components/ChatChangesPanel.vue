<script setup lang="ts">
import { computed, ref, watch, onUnmounted } from "vue";
import { useChatStore } from "../stores/chat";
import { useChatChangesStore } from "../stores/chatChanges";
import { useProjectStore } from "../stores/project";
import { useUiStore } from "../stores/ui";
import { diffSingleFile, createRequestToken, isTokenStale } from "../services/diff";
import { normalizeAppError } from "../services/errors";
import { findUndoRestoreUserText } from "../services/chatUndo";
import { selectUnityAsset, openFileExternal } from "../services/unity";
import { t } from "../i18n";
import { useNotificationStore } from "../stores/notification";
import FileDiffPopover from "./diff/FileDiffPopover.vue";
import type { GitFileChange, FileDiffPayload, UndoConflictInfo } from "../types";
import type { ChatMergedFileItem } from "../services/chatChanges";
import { useHideMeta, isMetaFile, canOpenInEditor } from "../composables/useHideMeta";

const { hideMeta } = useHideMeta();
const projectStore = useProjectStore();
const notificationStore = useNotificationStore();

const emit = defineEmits<{ close: [] }>();
const props = defineProps<{
  embedded?: boolean;
  showClose?: boolean;
}>();

const chatStore = useChatStore();
const changesStore = useChatChangesStore();
const uiStore = useUiStore();

const mode = computed(() => changesStore.currentMode);

// Close any stale diff UI when session changes and invalidate in-flight click requests.
let clickSeq = 0;
watch(() => chatStore.activeSessionId, () => {
  clickSeq++;
  changesStore.closeInlineDiff();
});

// ── Hover preview state ──
const hoverAnchor = ref<HTMLElement | null>(null);
const showPopover = ref(false);
const previewPayload = ref<FileDiffPayload | null>(null);
let hoverTimer: ReturnType<typeof setTimeout> | null = null;

function clearHover() {
  if (hoverTimer) {
    clearTimeout(hoverTimer);
    hoverTimer = null;
  }
  createRequestToken(); // bump to stale any in-flight
  showPopover.value = false;
  previewPayload.value = null;
  hoverAnchor.value = null;
}

onUnmounted(() => { if (hoverTimer) clearTimeout(hoverTimer); });

// ── Data ──

interface DisplayItem {
  key: string;
  fileChange: GitFileChange;
  assistantMessageId: string;
  roundCount?: number;
}

const currentModeItems = computed<DisplayItem[]>(() => {
  const turnRounds = changesStore.latestTurnRounds;
  const turnFiles = changesStore.latestTurnFiles;
  if (turnRounds.length === 0) return [];
  // Use the first round's assistantMessageId so diff/undo span the whole current run.
  const msgId = turnRounds[0].assistantMessageId;
  return turnFiles.map((f, i) => ({
    key: `cur-${i}-${f.path}`,
    fileChange: { path: f.path, oldPath: f.oldPath, status: f.status } as GitFileChange,
    assistantMessageId: msgId,
  }));
});

const allModeItems = computed<DisplayItem[]>(() => {
  return (changesStore.currentFiles as ChatMergedFileItem[]).map((item) => ({
    key: `all-${item.id}`,
    fileChange: {
      path: item.finalPath,
      oldPath: item.baseOldPath,
      status: item.status,
    } as GitFileChange,
    assistantMessageId: item.baseAssistantMessageId,
    roundCount: item.roundCount,
  }));
});

const displayItems = computed(() => {
  const items = mode.value === "current" ? currentModeItems.value : allModeItems.value;
  return hideMeta.value ? items.filter((item) => !isMetaFile(item.fileChange.path)) : items;
});

// ── Helpers ──

function fileName(path: string): string {
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}

function dirPath(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  const lastSlash = normalized.lastIndexOf("/");
  return lastSlash > 0 ? normalized.substring(0, lastSlash + 1) : "";
}

function buildRequest(item: DisplayItem, detail: "preview" | "full") {
  return {
    source: "chatCheckpoint" as const,
    filePath: item.fileChange.path,
    oldPath: item.fileChange.oldPath,
    sessionId: chatStore.activeSessionId ?? undefined,
    assistantMessageId: item.assistantMessageId,
    detail,
  };
}

// ── Hover ──

function onItemMouseEnter(ev: MouseEvent, item: DisplayItem) {
  const el = ev.currentTarget as HTMLElement;
  hoverTimer = setTimeout(async () => {
    const token = createRequestToken();
    try {
      const payload = await diffSingleFile(buildRequest(item, "preview"));
      if (isTokenStale(token)) return;
      previewPayload.value = payload;
      hoverAnchor.value = el;
      showPopover.value = true;
    } catch { /* silently ignore */ }
  }, 150);
}

function onItemMouseLeave() {
  clearHover();
}

// ── Click → inline diff ──

async function onItemClick(item: DisplayItem) {
  clearHover();
  const seq = ++clickSeq;
  changesStore.setInlineDiffLoading(true);
  try {
    const payload = await diffSingleFile(buildRequest(item, "full"));
    if (seq !== clickSeq) return; // stale — newer click or session switch
    changesStore.openInlineDiff(payload, item.assistantMessageId);
  } catch (e) {
    if (seq !== clickSeq) return;
    const err = normalizeAppError(e);
    changesStore.setInlineDiffError(err.message);
    console.error("[ChatChangesPanel] failed to fetch full diff:", e);
  }
}

// ── Undo with confirmation ──

const showUndoConfirm = ref(false);
const showUndoConflictConfirm = ref(false);
const checkingUndoConflicts = ref(false);
const undoConflicts = ref<UndoConflictInfo[]>([]);

/** The assistantMessageId to undo — depends on mode */
const undoTargetId = computed(() => {
  if (mode.value === "current") {
    // Earliest assistantMessageId in the latest run so undo covers the whole run.
    const turns = changesStore.latestTurnRounds;
    return turns.length > 0 ? turns[0].assistantMessageId : null;
  }
  // "all" mode: earliest round's assistantMessageId to undo everything
  const rounds = changesStore.currentRounds;
  return rounds.length > 0 ? rounds[0].assistantMessageId : null;
});

const undoRestoreText = computed(() => {
  if (mode.value !== "current" || !undoTargetId.value) return null;
  return findUndoRestoreUserText(chatStore.messages, undoTargetId.value);
});

function sessionLabel(conflict: UndoConflictInfo): string {
  return conflict.sessionTitle?.trim() || conflict.sessionId;
}

function conflictFilesLabel(conflict: UndoConflictInfo): string {
  return conflict.changedFiles.map((file) => file.path).join(", ");
}

async function onUndoClick() {
  if (!undoTargetId.value) return;
  checkingUndoConflicts.value = true;
  try {
    undoConflicts.value = await chatStore.checkUndoConflicts(undoTargetId.value);
    if (undoConflicts.value.length > 0) {
      showUndoConflictConfirm.value = true;
      return;
    }
    showUndoConfirm.value = true;
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "undo",
    });
  } finally {
    checkingUndoConflicts.value = false;
  }
}

async function confirmUndo(force = false) {
  if (!undoTargetId.value) return;
  const restoreText = undoRestoreText.value;
  showUndoConfirm.value = false;
  showUndoConflictConfirm.value = false;
  changesStore.closeInlineDiff();
  const undone = await chatStore.performUndo(undoTargetId.value, { force });
  if (undone && restoreText) {
    uiStore.stageChatPrefill(restoreText);
  }
}

function cancelUndo() {
  showUndoConfirm.value = false;
  showUndoConflictConfirm.value = false;
  undoConflicts.value = [];
}

function onSelectInUnity(ev: MouseEvent, path: string) {
  ev.stopPropagation();
  selectUnityAsset(path);
}

function onOpenInEditor(ev: MouseEvent, path: string) {
  ev.stopPropagation();
  openFileExternal(path);
}
</script>

<template>
  <aside class="changes-panel" :class="{ embedded: props.embedded }">
    <div class="panel-header">
      <span class="panel-title">{{ t("chat.changes.title") }}</span>
      <div class="mode-tabs">
        <button
          class="mode-tab"
          :class="{ active: mode === 'current' }"
          @click="changesStore.setMode('current')"
        >
          {{ t("chat.changes.modeCurrent") }}
        </button>
        <button
          class="mode-tab"
          :class="{ active: mode === 'all' }"
          @click="changesStore.setMode('all')"
        >
          {{ t("chat.changes.modeAll") }}
        </button>
      </div>
      <button
        class="hide-meta-btn"
        :class="{ active: hideMeta }"
        @click="hideMeta = !hideMeta"
        :title="t('common.hideMeta')"
      >.meta</button>
      <button v-if="props.showClose ?? true" class="close-btn" @click="emit('close')" :title="t('todo.close')">&times;</button>
    </div>
    <div class="file-list">
      <div v-if="changesStore.currentLoading" class="empty-hint">{{ t("chat.changes.loading") }}</div>
      <div v-else-if="changesStore.currentError" class="empty-hint error">{{ changesStore.currentError }}</div>
      <div v-else-if="displayItems.length === 0" class="empty-hint">{{ t("chat.changes.empty") }}</div>
      <template v-else>
        <div
          v-for="item in displayItems"
          :key="item.key"
          class="file-item"
          @mouseenter="onItemMouseEnter($event, item)"
          @mouseleave="onItemMouseLeave"
          @click="onItemClick(item)"
        >
          <span class="file-status" :class="'status-' + item.fileChange.status.charAt(0).toLowerCase()">
            {{ item.fileChange.status }}
          </span>
          <span class="file-name">{{ fileName(item.fileChange.path) }}</span>
          <span class="file-dir">{{ dirPath(item.fileChange.path) }}</span>
          <span class="file-actions">
            <button
              v-if="projectStore.unityConnected"
              class="file-action-btn"
              :title="t('common.selectInUnity')"
              @click="onSelectInUnity($event, item.fileChange.path)"
            >
              <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M6.4 1L1 8l5.4 7h3.2L6.2 9.5H15v-3H6.2L9.6 1H6.4z"/></svg>
            </button>
            <button
              v-if="canOpenInEditor(item.fileChange.path)"
              class="file-action-btn"
              :title="t('common.openInEditor')"
              @click="onOpenInEditor($event, item.fileChange.path)"
            >
              <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M8 1C4.1 1 1 4.1 1 8s3.1 7 7 7 7-3.1 7-7-3.1-7-7-7zm0 12.5c-3 0-5.5-2.5-5.5-5.5S5 2.5 8 2.5s5.5 2.5 5.5 5.5-2.5 5.5-5.5 5.5zM6 5l6 3-6 3V5z"/></svg>
            </button>
          </span>
        </div>
      </template>
    </div>

    <!-- Undo footer -->
    <div v-if="displayItems.length > 0 && !chatStore.isStreaming" class="panel-footer">
      <button class="undo-btn" :disabled="checkingUndoConflicts" @click="onUndoClick">
        {{ mode === 'current' ? t('chat.changes.undoCurrent') : t('chat.changes.undoAll') }}
      </button>
    </div>

    <!-- Undo confirm dialog -->
    <Transition name="confirm-fade">
      <div v-if="showUndoConfirm" class="confirm-backdrop" @click.self="cancelUndo">
        <div class="confirm-dialog">
          <p class="confirm-message">
            {{ mode === 'current' ? t('chat.changes.undoCurrentConfirm') : t('chat.changes.undoAllConfirm') }}
          </p>
          <div class="confirm-actions">
            <button class="confirm-cancel" @click="cancelUndo">{{ t('chat.changes.cancel') }}</button>
            <button class="confirm-ok" @click="confirmUndo()">{{ t('chat.changes.confirmOk') }}</button>
          </div>
        </div>
      </div>
    </Transition>

    <Transition name="confirm-fade">
      <div v-if="showUndoConflictConfirm" class="confirm-backdrop" @click.self="cancelUndo">
        <div class="confirm-dialog conflict-dialog">
          <p class="confirm-message">
            {{ t("chat.changes.undoConflictMessage") }}
          </p>
          <div class="conflict-list">
            <div v-for="conflict in undoConflicts" :key="`${conflict.sessionId}-${conflict.assistantMessageId}`" class="conflict-item">
              <div class="conflict-session">{{ sessionLabel(conflict) }}</div>
              <div class="conflict-files">{{ conflictFilesLabel(conflict) }}</div>
            </div>
          </div>
          <div class="confirm-actions">
            <button class="confirm-cancel" @click="cancelUndo">{{ t('chat.changes.cancel') }}</button>
            <button class="confirm-ok" @click="confirmUndo(true)">{{ t('chat.changes.undoConflictForce') }}</button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- Hover preview popover -->
    <FileDiffPopover
      v-if="showPopover && previewPayload && hoverAnchor"
      :payload="previewPayload"
      :anchor="hoverAnchor"
      @close="clearHover"
    />
  </aside>
</template>

<style scoped>
.changes-panel {
  width: 280px;
  min-width: 280px;
  height: 100%;
  background: var(--msg-assistant-bg);
  border-left: 1px solid var(--border-color);
  display: flex;
  flex-direction: column;
}

.changes-panel.embedded {
  width: auto;
  min-width: 0;
  background: transparent;
  border-left: none;
}

.panel-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border-color);
}

.panel-title {
  font-size: 14px;
  font-weight: 600;
  white-space: nowrap;
}

.mode-tabs {
  flex: 1;
  display: flex;
  gap: 2px;
  background: var(--input-bg);
  border-radius: 4px;
  padding: 2px;
}

.mode-tab {
  flex: 1;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: 11px;
  padding: 3px 6px;
  border-radius: 3px;
  cursor: pointer;
  white-space: nowrap;
}

.mode-tab.active {
  background: var(--bg-color);
  color: var(--text-color);
  font-weight: 500;
}

.hide-meta-btn {
  border: 1px solid var(--border-color);
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
  padding: 2px 8px;
  cursor: pointer;
  white-space: nowrap;
  text-decoration: none;
  transition: background 0.15s, border-color 0.15s, color 0.15s, text-decoration-color 0.15s;
}

.hide-meta-btn.active,
.hide-meta-btn.active:hover {
  text-decoration: line-through;
  text-decoration-color: var(--text-secondary);
}

.hide-meta-btn:hover {
  background: var(--hover-bg);
  border-color: var(--text-secondary);
  color: var(--text-color);
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

.file-list {
  flex: 1;
  overflow-y: auto;
  padding: 4px;
}

.file-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 12px;
}

.file-item:hover {
  background: var(--hover-bg);
}

.file-item:hover .file-actions {
  opacity: 1;
}

.file-status {
  flex-shrink: 0;
  font-size: 10px;
  font-weight: 600;
  width: 16px;
  text-align: center;
}

.status-m { color: #d69e2e; }
.status-a { color: #38a169; }
.status-d { color: #e53e3e; }
.status-r { color: #805ad5; }

.file-name {
  font-size: 12px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.file-dir {
  flex: 1;
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  direction: rtl;
  text-align: left;
}

.file-actions {
  display: flex;
  gap: 2px;
  flex-shrink: 0;
  opacity: 0;
  transition: opacity 0.1s;
}

.file-action-btn {
  width: 20px;
  height: 20px;
  border: none;
  border-radius: 3px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
}

.file-action-btn:hover {
  background: var(--active-bg, rgba(255, 255, 255, 0.1));
  color: var(--text-color);
}

.empty-hint {
  text-align: center;
  color: var(--text-secondary);
  font-size: 13px;
  padding: 24px 0;
}

.empty-hint.error {
  color: #e53e3e;
}

/* ── Undo footer ── */
.panel-footer {
  flex-shrink: 0;
  padding: 8px 12px;
  border-top: 1px solid var(--border-color);
}

.undo-btn {
  width: 100%;
  padding: 6px 0;
  border: 1px solid #e53e3e;
  border-radius: 4px;
  background: none;
  color: #e53e3e;
  font-size: 12px;
  cursor: pointer;
}

.undo-btn:disabled {
  opacity: 0.6;
  cursor: wait;
}

.undo-btn:hover {
  background: rgba(229, 62, 62, 0.1);
}

/* ── Confirm dialog ── */
.confirm-backdrop {
  position: fixed;
  inset: 0;
  z-index: 300;
  background: rgba(0, 0, 0, 0.35);
  display: flex;
  align-items: center;
  justify-content: center;
}

.confirm-dialog {
  background: var(--sidebar-bg);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  padding: 20px 24px;
  max-width: 360px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.2);
}

.conflict-dialog {
  max-width: 520px;
}

.confirm-message {
  margin: 0 0 16px;
  font-size: 13px;
  color: var(--text-color);
  line-height: 1.5;
}

.conflict-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-bottom: 16px;
  max-height: 220px;
  overflow-y: auto;
}

.conflict-item {
  padding: 10px 12px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--bg-color);
}

.conflict-session {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  margin-bottom: 4px;
}

.conflict-files {
  font-size: 11px;
  color: var(--text-secondary);
  line-height: 1.5;
  word-break: break-word;
  font-family: var(--font-mono-identifier);
}

.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.confirm-cancel,
.confirm-ok {
  padding: 5px 16px;
  border-radius: 4px;
  font-size: 12px;
  cursor: pointer;
  border: 1px solid var(--border-color);
}

.confirm-cancel {
  background: none;
  color: var(--text-color);
}

.confirm-cancel:hover {
  background: var(--hover-bg);
}

.confirm-ok {
  background: #e53e3e;
  color: #fff;
  border-color: #e53e3e;
}

.confirm-ok:hover {
  background: #c53030;
}

/* Transition */
.confirm-fade-enter-active,
.confirm-fade-leave-active {
  transition: opacity 0.15s ease;
}
.confirm-fade-enter-from,
.confirm-fade-leave-to {
  opacity: 0;
}
</style>
