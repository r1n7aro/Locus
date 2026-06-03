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
    v-else-if="preview.kind === 'model'"
    :preview="preview"
    :diff-key="diffKey"
    :mode="mode"
    :compact="compact"
  />
</template>
