<script setup lang="ts">
import { computed, ref, watch, onUnmounted } from "vue";
import { useChatStore } from "../stores/chat";
import { useChatChangesStore } from "../stores/chatChanges";
import { useProjectStore } from "../stores/project";
import { useUiStore } from "../stores/ui";
import { diffSingleFile, createRequestToken, isTokenStale } from "../services/diff";
import { normalizeAppError } from "../services/errors";
import { findUndoRestoreUserMessage } from "../services/chatUndo";
import { selectUnityAsset, openFileExternal } from "../services/unity";
import { openChatDiffReviewWindow } from "../services/chatDiffReviewWindow";
import { t } from "../i18n";
import { useNotificationStore } from "../stores/notification";
import FileDiffPopover from "./diff/FileDiffPopover.vue";
import type { ChangedFile, GitFileChange, FileDiffPayload, UndoConflictInfo } from "../types";
import type { ChatMergedFileItem } from "../services/chatChanges";
import { buildUserMessageDraft } from "../composables/chatMessageDraft";
import { useHideMeta, isMetaFile, canOpenInEditor } from "../composables/useHideMeta";
import { buildStagingTreeRows, collectStagingFolderPaths } from "./collab/stagingTree";
import type { StagingTreeRow } from "./collab/stagingTree";
import type { StagingViewMode } from "./collab/stagingLayout";
import { useDisplaySettings } from "../composables/useDisplaySettings";

const { hideMeta } = useHideMeta();
const projectStore = useProjectStore();
const notificationStore = useNotificationStore();
const CHAT_CHANGES_VIEW_MODE_STORAGE_KEY = "locus.chat.changesViewMode";

const emit = defineEmits<{ close: [] }>();
const props = defineProps<{
  embedded?: boolean;
  showClose?: boolean;
}>();

const chatStore = useChatStore();
const changesStore = useChatChangesStore();
const uiStore = useUiStore();
const { state: displaySettings } = useDisplaySettings();

const mode = computed(() => changesStore.currentMode);
const fileViewMode = ref<StagingViewMode>(readStoredChatChangesViewMode());
const collapsedFolders = ref(new Set<string>());

// Close any stale diff UI when session changes and invalidate in-flight click requests.
let clickSeq = 0;
watch(() => chatStore.activeSessionId, () => {
  clickSeq++;
  collapsedFolders.value = new Set();
  changesStore.closeInlineDiff();
});

// ── Hover preview state ──
const hoverAnchor = ref<HTMLElement | null>(null);
const showPopover = ref(false);
const previewPayload = ref<FileDiffPayload | null>(null);
const previewItem = ref<DisplayItem | null>(null);
let hoverTimer: ReturnType<typeof setTimeout> | null = null;
let hoverCloseTimer: ReturnType<typeof setTimeout> | null = null;
let hoverSeq = 0;
const HOVER_CLOSE_DELAY_MS = 140;

function clearHover() {
  if (hoverTimer) {
    clearTimeout(hoverTimer);
    hoverTimer = null;
  }
  if (hoverCloseTimer) {
    clearTimeout(hoverCloseTimer);
    hoverCloseTimer = null;
  }
  hoverSeq++;
  createRequestToken(); // bump to stale any in-flight
  showPopover.value = false;
  previewPayload.value = null;
  hoverAnchor.value = null;
  previewItem.value = null;
}

watch(() => displaySettings.fileChangePopoverEnabled, (enabled) => {
  if (!enabled) clearHover();
});

function cancelHoverClose() {
  if (hoverCloseTimer) {
    clearTimeout(hoverCloseTimer);
    hoverCloseTimer = null;
  }
}

function scheduleHoverClose() {
  if (hoverTimer) {
    clearTimeout(hoverTimer);
    hoverTimer = null;
  }
  cancelHoverClose();
  hoverCloseTimer = setTimeout(clearHover, HOVER_CLOSE_DELAY_MS);
}

