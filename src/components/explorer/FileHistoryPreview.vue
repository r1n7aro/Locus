<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, ref, watch } from "vue";
import { t } from "../../i18n";
import type { FileHistoryPreview } from "../../services/fileHistory";
import type { WorkspaceRef } from "../../services/project";
import { previewWorkspaceAssetTarget } from "../../services/asset";
import { releasePreviewBlob } from "../../services/workspaceExplorer";
import type { SemanticTargetInspector } from "../../types";
import { parseCsvDocument } from "../../document/csv/csvDocument";
import { defaultCsvView, parseCsvView, reconcileCsvView } from "../../document/csv/csvView";
import AssetTextViewer from "../asset/AssetTextViewer.vue";
import AssetPreviewHost from "../asset/AssetPreviewHost.vue";

const props = defineProps<{ preview: FileHistoryPreview; workspaceRef: WorkspaceRef | null }>();
const BaseMarkdownEditor = defineAsyncComponent(() => import("../ui/BaseMarkdownEditor.vue"));
const CsvGrid = defineAsyncComponent(() => import("../csv/CsvGrid.vue"));
const file = computed(() => props.preview.file);
const csv = computed(() => {
  if (file.value.extension !== "csv" || file.value.kind !== "text" || file.value.truncated) return null;
  try {
    const document = parseCsvDocument(file.value.text ?? "");
    let view = defaultCsvView();
    if (props.preview.csvView) {
      try { view = parseCsvView(props.preview.csvView); } catch { /* Older versions may lack compatible view settings. */ }
    }
    return { document, view: reconcileCsvView(view, document), error: "" };
  } catch (error) { return { document: null, view: null, error: t(String(error instanceof Error ? error.message : error)) }; }
});
// The snapshot's media bytes sit in the backend binary cache; free them with the view.
const blobUrl = computed(() => file.value.blobUrl ?? "");
watch(blobUrl, (_next, previous) => { if (previous) void releasePreviewBlob(previous); });
onBeforeUnmount(() => { if (blobUrl.value) void releasePreviewBlob(blobUrl.value); });
const targetCache = ref(new Map<string, SemanticTargetInspector>());
const activeTarget = ref<string | null>(null);
const targetLoading = ref(false);
const targetError = ref("");
let request = 0;
watch(() => props.preview, () => {
  request++;
  targetCache.value = new Map();
  activeTarget.value = null;
  targetLoading.value = false;
  targetError.value = "";
});
async function loadTarget(previewKey: string, targetId: string): Promise<SemanticTargetInspector | null> {
  if (!props.workspaceRef) return null;
  const id = ++request;
  activeTarget.value = targetId;
  targetLoading.value = true;
  targetError.value = "";
  try {
    const value = await previewWorkspaceAssetTarget(previewKey, targetId, props.workspaceRef);
    if (id !== request) return null;
    targetCache.value.set(targetId, value);
    return value;
  } catch (error) {
    if (id === request) targetError.value = String(error);
    return null;
  } finally { if (id === request) targetLoading.value = false; }
}
</script>

<template>
  <div class="history-preview">
    <p v-if="targetError" class="history-preview-error" role="alert">{{ targetError }}</p>
    <AssetPreviewHost v-if="preview.asset" :payload="preview.asset" :loading="false" error=""
      :selected-name="file.name" selected-path="" :show-close="false" :show-identity="false"
      :active-target-id="activeTarget" :target-cache="targetCache" :target-loading="targetLoading" :load-target="loadTarget" />
    <template v-else-if="csv">
      <CsvGrid v-if="csv.document && csv.view" :document="csv.document" :view="csv.view" read-only />
      <p v-else class="history-preview-error" role="alert">{{ csv.error }}</p>
    </template>
    <div v-else-if="file.kind === 'text' && /^(md|markdown)$/.test(file.extension)" class="history-markdown">
      <BaseMarkdownEditor :model-value="file.text ?? ''" :content-path="file.path" :content-key="`history:${preview.contentHash}`"
        :workspace-ref="workspaceRef" view-mode="rendered" disabled show-properties auto-grow />
      <p v-if="file.truncated" class="history-preview-note">{{ t('fileHistory.truncated') }}</p>
    </div>
    <AssetTextViewer v-else-if="file.kind === 'text'" :snippet="file.text ?? ''" :truncated="file.truncated"
      :total-lines="file.totalLines ?? 0" :language="file.extension" />
    <img v-else-if="file.kind === 'image' && file.blobUrl" class="history-image" :src="file.blobUrl" :alt="file.name" />
    <iframe v-else-if="file.kind === 'pdf' && file.blobUrl" class="history-media" :src="file.blobUrl" :title="file.name" />
    <audio v-else-if="file.kind === 'audio' && file.blobUrl" controls :src="file.blobUrl" />
    <video v-else-if="file.kind === 'video' && file.blobUrl" class="history-media" controls :src="file.blobUrl" />
    <p v-else class="history-preview-note">{{ t('fileHistory.unsupported') }}</p>
  </div>
</template>

<style scoped>
.history-preview { flex: 1; display: flex; flex-direction: column; min-height: 0; min-width: 0; overflow: hidden; }
.history-markdown { flex: 1; overflow: auto; padding: 24px 32px; }
.history-image { min-height: 0; width: 100%; height: 100%; object-fit: contain; }
.history-media { flex: 1; width: 100%; min-height: 0; border: 0; }
audio { margin: auto; max-width: 100%; }
.history-preview-note, .history-preview-error { margin: auto; padding: 20px; color: var(--text-secondary); font-size: 12px; }
.history-preview-error { color: var(--status-danger-fg); }
</style>
