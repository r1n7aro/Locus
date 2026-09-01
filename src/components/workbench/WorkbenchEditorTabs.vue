<script setup lang="ts">
import { computed, ref } from "vue";
import { BookOpen, File, Folder, GitBranch, MessageSquare, PanelsTopLeft } from "lucide";
import { t } from "../../i18n";
import {
  isAnimatedSessionTreeStatus,
  sessionTreeStatusForSession,
} from "../chat/sessionTree";
import { useChatStore } from "../../stores/chat";
import { shouldShowWorkbenchTabStrip } from "../../stores/workbench";
import type { WorkbenchEditorGroup, WorkbenchEditorInput } from "../../types/workbench";
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
  dropActive?: boolean;
  dropIndex?: number;
}>();

const emit = defineEmits<{
  (event: "activate", editorId: string): void;
  (event: "close", editorId: string): void;
  (event: "close-many", editorIds: string[]): void;
  (event: "pin", editorId: string): void;
  (event: "drag-externalize", tab: BaseTabStripItem): void;
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
      return PanelsTopLeft;
    case "section":
      if (editor.resource.section === "knowledge") return BookOpen;
      if (editor.resource.section === "collab") return GitBranch;
      return File;
    default:
      return File;
  }
}

const tabItems = computed<BaseTabStripItem[]>(() => props.group.tabs.map((editor) => ({
  id: editor.editorId,
  title: editor.title,
  icon: editorIcon(editor),
  dirty: editor.dirty,
  preview: editor.preview && !editor.pinned,
  unavailable: editor.availability === "unavailable",
  running: editor.resource.kind === "session"
    && runningSessionIds.value.has(editor.resource.sessionId),
  closeable: true,
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
  const editorIds = closeIds(scope);
  tabContextMenu.value = null;
  if (editorIds.length > 0) emit("close-many", editorIds);
}

</script>

<template>
  <template v-if="visible">
    <BaseTabStrip
      class="workbench-editor-tabs"
      :data-workbench-pane-id="group.paneId"
      :tabs="tabItems"
      :active-id="group.activeEditorId"
      :label="t('workbench.tabs')"
      :drag-type="WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE"
      :drag-data="editorDragData"
      :drag-source-id="editorDragSourceId"
      :drag-externalize="(tab) => emit('drag-externalize', tab)"
      :cancel-drag-on-window-blur="false"
      :drop-active="dropActive"
      :drop-index="dropIndex"
      tab-id-attribute="data-workbench-tab-id"
      pin-on-double-click
      @activate="emit('activate', $event)"
      @close="emit('close', $event)"
      @pin="emit('pin', $event)"
      @tab-contextmenu="openTabContextMenu"
    />
    <BaseContextMenu
      v-if="tabContextMenu"
      :x="tabContextMenu.x"
      :y="tabContextMenu.y"
      :min-width="156"
      :aria-label="t('workbench.tabs.menu')"
      @close="tabContextMenu = null"
    >
      <button type="button" @click="closeFromContextMenu('current')">
        {{ t("common.close") }}
      </button>
      <div class="base-context-menu-separator" role="separator" />
      <button
        type="button"
        :disabled="closeIds('left').length === 0"
        @click="closeFromContextMenu('left')"
      >
        {{ t("workbench.tabs.closeLeft") }}
      </button>
      <button
        type="button"
        :disabled="closeIds('right').length === 0"
        @click="closeFromContextMenu('right')"
      >
        {{ t("workbench.tabs.closeRight") }}
      </button>
      <button type="button" @click="closeFromContextMenu('all')">
        {{ t("workbench.tabs.closeAll") }}
      </button>
    </BaseContextMenu>
  </template>
</template>

<style scoped>
.workbench-editor-tabs {
  width: 100%;
}
</style>
