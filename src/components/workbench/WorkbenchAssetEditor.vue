<script setup lang="ts">
import { computed } from "vue";
import type { WorkspaceRef } from "../../services/project";
import type { WorkbenchEditorInput } from "../../types/workbench";
import WorkspaceAssetPreview from "../asset/WorkspaceAssetPreview.vue";

const props = defineProps<{
  editor: WorkbenchEditorInput;
  workspaceRef: WorkspaceRef | null;
}>();

const previewKind = computed<"asset" | "sceneObject">(() => (
  props.editor.resource.kind === "sceneObject" ? "sceneObject" : "asset"
));
const previewPath = computed(() => {
  const resource = props.editor.resource;
  if (resource.kind === "asset") return resource.path;
  if (resource.kind === "sceneObject") return `${resource.scenePath}/${resource.objectPath}`;
  return "";
});
</script>

<template>
  <WorkspaceAssetPreview
    :workspace-ref="workspaceRef"
    :kind="previewKind"
    :path="previewPath"
    :title="editor.title"
    :auto-load-preview="true"
    :show-header="false"
  />
</template>
