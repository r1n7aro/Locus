<script setup lang="ts">
import type { SessionSummary, SaveRawContextRequest } from "../../types";
import type { SessionTreeNode, SessionTreeSessionNode } from "./sessionTree";
import { computed, ref, nextTick } from "vue";
import { t } from "../../i18n";
import { buildSessionTree, nodeContainsSession, nodeHasActiveDescendant } from "./sessionTree";
import BaseButton from "../ui/BaseButton.vue";
import { formatShortcut, useKeyboardShortcuts } from "../../composables/useKeyboardShortcuts";

interface VisibleTreeRow {
  node: SessionTreeNode;
  depth: number;
  expanded: boolean;
  hasChildren: boolean;
}

const STORAGE_KEY_EXPANDED = "locus:sessionPanelExpanded";

const props = defineProps<{
  sessions: SessionSummary[];
  activeSessionId: string | null;
  streamingSessionIds?: Set<string>;
  sessionPanelWidth: number;
}>();

const emit = defineEmits<{
  selectSession: [id: string];
  newChat: [];
  archiveSession: [id: string];
  deleteSession: [id: string];
  renameSession: [id: string, title: string];
  saveRawContext: [request: SaveRawContextRequest];
}>();

function loadExpandedState(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY_EXPANDED);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

const expandedState = ref<Record<string, boolean>>(loadExpandedState());

function persistExpandedState() {
  try {
    localStorage.setItem(STORAGE_KEY_EXPANDED, JSON.stringify(expandedState.value));
  } catch {
    // ignore persistence failures
  }
}

const { state: shortcutState } = useKeyboardShortcuts();
const newChatTitle = computed(() =>
  t("chat.session.newWithShortcut", formatShortcut(shortcutState.newChat)),
);

try {
  localStorage.removeItem("locus:sessionPanelPinned");
} catch {
  // ignore persistence failures
}

const sessionTree = computed(() => buildSessionTree({
  sessions: props.sessions,
  streamingSessionIds: props.streamingSessionIds,
}));

function isNodeExpanded(node: SessionTreeNode): boolean {
  const stored = expandedState.value[node.key];
  if (stored !== undefined) return stored;
  // Default: auto-expand if contains active session
  if (nodeContainsSession(node, props.activeSessionId) || nodeHasActiveDescendant(node)) {
    return true;
  }
  return false;
}

function setNodeExpanded(key: string, value: boolean) {
  expandedState.value = { ...expandedState.value, [key]: value };
  persistExpandedState();
}

function toggleNode(row: VisibleTreeRow) {
  setNodeExpanded(row.node.key, !row.expanded);
}

const visibleRows = computed<VisibleTreeRow[]>(() => {
  const rows: VisibleTreeRow[] = [];
  const walk = (nodes: SessionTreeNode[], depth: number) => {
    for (const node of nodes) {
      const expanded = isNodeExpanded(node);
      const hasChildren = node.children.length > 0;
      rows.push({ node, depth, expanded, hasChildren });
      if (hasChildren && expanded) {
        walk(node.children, depth + 1);
      }
    }
  };
  walk(sessionTree.value, 0);
  return rows;
});

function formatSessionTime(ts: number): string {
  const nowTs = Math.floor(Date.now() / 1000);
  const diff = Math.max(0, nowTs - ts);

  if (diff < 60) return t("common.timeJustNow");

  const units: Array<[number, string]> = [
    [60, "chat.session.time.minute"],
    [60 * 60, "chat.session.time.hour"],
    [60 * 60 * 24, "chat.session.time.day"],
    [60 * 60 * 24 * 7, "chat.session.time.week"],
    [60 * 60 * 24 * 30, "chat.session.time.month"],
    [60 * 60 * 24 * 365, "chat.session.time.year"],
  ];

  for (let i = units.length - 1; i >= 0; i--) {
    const [seconds, key] = units[i];
    if (diff >= seconds) {
      return t(key, Math.floor(diff / seconds));
    }
  }

  return t("common.timeJustNow");
}

