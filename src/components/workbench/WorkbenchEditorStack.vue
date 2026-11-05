<script setup lang="ts">
import { computed, provide, reactive, shallowRef, watch } from "vue";
import { t } from "../../i18n";
import type { WorkbenchEditorGroup, WorkbenchEditorInput } from "../../types/workbench";
import { isDocumentPreview } from "./workbenchDocumentPreview";
import { isWorkbenchDocumentPath } from "../../stores/workbench";
import { WORKSPACE_FILE_PREVIEW_CACHE, WorkspaceFilePreviewCache } from "./workspaceFilePreviewCache";
import { selectMountedEditorIds } from "./workbenchEditorMounting";

const props = withDefaults(defineProps<{
  group: WorkbenchEditorGroup;
  /** Evictable editors mount when first shown; at most this many stay mounted (zero: never unloaded). */
  mountLimit?: number;
  isEvictable?: (editor: WorkbenchEditorInput) => boolean;
  /** Evictable editors that must stay mounted regardless of the limit. */
  retainedEditorIds?: Set<string>;
}>(), {
  mountLimit: 0,
  isEvictable: undefined,
  retainedEditorIds: () => new Set<string>(),
});
const emit = defineEmits<{
  (event: "pin", editorId: string): void;
  /** Sent while the editor is still mounted, before the limit unmounts an open tab. */
  (event: "evict", editor: WorkbenchEditorInput): void;
}>();
provide(WORKSPACE_FILE_PREVIEW_CACHE, new WorkspaceFilePreviewCache());
const readyEditors = reactive(new Set<string>());
const presentedEditor = shallowRef<WorkbenchEditorInput | null>(null);
const requestedEditor = computed(() => props.group.tabs.find(
  (editor) => editor.editorId === props.group.activeEditorId,
) ?? null);

function waitsForContent(editor: WorkbenchEditorInput): boolean {
  const resource = editor.resource;
  return editor.availability !== "unavailable"
    && (resource.kind === "session" || resource.kind === "knowledge"
      || resource.kind === "workspaceFile"
      || (resource.kind === "localFile" && !!editor.sourcePath)
      || (resource.kind === "asset" && isWorkbenchDocumentPath(resource.path)));
}

watch(
  () => [requestedEditor.value, requestedEditor.value?.availability,
    requestedEditor.value ? readyEditors.has(requestedEditor.value.editorId) : false] as const,
  ([editor, , ready]) => {
    if (!editor || !waitsForContent(editor) || ready) presentedEditor.value = editor;
  },
  { immediate: true },
);

// Activation order of this group, most recent first. Synchronous so the mount
// selection below already sees the editor that was active before a switch.
const recentEditorIds = shallowRef<string[]>([]);
watch(() => props.group.activeEditorId, (editorId) => {
  if (!editorId) return;
  const open = new Set(props.group.tabs.map((editor) => editor.editorId));
  recentEditorIds.value = [
    editorId,
    ...recentEditorIds.value.filter((id) => id !== editorId && open.has(id)),
  ];
}, { immediate: true, flush: "sync" });

// Each group mounts at most `mountLimit` evictable editors (session tabs), so
// many open tabs do not keep their transcripts, subscriptions and polls alive.
// The previous selection is what is rendered, so a freed slot never remounts an
// unloaded tab in the background.
const mountedEditorIds = computed<Set<string> | null>((previous) => props.isEvictable
  ? selectMountedEditorIds({
    tabs: props.group.tabs,
    activeEditorId: props.group.activeEditorId,
    presentedEditorId: presentedEditor.value?.editorId ?? null,
    recentEditorIds: recentEditorIds.value,
    mountedEditorIds: previous,
    limit: props.mountLimit,
    isEvictable: props.isEvictable,
    retainedEditorIds: props.retainedEditorIds,
  })
  : null);

