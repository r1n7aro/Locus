<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { RefreshCw, RotateCcw } from "lucide";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import { readRule, saveRule, readWorkspaceAgentDocument, saveWorkspaceAgentDocument, type AgentDocumentKind } from "../../services/agent";
import type { WorkspaceRef } from "../../services/project";
import BaseButton from "../ui/BaseButton.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";
import BaseMarkdownEditor from "../ui/BaseMarkdownEditor.vue";
import LucideIcon from "../icons/LucideIcon.vue";
import { useMarkdownEditorViewMode } from "../ui/markdownEditorViewMode";
import { MarkdownEditorSessionCache } from "../ui/markdown-editor/markdownEditorSessionCache";

export interface AgentDocumentTarget {
  kind: AgentDocumentKind | "rule";
  name: string;
  title: string;
}
interface Draft {
  target: AgentDocumentTarget;
  workspaceRef: WorkspaceRef;
  agentId: string;
  content: string;
  saved: string;
  revision: string | null;
  path: string;
  saving: boolean;
}
const props = defineProps<{
  target: AgentDocumentTarget | null;
  workspaceRef: WorkspaceRef | null;
  workingDir: string;
  agentId: string;
  active?: boolean;
}>();
const emit = defineEmits<{ dirtyChange: [dirty: boolean]; saved: []; details: [] }>();
const drafts = reactive(new Map<string, Draft>());
const editorSessions = new MarkdownEditorSessionCache();
const key = computed(() => props.target && props.workspaceRef
  ? `${props.workspaceRef.checkoutId}:${props.workspaceRef.expectedGeneration}:${props.workspaceRef.expectedMaterializationEpoch}:${props.agentId}:${props.target.kind}:${props.target.name}` : "");
const draft = computed(() => drafts.get(key.value));
const error = ref("");
const loading = ref(false);
let request = 0;
const dirty = computed(() => [...drafts.values()].some(item => item.content !== item.saved || item.saving));
watch(dirty, value => emit("dirtyChange", value), { immediate: true });
const { markdownEditorViewMode, setMarkdownEditorViewMode } = useMarkdownEditorViewMode();
const modes = computed(() => [
  { value: "rendered", label: t("knowledge.editor.view.rendered") },
  { value: "native", label: t("knowledge.editor.view.native") },
]);

async function load(force = false) {
  const id = ++request;
  error.value = "";
  loading.value = false;
  const target = props.target;
  const workspaceRef = props.workspaceRef;
  if (!target || !workspaceRef) return;
  const documentKey = key.value;
  if (!force && drafts.has(documentKey)) return;
  const agentId = props.agentId;
  const workingDir = props.workingDir;
  loading.value = true;
  try {
    const document = target.kind === "rule"
      ? { content: await readRule(workspaceRef, agentId, target.name), revision: null,
        path: `${workingDir}/Locus/agent/${agentId}/rule/${target.name}` }
      : await readWorkspaceAgentDocument(workspaceRef, agentId, target.kind, target.name);
    if (id !== request) return;
    drafts.set(documentKey, { target: { ...target }, workspaceRef: { ...workspaceRef }, agentId,
      content: document.content, saved: document.content, revision: document.revision, path: document.path, saving: false });
  } catch (e) {
    if (id === request) error.value = normalizeAppError(e).message;
  } finally {
    if (id === request) loading.value = false;
  }
}
watch(key, () => { void load(); }, { immediate: true });