function rowLabel(node: SessionTreeNode): string {
  if (node.kind === "folder") return node.label;
  return node.title || t("chat.session.newSession");
}

function isSubagentNode(node: SessionTreeNode): boolean {
  return node.kind === "session" && node.sessionType === "chat" && !!node.parentSessionId;
}

function isDevNode(node: SessionTreeNode): boolean {
  return node.kind === "session" && node.sessionType === "chat" && !node.parentSessionId;
}

function rowRoleClass(node: SessionTreeNode): string {
  if (node.kind === "folder") return "role-folder";
  if (isSubagentNode(node)) return "role-subagent";
  if (isDevNode(node)) return "role-dev";
  return `role-${node.sessionType}`;
}

function sessionStatusLabel(status: SessionTreeNode["status"]): string {
  if (!status) return "";
  return t(`chat.session.status.${status}`);
}

/* Multi-selection state (Ctrl/Cmd toggle, Shift range) */
const selectedIds = ref<Set<string>>(new Set());
const lastAnchorId = ref<string | null>(null);

function clearMultiSelection() {
  if (selectedIds.value.size > 0) {
    selectedIds.value = new Set();
  }
}

type SessionVisibleTreeRow = VisibleTreeRow & { node: SessionTreeSessionNode };

function isSelectableSessionRow(row: VisibleTreeRow): row is SessionVisibleTreeRow {
  return row.node.kind === "session" && row.node.selectable && !!row.node.sessionId;
}

function selectableSessionRows(): SessionVisibleTreeRow[] {
  return visibleRows.value.filter(isSelectableSessionRow);
}

function onRowClick(row: VisibleTreeRow, e: MouseEvent) {
  if (row.node.kind === "folder") {
    if (row.hasChildren) toggleNode(row);
    return;
  }
  if (!row.node.selectable || !row.node.sessionId) return;
  const id = row.node.sessionId;

  if (e.ctrlKey || e.metaKey) {
    const next = new Set(selectedIds.value);
    if (next.has(id)) {
      next.delete(id);
    } else {
      // When starting a multi-selection from single-selected state,
      // seed the set with the currently active session so it feels natural.
      if (next.size === 0 && props.activeSessionId) {
        next.add(props.activeSessionId);
      }
      next.add(id);
    }
    selectedIds.value = next;
    lastAnchorId.value = id;
    return;
  }

  if (e.shiftKey && lastAnchorId.value) {
    const rows = selectableSessionRows();
    const anchorIdx = rows.findIndex((r) => r.node.sessionId === lastAnchorId.value);
    const currIdx = rows.findIndex((r) => r.node.sessionId === id);
    if (anchorIdx >= 0 && currIdx >= 0) {
      const [lo, hi] = anchorIdx <= currIdx ? [anchorIdx, currIdx] : [currIdx, anchorIdx];
      const next = new Set<string>();
      for (let i = lo; i <= hi; i++) {
        const sid = rows[i].node.sessionId;
        if (sid) next.add(sid);
      }
      selectedIds.value = next;
      return;
    }
  }

  // Plain click — reset multi-selection and activate this session.
  clearMultiSelection();
  lastAnchorId.value = id;
  emit("selectSession", id);
}

/* Context menu */
const ctxMenu = ref<{
  x: number;
  y: number;
  session: SessionSummary;
  ids: string[]; // targets — may include the single right-clicked session or the whole selection
} | null>(null);

const DELETE_CONFIRM_WIDTH = 244;
const DELETE_CONFIRM_HEIGHT = 136;
const DELETE_CONFIRM_GAP = 8;

const deleteConfirm = ref<{
  x: number;
  y: number;
  ids: string[];
} | null>(null);

