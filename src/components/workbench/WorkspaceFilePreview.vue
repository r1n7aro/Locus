<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import { previewWorkspaceAsset, previewWorkspaceAssetTarget } from "../../services/asset";
import { projectExplorerPreviewFile } from "../../services/workspaceExplorer";
import type { AssetPreviewPayload, SemanticTargetInspector } from "../../types";
import type { ProjectExplorerFilePreview } from "../../types/workbench";
import AssetPreviewHost from "../asset/AssetPreviewHost.vue";
import AssetTextViewer from "../asset/AssetTextViewer.vue";

const props = defineProps<{
  projectId: string;
  path: string;
}>();

const preview = ref<ProjectExplorerFilePreview | null>(null);
const loading = ref(false);
const error = ref("");
const assetPayload = ref<AssetPreviewPayload | null>(null);
const activeTargetId = ref<string | null>(null);
const targetCache = ref<Map<string, SemanticTargetInspector>>(new Map());
const targetLoading = ref(false);
let requestEpoch = 0;

const language = computed(() => {
  const extension = preview.value?.extension ?? "";
  return ({
    ts: "typescript",
    tsx: "typescript",
    js: "javascript",
    jsx: "javascript",
    cs: "csharp",
    py: "python",
    rs: "rust",
    md: "markdown",
    yml: "yaml",
    sh: "bash",
    ps1: "powershell",
  } as Record<string, string>)[extension] ?? extension;
});

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

async function loadPreview(): Promise<void> {
  const epoch = ++requestEpoch;
  loading.value = true;
  error.value = "";
  preview.value = null;
  assetPayload.value = null;
  activeTargetId.value = null;
  targetCache.value = new Map();
  try {
    const next = await projectExplorerPreviewFile(props.projectId, props.path);
    if (epoch !== requestEpoch) return;
    preview.value = next;
    if (
      next.kind === "unity"
      && next.checkoutId
      && next.workspaceRelativePath
    ) {
      assetPayload.value = await previewWorkspaceAsset(
        next.workspaceRelativePath,
        undefined,
        {
          checkoutId: next.checkoutId,
          expectedGeneration: next.workspaceGeneration,
        },
      );
    }
  } catch (cause) {
    if (epoch !== requestEpoch) return;
    error.value = normalizeAppError(cause).message;
  } finally {
    if (epoch === requestEpoch) loading.value = false;
  }
}

async function loadTarget(
  previewKey: string,
  targetId: string,
): Promise<SemanticTargetInspector | null> {
  const file = preview.value;
  if (!file?.checkoutId) return null;
  const cached = targetCache.value.get(targetId);
  activeTargetId.value = targetId;
  if (cached) return cached;
  targetLoading.value = true;
  try {
    const inspector = await previewWorkspaceAssetTarget(
      previewKey,
      targetId,
      {
        checkoutId: file.checkoutId,
        expectedGeneration: file.workspaceGeneration,
      },
    );
    const next = new Map(targetCache.value);
    next.set(targetId, inspector);
    targetCache.value = next;
    return inspector;
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
    return null;
  } finally {
    targetLoading.value = false;
  }
}

watch(() => [props.projectId, props.path] as const, loadPreview, { immediate: true });
</script>

<template>
  <section class="workspace-file-preview">
    <div v-if="loading && !preview" class="workspace-file-preview-state">
      {{ t("development.preview.loading") }}
    </div>
    <div v-else-if="error && !preview" class="workspace-file-preview-state error">{{ error }}</div>

    <AssetPreviewHost
      v-else-if="preview?.kind === 'unity'"
      :payload="assetPayload"
      :loading="loading"
      :error="error"
      :selected-name="preview.name"
      :selected-path="preview.workspaceRelativePath || preview.path"
      :active-target-id="activeTargetId"
      :target-cache="targetCache"
      :target-loading="targetLoading"
      :load-target="loadTarget"
      :show-close="false"
    />

    <template v-else-if="preview">
      <header class="workspace-file-preview-header">
        <span>{{ preview.name }}</span>
        <span :title="preview.path">{{ preview.path }}</span>
      </header>
      <div class="workspace-file-preview-body">
        <AssetTextViewer
          v-if="preview.kind === 'text'"
          :snippet="preview.text || ''"
          :truncated="preview.truncated"
          :total-lines="preview.totalLines || 1"
          :language="language"
        />
        <div v-else-if="preview.kind === 'image'" class="workspace-media-preview image">
          <img :src="preview.dataUrl" :alt="preview.name" />
        </div>
        <iframe
          v-else-if="preview.kind === 'pdf'"
          class="workspace-pdf-preview"
          :src="preview.dataUrl"
          :title="preview.name"
        />
        <div v-else-if="preview.kind === 'audio'" class="workspace-media-preview">
          <audio :src="preview.dataUrl" controls />
        </div>
        <div v-else-if="preview.kind === 'video'" class="workspace-media-preview video">
          <video :src="preview.dataUrl" controls />
        </div>
        <div v-else class="workspace-binary-preview">
          <div>{{ preview.name }}</div>
          <dl>
            <dt>{{ t("development.preview.path") }}</dt><dd>{{ preview.path }}</dd>
            <dt>{{ t("development.preview.size") }}</dt><dd>{{ formatSize(preview.size) }}</dd>
            <dt>{{ t("development.preview.type") }}</dt><dd>{{ preview.mimeType }}</dd>
          </dl>
        </div>
      </div>
    </template>
  </section>
</template>

<style scoped>
.workspace-file-preview {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--panel-bg);
}

.workspace-file-preview-state {
  margin: auto;
  color: var(--text-secondary);
  font-size: 12px;
}

.workspace-file-preview-state.error {
  color: var(--status-error-fg, var(--text-color));
}

.workspace-file-preview-header {
  min-height: 38px;
  padding: 0 12px;
  display: flex;
  align-items: center;
  gap: 10px;
  border-bottom: 1px solid var(--border-color);
}

.workspace-file-preview-header > span:first-child {
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 600;
}

.workspace-file-preview-header > span:last-child {
  min-width: 0;
  overflow: hidden;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.workspace-file-preview-body {
  flex: 1;
  min-height: 0;
  display: flex;
}

.workspace-media-preview {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 18px;
  overflow: auto;
}

.workspace-media-preview.image img,
.workspace-media-preview.video video {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
}

.workspace-media-preview audio {
  width: min(520px, 100%);
}

.workspace-pdf-preview {
  flex: 1;
  width: 100%;
  min-height: 0;
  border: 0;
  background: var(--panel-bg);
}

.workspace-binary-preview {
  width: min(620px, calc(100% - 36px));
  margin: 24px auto auto;
  padding: 14px 16px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  font-size: 12px;
}

.workspace-binary-preview > div {
  margin-bottom: 10px;
  font-weight: 600;
}

.workspace-binary-preview dl {
  margin: 0;
  display: grid;
  grid-template-columns: 58px minmax(0, 1fr);
  gap: 6px 10px;
}

.workspace-binary-preview dt {
  color: var(--text-secondary);
}

.workspace-binary-preview dd {
  min-width: 0;
  margin: 0;
  overflow-wrap: anywhere;
  font-family: var(--font-mono-identifier);
}
</style>
