<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { ChevronRight, Folder, FolderOpen } from "lucide";
import { t } from "../../i18n";
import type { SessionSummary } from "../../types";
import { formatShortcut, useKeyboardShortcuts } from "../../composables/useKeyboardShortcuts";
import { normalizeAppError } from "../../services/errors";
import { getLocusRuntime, type RuntimeUnsubscribe } from "../../services/locusRuntime";
import {
  checkViewOpenRequirements,
  normalizeViewError,
  viewRun,
  viewTree,
  type ViewFolderSummary,
  type ViewPackageSummary,
} from "../../services/view";
import { useNotificationStore } from "../../stores/notification";
import LucideIcon from "../icons/LucideIcon.vue";
import { resolveLocusViewIcon } from "../icons/locusViewIcons";

const MAX_RECENT_SESSIONS = 12;
const STORAGE_KEY_VIEW_EXPANDED = "locus:sessionPanelViewExpanded";
const VIEW_TREE_INDENT_BASE_PX = 8;
const VIEW_TREE_INDENT_STEP_PX = 18;

const props = defineProps<{
  sessions: SessionSummary[];
  activeSessionId: string | null;
  streamingSessionIds?: Set<string>;
  showExpandPanelButton?: boolean;
  workingDir?: string;
  showViews?: boolean;
}>();

interface ViewTreeNode {
  kind: "folder" | "view";
  key: string;
  label: string;
  relPath: string;
  children: ViewTreeNode[];
  folder?: ViewFolderSummary;
  view?: ViewPackageSummary;
}

interface VisibleViewRow {
  node: ViewTreeNode;
  depth: number;
  expanded: boolean;
  hasChildren: boolean;
}

const emit = defineEmits<{
  selectSession: [id: string];
  newChat: [];
  expandPanel: [];
}>();

const open = ref(false);
const pickerRef = ref<HTMLElement | null>(null);
const { state: shortcutState } = useKeyboardShortcuts();
const notificationStore = useNotificationStore();
const viewSummaries = ref<ViewPackageSummary[]>([]);
const viewFolders = ref<ViewFolderSummary[]>([]);
const viewTreeOrder = ref<string[]>([]);
const viewsLoading = ref(false);
const viewOpeningKey = ref("");
const viewExpandedState = ref<Record<string, boolean>>(loadViewExpandedState());
let viewReloadUnsubscribe: RuntimeUnsubscribe | null = null;
let viewTreeChangedUnsubscribe: RuntimeUnsubscribe | null = null;

const hasWorkspace = computed(() => !!props.workingDir?.trim());
const showSessionViews = computed(() => props.showViews !== false);

const sortedSessions = computed(() =>
  [...props.sessions].sort((a, b) => b.updatedAt - a.updatedAt),
);

const recentSessions = computed(() => sortedSessions.value.slice(0, MAX_RECENT_SESSIONS));

const activeSession = computed(() =>
  props.activeSessionId
    ? props.sessions.find((session) => session.id === props.activeSessionId) ?? null
    : null,
);

