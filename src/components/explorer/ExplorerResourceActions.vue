<script setup lang="ts">
import { nextTick, ref } from "vue";
import { confirm, save } from "@tauri-apps/plugin-dialog";
import { t } from "../../i18n";
import { useNotificationStore } from "../../stores/notification";
import { useWorkspaceExplorerStore } from "../../stores/workspaceExplorer";
import { normalizeAppError } from "../../services/errors";
import { explorerFileAction } from "../../services/workspaceExplorer";
import { deleteSkillPackage, exportSkillPackage, knowledgeDelete, knowledgeMove, knowledgeRevealTarget } from "../../services/knowledge";
import { showInFolder } from "../../services/unity";
import { isSkillPackageRootDocument, skillPackageIdForDocument } from "../../composables/useKnowledgeState";
import { useExplorerPathDisplay } from "../../composables/useExplorerPathDisplay";
import { knowledgeResourceManagedHint } from "../knowledge/knowledgeResourceActions";
import { resourceName, resourceRelativePath, validResourceName, type ExplorerResourceAction, type ExplorerResourceTarget } from "./explorerResourceActions";
import BaseButton from "../ui/BaseButton.vue";

const emit = defineEmits<{ changed: [target: ExplorerResourceTarget, newPath: string | null] }>();
const notificationStore = useNotificationStore();
const explorerStore = useWorkspaceExplorerStore();
const { movePathPreference } = useExplorerPathDisplay();
const draft = ref<{ target: ExplorerResourceTarget; name: string; busy: boolean; error: string } | null>(null);
const input = ref<HTMLInputElement | null>(null);
const inputId = `resource-rename-${crypto.randomUUID()}`;
function cancel() { if (!draft.value?.busy) draft.value = null; }
function containFocus(event: KeyboardEvent) {
  const controls = (event.currentTarget as HTMLElement).querySelectorAll<HTMLElement>("input:not(:disabled), button:not(:disabled)");
  const first = controls[0];
  const last = controls[controls.length - 1];
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
}

async function mutate(target: ExplorerResourceTarget, name: string | null) {
  const document = target.document;
  if (document && knowledgeResourceManagedHint(document)) return;
  let nextPath: string | null = null;
  if (document) {
    if (!target.workspaceRef) throw new Error(t("development.workspaceUnavailable"));
    if (name !== null) {
      nextPath = document.path.replace(/[^/\\]+$/, name);
      await knowledgeMove({ kind: "document", type: document.type, path: document.path, newPath: nextPath }, target.workspaceRef);
    } else if (isSkillPackageRootDocument(document)) {
      await deleteSkillPackage(skillPackageIdForDocument(document)!, target.workspaceRef);
    } else {
      await knowledgeDelete({ kind: "document", type: document.type, path: document.path }, target.workspaceRef);
    }
  } else {
    nextPath = await explorerFileAction(target.projectId, target.path, name, target.mounted ? null : target.workspaceRef);
    if (nextPath) movePathPreference(target.path, nextPath);
  }
  emit("changed", target, nextPath);
  // A refresh failure must not present an already completed rename as a failed
  // mutation or invite a retry against the old path.
  try {
    if (document) await explorerStore.refreshProjectKnowledge(target.projectId);
    else await explorerStore.loadProject(target.projectId, true);
    const mounts = explorerStore.snapshots[target.projectId]?.nodes.filter((node) => node.nodeKind === "folder" && node.sourcePath) ?? [];
    const results = await Promise.allSettled(mounts.map((node) => explorerStore.loadMount(target.projectId, node.nodeId, true)));
    const failure = results.find((result) => result.status === "rejected");
    if (failure?.status === "rejected") throw failure.reason;
  } catch (error) { notificationStore.addNotice("warning", normalizeAppError(error).message); }
}

async function commit() {
  const current = draft.value;
  if (!current || current.busy) return;
  const name = current.name.trim();
  if (!validResourceName(name)) { current.error = t("development.invalidFileName"); return; }
  if (name === resourceName(current.target)) { draft.value = null; return; }
  current.busy = true;
  current.error = "";
  try { await mutate(current.target, name); draft.value = null; }
  catch (error) { current.error = normalizeAppError(error).message; }
  finally { current.busy = false; }
}