function onContextMenu(e: MouseEvent, session: SessionSummary) {
  e.preventDefault();
  e.stopPropagation();
  deleteConfirm.value = null;
  let ids: string[];
  if (selectedIds.value.size > 1 && selectedIds.value.has(session.id)) {
    // Right-click inside an existing multi-selection → act on the whole set.
    ids = Array.from(selectedIds.value);
  } else {
    // Right-click outside selection → reset and target this one session.
    clearMultiSelection();
    ids = [session.id];
  }
  ctxMenu.value = { x: e.clientX, y: e.clientY, session, ids };
}

function closeCtxMenu() {
  ctxMenu.value = null;
  deleteConfirm.value = null;
}

/* Inline rename */
const editingId = ref<string | null>(null);
const editingTitle = ref("");
const renameInput = ref<HTMLInputElement | null>(null);

function startRename(session: SessionSummary) {
  closeCtxMenu();
  editingId.value = session.id;
  editingTitle.value = session.title || "";
  nextTick(() => {
    renameInput.value?.focus();
    renameInput.value?.select();
  });
}

function commitRename() {
  if (editingId.value && editingTitle.value.trim()) {
    emit("renameSession", editingId.value, editingTitle.value.trim());
  }
  editingId.value = null;
  editingTitle.value = "";
}

function cancelRename() {
  editingId.value = null;
  editingTitle.value = "";
}

function performArchive(ids: string[]) {
  if (ids.length === 0) return;
  for (const id of ids) {
    emit("archiveSession", id);
  }
  clearMultiSelection();
  closeCtxMenu();
}

function requestArchive(ids: string[]) {
  performArchive(ids);
}

function performDelete(ids: string[]) {
  if (ids.length === 0) return;
  for (const id of ids) {
    emit("deleteSession", id);
  }
  clearMultiSelection();
  closeCtxMenu();
}

function deleteConfirmLabel(ids: string[]): string {
  return ids.length > 1
    ? t("chat.session.deleteMany", ids.length)
    : t("chat.session.delete");
}

function deleteConfirmMessage(ids: string[]): string {
  return ids.length > 1
    ? t("chat.session.deleteManyConfirm", ids.length)
    : t("chat.session.deleteConfirm");
}

function requestDelete(e: MouseEvent) {
  if (!ctxMenu.value) return;
  const anchor = e.currentTarget as HTMLElement | null;
  if (!anchor) return;
  const rect = anchor.getBoundingClientRect();
  const margin = 12;

  let x = rect.right + DELETE_CONFIRM_GAP;
  if (x + DELETE_CONFIRM_WIDTH > window.innerWidth - margin) {
    x = Math.max(margin, rect.left - DELETE_CONFIRM_WIDTH - DELETE_CONFIRM_GAP);
  }

  const maxY = Math.max(margin, window.innerHeight - DELETE_CONFIRM_HEIGHT - margin);
  const y = Math.min(Math.max(margin, rect.top - 10), maxY);

  deleteConfirm.value = {
    x,
    y,
    ids: [...ctxMenu.value.ids],
  };
}

function confirmDelete() {
  if (!deleteConfirm.value) return;
  performDelete(deleteConfirm.value.ids);
}

function ctxSaveContext(includeSystemPrompt: boolean) {
  if (ctxMenu.value) {
    emit("saveRawContext", {
      sessionId: ctxMenu.value.session.id,
      includeSystemPrompt,
    });
  }
  closeCtxMenu();
}

function ctxArchive() {
  if (ctxMenu.value) performArchive(ctxMenu.value.ids);
}
</script>