const currentTitle = computed(() =>
  activeSession.value?.title || t("chat.session.newSession"),
);
const showNewButton = computed(() => props.activeSessionId !== null);
const newChatShortcutLabel = computed(() => formatShortcut(shortcutState.newChat));
const viewTreeNodes = computed(() =>
  buildViewTree(viewSummaries.value, viewFolders.value, viewTreeOrder.value),
);
const visibleViewRows = computed<VisibleViewRow[]>(() => {
  const rows: VisibleViewRow[] = [];
  const walk = (nodes: ViewTreeNode[], depth: number) => {
    for (const node of nodes) {
      const expanded = isViewNodeExpanded(node);
      const hasChildren = node.children.length > 0;
      rows.push({ node, depth, expanded, hasChildren });
      if (node.kind === "folder" && hasChildren && expanded) {
        walk(node.children, depth + 1);
      }
    }
  };
  walk(viewTreeNodes.value, 0);
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

function loadViewExpandedState(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY_VIEW_EXPANDED);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function persistViewExpandedState() {
  try {
    localStorage.setItem(STORAGE_KEY_VIEW_EXPANDED, JSON.stringify(viewExpandedState.value));
  } catch {
    // ignore persistence failures
  }
}

function normalizeViewPath(value: string): string {
  return value.replace(/\\/g, "/").replace(/\/+/g, "/").replace(/\/$/, "");
}

function physicalPackageRelPath(view: ViewPackageSummary): string {
  const explicitRelPath = view.packageRelPath?.trim();
  if (explicitRelPath) return normalizeViewPath(explicitRelPath);

  const packageRoot = normalizeViewPath(view.packageRoot);
  const workingDir = props.workingDir?.trim();
  if (workingDir) {
    const viewsRoot = normalizeViewPath(`${workingDir}/Locus/View`);
    const lowerPackageRoot = packageRoot.toLowerCase();
    const lowerViewsRoot = viewsRoot.toLowerCase();
    if (lowerPackageRoot.startsWith(`${lowerViewsRoot}/`)) {
      return packageRoot.slice(viewsRoot.length + 1);
    }
  }

  const parts = packageRoot.split("/").filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : view.id;
}

function viewDisplayPath(view: ViewPackageSummary): string {
  const explicitDisplayPath = view.displayPath?.trim();
  if (explicitDisplayPath) return normalizeViewPath(explicitDisplayPath);
  return physicalPackageRelPath(view);
}

function makeFolderNode(relPath: string, label: string, folder?: ViewFolderSummary): ViewTreeNode {
  return {
    kind: "folder",
    key: `view-dir:${relPath}`,
    label,
    relPath,
    children: [],
    folder,
  };
}

function buildViewTree(
  views: ViewPackageSummary[],
  folders: ViewFolderSummary[],
  order: string[],
): ViewTreeNode[] {
  const root = makeFolderNode("", "views");
  const folderMap = new Map<string, ViewTreeNode>([["", root]]);
  const orderMap = new Map(
    order.map((relPath, index) => [normalizeViewPath(relPath), index] as const),
  );
  const ensureFolder = (relPath: string, folder?: ViewFolderSummary) => {
    const normalized = normalizeViewPath(relPath);
    if (!normalized) return root;
    const parts = normalized.split("/").filter(Boolean);
    let parent = root;
    let currentPath = "";
    for (const part of parts) {
      currentPath = currentPath ? `${currentPath}/${part}` : part;
      let node = folderMap.get(currentPath);
      if (!node) {
        node = makeFolderNode(currentPath, part);
        folderMap.set(currentPath, node);
        parent.children.push(node);
      }
      if (folder && currentPath === normalized) {
        node.folder = folder;
        node.label = folder.name || part;
      }
      parent = node;
    }
    return parent;
  };

  const sortedFolders = [...folders].sort((left, right) =>
    normalizeViewPath(left.relPath).localeCompare(
      normalizeViewPath(right.relPath),
      undefined,
      { sensitivity: "base" },
    ),
  );
  for (const folder of sortedFolders) {
    ensureFolder(folder.relPath, folder);
  }

  const sortedViews = [...views].sort((left, right) =>
    viewDisplayPath(left).localeCompare(viewDisplayPath(right), undefined, { sensitivity: "base" })
      || left.name.localeCompare(right.name, undefined, { sensitivity: "base" })
      || left.id.localeCompare(right.id, undefined, { sensitivity: "base" }),
  );
  for (const view of sortedViews) {
    const relPath = viewDisplayPath(view);
    const parts = relPath.split("/").filter(Boolean);
    const parent = ensureFolder(parts.length > 1 ? parts.slice(0, -1).join("/") : "");
    parent.children.push({
      kind: "view",
      key: `view:${relPath}:${view.id}`,
      label: view.name || view.id,
      relPath,
      children: [],
      view,
    });
  }

  const sortChildren = (node: ViewTreeNode) => {
    node.children.sort((left, right) => {
      const leftOrder = orderMap.get(normalizeViewPath(left.relPath));
      const rightOrder = orderMap.get(normalizeViewPath(right.relPath));
      if (leftOrder !== undefined && rightOrder !== undefined) return leftOrder - rightOrder;
      if (leftOrder !== undefined) return -1;
      if (rightOrder !== undefined) return 1;
      if (left.kind !== right.kind) return left.kind === "folder" ? -1 : 1;
      return left.label.localeCompare(right.label, undefined, { sensitivity: "base" });
    });
    node.children.forEach(sortChildren);
  };
  sortChildren(root);
  return root.children;
}

function isViewNodeExpanded(node: ViewTreeNode): boolean {
  if (node.kind !== "folder") return false;
  const stored = viewExpandedState.value[node.key];
  return stored ?? true;
}

function setViewNodeExpanded(key: string, value: boolean) {
  viewExpandedState.value = { ...viewExpandedState.value, [key]: value };
  persistViewExpandedState();
}

function toggleViewRow(row: VisibleViewRow) {
  if (!row.hasChildren) return;
  setViewNodeExpanded(row.node.key, !row.expanded);
}

function viewTreeIndentPx(depth: number): number {
  return VIEW_TREE_INDENT_BASE_PX + Math.max(0, depth) * VIEW_TREE_INDENT_STEP_PX;
}

async function loadViews() {
  if (!showSessionViews.value || !hasWorkspace.value) {
    viewSummaries.value = [];
    viewFolders.value = [];
    viewTreeOrder.value = [];
    return;
  }
  viewsLoading.value = true;
  try {
    const snapshot = await viewTree();
    viewSummaries.value = snapshot.views;
    viewFolders.value = snapshot.folders;
    viewTreeOrder.value = snapshot.order ?? [];
  } catch (error) {
    const err = normalizeAppError(error);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadSessionCompactViews",
      replaceOperation: true,
      skipConsoleLog: true,
    });
  } finally {
    viewsLoading.value = false;
  }
}

