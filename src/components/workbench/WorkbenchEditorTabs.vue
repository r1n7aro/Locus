<script setup lang="ts">
import { computed, ref } from "vue";
import {
  BookOpen, CopyX, File, Folder, FolderOpen, GitBranch, History, ListX, LocateFixed, MessageSquare,
  PanelLeftClose, PanelRightClose, X,
} from "lucide";
import { t } from "../../i18n";
import {
  isAnimatedSessionTreeStatus,
  sessionTreeStatusForSession,
} from "../chat/sessionTree";
import { useChatStore } from "../../stores/chat";
import { shouldShowWorkbenchTabStrip } from "../../stores/workbench";
import { isDocumentPreview } from "./workbenchDocumentPreview";
import { isExternalWorkspaceFileEditor } from "./workbenchExternalFile";
import { canOpenEditorInFileExplorer, canRevealEditorInWorkspace } from "./workbenchTabReveal";
import type { WorkbenchEditorGroup, WorkbenchEditorInput } from "../../types/workbench";
import LucideIcon from "../icons/LucideIcon.vue";
import { resolveLocusViewIcon } from "../icons/locusViewIcons";
import BaseContextMenu from "../ui/BaseContextMenu.vue";
import BaseTabStrip, {
  type BaseTabContextMenuPayload,
  type BaseTabStripItem,
} from "../ui/BaseTabStrip.vue";
import {
  workbenchTabCloseIds,
  type WorkbenchTabCloseScope,
} from "./workbenchTabClose";
import {
  WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE,
  type WorkbenchEditorTabInternalDragData,
} from "./workbenchDrag";

const props = defineProps<{
  windowId: string;
  group: WorkbenchEditorGroup;
  showSingleTab?: boolean;
  windowTitle?: boolean;
  dropActive?: boolean;
  dropIndex?: number;
  viewIcon?: (editor: WorkbenchEditorInput) => string | null | undefined;
  workspaceLabel?: (editor: WorkbenchEditorInput) => string | null | undefined;
  /** The window shows a workspace tree that tabs can be located in. */
  workspaceReveal?: boolean;
}>();

const emit = defineEmits<{
  (event: "activate", editorId: string): void;
  (event: "close", editorId: string): void;
  (event: "close-many", editorIds: string[]): void;
  (event: "pin", editorId: string): void;
  (event: "reveal-in-workspace", editorId: string): void;
  (event: "open-in-file-explorer", editorId: string): void;
  (event: "drag-externalize", tab: BaseTabStripItem): void;
  (event: "title-keydown", keyEvent: KeyboardEvent): void;
}>();

const visible = computed(() => shouldShowWorkbenchTabStrip(props.group, props.showSingleTab));
const chatStore = useChatStore();
const tabContextMenu = ref<{ x: number; y: number; editorId: string } | null>(null);

const runningSessionIds = computed(() => {
  const ids = new Set(chatStore.streamingSessionIds);
  for (const session of chatStore.sessions) {
    if (isAnimatedSessionTreeStatus(
      sessionTreeStatusForSession(session, chatStore.streamingSessionIds),
    )) {
      ids.add(session.id);
    }
  }
  return ids;
});

function editorIcon(editor: WorkbenchEditorInput) {
  switch (editor.resource.kind) {
    case "fileHistory":
      return History;
    case "session":
    case "newSession":
      return MessageSquare;
    case "knowledge":
    case "knowledgeRoot":
      return BookOpen;
    case "checkout":
    case "collaboration":
      return GitBranch;
    case "folder":
    case "localDirectory":
      return Folder;
    case "view":
      return resolveLocusViewIcon(props.viewIcon?.(editor));
    case "section":
      if (editor.resource.section === "knowledge") return BookOpen;
      if (editor.resource.section === "collab") return GitBranch;
      return File;
    default:
      return File;
  }
}

function showDirtyIndicator(editor: WorkbenchEditorInput): boolean {
  if (!editor.dirty) return false;
  const resource = editor.resource;
  // Knowledge tabs are titled with the file name; only CSV saves manually.
  if (resource.kind === "knowledge") return /\.csv$/i.test(editor.title);
  const path = resource.kind === "workspaceFile" || resource.kind === "asset"
    ? resource.path
    : resource.kind === "localFile" ? editor.sourcePath : null;
  // Markdown edits save automatically; keep the dirty state for close guards.
  return !path || !/\.(md|markdown)$/i.test(path);
}

const tabItems = computed<BaseTabStripItem[]>(() => props.group.tabs.map((editor) => ({
  id: editor.editorId,
  title: editor.title,
  detail: props.workspaceLabel?.(editor)
    ?? (isExternalWorkspaceFileEditor(editor) ? t("workbench.tabs.outsideWorkspace") : null),
  icon: editorIcon(editor),
  dirty: showDirtyIndicator(editor),
  preview: editor.preview && !editor.pinned,
  unavailable: editor.availability === "unavailable",
  running: editor.resource.kind === "session"
    && runningSessionIds.value.has(editor.resource.sessionId),
  closeable: !props.windowTitle,
})));

function editorDragData(tab: BaseTabStripItem): WorkbenchEditorTabInternalDragData {
  return {
    windowId: props.windowId,
    paneId: props.group.paneId,
    editorId: tab.id,
    title: tab.title,
  };
}

function editorDragSourceId(tab: BaseTabStripItem): string {
  return `workbench-editor:${props.windowId}:${props.group.paneId}:${tab.id}`;
}

function activateTab(editorId: string): void {
  const editor = props.group.tabs.find((candidate) => candidate.editorId === editorId);
  if (editor && isDocumentPreview(editor)) emit("pin", editorId);
  emit("activate", editorId);
}