<template>
  <div class="session-panel" :style="{ width: sessionPanelWidth + 'px', minWidth: sessionPanelWidth + 'px' }">
    <div class="sp-header">
      <span class="sp-title">{{ t('chat.session.title') }}</span>
      <button class="sp-new-btn" @click="emit('newChat')" :title="newChatTitle">+</button>
    </div>

    <div class="sp-session-list">
      <div
        v-for="row in visibleRows"
        :key="row.node.key"
        class="sp-session-item sp-tree-row"
        :class="[
          rowRoleClass(row.node),
          {
            active: row.node.kind === 'session' && !!row.node.sessionId && (row.node.sessionId === activeSessionId || selectedIds.has(row.node.sessionId) || (ctxMenu && ctxMenu.ids.includes(row.node.sessionId))),
            streaming: row.node.status === 'running',
            folder: row.node.kind === 'folder',
            child: row.depth > 0,
            virtual: row.node.kind === 'session' && row.node.isVirtual,
            disabled: row.node.kind === 'session' && !row.node.selectable,
            expandable: row.hasChildren,
          },
        ]"
        :style="{ paddingLeft: `${6 + row.depth * 12}px` }"
        @click="onRowClick(row, $event)"
        @contextmenu="row.node.kind === 'session' && row.node.session ? onContextMenu($event, row.node.session) : undefined"
      >
        <button
          v-if="row.hasChildren"
          class="sp-expand-btn"
          :class="{
            open: row.expanded,
            'is-running': row.node.status === 'running',
          }"
          @click.stop="toggleNode(row)"
          :title="row.expanded ? t('chat.session.collapse') : t('chat.session.expand')"
        >
          <svg viewBox="0 0 12 12" width="10" height="10" fill="currentColor" aria-hidden="true">
            <path d="M4 2.5 8 6 4 9.5z" />
          </svg>
        </button>
        <span
          v-else
          class="sp-expand-spacer"
          :title="row.node.status ? sessionStatusLabel(row.node.status) : undefined"
        >
          <span
            class="sp-session-dot"
            :class="row.node.status ? `is-${row.node.status}` : ''"
            aria-hidden="true"
          ></span>
        </span>

        <div class="sp-session-info">
          <template v-if="row.node.kind === 'session' && editingId === row.node.sessionId">
            <input
              ref="renameInput"
              class="sp-rename-input"
              v-model="editingTitle"
              @click.stop
              @keydown.enter="commitRename"
              @keydown.escape="cancelRename"
              @blur="commitRename"
            />
          </template>
          <template v-else>
            <div class="sp-session-main">
              <span class="sp-session-title">{{ rowLabel(row.node) }}</span>
              <div class="sp-session-meta">
                <span
                  v-if="row.node.status && row.node.status !== 'running'"
                  class="sp-session-status"
                  :class="`is-${row.node.status}`"
                >
                  {{ sessionStatusLabel(row.node.status) }}
                </span>
                <span class="sp-session-time">{{ formatSessionTime(row.node.updatedAt) }}</span>
                <button
                  v-if="row.node.kind === 'session' && row.node.sessionId"
                  class="sp-row-archive-btn"
                  :title="t('chat.session.archive')"
                  @click.stop="requestArchive([row.node.sessionId])"
                >
                  <svg viewBox="0 0 16 16" width="13" height="13" fill="none" aria-hidden="true">
                    <path d="M3.75 4.5h8.5m-7.75 0v5.7c0 .43.35.8.78.8h5.44c.43 0 .78-.37.78-.8V4.5m-5.82 3.1h4.68M6 2.75h4c.28 0 .5.22.5.5v1.25h-5V3.25c0-.28.22-.5.5-.5Z" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" stroke-linejoin="round"/>
                  </svg>
                </button>
              </div>
            </div>
          </template>
        </div>
      </div>
      <div v-if="visibleRows.length === 0" class="sp-empty-hint">{{ t('chat.session.noSessions') }}</div>
    </div>

    <Teleport to="body">
      <div v-if="ctxMenu" class="sp-ctx-backdrop" @click="closeCtxMenu" @contextmenu.prevent="closeCtxMenu">
        <div class="sp-ctx-menu" :style="{ left: ctxMenu.x + 'px', top: ctxMenu.y + 'px' }" @click.stop>
          <template v-if="ctxMenu.ids.length <= 1">
            <div class="sp-ctx-item" @click="startRename(ctxMenu!.session)">{{ t('chat.session.rename') }}</div>
            <div class="sp-ctx-item" @click="ctxSaveContext(true)">{{ t('chat.saveContextWithSystemPrompt') }}</div>
            <div class="sp-ctx-item" @click="ctxSaveContext(false)">{{ t('chat.saveContextWithoutSystemPrompt') }}</div>
            <div class="sp-ctx-sep"></div>
            <div class="sp-ctx-item" @click="ctxArchive">{{ t('chat.session.archive') }}</div>
            <div class="sp-ctx-item danger" @click.stop="requestDelete">{{ t('chat.session.delete') }}</div>
          </template>
          <template v-else>
            <div class="sp-ctx-item" @click="ctxArchive">{{ t('chat.session.archiveMany', ctxMenu.ids.length) }}</div>
            <div class="sp-ctx-item danger" @click.stop="requestDelete">{{ t('chat.session.deleteMany', ctxMenu.ids.length) }}</div>
          </template>
        </div>
        <div
          v-if="deleteConfirm"
          class="sp-delete-confirm"
          :style="{ left: deleteConfirm.x + 'px', top: deleteConfirm.y + 'px' }"
          @click.stop
        >
          <div class="sp-delete-confirm-title">{{ deleteConfirmLabel(deleteConfirm.ids) }}</div>
          <div class="sp-delete-confirm-text">{{ deleteConfirmMessage(deleteConfirm.ids) }}</div>
          <div class="sp-delete-confirm-actions">
            <BaseButton class="sp-delete-confirm-btn" @click="deleteConfirm = null">
              {{ t('common.cancel') }}
            </BaseButton>
            <BaseButton class="sp-delete-confirm-btn" variant="danger" @click="confirmDelete">
              {{ t('common.confirm') }}
            </BaseButton>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.session-panel {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
  overflow: hidden;
}

