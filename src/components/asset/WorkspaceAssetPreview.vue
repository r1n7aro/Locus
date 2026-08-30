<script setup lang="ts">
import { computed } from "vue";
import { X } from "lucide";
import { t } from "../../i18n";
import type { AssetPreviewPayload } from "../../types";
import type { WorkspaceRef } from "../../services/project";
import LucideIcon from "../icons/LucideIcon.vue";
import UnityObjectPreview from "../unity-preview/UnityObjectPreview.vue";
import type {
  UnityObjectPreviewInput,
  UnityObjectPreviewSourceState,
} from "../unity-preview/unityObjectPreview";

const props = withDefaults(defineProps<{
  workspaceRef: WorkspaceRef | null;
  path: string;
  kind?: "asset" | "sceneObject";
  title?: string;
  payload?: AssetPreviewPayload | null;
  loading?: boolean;
  error?: string;
  focusLine?: number | null;
  autoLoadPreview?: boolean;
  writable?: boolean;
  draggable?: boolean;
  showClose?: boolean;
  showHeader?: boolean;
}>(), {
  kind: "asset",
  title: "",
  payload: null,
  loading: false,
  error: "",
  focusLine: null,
  autoLoadPreview: true,
  writable: true,
  draggable: true,
  showClose: false,
  showHeader: true,
});

const emit = defineEmits<{
  (event: "sourceChange", state: UnityObjectPreviewSourceState): void;
  (event: "close"): void;
}>();

const normalizedPath = computed(() => props.path.trim().replace(/\\/g, "/").replace(/\/+$/, ""));
const displayTitle = computed(() => {
  if (props.title.trim()) return props.title.trim();
  return normalizedPath.value.split("/").filter(Boolean).pop() || normalizedPath.value;
});
const model = computed<UnityObjectPreviewInput>(() => ({
  kind: props.kind,
  path: normalizedPath.value,
  title: displayTitle.value,
  writable: props.writable,
  previewPayload: props.payload ?? undefined,
  capabilities: {
    inspect: true,
    edit: props.writable,
    preview: true,
    select: true,
    drag: props.draggable,
  },
}));
</script>

<template>
  <div class="workspace-asset-preview">
    <button
      v-if="showClose"
      type="button"
      class="workspace-asset-preview-close"
      :title="t('asset.preview.close')"
      :aria-label="t('asset.preview.close')"
      @click="emit('close')"
    >
      <LucideIcon :icon="X" :size="14" :stroke-width="1.5" />
    </button>
    <UnityObjectPreview
      v-if="normalizedPath && workspaceRef"
      :key="`${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? 'current'}:${kind}:${normalizedPath}`"
      :model="model"
      :workspace-ref="workspaceRef"
      level="inspector"
      :loading="loading"
      :error="error"
      :focus-line="focusLine"
      :auto-load-preview="autoLoadPreview"
      :draggable="draggable"
      :collapsible="false"
      :show-header="showHeader"
      @source-change="emit('sourceChange', $event)"
    />
  </div>
</template>

<style scoped>
.workspace-asset-preview {
  position: relative;
  flex: 1 1 0;
  min-width: 0;
  min-height: 0;
  display: flex;
  overflow: hidden;
  background: var(--panel-bg);
  color: var(--text-color);
}

.workspace-asset-preview-close {
  position: absolute;
  top: 5px;
  right: 6px;
  z-index: 2;
  width: 26px;
  height: 26px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.workspace-asset-preview-close:hover,
.workspace-asset-preview-close:focus-visible {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
  outline: none;
}

.workspace-asset-preview :deep(.unity-object-preview.level-inspector) {
  flex: 1 1 0;
  border: 0;
  border-radius: 0;
}
</style>