async function openView(view: ViewPackageSummary) {
  if (viewOpeningKey.value) return;
  const key = `${viewDisplayPath(view)}:${view.id}`;
  viewOpeningKey.value = key;
  try {
    const requirementError = await checkViewOpenRequirements(view);
    if (requirementError) {
      notificationStore.addNotice("error", requirementError.message, {
        code: requirementError.code,
        operation: "openViewFromSessionCompactPicker",
        skipConsoleLog: true,
      });
      return;
    }
    open.value = false;
    await viewRun(view.id);
  } catch (error) {
    const err = normalizeViewError(error, { viewName: view.name });
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "openViewFromSessionCompactPicker",
      skipConsoleLog: true,
    });
  } finally {
    viewOpeningKey.value = "";
  }
}

function onViewRowClick(row: VisibleViewRow) {
  if (row.node.kind === "folder") {
    toggleViewRow(row);
    return;
  }
  if (row.node.view) {
    void openView(row.node.view);
  }
}

function toggle() {
  open.value = !open.value;
}

function selectSession(id: string) {
  emit("selectSession", id);
  open.value = false;
}

function newChat() {
  emit("newChat");
  open.value = false;
}

function onClickOutside(event: MouseEvent) {
  if (pickerRef.value && !pickerRef.value.contains(event.target as Node)) {
    open.value = false;
  }
}

watch(
  () => [props.workingDir, showSessionViews.value] as const,
  () => {
    void loadViews();
  },
);

onMounted(async () => {
  document.addEventListener("click", onClickOutside);
  viewReloadUnsubscribe = await getLocusRuntime().subscribe<ViewPackageSummary>(
    "view-package-reloaded",
    () => {
      void loadViews();
    },
  );
  viewTreeChangedUnsubscribe = await getLocusRuntime().subscribe(
    "view-tree-changed",
    () => {
      void loadViews();
    },
  );
  void loadViews();
});

onUnmounted(() => {
  document.removeEventListener("click", onClickOutside);
  viewReloadUnsubscribe?.();
  viewReloadUnsubscribe = null;
  viewTreeChangedUnsubscribe?.();
  viewTreeChangedUnsubscribe = null;
});
</script>