.sp-session-list {
  flex: 1 1 0;
  min-height: 0;
  height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
}

.sp-tree-row.virtual {
  opacity: 0.86;
}

.sp-tree-row.disabled {
  cursor: default;
}

.sp-tree-row.child {
  position: relative;
}

.sp-expand-btn,
.sp-expand-spacer {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.sp-expand-btn {
  border: none;
  background: transparent;
  color: var(--text-secondary);
  border-radius: 3px;
  cursor: pointer;
  padding: 0;
  box-shadow: none;
  opacity: 0.5;
  margin-right: 2px;
}

.sp-tree-row:hover .sp-expand-btn {
  opacity: 1;
}

.sp-expand-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.sp-expand-btn.is-running {
  color: var(--accent-color);
  opacity: 0.92;
}

.sp-expand-btn svg {
  transition: transform 0.15s ease;
}

.sp-expand-btn.is-running svg {
  animation: sp-session-pulse 1.2s ease-in-out infinite;
}

.sp-expand-btn.open svg {
  transform: rotate(90deg);
}

.sp-expand-spacer {
  margin-right: 0;
}

.sp-session-main {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  width: 100%;
}

.sp-tree-row.folder .sp-session-title {
  font-weight: 600;
}

.sp-session-title {
  min-width: 0;
  flex: 1 1 auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sp-session-meta {
  margin-left: auto;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  position: relative;
  flex-shrink: 0;
}

.sp-session-time {
  font-size: 12px;
  color: var(--text-secondary);
  transition: opacity 0.12s ease;
}

.sp-row-archive-btn {
  position: absolute;
  right: 0;
  top: 50%;
  z-index: 2;
  width: 18px;
  height: 18px;
  min-width: 18px;
  padding: 0;
  border: 1px solid color-mix(in srgb, var(--border-color) 75%, transparent);
  border-radius: 4px;
  background: color-mix(in srgb, var(--sidebar-bg) 92%, var(--hover-bg));
  color: var(--text-secondary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  opacity: 0;
  pointer-events: none;
  box-shadow: none;
  transform: translateY(-50%) scale(0.92);
  transition: opacity 0.12s ease, transform 0.12s ease, background 0.12s ease, color 0.12s ease, border-color 0.12s ease;
}

.sp-tree-row:hover .sp-row-archive-btn,
.sp-row-archive-btn:focus-visible {
  opacity: 1;
  pointer-events: auto;
  transform: translateY(-50%) scale(1);
}

.sp-row-archive-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.sp-row-archive-btn:focus-visible {
  outline: none;
  border-color: color-mix(in srgb, var(--accent-color) 28%, var(--border-color));
  color: var(--text-color);
}

.sp-row-archive-btn svg {
  width: 12px;
  height: 12px;
}

.sp-tree-row:hover .sp-session-time {
  opacity: 0;
}

.sp-session-status {
  display: inline-flex;
  align-items: center;
  min-height: 18px;
  padding: 0 6px;
  border-radius: 4px;
  border: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 88%, var(--hover-bg));
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1;
  white-space: nowrap;
}

.sp-session-status.is-running {
  border-color: color-mix(in srgb, var(--accent-color) 26%, var(--border-color));
  background: color-mix(in srgb, var(--accent-color) 8%, transparent);
  color: var(--accent-color);
}

.sp-session-status.is-queued,
.sp-session-status.is-starting {
  border-color: color-mix(in srgb, var(--status-warn-border, var(--border-color)) 78%, var(--border-color));
  background: color-mix(in srgb, var(--status-warn-bg, var(--hover-bg)) 82%, transparent);
  color: var(--status-warn-fg, var(--text-color));
}

.sp-session-status.is-error {
  border-color: var(--status-danger-border);
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

@keyframes sp-session-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.35; }
}

.sp-session-dot {
  width: 4px;
  height: 4px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 36%, transparent);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--text-secondary) 20%, transparent);
  transition: opacity 0.12s ease;
}