onUnmounted(() => {
  if (hoverTimer) clearTimeout(hoverTimer);
  if (hoverCloseTimer) clearTimeout(hoverCloseTimer);
});

// ── Data ──

interface DisplayItem {
  key: string;
  fileChange: GitFileChange;
  assistantMessageId: string;
  roundCount?: number;
}

type DisplayTreeFile = GitFileChange & {
  displayItem: DisplayItem;
};

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

const treeFiles = computed<DisplayTreeFile[]>(() =>
  displayItems.value.map((item) => ({
    ...item.fileChange,
    displayItem: item,
  })),
);

const treeRows = computed<StagingTreeRow<DisplayTreeFile>[]>(() =>
  buildStagingTreeRows(treeFiles.value, collapsedFolders.value, (file) => file.displayItem.key),
);

watch(treeFiles, (files) => {
  pruneCollapsedFolders(collectStagingFolderPaths(files));
});

watch(fileViewMode, (nextMode) => {
  persistChatChangesViewMode(nextMode);
});

// ── Helpers ──

function readStoredChatChangesViewMode(): StagingViewMode {
  try {
    const raw = localStorage.getItem(CHAT_CHANGES_VIEW_MODE_STORAGE_KEY);
    if (raw === "tree") return "tree";
    if (raw === "list") return "list";
  } catch {
    // Use the default tree view when local storage is unavailable.
  }
  return "tree";
}

function persistChatChangesViewMode(nextMode: StagingViewMode) {
  try {
    localStorage.setItem(CHAT_CHANGES_VIEW_MODE_STORAGE_KEY, nextMode);
  } catch {
    // The current in-memory view mode still applies.
  }
}

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

function fileStatusLabel(status: string): string {
  switch (status) {
    case "M": return "M";
    case "A": return "A";
    case "?": return "A";
    case "D": return "D";
    case "R": return "R";
    default: return status;
  }
}

function fileStatusClass(status: string): string {
  switch (status) {
    case "M": return "status-modified";
    case "A": case "?": return "status-added";
    case "D": return "status-deleted";
    case "R": return "status-renamed";
    default: return "status-modified";
  }
}

function treeIndentPx(depth: number) {
  if (depth <= 0) return 10;
  return 10 + depth * 20;
}

function toggleFileViewMode(nextMode: StagingViewMode) {
  fileViewMode.value = nextMode;
}

function toggleTreeFolder(chainPaths: readonly string[], expanded: boolean) {
  const next = new Set(collapsedFolders.value);
  if (expanded) {
    const collapsedPath = chainPaths[chainPaths.length - 1];
    if (collapsedPath) next.add(collapsedPath);
  } else {
    for (const path of chainPaths) {
      next.delete(path);
    }
  }
  collapsedFolders.value = next;
}

function pruneCollapsedFolders(validPaths: Set<string>) {
  if (collapsedFolders.value.size === 0) return;
  const next = new Set([...collapsedFolders.value].filter((path) => validPaths.has(path)));
  if (next.size !== collapsedFolders.value.size) {
    collapsedFolders.value = next;
  }
}

// ── Hover ──

function onItemMouseEnter(ev: MouseEvent, item: DisplayItem) {
  if (!displaySettings.fileChangePopoverEnabled) return;
  const el = ev.currentTarget as HTMLElement;
  cancelHoverClose();
  if (showPopover.value && previewPayload.value && previewItem.value?.key === item.key) return;
  if (hoverTimer) {
    clearTimeout(hoverTimer);
    hoverTimer = null;
  }
  const seq = ++hoverSeq;
  hoverTimer = setTimeout(async () => {
    const token = createRequestToken();
    try {
      const payload = await diffSingleFile(buildRequest(item, "preview"));
      if (seq !== hoverSeq || isTokenStale(token)) return;
      previewPayload.value = payload;
      hoverAnchor.value = el;
      previewItem.value = item;
      showPopover.value = true;
    } catch { /* silently ignore */ }
  }, 150);
}

function onItemMouseLeave() {
  scheduleHoverClose();
}