<template>
  <div ref="pickerRef" class="session-compact-picker">
    <button
      v-if="props.showExpandPanelButton"
      type="button"
      class="session-compact-expand"
      :title="t('chat.session.expandList')"
      :aria-label="t('chat.session.expandList')"
      @click="emit('expandPanel')"
    >
      <svg viewBox="0 0 16 16" width="13" height="13" fill="currentColor" aria-hidden="true">
        <path d="M3 3.75A1.75 1.75 0 0 1 4.75 2h6.5A1.75 1.75 0 0 1 13 3.75v8.5A1.75 1.75 0 0 1 11.25 14h-6.5A1.75 1.75 0 0 1 3 12.25v-8.5Zm1.5 0v8.5c0 .14.11.25.25.25H6.5v-9H4.75a.25.25 0 0 0-.25.25Zm3.5-.25v9h3.25c.14 0 .25-.11.25-.25v-8.5a.25.25 0 0 0-.25-.25H8Z"/>
      </svg>
    </button>
    <button
      type="button"
      class="session-compact-trigger"
      :class="{ open }"
      :title="currentTitle"
      @click="toggle"
    >
      <span class="session-compact-title">{{ currentTitle }}</span>
      <svg class="session-compact-chevron" viewBox="0 0 16 16" fill="currentColor" width="10" height="10" aria-hidden="true">
        <path d="M4.427 5.427a.75.75 0 0 1 1.06-.013L8 7.867l2.513-2.453a.75.75 0 1 1 1.047 1.073l-3 2.927a.75.75 0 0 1-1.047 0l-3-2.927a.75.75 0 0 1-.013-1.06z"/>
      </svg>
    </button>
    <button
      v-if="showNewButton"
      type="button"
      class="session-compact-new"
      :title="t('chat.session.new')"
      @click="newChat"
    >
      +
    </button>

    <Transition name="session-compact-dropdown">
      <div v-if="open" class="session-compact-dropdown">
        <div class="session-compact-session-region">
          <button
            type="button"
            class="session-compact-option"
            :class="{ active: activeSessionId === null }"
            @click="newChat"
          >
            <span class="session-compact-option-plus" aria-hidden="true">+</span>
            <span class="session-compact-option-title">{{ t("chat.session.newSession") }}</span>
            <span class="session-compact-option-shortcut">{{ newChatShortcutLabel }}</span>
          </button>
          <div class="session-compact-divider"></div>
          <div v-if="recentSessions.length === 0" class="session-compact-empty">
            {{ t("chat.session.noSessions") }}
          </div>
          <template v-else>
            <button
              v-for="session in recentSessions"
              :key="session.id"
              type="button"
              class="session-compact-option"
              :class="{
                active: session.id === activeSessionId,
                running: streamingSessionIds?.has(session.id),
              }"
              @click="selectSession(session.id)"
            >
              <span class="session-compact-option-dot"></span>
              <span class="session-compact-option-title">{{ session.title || t("chat.session.newSession") }}</span>
              <span class="session-compact-option-time">{{ formatSessionTime(session.updatedAt) }}</span>
            </button>
          </template>
        </div>

        <template v-if="showSessionViews && hasWorkspace">
          <section class="session-compact-view-section" :aria-label="t('view.list.title')">
            <div class="session-compact-view-header">{{ t("view.list.title") }}</div>
            <div class="session-compact-view-list">
              <button
                v-for="row in visibleViewRows"
                :key="row.node.key"
                type="button"
                class="session-compact-view-row"
                :class="{
                  folder: row.node.kind === 'folder',
                  opening: row.node.kind === 'view' && row.node.view && viewOpeningKey === `${viewDisplayPath(row.node.view)}:${row.node.view.id}`,
                }"
                :style="{ paddingLeft: `${viewTreeIndentPx(row.depth)}px` }"
                :title="row.node.view?.packageRoot || row.node.folder?.packageRoot || row.node.label"
                :disabled="!!viewOpeningKey && row.node.kind === 'view'"
                @click="onViewRowClick(row)"
              >
                <span
                  v-if="row.node.kind === 'folder' && row.hasChildren"
                  class="session-compact-view-branch"
                  @click.stop="toggleViewRow(row)"
                >
                  <LucideIcon
                    class="session-compact-view-chevron"
                    :class="{ open: row.expanded }"
                    :icon="ChevronRight"
                    :size="10"
                    :stroke-width="2.4"
                  />
                </span>
                <span v-else class="session-compact-view-branch-spacer" aria-hidden="true"></span>
                <span
                  v-if="row.node.kind === 'folder'"
                  class="session-compact-view-icon folder"
                  :class="{ open: row.expanded }"
                  aria-hidden="true"
                >
                  <LucideIcon
                    :icon="row.expanded ? FolderOpen : Folder"
                    :size="13"
                    :stroke-width="2"
                  />
                </span>
                <span v-else class="session-compact-view-icon view" aria-hidden="true">
                  <LucideIcon
                    :icon="resolveLocusViewIcon(row.node.view?.icon)"
                    :size="13"
                    :stroke-width="2"
                  />
                </span>
                <span class="session-compact-view-label">{{ row.node.label }}</span>
              </button>
              <div v-if="viewsLoading" class="session-compact-view-empty">
                {{ t("common.loading") }}
              </div>
              <div v-else-if="!viewTreeNodes.length" class="session-compact-view-empty">
                {{ t("view.list.empty") }}
              </div>
            </div>
          </section>
        </template>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.session-compact-picker {
  position: relative;
  z-index: 6;
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
  min-height: 38px;
  padding: 6px 10px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--msg-assistant-bg) 82%, var(--bg-color) 18%);
}