.sp-session-dot.is-running {
  width: 6px;
  height: 6px;
  background: var(--accent-color);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-color) 28%, transparent);
  animation: sp-session-pulse 1.2s ease-in-out infinite;
}

.sp-session-dot.is-queued,
.sp-session-dot.is-starting {
  width: 6px;
  height: 6px;
  background: var(--status-warn-fg, var(--text-color));
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--status-warn-border, var(--border-color)) 58%, transparent);
}

.sp-session-dot.is-error {
  width: 6px;
  height: 6px;
  background: var(--status-danger-fg);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--status-danger-border) 60%, transparent);
}

.sp-ctx-backdrop {
  position: fixed;
  inset: 0;
  z-index: 9999;
}

.sp-ctx-menu {
  position: fixed;
  min-width: 120px;
  background: var(--sidebar-bg);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  padding: 4px 0;
  box-shadow: 0 4px 16px rgba(0,0,0,.15);
  z-index: 10000;
}

.sp-ctx-item {
  padding: 6px 16px;
  font-size: 13px;
  cursor: pointer;
  color: var(--text-color);
}

.sp-ctx-item:hover {
  background: var(--hover-bg);
}

.sp-ctx-item.danger {
  color: var(--status-danger-fg);
}

.sp-ctx-item.danger:hover {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.sp-ctx-sep {
  height: 1px;
  background: var(--border-color);
  margin: 4px 0;
}

.sp-delete-confirm {
  position: fixed;
  z-index: 10001;
  width: 244px;
  padding: 12px;
  border: 1px solid color-mix(in srgb, var(--status-danger-border) 72%, var(--border-color));
  border-radius: 10px;
  background: var(--sidebar-bg);
  box-shadow: 0 12px 28px rgba(0, 0, 0, 0.18);
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.sp-delete-confirm-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.sp-delete-confirm-text {
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-secondary);
}

.sp-delete-confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.sp-delete-confirm-btn {
  min-width: 68px;
}

.sp-rename-input {
  width: 100%;
  background: var(--input-bg);
  color: var(--text-color);
  border: 1px solid var(--accent-color);
  border-radius: 4px;
  padding: 2px 6px;
  font-size: 13px;
  outline: none;
}
</style>
