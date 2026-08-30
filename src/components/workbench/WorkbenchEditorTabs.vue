<script setup lang="ts">
import { computed } from "vue";
import { BookOpen, File, Folder, GitBranch, MessageSquare } from "lucide";
import { t } from "../../i18n";
import { shouldShowWorkbenchTabStrip } from "../../stores/workbench";
import type { WorkbenchEditorGroup, WorkbenchEditorInput } from "../../types/workbench";
import BaseTabStrip, { type BaseTabStripItem } from "../ui/BaseTabStrip.vue";
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
  (event: "pin", editorId: string): void;
}>();

const visible = computed(() => shouldShowWorkbenchTabStrip(props.group, props.showSingleTab));

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
</script>

<template>
  <BaseTabStrip
    v-if="visible"
    class="workbench-editor-tabs"
    :data-workbench-pane-id="group.paneId"
    :tabs="tabItems"
    :active-id="group.activeEditorId"
    :label="t('workbench.tabs')"
    :drag-type="WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE"
    :drag-data="editorDragData"
    :drag-source-id="editorDragSourceId"
    :drop-active="dropActive"
    :drop-index="dropIndex"
    tab-id-attribute="data-workbench-tab-id"
    pin-on-double-click
    @activate="emit('activate', $event)"
    @close="emit('close', $event)"
    @pin="emit('pin', $event)"
  />
</template>

<style scoped>
.workbench-editor-tabs {
  width: 100%;
}
</style>
