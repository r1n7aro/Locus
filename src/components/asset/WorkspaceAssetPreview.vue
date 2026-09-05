<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { X } from "lucide";
import { t } from "../../i18n";
import { useFileChangeRevalidation } from "../../composables/useFileChangeRevalidation";
import type { AssetPreviewPayload } from "../../types";
import type { WorkspaceRef } from "../../services/project";
import { workspaceFileRevision } from "../../services/workspaceExplorer";
import type { ProjectExplorerFileRevision } from "../../types/workbench";
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
  previewRevision?: string;
  autoLoadPreview?: boolean;
  writable?: boolean;
  draggable?: boolean;
  showClose?: boolean;
  showHeader?: boolean;
  active?: boolean;
}>(), {
  kind: "asset",
  title: "",
  payload: null,
  loading: false,
  error: "",
  focusLine: null,
  previewRevision: "",
  autoLoadPreview: true,
  writable: true,
  draggable: true,
  showClose: false,
  showHeader: true,
  active: true,
});

const observedRevision = ref<ProjectExplorerFileRevision | null>(null);
const revisionProbeFinished = ref(false);
const revisionChanged = ref(false);

const emit = defineEmits<{
  (event: "sourceChange", state: UnityObjectPreviewSourceState): void;
  (event: "close"): void;
}>();

const normalizedPath = computed(() => props.path.trim().replace(/\\/g, "/").replace(/\/+$/, ""));
const displayTitle = computed(() => {
  if (props.title.trim()) return props.title.trim();
  return normalizedPath.value.split("/").filter(Boolean).pop() || normalizedPath.value;
});
const revisionPath = computed(() => {
  if (props.kind !== "sceneObject") return normalizedPath.value;
  const marker = normalizedPath.value.toLocaleLowerCase().indexOf(".unity/");
  return marker >= 0 ? normalizedPath.value.slice(0, marker + ".unity".length) : normalizedPath.value;
});
const propRevision = computed<ProjectExplorerFileRevision | null>(() => (
  props.previewRevision
    ? {
        exists: true,
        size: 0,
        modifiedAtNanos: "",
        key: props.previewRevision,
      }
    : null
));
const effectiveRevision = computed(() => observedRevision.value ?? propRevision.value);
const revisionReady = computed(() => (
  !!effectiveRevision.value || revisionProbeFinished.value || !props.workspaceRef
));

const { checkNow: refreshIfChanged } = useFileChangeRevalidation({
  active: () => props.active !== false,
  currentRevision: () => effectiveRevision.value,
  probe: async () => {
    if (!props.workspaceRef) throw new Error("Workspace unavailable");
    return workspaceFileRevision(revisionPath.value, props.workspaceRef);
  },
  workspaceRef: () => props.workspaceRef,
  workspacePath: () => revisionPath.value,
  probeOnMount: !props.previewRevision,
  onBaseline: (revision) => {
    observedRevision.value = revision;
    revisionProbeFinished.value = true;
    revisionChanged.value = false;
  },
  onChanged: (revision) => {
    observedRevision.value = revision;
    revisionProbeFinished.value = true;
    revisionChanged.value = true;
  },
  onError: () => {
    revisionProbeFinished.value = true;
  },
});

watch(() => props.previewRevision, () => {
  observedRevision.value = null;
  revisionChanged.value = false;
});

watch(() => props.payload, (next, previous) => {
  if (next !== previous && next) revisionChanged.value = false;
});

watch(
  () => [
    props.workspaceRef?.checkoutId ?? "",
    props.workspaceRef?.expectedGeneration ?? null,
    revisionPath.value,
  ] as const,
  (_next, previous) => {
    if (!previous) return;
    observedRevision.value = null;
    revisionProbeFinished.value = false;
    revisionChanged.value = false;
    void refreshIfChanged("manual");
  },
);
const model = computed<UnityObjectPreviewInput>(() => ({
  kind: props.kind,
  path: normalizedPath.value,
  title: displayTitle.value,
  writable: props.writable,
  previewPayload: (revisionChanged.value ? null : props.payload) ?? undefined,
  capabilities: {
    inspect: true,
    edit: props.writable,
    preview: true,
    select: true,
    drag: props.draggable,
  },
}));

defineExpose({ refreshIfChanged });
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
    <div v-if="!revisionReady" class="workspace-asset-preview-state">
      {{ t("development.preview.loading") }}
    </div>
    <UnityObjectPreview
      v-else-if="normalizedPath && workspaceRef"
      :key="`${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? 'current'}:${kind}:${normalizedPath}:${effectiveRevision?.key || 'unverified'}`"
      :model="model"
      :workspace-ref="workspaceRef"
      level="inspector"
      :loading="loading"
      :error="error"
      :focus-line="focusLine"
      :preview-revision="effectiveRevision?.key || 'unverified'"
      :auto-load-preview="autoLoadPreview || revisionChanged"
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

.workspace-asset-preview-state {
  margin: auto;
  color: var(--text-secondary);
  font-size: 12px;
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