async function saveDraft(item: Draft): Promise<boolean> {
  if (item.saving) return false;
  if (item.content === item.saved) return true;
  item.saving = true;
  error.value = "";
  const content = item.content;
  try {
    if (item.target.kind === "rule") {
      await saveRule(item.workspaceRef, item.agentId, item.target.name, content, item.saved);
    } else {
      const result = await saveWorkspaceAgentDocument(item.workspaceRef, item.agentId, item.target.kind, item.target.name, content, item.revision);
      item.revision = result.revision;
    }
    item.saved = content;
    emit("saved");
    return item.content === item.saved;
  } catch (e) {
    error.value = normalizeAppError(e).message;
    return false;
  } finally { item.saving = false; }
}
async function saveFile(): Promise<boolean> {
  for (const item of drafts.values()) {
    if (!await saveDraft(item)) return false;
  }
  return true;
}
function forgetRule(name: string) {
  for (const [draftKey, item] of drafts) {
    if (item.agentId === props.agentId && item.target.kind === "rule" && item.target.name === name) drafts.delete(draftKey);
  }
}
defineExpose({ saveFile, forgetRule });
</script>

<template>
  <section class="agent-document-editor">
    <header class="document-toolbar">
      <span class="document-title">{{ target?.title }}</span>
      <span v-if="draft && draft.content !== draft.saved" class="document-dirty">{{ t('agent.editor.unsaved') }}</span>
      <span class="toolbar-spacer" />
      <slot name="actions" />
      <BaseButton v-if="target?.kind === 'tool' || target?.kind === 'env'" size="sm" @click="emit('details')">{{ t('agent.editor.details') }}</BaseButton>
      <BaseSegmented :model-value="markdownEditorViewMode" :options="modes" size="sm" @update:model-value="setMarkdownEditorViewMode($event === 'native' ? 'native' : 'rendered')" />
      <BaseButton class="document-icon" size="sm" :title="t('common.refresh')" :aria-label="t('common.refresh')" :disabled="loading || !!draft?.saving || (!!draft && draft.content !== draft.saved)" @click="load(true)"><LucideIcon :icon="RefreshCw" :size="14" /></BaseButton>
      <BaseButton v-if="draft && draft.content !== draft.saved" class="document-icon" size="sm" :title="t('agent.editor.discard')" :aria-label="t('agent.editor.discard')" :disabled="draft.saving" @click="draft.content = draft.saved"><LucideIcon :icon="RotateCcw" :size="14" /></BaseButton>
      <BaseButton size="sm" :disabled="!draft || draft.saving || draft.content === draft.saved" @click="draft && saveDraft(draft)">{{ t('common.save') }}</BaseButton>
    </header>
    <div v-if="error" class="document-error" role="alert">{{ error }}</div>
    <div v-if="loading" class="document-loading">{{ t('common.loading') }}</div>
    <BaseMarkdownEditor v-else-if="draft" v-model="draft.content" class="document-body"
      :session-cache="editorSessions" :session-pinned="draft.content !== draft.saved"
      :content-key="key" :content-path="target?.kind === 'tool' ? `${draft.path}.md` : draft.path"
      :workspace-ref="draft.workspaceRef" :active="active !== false && !!target" :view-mode="markdownEditorViewMode"
      @shortcut-save="saveDraft(draft)" />
    <footer v-if="draft" class="document-footer" :title="draft.path">
      <span>{{ t('agent.editor.projectFile') }}</span>
      <span class="document-path">{{ draft.path }}</span>
    </footer>
  </section>
</template>

<style scoped>
.agent-document-editor { display: flex; flex: 1; flex-direction: column; min-width: 0; min-height: 0; background: var(--panel-bg); }
.document-toolbar { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; min-height: 40px; padding: 6px 12px; border-bottom: 1px solid var(--border-color); }
.document-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; font-weight: 600; }
.document-dirty, .document-footer { font-size: 11px; color: var(--text-secondary); }
.toolbar-spacer { flex: 1; }
.document-icon { width: 28px; min-width: 28px; padding: 0; border-color: transparent; }
.document-body { flex: 1; min-height: 0; }
.document-error { padding: 8px 12px; color: var(--status-danger-fg); font-size: 12px; }
.document-loading { padding: 16px; color: var(--text-secondary); }
.document-footer { display: flex; align-items: center; gap: 12px; padding: 6px 12px; border-top: 1px solid var(--border-color); }
.document-path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font-mono-identifier); }
</style>
