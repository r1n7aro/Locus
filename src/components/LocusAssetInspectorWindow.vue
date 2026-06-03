<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { X } from "lucide";
import {
  getLocusAssetInspectorWindowPayload,
  LOCUS_ASSET_INSPECTOR_WINDOW_EVENT,
  LOCUS_ASSET_INSPECTOR_WINDOW_TITLE,
  type LocusAssetInspectorWindowPayload,
} from "../services/locusAssetInspectorWindow";
import { t } from "../i18n";
import LucideIcon from "./icons/LucideIcon.vue";
import UnityObjectPreview from "./unity-preview/UnityObjectPreview.vue";
import type {
  UnityObjectPreviewInput,
  UnityObjectPreviewSourceState,
} from "./unity-preview/unityObjectPreview";

const assetPath = ref("");
const inspectorSourceState = ref<UnityObjectPreviewSourceState>("disk");
let unlistenPayload: UnlistenFn | null = null;

const selectedName = computed(() => {
  const segments = assetPath.value.split("/").filter(Boolean);
  return segments[segments.length - 1] || LOCUS_ASSET_INSPECTOR_WINDOW_TITLE;
});
const previewModel = computed<UnityObjectPreviewInput>(() => ({
  kind: "asset",
  path: assetPath.value,
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
const inspectorSourceBadgeState = computed(() => {
  if (!assetPath.value) return "disk";
  return inspectorSourceState.value;
});
const inspectorSourceLabel = computed(() => {
  if (inspectorSourceBadgeState.value === "live") return t("asset.inspector.source.live");
  if (inspectorSourceBadgeState.value === "loading") return t("asset.inspector.source.loading");
  return t("asset.inspector.source.disk");
});
const inspectorSourceTitle = computed(() => {
  if (inspectorSourceBadgeState.value === "live") return t("asset.inspector.source.title.live");
  if (inspectorSourceBadgeState.value === "loading") return t("asset.inspector.source.title.loading");
  return t("asset.inspector.source.title.disk");
});

function normalizeAssetPath(value: string | null | undefined): string {
  return (value ?? "").trim().replace(/\\/g, "/").replace(/\/+$/, "");
}

function applyWindowPayload(next: LocusAssetInspectorWindowPayload) {
  assetPath.value = normalizeAssetPath(next.assetPath);
  inspectorSourceState.value = assetPath.value ? "loading" : "disk";
}

function handlePreviewSourceChange(state: UnityObjectPreviewSourceState) {
  inspectorSourceState.value = state;
}

async function closeWindow() {
  const currentWindow = getCurrentWindow();
  try {
    await currentWindow.close();
  } catch (error) {
    console.warn("[LocusAssetInspectorWindow] close failed:", error);
  }
  await currentWindow.destroy().catch(() => {});
}

onMounted(async () => {
  applyWindowPayload(getLocusAssetInspectorWindowPayload());
  unlistenPayload = await listen<LocusAssetInspectorWindowPayload>(
    LOCUS_ASSET_INSPECTOR_WINDOW_EVENT,
    (event) => applyWindowPayload(event.payload),
  );
});

onUnmounted(() => {
  unlistenPayload?.();
  unlistenPayload = null;
});
</script>

<template>
  <div class="locus-asset-inspector-window-root">
    <div class="locus-asset-inspector-titlebar">
      <div class="locus-asset-inspector-title">
        <span class="locus-asset-inspector-title-main">{{ t("asset.inspector.windowTitle") }}</span>
        <span class="locus-asset-inspector-title-path" :title="assetPath">{{ assetPath }}</span>
      </div>
      <span
        class="locus-asset-inspector-source"
        :class="`source-${inspectorSourceBadgeState}`"
        :title="inspectorSourceTitle"
      >
        {{ inspectorSourceLabel }}
      </span>
      <button
        type="button"
        class="locus-asset-inspector-close"
        data-window-no-drag
        :title="t('app.win.close')"
        @pointerdown.stop
        @click="closeWindow"
      >
        <LucideIcon :icon="X" :size="14" />
      </button>
    </div>

    <div class="locus-asset-inspector-preview">
      <div v-if="!assetPath" class="locus-asset-inspector-state">
        {{ t("asset.inspector.missingAsset") }}
      </div>
      <UnityObjectPreview
        v-else
        :key="assetPath"
        :model="previewModel"
        level="inspector"
        :auto-load-preview="true"
        @source-change="handlePreviewSourceChange"
      />
    </div>
  </div>
</template>

<style scoped>
.locus-asset-inspector-window-root {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--border-strong);
  background: var(--panel-bg);
  color: var(--text-color);
}

.locus-asset-inspector-titlebar {
  -webkit-app-region: drag;
  min-height: 38px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 10px 0 14px;
  border-bottom: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}

.locus-asset-inspector-title {
  min-width: 0;
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  gap: 8px;
}

.locus-asset-inspector-title-main {
  flex-shrink: 0;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
}

.locus-asset-inspector-title-path {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}

.locus-asset-inspector-close,
.locus-asset-inspector-source,
.locus-asset-inspector-close * {
  -webkit-app-region: no-drag;
}

.locus-asset-inspector-source {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  min-height: 20px;
  padding: 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--sidebar-bg) 28%);
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  line-height: 1;
}

.locus-asset-inspector-source.source-live {
  border-color: color-mix(in srgb, var(--status-good-fg) 34%, var(--border-color));
  background: color-mix(in srgb, var(--status-good-bg) 22%, var(--panel-bg));
  color: var(--status-good-fg);
}

.locus-asset-inspector-source.source-loading {
  color: var(--text-secondary);
}

.locus-asset-inspector-close {
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.locus-asset-inspector-close:hover,
.locus-asset-inspector-close:focus-visible {
  background: var(--hover-bg);
  border-color: var(--border-color);
  color: var(--text-color);
  outline: none;
}

.locus-asset-inspector-preview {
  flex: 1;
  min-height: 0;
  display: flex;
  overflow: hidden;
}

.locus-asset-inspector-preview :deep(.unity-object-preview.level-inspector) {
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
