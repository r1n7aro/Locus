<script setup lang="ts">
import { defineAsyncComponent, computed, ref } from "vue";
import { t } from "../../i18n";
import { useProjectStore } from "../../stores/project";
import { selectUnityAsset } from "../../services/unity";
import { isUnityConnectionError } from "../../services/errors";
import type { AssetPreviewPayload, SemanticTargetInspector, SemanticDisplayMode } from "../../types";
import BaseButton from "../ui/BaseButton.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";

const props = withDefaults(defineProps<{
  payload: AssetPreviewPayload | null;
  loading: boolean;
  error: string;
  selectedName: string;
  selectedPath: string;
  activeTargetId: string | null;
  targetCache: Map<string, SemanticTargetInspector>;
  targetLoading: boolean;
  loadTarget: (previewKey: string, targetId: string) => Promise<SemanticTargetInspector | null>;
  showClose?: boolean;
}>(), {
  showClose: true,
});

const emit = defineEmits<{
  (e: "close"): void;
}>();

const AssetTextViewer = defineAsyncComponent(() => import("./AssetTextViewer.vue"));
const AssetBinaryInfoCard = defineAsyncComponent(() => import("./AssetBinaryInfoCard.vue"));
const BinaryPreviewHost = defineAsyncComponent(
  () => import("../diff/BinaryPreviewHost.vue"),
);
const UnityHierarchyPane = defineAsyncComponent(
  () => import("../diff/UnityHierarchyPane.vue"),
);
const UnityInspectorPane = defineAsyncComponent(
  () => import("../diff/UnityInspectorPane.vue"),
);

const projectStore = useProjectStore();
const displayMode = ref<SemanticDisplayMode>("optimized");
const displayErrorRequiresUnity = computed(() => isUnityConnectionError(props.error));
const displayError = computed(() => {
  if (!props.error) return "";
  return displayErrorRequiresUnity.value ? t("asset.preview.unityConnectionRequired") : props.error;
});

const activeInspector = computed<SemanticTargetInspector | null>(() => {
  if (!props.activeTargetId) return null;
  return props.targetCache.get(props.activeTargetId) ?? null;
});

const canSelectInUnity = computed(() =>
  projectStore.unityConnected &&
  /^(Assets|Packages)\//.test(props.selectedPath),
);
const displayModeOptions = computed(() => [
  { value: "optimized", label: t("diff.mode.optimized") },
  { value: "full", label: t("diff.mode.full") },
]);

function onSelectInUnity() {
  selectUnityAsset(props.selectedPath).catch((err: unknown) => {
    console.warn("[AssetPreviewHost] selectUnityAsset failed:", err);
  });
}

async function onTreeSelect(targetId: string) {
  if (!props.payload || props.payload.kind !== "structured") return;
  await props.loadTarget(props.payload.previewKey, targetId);
}
</script>

<template>
  <div class="aph-root">
    <div class="aph-header">
      <span class="aph-name">{{ selectedName }}</span>
      <span class="aph-path">{{ selectedPath }}</span>
      <span class="aph-spacer" />
      <BaseButton
        v-if="canSelectInUnity"
        class="aph-header-btn"
        :title="t('common.selectInUnity')"
        @click="onSelectInUnity"
      >
        <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor" aria-hidden="true">
          <path d="M6.4 1L1 8l5.4 7h3.2L6.2 9.5H15v-3H6.2L9.6 1H6.4z" />
        </svg>
        <span>{{ t("common.selectInUnity") }}</span>
      </BaseButton>
      <template v-if="payload?.kind === 'structured'">
        <BaseSegmented
          v-model="displayMode"
          class="aph-mode-segmented"
          size="sm"
          :options="displayModeOptions"
        />
      </template>
      <button
        v-if="showClose"
        class="aph-close"
        :title="t('asset.preview.close')"
        @click="emit('close')"
      >×</button>
    </div>

    <div class="aph-body">
      <div v-if="loading && !payload" class="aph-state">{{ t("asset.preview.loading") }}</div>
      <div
        v-else-if="displayError && !payload"
        class="aph-state"
        :class="{ 'aph-error': !displayErrorRequiresUnity }"
      >
        {{ displayError }}
      </div>

      <template v-else-if="payload">
        <!-- text -->
        <AssetTextViewer
          v-if="payload.kind === 'text'"
          :snippet="payload.snippet"
          :truncated="payload.truncated"
          :total-lines="payload.totalLines"
          :language="payload.language"
        />

        <!-- binaryPreview: image / psd / model -->
        <div v-else-if="payload.kind === 'binaryPreview'" class="aph-binary-wrap">
          <BinaryPreviewHost
            :preview="payload.preview"
            :diff-key="`asset:${selectedPath}`"
            mode="neutral"
            :asset-meta="payload.meta"
            :unity-texture-meta="payload.meta.unityTexture"
          />
        </div>

        <!-- binaryInfo -->
        <AssetBinaryInfoCard
          v-else-if="payload.kind === 'binaryInfo'"
          :meta="payload.meta"
        />

        <!-- structured: scene/prefab or YAML asset -->
        <div v-else-if="payload.kind === 'structured'" class="aph-structured">
          <div class="aph-hierarchy">
            <UnityHierarchyPane
              :nodes="payload.tree"
              :selected-id="activeTargetId"
              :show-collapse-all="true"
              :auto-collapse-when-overflow="payload.layout === 'sceneHierarchyInspector'"
              @select="onTreeSelect"
            />
          </div>
          <div class="aph-inspector">
            <div v-if="!activeTargetId" class="aph-placeholder">
              {{ t("asset.preview.selectObject") }}
            </div>
            <UnityInspectorPane
              v-else
              :inspector="activeInspector"
              :loading="targetLoading"
              :include-unchanged="true"
              :display-mode="displayMode"
              header-layout="unity"
              hide-toolbar
            />
          </div>
        </div>
      </template>

      <div v-if="loading && payload" class="aph-loading-overlay">
        {{ t("asset.preview.loading") }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.aph-root {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
}
.aph-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 14px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--bg-color) 16%);
  flex-shrink: 0;
}
.aph-name {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}
.aph-path {
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}
.aph-spacer {
  flex: 1;
}
.aph-header-btn,
.aph-mode-segmented {
  flex-shrink: 0;
}
.aph-close {
  background: none;
  border: none;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 18px;
  line-height: 1;
  padding: 0 6px;
  flex-shrink: 0;
}
.aph-close:hover { color: var(--text-color); }
.aph-body {
  position: relative;
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
  background: var(--panel-bg);
}
.aph-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  font-size: 12px;
  color: var(--text-secondary);
}
.aph-error { color: var(--status-danger-fg); }
.aph-loading-overlay {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  font-size: 12px;
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--panel-bg) 84%, transparent);
  backdrop-filter: blur(2px);
  z-index: 1;
}
.aph-binary-wrap {
  flex: 1;
  display: flex;
  min-height: 0;
  overflow: hidden;
}
.aph-structured {
  flex: 1;
  display: flex;
  flex-direction: row;
  min-height: 0;
  overflow: hidden;
}
.aph-hierarchy {
  width: 32%;
  min-width: 240px;
  max-width: 360px;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color) 18%);
  overflow: hidden;
}
.aph-inspector {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  overflow: hidden;
}
.aph-placeholder {
  padding: 24px;
  text-align: center;
  color: var(--text-secondary);
  font-size: 12px;
}
</style>