async function run(action: ExplorerResourceAction, target: ExplorerResourceTarget) {
  if ((action === "rename" || action === "delete") && target.document && knowledgeResourceManagedHint(target.document)) return;
  try {
    if (action === "rename") {
      if (target.document && isSkillPackageRootDocument(target.document)) return;
      draft.value = { target, name: resourceName(target), busy: false, error: "" };
      await nextTick();
      input.value?.focus();
      const dot = draft.value.name.lastIndexOf(".");
      input.value?.setSelectionRange(0, dot > 0 ? dot : draft.value.name.length);
    } else if (action === "delete") {
      const packageRoot = target.document && isSkillPackageRootDocument(target.document);
      const approved = await confirm(t(packageRoot ? "knowledge.explorer.deletePackageConfirm" : "knowledge.explorer.deleteDocumentConfirm", resourceName(target)), {
        title: t("common.confirmDelete"), kind: "warning", okLabel: t("common.delete"), cancelLabel: t("common.cancel"),
      });
      if (approved) await mutate(target, null);
    } else if (action === "copy") {
      await navigator.clipboard.writeText(resourceRelativePath(target));
      notificationStore.addNotice("success", t("knowledge.explorer.relativePathCopied"));
    } else if (action === "reveal" && target.workspaceRef) {
      if (target.document) await knowledgeRevealTarget({ kind: "document", docType: target.document.type, path: target.document.path }, target.workspaceRef);
      else await showInFolder(target.workspaceRef, target.path);
    } else if (action === "export" && target.document && target.workspaceRef) {
      const packageId = skillPackageIdForDocument(target.document);
      if (!packageId || target.document.externalSource?.locator?.startsWith("external://")) return;
      const path = await save({ defaultPath: `${packageId.replace(/[/\\]/g, "-")}.zip`, filters: [{ name: t("knowledge.skillPackage.archiveFilter"), extensions: ["zip"] }] });
      if (path) await exportSkillPackage(packageId, path, target.workspaceRef);
    }
  } catch (error) { notificationStore.addNotice("error", normalizeAppError(error).message); }
}
defineExpose({ run });
</script>

<template>
  <Teleport to="body">
    <div v-if="draft" class="resource-rename-backdrop" @click.self="cancel" @keydown.esc.prevent.stop="cancel">
      <form class="resource-rename-dialog" role="dialog" aria-modal="true" :aria-label="t('common.rename')" @submit.prevent="commit" @keydown.tab="containFocus">
        <label :for="inputId">{{ t('common.rename') }}</label>
        <input :id="inputId" ref="input" v-model="draft.name" :disabled="draft.busy" :aria-label="t('common.rename')" />
        <div v-if="draft.error" class="resource-rename-error" role="alert">{{ draft.error }}</div>
        <div class="resource-rename-actions">
          <BaseButton :disabled="draft.busy" @click="cancel">{{ t('common.cancel') }}</BaseButton>
          <BaseButton type="submit" :disabled="draft.busy || !draft.name.trim()">{{ t('common.confirm') }}</BaseButton>
        </div>
      </form>
    </div>
  </Teleport>
</template>

<style scoped>
.resource-rename-backdrop { position: fixed; inset: 0; z-index: 1000; display: grid; place-items: center; background: color-mix(in srgb, var(--bg-color) 44%, transparent); }
.resource-rename-dialog { display: flex; flex-direction: column; gap: 12px; width: 360px; max-width: calc(100vw - 32px); padding: 16px; border: 1px solid var(--border-color); border-radius: 8px; background: var(--panel-bg); color: var(--text-color); font-size: 13px; }
.resource-rename-dialog input { width: 100%; box-sizing: border-box; padding: 6px 8px; border: 1px solid var(--border-color); border-radius: 4px; background: var(--bg-color); color: var(--text-color); font: inherit; }
.resource-rename-dialog input:focus { outline: 1px solid var(--accent-color); }
.resource-rename-actions { display: flex; justify-content: flex-end; gap: 8px; }
.resource-rename-error { color: var(--status-danger-fg); overflow-wrap: anywhere; }
</style>
