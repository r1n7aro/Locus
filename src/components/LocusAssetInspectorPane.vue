<script setup lang="ts">
import { computed } from "vue";
import {
  locusAssetInspectorTargetPath,
  LOCUS_ASSET_INSPECTOR_WINDOW_TITLE,
  type LocusAssetInspectorWindowPayload,
} from "../services/locusAssetInspectorWindow";
import { t } from "../i18n";
import UnityObjectPreview from "./unity-preview/UnityObjectPreview.vue";
import type { UnityObjectPreviewInput } from "./unity-preview/unityObjectPreview";

const props = defineProps<{
  payload: LocusAssetInspectorWindowPayload;
}>();

const targetPath = computed(() => locusAssetInspectorTargetPath(props.payload));
const selectedName = computed(() => {
  const path = props.payload.kind === "sceneObject"
    ? props.payload.objectPath ?? ""
    : targetPath.value;
  const segments = path.split("/").filter(Boolean);
  return segments[segments.length - 1] || LOCUS_ASSET_INSPECTOR_WINDOW_TITLE;
});
const previewModel = computed<UnityObjectPreviewInput>(() => ({
  kind: props.payload.kind ?? "asset",
  path: targetPath.value,
  title: selectedName.value,
  writable: true,
  capabilities: {
    inspect: true,
    edit: true,
    preview: true,
    select: true,
    drag: true,
  },
}));
</script>

<template>
  <div class="locus-asset-inspector-pane">
    <div v-if="!targetPath" class="locus-asset-inspector-state">
      {{ t("asset.inspector.missingAsset") }}
    </div>
    <UnityObjectPreview
      v-else
      :key="`${previewModel.kind}:${targetPath}`"
      :model="previewModel"
      level="inspector"
      :auto-load-preview="true"
    />
  </div>
</template>

<style scoped>
.locus-asset-inspector-pane {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  overflow: hidden;
  background: var(--panel-bg);
  color: var(--text-color);
}

.locus-asset-inspector-pane :deep(.unity-object-preview.level-inspector) {
  flex: 1;
  border: 0;
  border-radius: 0;
}

.locus-asset-inspector-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  color: var(--text-secondary);
  font-size: 12px;
}
</style>
