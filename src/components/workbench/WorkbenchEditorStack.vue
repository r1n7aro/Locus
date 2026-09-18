<script setup lang="ts">
import { computed, reactive, shallowRef, watch } from "vue";
import { t } from "../../i18n";
import type { WorkbenchEditorGroup, WorkbenchEditorInput } from "../../types/workbench";

const props = defineProps<{ group: WorkbenchEditorGroup }>();
const readyEditors = reactive(new Set<string>());
const presentedEditor = shallowRef<WorkbenchEditorInput | null>(null);
const requestedEditor = computed(() => props.group.tabs.find(
  (editor) => editor.editorId === props.group.activeEditorId,
) ?? null);

function waitsForContent(editor: WorkbenchEditorInput): boolean {
  return editor.availability !== "unavailable"
    && (editor.resource.kind === "session" || editor.resource.kind === "knowledge");
}

watch(
  () => [requestedEditor.value, requestedEditor.value?.availability,
    requestedEditor.value ? readyEditors.has(requestedEditor.value.editorId) : false] as const,
  ([editor, , ready]) => {
    if (!editor || !waitsForContent(editor) || ready) presentedEditor.value = editor;
  },
  { immediate: true },
);

// Retain only the last visible surface while its replacement mounts and loads.
// This is separate from the persisted tabs: a removed preview must survive long
// enough to cover the load, without becoming an unbounded KeepAlive cache.
const renderedEditors = computed(() => {
  const previous = presentedEditor.value;
  return previous && !props.group.tabs.some((editor) => editor.editorId === previous.editorId)
    ? [...props.group.tabs, previous]
    : props.group.tabs;
});
const pending = computed(() => requestedEditor.value !== null
  && requestedEditor.value.editorId !== presentedEditor.value?.editorId);

watch(
  () => renderedEditors.value.map((editor) => editor.editorId),
  (ids) => {
    const retained = new Set(ids);
    for (const id of readyEditors) if (!retained.has(id)) readyEditors.delete(id);
  },
);

function markReady(editorId: string): void {
  // A superseded load may finish after its editor has already been removed.
  if (props.group.tabs.some((editor) => editor.editorId === editorId)) readyEditors.add(editorId);
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
    >
      <slot
        :editor="editor"
        :content-active="presentedEditor?.editorId === editor.editorId || group.activeEditorId === editor.editorId"
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