.session-compact-trigger,
.session-compact-new,
.session-compact-expand,
.session-compact-option {
  font-family: inherit;
}

.session-compact-trigger {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  max-width: min(360px, calc(100vw - 72px));
  min-height: 26px;
  padding: 0 4px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  box-shadow: none;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.session-compact-trigger:hover,
.session-compact-trigger.open {
  background: var(--hover-bg);
  border-color: transparent;
  color: var(--text-color);
}

.session-compact-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 14px;
  font-weight: 600;
  color: var(--text-color);
}

.session-compact-chevron {
  flex-shrink: 0;
  opacity: 0.5;
  transition: transform 0.15s ease;
}

.session-compact-trigger.open .session-compact-chevron {
  transform: rotate(180deg);
}

.session-compact-new,
.session-compact-expand {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 18px;
  line-height: 1;
  cursor: pointer;
  box-shadow: none;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.session-compact-new:hover,
.session-compact-new:focus-visible,
.session-compact-expand:hover,
.session-compact-expand:focus-visible {
  background: var(--hover-bg);
  border-color: var(--border-strong);
  color: var(--text-color);
  outline: none;
}

.session-compact-expand svg {
  width: 15px;
  height: 15px;
}

.session-compact-trigger {
  order: 1;
}

.session-compact-new {
  order: 2;
}

.session-compact-expand {
  order: 3;
  margin-left: auto;
}

.session-compact-dropdown {
  position: absolute;
  left: 10px;
  top: calc(100% + 4px);
  width: min(360px, calc(100vw - 20px));
  max-height: min(360px, calc(100vh - 96px));
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding: 0;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--surface-elevated);
  box-shadow: 0 10px 28px rgba(15, 17, 21, 0.12);
}

:root[data-theme="dark"] .session-compact-dropdown {
  box-shadow: 0 14px 32px rgba(0, 0, 0, 0.34);
}

.session-compact-session-region {
  flex: 1 1 auto;
  min-height: 92px;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 4px;
}

.session-compact-option {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 30px;
  padding: 4px 8px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  text-align: left;
  cursor: pointer;
  box-shadow: none;
}

.session-compact-option:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.session-compact-option.active {
  background: var(--active-bg);
  border-color: color-mix(in srgb, var(--accent-color) 18%, transparent);
  color: var(--text-color);
}

.session-compact-option-dot {
  width: 5px;
  height: 5px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 38%, transparent);
  flex-shrink: 0;
}

