<script setup lang="ts">
import { defineAsyncComponent } from "vue";
import type { AssetBinaryMeta, BinaryPreview, UnityTexturePreviewMeta } from "../../types";

defineProps<{
  preview: BinaryPreview;
  compact?: boolean;
  diffKey: string;
  mode?: "diff" | "neutral";
  assetMeta?: AssetBinaryMeta;
  unityTextureMeta?: UnityTexturePreviewMeta;
}>();

const RasterPreview = defineAsyncComponent(
  () => import("./RasterBinaryPreview.vue"),
);
const FbxPreview = defineAsyncComponent(
  () => import("./FbxBinaryPreview.vue"),
);
</script>

<template>
  <RasterPreview
    v-if="preview.kind === 'image'"
    :preview="preview"
    preview-kind="image"
    :compact="compact"
    :diff-key="diffKey"
    :mode="mode"
    :asset-meta="assetMeta"
    :unity-texture-meta="unityTextureMeta"
  />
  <RasterPreview
    v-else-if="preview.kind === 'psd'"
    :preview="preview"
    preview-kind="psd"
    :compact="compact"
    :diff-key="diffKey"
    :mode="mode"
    :asset-meta="assetMeta"
    :unity-texture-meta="unityTextureMeta"
  />
  <FbxPreview
    v-else-if="preview.kind === 'model' && !compact"
    :preview="preview"
    :diff-key="diffKey"
    :mode="mode"
  />
  <div v-else-if="preview.kind === 'model' && compact" class="binary-compact-fallback">
    3D Model — open full view to preview
  </div>
</template>

<style scoped>
.binary-compact-fallback {
  padding: 12px 16px;
  text-align: center;
  color: var(--text-secondary);
  font-size: 12px;
}
</style>