function openTabContextMenu(payload: BaseTabContextMenuPayload): void {
  tabContextMenu.value = {
    x: payload.event.clientX,
    y: payload.event.clientY,
    editorId: payload.tab.id,
  };
}

function closeIds(scope: WorkbenchTabCloseScope): string[] {
  const editorId = tabContextMenu.value?.editorId;
  if (!editorId) return [];
  return workbenchTabCloseIds(
    props.group.tabs.map((editor) => editor.editorId),
    editorId,
    scope,
  );
}

function closeFromContextMenu(scope: WorkbenchTabCloseScope): void {
  const editorId = tabContextMenu.value?.editorId;
  const editorIds = closeIds(scope);
  tabContextMenu.value = null;
  if (editorIds.length === 0) return;
  if (scope === "others" && editorId) emit("activate", editorId);
  emit("close-many", editorIds);
}

const contextEditor = computed(() => {
  const editorId = tabContextMenu.value?.editorId;
  return editorId ? props.group.tabs.find((editor) => editor.editorId === editorId) ?? null : null;
});
const contextRevealInWorkspace = computed(() => (
  !!props.workspaceReveal && !!contextEditor.value && canRevealEditorInWorkspace(contextEditor.value)
));
const contextOpenInFileExplorer = computed(() => (
  !!contextEditor.value && canOpenEditorInFileExplorer(contextEditor.value)
));

function runContextEditorAction(event: "reveal-in-workspace" | "open-in-file-explorer"): void {
  const editorId = tabContextMenu.value?.editorId;
  tabContextMenu.value = null;
  if (!editorId) return;
  if (event === "reveal-in-workspace") emit("reveal-in-workspace", editorId);
  else emit("open-in-file-explorer", editorId);
}

</script>

<template>
  <template v-if="visible">
    <BaseTabStrip
      class="workbench-editor-tabs"
      :class="{ 'is-window-title': windowTitle }"
      :data-workbench-pane-id="group.paneId"
      :tabs="tabItems"
      :active-id="group.activeEditorId"
      :label="t('workbench.tabs')"
      :drag-type="WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE"
      :drag-data="editorDragData"
      :drag-source-id="editorDragSourceId"
      :allowed-operations="['move', 'copy']"
      :drag-externalize="(tab) => emit('drag-externalize', tab)"
      :cancel-drag-on-window-blur="false"
      :drop-active="dropActive"
      :drop-index="dropIndex"
      tab-id-attribute="data-workbench-tab-id"
      :pin-on-double-click="!windowTitle"
      @activate="activateTab"
      @close="emit('close', $event)"
      @pin="emit('pin', $event)"
      @tab-contextmenu="openTabContextMenu"
      @keydown.capture="windowTitle && emit('title-keydown', $event)"
    />
    <BaseContextMenu
      v-if="tabContextMenu"
      :x="tabContextMenu.x"
      :y="tabContextMenu.y"
      :min-width="156"
      :aria-label="t('workbench.tabs.menu')"
      @close="tabContextMenu = null"
    >
      <button type="button" role="menuitem" @click="closeFromContextMenu('current')">
        <LucideIcon :icon="X" :size="13" />
        {{ t("common.close") }}
      </button>
      <div class="base-context-menu-separator" role="separator" />
      <button
        type="button"
        role="menuitem"
        :disabled="closeIds('others').length === 0"
        @click="closeFromContextMenu('others')"
      >
        <LucideIcon :icon="CopyX" :size="13" />
        {{ t("workbench.tabs.closeOthers") }}
      </button>
      <button
        type="button"
        role="menuitem"
        :disabled="closeIds('left').length === 0"
        @click="closeFromContextMenu('left')"
      >
        <LucideIcon :icon="PanelLeftClose" :size="13" />
        {{ t("workbench.tabs.closeLeft") }}
      </button>
      <button
        type="button"
        role="menuitem"
        :disabled="closeIds('right').length === 0"
        @click="closeFromContextMenu('right')"
      >
        <LucideIcon :icon="PanelRightClose" :size="13" />
        {{ t("workbench.tabs.closeRight") }}
      </button>
      <button type="button" role="menuitem" @click="closeFromContextMenu('all')">
        <LucideIcon :icon="ListX" :size="13" />
        {{ t("workbench.tabs.closeAll") }}
      </button>
      <template v-if="contextRevealInWorkspace || contextOpenInFileExplorer">
        <div class="base-context-menu-separator" role="separator" />
        <button
          v-if="contextRevealInWorkspace"
          type="button"
          role="menuitem"
          @click="runContextEditorAction('reveal-in-workspace')"
        >
          <LucideIcon :icon="LocateFixed" :size="13" />
          {{ t("workbench.tabs.revealInWorkspace") }}
        </button>
        <button
          v-if="contextOpenInFileExplorer"
          type="button"
          role="menuitem"
          @click="runContextEditorAction('open-in-file-explorer')"
        >
          <LucideIcon :icon="FolderOpen" :size="13" />
          {{ t("workbench.tabs.openInFileBrowser") }}
        </button>
      </template>
    </BaseContextMenu>
  </template>
</template>

<style scoped>
.workbench-editor-tabs {
  width: 100%;
}

.workbench-editor-tabs.is-window-title {
  flex: 0 1 auto;
  width: auto;
  height: 100%;
  border: 0;
  background: transparent;
}

.is-window-title :deep(.base-tab-shell) {
  flex: 0 1 auto;
  min-width: 0;
  max-width: none;
  border: 0;
  background: transparent;
  color: var(--text-secondary);
}

.is-window-title :deep(.base-tab-shell.active::after) {
  display: none;
}

.is-window-title :deep(.base-tab) {
  padding: 0 10px;
  font-weight: 600;
}
</style>