.session-compact-option-plus {
  width: 8px;
  flex-shrink: 0;
  color: var(--text-secondary);
  font-size: 13px;
  font-weight: 600;
  line-height: 1;
  text-align: center;
}

.session-compact-option.running .session-compact-option-dot {
  width: 6px;
  height: 6px;
  background: var(--accent-color);
  animation: session-compact-pulse 1.2s ease-in-out infinite;
}

.session-compact-option-title {
  min-width: 0;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  font-weight: 500;
}

.session-compact-option-time {
  flex-shrink: 0;
  font-size: 11px;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}

.session-compact-option-shortcut {
  flex-shrink: 0;
  font-size: 11px;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}

.session-compact-empty {
  padding: 10px 8px;
  color: var(--text-secondary);
  font-size: 12px;
  text-align: center;
}

.session-compact-divider {
  height: 1px;
  margin: 4px 4px;
  background: var(--border-color);
}

.session-compact-view-section {
  flex: 0 0 142px;
  display: flex;
  flex-direction: column;
  width: 100%;
  min-width: 0;
  overflow: hidden;
  border-top: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--surface-elevated) 94%, var(--sidebar-bg) 6%);
}

.session-compact-view-header {
  flex: 0 0 auto;
  min-height: 24px;
  padding: 6px 12px 3px;
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.4px;
  text-transform: uppercase;
}

.session-compact-view-list {
  flex: 1 1 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 0 4px 6px;
}

.session-compact-view-row {
  width: 100%;
  min-width: 0;
  min-height: 28px;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 8px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  text-align: left;
  cursor: pointer;
  box-shadow: none;
}

.session-compact-view-row:hover {
  background: var(--hover-bg);
}

.session-compact-view-row:focus-visible {
  outline: none;
  border-color: color-mix(in srgb, var(--accent-color) 28%, var(--border-color));
}

.session-compact-view-row:disabled {
  cursor: progress;
  opacity: 0.76;
}

.session-compact-view-row.opening,
.session-compact-view-row.opening:hover {
  background: color-mix(in srgb, var(--active-bg) 74%, var(--surface-elevated) 26%);
}

.session-compact-view-row.folder .session-compact-view-label {
  font-weight: 600;
}

.session-compact-view-branch,
.session-compact-view-branch-spacer,
.session-compact-view-icon {
  width: 14px;
  min-width: 14px;
  height: 16px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.session-compact-view-branch {
  border-radius: 4px;
}

.session-compact-view-branch:hover {
  background: color-mix(in srgb, var(--hover-bg) 78%, transparent);
}

.session-compact-view-chevron {
  opacity: 0.58;
  transition: transform 0.15s ease, opacity 0.12s ease;
}

.session-compact-view-row:hover .session-compact-view-chevron {
  opacity: 0.9;
}

.session-compact-view-chevron.open {
  transform: rotate(90deg);
}

.session-compact-view-icon.folder {
  color: color-mix(in srgb, var(--accent-color) 38%, var(--text-secondary) 62%);
}

.session-compact-view-icon.folder.open {
  color: color-mix(in srgb, var(--accent-color) 54%, var(--text-secondary) 46%);
}

.session-compact-view-icon.view {
  color: color-mix(in srgb, var(--accent-color) 74%, var(--text-color) 26%);
}

.session-compact-view-row.opening .session-compact-view-icon.view,
.session-compact-view-row:hover .session-compact-view-icon.view {
  color: var(--accent-color);
}

.session-compact-view-label {
  min-width: 0;
  flex: 1 1 auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: currentColor;
  font-size: 12px;
  font-weight: 500;
  line-height: 1.35;
}

.session-compact-view-empty {
  padding: 8px;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.5;
}

.session-compact-dropdown-enter-active,
.session-compact-dropdown-leave-active {
  transition: opacity 0.12s ease, transform 0.12s ease;
}

.session-compact-dropdown-enter-from,
.session-compact-dropdown-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}

@keyframes session-compact-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.35; }
}
</style>
