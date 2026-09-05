<script setup lang="ts">
import { computed, ref } from "vue";
import type { FileChangeProbeReason } from "../../composables/useFileChangeRevalidation";
import type { WorkspaceRef } from "../../services/project";
import type { WorkbenchEditorInput } from "../../types/workbench";
import WorkspaceAssetPreview from "../asset/WorkspaceAssetPreview.vue";

const props = defineProps<{
  editor: WorkbenchEditorInput;
  workspaceRef: WorkspaceRef | null;
  active?: boolean;
}>();

const assetPreview = ref<InstanceType<typeof WorkspaceAssetPreview> | null>(null);

const previewKind = computed<"asset" | "sceneObject">(() => (
  props.editor.resource.kind === "sceneObject" ? "sceneObject" : "asset"
));
const previewPath = computed(() => {
  const resource = props.editor.resource;
  if (resource.kind === "asset") return resource.path;
  if (resource.kind === "sceneObject") return `${resource.scenePath}/${resource.objectPath}`;
  return "";
});

async function refreshIfChanged(reason: FileChangeProbeReason = "manual"): Promise<void> {
  await assetPreview.value?.refreshIfChanged(reason);
}

defineExpose({ refreshIfChanged });
</script>

<template>
  <WorkspaceAssetPreview
    ref="assetPreview"
    :workspace-ref="workspaceRef"
    :kind="previewKind"
    :path="previewPath"
    :title="editor.title"
    :active="active !== false"
    :auto-load-preview="true"
    :show-header="false"
  />
</template>