// Retain only the last visible surface while its replacement mounts and loads.
// This is separate from the persisted tabs: a removed preview must survive long
// enough to cover the load, without becoming an unbounded KeepAlive cache.
const renderedEditors = computed(() => {
  const mounted = mountedEditorIds.value;
  const tabs = mounted
    ? props.group.tabs.filter((editor) => mounted.has(editor.editorId))
    : props.group.tabs;
  const previous = presentedEditor.value;
  return previous && !props.group.tabs.some((editor) => editor.editorId === previous.editorId)
    ? [...tabs, previous]
    : tabs;
});
const pending = computed(() => requestedEditor.value !== null
  && requestedEditor.value.editorId !== presentedEditor.value?.editorId);

// A pre-flush watcher runs before this stack re-renders, so an evicted editor is
// still mounted when the host captures its work state.
watch(
  () => renderedEditors.value.map((editor) => editor.editorId),
  (ids, previousIds) => {
    const retained = new Set(ids);
    for (const id of readyEditors) if (!retained.has(id)) readyEditors.delete(id);
    for (const id of previousIds ?? []) {
      if (retained.has(id)) continue;
      const editor = props.group.tabs.find((candidate) => candidate.editorId === id);
      if (editor) emit("evict", editor);
    }
  },
);

function markReady(editorId: string): void {
  // A superseded load may finish after its editor has already been removed.
  if (props.group.tabs.some((editor) => editor.editorId === editorId)) readyEditors.add(editorId);
}

function pinDocumentPreview(editor: WorkbenchEditorInput): void {
  if (!pending.value && props.group.activeEditorId === editor.editorId && isDocumentPreview(editor)) {
    emit("pin", editor.editorId);
  }
}

function handleDocumentSelection(editor: WorkbenchEditorInput, event: KeyboardEvent): void {
  const selectAll = (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a";
  const extendSelection = event.shiftKey
    && ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"].includes(event.key);
  if (selectAll || extendSelection) pinDocumentPreview(editor);
}
</script>

<template>
  <div class="workbench-editor-stack" :aria-busy="pending">
    <div
      v-for="editor in renderedEditors"
      :key="editor.editorId"
      v-show="presentedEditor?.editorId === editor.editorId || group.activeEditorId === editor.editorId"
      class="workbench-editor-instance"
      :class="{ 'is-preparing': pending && group.activeEditorId === editor.editorId }"
      :data-editor-id="editor.editorId"
      :inert="pending || group.activeEditorId !== editor.editorId ? true : undefined"
      :aria-hidden="presentedEditor?.editorId !== editor.editorId"
      @pointerdown.capture="$event.button === 0 && pinDocumentPreview(editor)"
      @keydown.capture="handleDocumentSelection(editor, $event)"
    >
      <slot
        :editor="editor"
        :content-active="presentedEditor?.editorId === editor.editorId || group.activeEditorId === editor.editorId"
        :presented="presentedEditor?.editorId === editor.editorId"
        :interactive="!pending && group.activeEditorId === editor.editorId"
        :ready="() => markReady(editor.editorId)"
      />
    </div>
    <div v-if="pending && !presentedEditor" class="workbench-editor-loading" role="status">
      {{ t('common.loading') }}
    </div>
    <slot v-if="group.tabs.length === 0" name="empty" />
  </div>
</template>

<style scoped>
.workbench-editor-stack,
.workbench-editor-instance {
  display: flex;
  flex: 1 1 0;
  width: 100%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.workbench-editor-stack {
  position: relative;
}

.workbench-editor-instance > :deep(*) {
  flex: 1 1 0;
  min-width: 0;
  min-height: 0;
}

/* Keep the incoming surface measurable for transcript scroll restoration and
   CodeMirror layout, while the outgoing surface still supplies the pixels. */
.workbench-editor-instance.is-preparing {
  position: absolute;
  inset: 0;
  visibility: hidden;
}

.workbench-editor-loading {
  display: flex;
  flex: 1 1 0;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 12px;
}
</style>