function onPopoverMouseEnter() {
  cancelHoverClose();
}

function onPopoverMouseLeave() {
  scheduleHoverClose();
}

function onPopoverOpen() {
  const item = previewItem.value;
  if (!item) return;
  void onItemClick(item);
}

// ── Click → inline diff ──

async function onItemClick(item: DisplayItem) {
  clearHover();
  const request = buildRequest(item, "full");
  if (displaySettings.chatDiffReviewTarget === "window") {
    try {
      const opened = await openChatDiffReviewWindow({ request });
      if (opened) {
        changesStore.closeInlineDiff();
        return;
      }
    } catch (e) {
      const err = normalizeAppError(e);
      notificationStore.addNotice("error", err.message, {
        code: err.code,
        operation: "openChatDiffReviewWindow",
      });
      console.error("[ChatChangesPanel] failed to open diff review window:", e);
    }
  }
  const seq = ++clickSeq;
  changesStore.setInlineDiffLoading(true);
  try {
    const payload = await diffSingleFile(request);
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
const isUndoing = ref(false);
const undoConflicts = ref<UndoConflictInfo[]>([]);
const undoDirtyFiles = ref<ChangedFile[]>([]);
/** Whether the dirty preflight completed — only then may confirm skip the backend re-check. */
const undoDirtyChecked = ref(false);

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

const undoRestoreDraft = computed(() => {
  if (mode.value !== "current" || !undoTargetId.value) return null;
  const message = findUndoRestoreUserMessage(chatStore.messages, undoTargetId.value);
  return message ? buildUserMessageDraft(message) : null;
});

const undoButtonBusy = computed(() => checkingUndoConflicts.value || isUndoing.value);

const undoButtonLabel = computed(() => {
  if (isUndoing.value) return t("chat.changes.undoing");
  return mode.value === "current" ? t("chat.changes.undoCurrent") : t("chat.changes.undoAll");
});

function sessionLabel(conflict: UndoConflictInfo): string {
  return conflict.sessionTitle?.trim() || conflict.sessionId;
}

function conflictFilesLabel(conflict: UndoConflictInfo): string {
  return conflict.changedFiles.map((file) => file.path).join(", ");
}

async function onUndoClick() {
  if (!undoTargetId.value || undoButtonBusy.value) return;
  checkingUndoConflicts.value = true;
  try {
    const [conflicts, dirty] = await Promise.all([
      chatStore.checkUndoConflicts(undoTargetId.value),
      // Dirty preflight is advisory: on failure proceed without it and let
      // undo_perform re-check on the backend.
      chatStore.checkUndoDirty(undoTargetId.value).then(
        (files) => ({ files, checked: true }),
        (e) => {
          console.warn("[ChatChangesPanel] undo_check_dirty failed:", e);
          return { files: [] as ChangedFile[], checked: false };
        },
      ),
    ]);
    undoConflicts.value = conflicts;
    undoDirtyFiles.value = dirty.files;
    undoDirtyChecked.value = dirty.checked;
    if (conflicts.length > 0) {
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
  const targetId = undoTargetId.value;
  if (!targetId || isUndoing.value) return;
  const restoreDraft = undoRestoreDraft.value;
  // The dirty list was shown to the user (or verified empty) in this dialog;
  // only skip the backend re-check when the preflight actually completed.
  const acceptDirty = force || undoDirtyChecked.value;
  isUndoing.value = true;
  changesStore.closeInlineDiff();
  try {
    const undone = await chatStore.performUndo(targetId, { force, acceptDirty });
    if (undone && restoreDraft) {
      uiStore.stageChatDraftPrefill(restoreDraft);
    }
  } finally {
    isUndoing.value = false;
    showUndoConfirm.value = false;
    showUndoConflictConfirm.value = false;
    undoConflicts.value = [];
    undoDirtyFiles.value = [];
    undoDirtyChecked.value = false;
  }
}

function cancelUndo() {
  if (isUndoing.value) return;
  showUndoConfirm.value = false;
  showUndoConflictConfirm.value = false;
  undoConflicts.value = [];
  undoDirtyFiles.value = [];
  undoDirtyChecked.value = false;
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
          type="button"
          class="mode-tab"
          :class="{ active: mode === 'current' }"
          @click="changesStore.setMode('current')"
        >
          {{ t("chat.changes.modeCurrent") }}
        </button>
        <button
          type="button"
          class="mode-tab"
          :class="{ active: mode === 'all' }"
          @click="changesStore.setMode('all')"
        >
          {{ t("chat.changes.modeAll") }}
        </button>
      </div>
      <div class="panel-actions">
        <button
          type="button"
          class="view-toggle-btn"
          :class="{ active: fileViewMode === 'tree' }"
          :aria-pressed="fileViewMode === 'tree'"
          :title="fileViewMode === 'tree' ? t('collab.view.list') : t('collab.view.tree')"
          @click="toggleFileViewMode(fileViewMode === 'tree' ? 'list' : 'tree')"
        >
          <svg v-if="fileViewMode === 'tree'" viewBox="0 0 16 16" width="14" height="14" fill="currentColor" aria-hidden="true">
            <path d="M2.75 3a.75.75 0 0 0 0 1.5h10.5a.75.75 0 0 0 0-1.5H2.75zm0 4.25a.75.75 0 0 0 0 1.5h10.5a.75.75 0 0 0 0-1.5H2.75zm0 4.25a.75.75 0 0 0 0 1.5h10.5a.75.75 0 0 0 0-1.5H2.75z"/>
          </svg>
          <svg v-else viewBox="0 0 16 16" width="14" height="14" fill="none" aria-hidden="true">
            <path d="M3 3.5a1 1 0 1 1 2 0 1 1 0 0 1-2 0zm8.25 0a.75.75 0 0 0 0 1.5h2a.75.75 0 0 0 0-1.5h-2zM5 4.25h2.5v3H11a1.75 1.75 0 0 1 1.75 1.75v1.75h.5a.75.75 0 0 1 0 1.5h-2a.75.75 0 0 1 0-1.5h.5V9A.25.25 0 0 0 11 8.75H7.5v2A1.75 1.75 0 0 1 5.75 12.5h-.5a1 1 0 1 1 0-1.5h.5a.25.25 0 0 0 .25-.25v-6.5H5z" fill="currentColor"/>
          </svg>
        </button>
        <button
          type="button"
          class="hide-meta-btn"
          :class="{ active: hideMeta }"
          @click="hideMeta = !hideMeta"
          :title="t('common.hideMeta')"
        >.meta</button>
        <button v-if="props.showClose ?? true" type="button" class="close-btn" @click="emit('close')" :title="t('todo.close')">&times;</button>
      </div>
    </div>
    <div class="file-list" :class="{ 'changes-tree-list': fileViewMode === 'tree' }">
      <div v-if="changesStore.currentLoading" class="empty-hint">{{ t("chat.changes.loading") }}</div>
      <div v-else-if="changesStore.currentError" class="empty-hint error">{{ changesStore.currentError }}</div>
      <div v-else-if="displayItems.length === 0" class="empty-hint">{{ t("chat.changes.empty") }}</div>
      <template v-else-if="fileViewMode === 'tree'">
        <div
          v-for="row in treeRows"
          :key="row.key"
        >
          <div v-if="row.kind === 'folder'" class="changes-tree-row changes-tree-folder-row">
            <button
              type="button"
              class="changes-tree-folder-btn"
              :style="{ paddingLeft: `${treeIndentPx(row.depth)}px` }"
              :title="row.path"
              :aria-label="row.expanded ? t('merge.tree.toggleCollapse', row.name) : t('merge.tree.toggleExpand', row.name)"
              @click="toggleTreeFolder(row.chainPaths, row.expanded)"
            >
              <span class="changes-tree-branch" :class="{ open: row.expanded }" aria-hidden="true">
                <svg class="changes-tree-chevron" viewBox="0 0 16 16" width="10" height="10" fill="currentColor">
                  <path d="M6.22 3.22a.75.75 0 0 1 1.06 0l4.25 4.25a.75.75 0 0 1 0 1.06l-4.25 4.25a.75.75 0 0 1-1.06-1.06L9.94 8 6.22 4.28a.75.75 0 0 1 0-1.06z"/>
                </svg>
              </span>
              <span class="changes-tree-folder-icon" :class="{ open: row.expanded }" aria-hidden="true">
                <svg viewBox="0 0 16 16" width="13" height="13" fill="none">
                  <path
                    v-if="!row.expanded"
                    d="M2.25 4.5A1.25 1.25 0 0 1 3.5 3.25h2.1c.32 0 .62.13.84.36l.8.82c.14.15.34.23.55.23h4.71A1.25 1.25 0 0 1 13.75 5.9v5.6a1.25 1.25 0 0 1-1.25 1.25H3.5a1.25 1.25 0 0 1-1.25-1.25V4.5Z"
                    fill="currentColor"
                  />
                  <path
                    v-else
                    d="M2.5 4.5a1.25 1.25 0 0 1 1.25-1.25h1.9c.28 0 .55.11.74.31l.98.98c.2.2.46.31.74.31h4.14a1.25 1.25 0 0 1 1.25 1.25v5.1a1.25 1.25 0 0 1-1.25 1.25h-8.5A1.25 1.25 0 0 1 2.5 11.2V4.5Z"
                    stroke="currentColor"
                    stroke-width="1.2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  />
                </svg>
              </span>
              <span class="changes-tree-folder-name">{{ row.name }}</span>
            </button>
          </div>

          <div
            v-else
            class="file-row changes-file-row"
            @mouseenter="onItemMouseEnter($event, row.file.displayItem)"
            @mouseleave="onItemMouseLeave"
          >
            <button
              type="button"
              class="file-item changes-file-main changes-tree-file-main"
              :style="{ paddingLeft: `${treeIndentPx(row.depth)}px` }"
              :title="row.file.path"
              @click="onItemClick(row.file.displayItem)"
            >
              <span class="file-status" :class="fileStatusClass(row.file.status)">
                {{ fileStatusLabel(row.file.status) }}
              </span>
              <span class="file-name">{{ fileName(row.file.path) }}</span>
            </button>
            <span class="file-actions">
              <button
                v-if="projectStore.unityConnected"
                type="button"
                class="file-action-btn"
                :title="t('common.selectInUnity')"
                @click="onSelectInUnity($event, row.file.path)"
              >
                <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M6.4 1L1 8l5.4 7h3.2L6.2 9.5H15v-3H6.2L9.6 1H6.4z"/></svg>
              </button>
              <button
                v-if="canOpenInEditor(row.file.path)"
                type="button"
                class="file-action-btn"
                :title="t('common.openInEditor')"
                @click="onOpenInEditor($event, row.file.path)"
              >
                <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M8 1C4.1 1 1 4.1 1 8s3.1 7 7 7 7-3.1 7-7-3.1-7-7-7zm0 12.5c-3 0-5.5-2.5-5.5-5.5S5 2.5 8 2.5s5.5 2.5 5.5 5.5-2.5 5.5-5.5 5.5zM6 5l6 3-6 3V5z"/></svg>
              </button>
            </span>
          </div>
        </div>
      </template>
      <template v-else>
        <div
          v-for="item in displayItems"
          :key="item.key"
          class="file-row changes-file-row"
          @mouseenter="onItemMouseEnter($event, item)"
          @mouseleave="onItemMouseLeave"
        >
          <button
            type="button"
            class="file-item changes-file-main"
            :title="item.fileChange.path"
            @click="onItemClick(item)"
          >
            <span class="file-status" :class="fileStatusClass(item.fileChange.status)">
              {{ fileStatusLabel(item.fileChange.status) }}
            </span>
            <span class="file-name">{{ fileName(item.fileChange.path) }}</span>
            <span class="file-dir">{{ dirPath(item.fileChange.path) }}</span>
          </button>
          <span class="file-actions">
            <button
              v-if="projectStore.unityConnected"
              type="button"
              class="file-action-btn"
              :title="t('common.selectInUnity')"
              @click="onSelectInUnity($event, item.fileChange.path)"
            >
              <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M6.4 1L1 8l5.4 7h3.2L6.2 9.5H15v-3H6.2L9.6 1H6.4z"/></svg>
            </button>
            <button
              v-if="canOpenInEditor(item.fileChange.path)"
              type="button"
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
      <button type="button" class="undo-btn" :disabled="undoButtonBusy" @click="onUndoClick">
        {{ undoButtonLabel }}
      </button>
    </div>

    <!-- Undo confirm dialog -->
    <Transition name="confirm-fade">
      <div v-if="showUndoConfirm" class="confirm-backdrop" @click.self="cancelUndo">
        <div class="confirm-dialog">
          <p class="confirm-message">
            {{ mode === 'current' ? t('chat.changes.undoCurrentConfirm') : t('chat.changes.undoAllConfirm') }}
          </p>
          <div v-if="undoDirtyFiles.length > 0" class="dirty-warning">
            <p class="dirty-message">{{ t('chat.changes.undoDirtyMessage') }}</p>
            <div class="dirty-files">
              <div v-for="file in undoDirtyFiles" :key="file.path" class="dirty-file">{{ file.path }}</div>
            </div>
          </div>
          <div class="confirm-actions">
            <button type="button" class="confirm-cancel" :disabled="isUndoing" @click="cancelUndo">{{ t('chat.changes.cancel') }}</button>
            <button type="button" class="confirm-ok" :disabled="isUndoing" @click="confirmUndo()">
              {{ isUndoing ? t('chat.changes.undoing') : t('chat.changes.confirmOk') }}
            </button>
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
          <div v-if="undoDirtyFiles.length > 0" class="dirty-warning">
            <p class="dirty-message">{{ t('chat.changes.undoDirtyMessage') }}</p>
            <div class="dirty-files">
              <div v-for="file in undoDirtyFiles" :key="file.path" class="dirty-file">{{ file.path }}</div>
            </div>
          </div>
          <div class="confirm-actions">
            <button type="button" class="confirm-cancel" :disabled="isUndoing" @click="cancelUndo">{{ t('chat.changes.cancel') }}</button>
            <button type="button" class="confirm-ok" :disabled="isUndoing" @click="confirmUndo(true)">
              {{ isUndoing ? t('chat.changes.undoing') : t('chat.changes.undoConflictForce') }}
            </button>
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
      @enter="onPopoverMouseEnter"
      @leave="onPopoverMouseLeave"
      @open="onPopoverOpen"
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
  min-width: 0;
}

.panel-title {
  font-size: 14px;
  font-weight: 600;
  white-space: nowrap;
}

.mode-tabs {
  flex: 1;
  min-width: 0;
  display: flex;
  gap: 2px;
  background: var(--input-bg);
  border-radius: 4px;
  padding: 2px;
}

.mode-tab {
  flex: 1;
  min-width: 0;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: 11px;
  padding: 3px 6px;
  border-radius: 3px;
  cursor: pointer;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.mode-tab.active {
  background: var(--bg-color);
  color: var(--text-color);
  font-weight: 500;
}

.panel-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.view-toggle-btn {
  width: 24px;
  height: 24px;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  transition: background 0.15s, border-color 0.15s, color 0.15s;
}

.view-toggle-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.view-toggle-btn.active {
  background: var(--active-bg);
  color: var(--text-color);
  border-color: color-mix(in srgb, var(--accent-color) 24%, var(--border-color));
}

.view-toggle-btn:focus-visible,
.hide-meta-btn:focus-visible,
.close-btn:focus-visible,
.file-action-btn:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.hide-meta-btn {
  height: 24px;
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
  padding: 4px 0;
}

.changes-tree-list {
  padding: 4px 0;
}

.file-row {
  display: flex;
  align-items: center;
  min-width: 0;
  border-radius: 4px;
}

.file-row:hover {
  background: var(--hover-bg);
}

.file-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px 6px 12px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 12px;
  min-width: 0;
  border: none;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  overflow: hidden;
  flex: 1 1 auto;
}

.file-status {
  flex-shrink: 0;
  font-size: 10px;
  font-weight: 700;
  width: 16px;
  text-align: center;
  line-height: 1;
}

.status-modified {
  color: var(--git-status-modified);
}

.status-added {
  color: var(--git-status-added);
}

.status-deleted {
  color: var(--git-status-deleted);
}

.status-renamed {
  color: var(--git-status-renamed);
}

.file-name {
  font-size: 12px;
  font-family: var(--font-mono-identifier);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  color: var(--text-color);
}

.file-dir {
  flex: 1;
  font-size: 11px;
  font-family: var(--font-mono-identifier);
  color: var(--text-secondary);
  opacity: 0.55;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  direction: rtl;
  text-align: left;
}

.file-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
  opacity: 0;
  transition: opacity 0.1s;
  padding-right: 8px;
}

.file-row:hover .file-actions,
.file-actions:focus-within {
  opacity: 1;
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
  background: var(--active-bg);
  color: var(--text-color);
}

.changes-tree-row {
  display: flex;
  align-items: center;
  min-width: 0;
}

.changes-tree-folder-btn {
  width: 100%;
  min-height: 28px;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 12px;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  font: inherit;
  text-align: left;
}

.changes-tree-folder-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.changes-tree-folder-btn:focus-visible,
.file-item:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.changes-tree-branch,
.changes-tree-folder-icon {
  width: 14px;
  height: 14px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.changes-tree-branch {
  color: var(--text-secondary);
}

.changes-tree-chevron {
  transition: transform 0.15s ease;
}

.changes-tree-branch.open .changes-tree-chevron {
  transform: rotate(90deg);
}

.changes-tree-folder-icon {
  color: color-mix(in srgb, var(--text-secondary) 82%, var(--text-color));
}

.changes-tree-folder-icon.open {
  color: var(--text-color);
}

.changes-tree-folder-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-color);
  font-size: 12px;
  font-family: var(--font-mono-identifier);
}

.changes-tree-file-main {
  gap: 6px;
}

.empty-hint {
  text-align: center;
  color: var(--text-secondary);
  font-size: 13px;
  padding: 24px 0;
}

.empty-hint.error {
  color: var(--status-danger-fg);
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
  border: 1px solid var(--status-danger-fg);
  border-radius: 4px;
  background: none;
  color: var(--status-danger-fg);
  font-size: 12px;
  cursor: pointer;
}

.undo-btn:disabled {
  opacity: 0.6;
  cursor: wait;
}

.undo-btn:not(:disabled):hover {
  background: var(--status-danger-bg);
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

.dirty-warning {
  margin-bottom: 16px;
  padding: 10px 12px;
  border: 1px solid var(--warning-border, var(--border-color));
  border-radius: 6px;
  background: var(--warning-bg, var(--bg-color));
}

.dirty-message {
  margin: 0 0 6px;
  font-size: 12px;
  color: var(--warning-text, var(--text-color));
  line-height: 1.5;
}

.dirty-files {
  max-height: 140px;
  overflow-y: auto;
  font-size: 11px;
  color: var(--text-secondary);
  line-height: 1.6;
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

.confirm-ok {
  background: var(--status-danger-fg);
  color: var(--bg-color);
  border-color: var(--status-danger-fg);
}

.confirm-cancel:disabled,
.confirm-ok:disabled {
  opacity: 0.6;
  cursor: wait;
}

.confirm-cancel:not(:disabled):hover {
  background: var(--hover-bg);
}

.confirm-ok:not(:disabled):hover {
  filter: brightness(0.92);
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
