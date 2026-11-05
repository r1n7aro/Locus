<script setup lang="ts">
import { computed, ref, watch, nextTick } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { Folder, Plus, X } from "lucide";
import { t } from "../i18n";
import { useWorkspaceSetup } from "../composables/useWorkspaceSetup";
import { normalizeAppError } from "../services/errors";
import BaseButton from "./ui/BaseButton.vue";
import BaseCheckbox from "./ui/BaseCheckbox.vue";
import BaseSegmented from "./ui/BaseSegmented.vue";
import type { WorkspaceServiceSetup, WorkspaceServiceSetupMode } from "../services/project";
import LucideIcon from "./icons/LucideIcon.vue";

const { request, saving, cancel, save } = useWorkspaceSetup();
const dialog = ref<HTMLDialogElement | null>(null);
const error = ref("");
const pickingFolder = ref(false);
watch(request, async current => {
  error.value = "";
  if (!current) return;
  await nextTick();
  dialog.value?.showModal();
});
const serviceModeOptions = computed(() => (["auto", "lazy", "off"] as const).map(mode => ({
  value: mode,
  label: t(`workspace.setup.serviceMode.${mode}`),
  hint: t(`workspace.setup.serviceMode.${mode}Hint`),
})));
const unityServiceOff = computed(() => (request.value?.setup.services ?? [])
  .some(entry => entry.service === "unity" && entry.mode === "off"));
function serviceLabel(entry: WorkspaceServiceSetup): string {
  return t(entry.service === "unity" ? "settings.tab.unity" : "settings.tab.blender");
}
function setServiceMode(entry: WorkspaceServiceSetup, mode: string) {
  if (saving.value) return;
  entry.mode = mode as WorkspaceServiceSetupMode;
}
function shortName(path: string): string {
  return path.replace(/\\/g, "/").split("/").filter(Boolean).pop() || path;
}
function normalizedPath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
}
async function addFolder() {
  const current = request.value;
  if (!current || saving.value || pickingFolder.value) return;
  pickingFolder.value = true;
  error.value = "";
  try {
    const selected = await open({ directory: true, multiple: false, defaultPath: current.path });
    if (request.value !== current || typeof selected !== "string") return;
    const key = normalizedPath(selected);
    const root = normalizedPath(current.path);
    if (key === root || key.startsWith(`${root}/`)) {
      error.value = t("extraWorkdirs.insideWorkspace");
      return;
    }
    const entries = current.setup.extraWorkdirs ?? [];
    if (entries.some(entry => normalizedPath(entry.path) === key)) {
      error.value = t("extraWorkdirs.duplicate");
      return;
    }
    current.setup.extraWorkdirs = [...entries, { path: selected, comment: "", readOnly: false, exists: true }];
  } catch (cause) {
    if (request.value === current) error.value = normalizeAppError(cause).message;
  } finally {
    pickingFolder.value = false;
  }
}
function removeFolder(index: number) {
  if (!request.value || saving.value) return;
  request.value.setup.extraWorkdirs?.splice(index, 1);
  error.value = "";
}
async function submit() {
  if (pickingFolder.value) return;
  error.value = "";
  try { await save(); }
  catch (cause) { error.value = normalizeAppError(cause).message; }
}
</script>

<template>
  <Teleport to="body">
    <dialog v-if="request" ref="dialog" class="workspace-setup-dialog" aria-labelledby="workspace-setup-title"
      @cancel.prevent="cancel" @click.self="cancel">
      <form @submit.prevent="submit">
        <header id="workspace-setup-title">{{ t(request.mode === 'add' ? 'workspace.setup.add' : 'workspace.setup.title') }}</header>
        <div class="workspace-setup-body">
          <div class="workspace-setup-path" :title="request.path">{{ request.path }}</div>
          <div class="workspace-setup-type">{{ t(request.setup.isUnityProject ? 'workspace.setup.unity' : 'workspace.setup.folder') }}</div>
          <label class="workspace-setup-option">
            <BaseCheckbox v-model="request.setup.knowledgeEnabled" :disabled="saving" :aria-label="t('workspace.setup.knowledge')" />
            <span>{{ t('workspace.setup.knowledge') }}</span>
          </label>
          <label v-if="request.setup.isUnityProject && !unityServiceOff" class="workspace-setup-option">
            <BaseCheckbox v-model="request.setup.unityTestToolsEnabled" :disabled="saving" :aria-label="t('workspace.setup.unityTests')" />
            <span>{{ t('workspace.setup.unityTests') }}</span>
          </label>
          <section v-if="request.setup.services?.length" class="workspace-setup-services" aria-labelledby="workspace-setup-services-title">
            <h2 id="workspace-setup-services-title">{{ t('workspace.setup.services') }}</h2>
            <div v-for="entry in request.setup.services" :key="entry.service" class="workspace-setup-service">
              <span class="workspace-setup-service-name">{{ serviceLabel(entry) }}</span>
              <BaseSegmented size="sm" :model-value="entry.mode" :options="serviceModeOptions"
                :aria-label="serviceLabel(entry)" @update:model-value="setServiceMode(entry, $event)" />
            </div>
          </section>
          <section class="workspace-setup-directories" aria-labelledby="workspace-setup-directories-title">
            <div class="workspace-setup-section-header">
              <h2 id="workspace-setup-directories-title">{{ t('extraWorkdirs.windowTitle') }}</h2>
              <BaseButton :disabled="saving || pickingFolder" @click="addFolder">
                <LucideIcon :icon="Plus" :size="13" />
                {{ t('extraWorkdirs.addFolder') }}
              </BaseButton>
            </div>
            <p v-if="!request.setup.extraWorkdirs?.length" class="workspace-setup-empty">{{ t('extraWorkdirs.empty') }}</p>
            <div v-for="(entry, index) in request.setup.extraWorkdirs" :key="entry.path" class="workspace-setup-directory">
              <div class="workspace-setup-directory-main">
                <LucideIcon :icon="Folder" :size="14" />
                <div class="workspace-setup-directory-text" :title="entry.path">
                  <span class="workspace-setup-directory-name">{{ shortName(entry.path) }}</span>
                  <span class="workspace-setup-directory-path">{{ entry.path }}</span>
                </div>
                <span v-if="!entry.exists" class="workspace-setup-directory-missing">{{ t('extraWorkdirs.missingBadge') }}</span>
                <BaseButton class="workspace-setup-remove" :disabled="saving"
                  :aria-label="`${t('extraWorkdirs.remove')} ${shortName(entry.path)}`"
                  :title="t('extraWorkdirs.remove')" @click="removeFolder(index)">
                  <LucideIcon :icon="X" :size="14" />
                </BaseButton>
              </div>
              <div class="workspace-setup-directory-fields">
                <input v-model="entry.comment" type="text" :disabled="saving" spellcheck="false"
                  :aria-label="`${shortName(entry.path)} ${t('extraWorkdirs.commentPlaceholder')}`"
                  :placeholder="t('extraWorkdirs.commentPlaceholder')" />
                <label class="workspace-setup-readonly" :title="t('extraWorkdirs.readOnlyHint')">
                  <BaseCheckbox v-model="entry.readOnly" :disabled="saving" :aria-label="`${shortName(entry.path)} ${t('extraWorkdirs.readOnly')}`" />
                  <span>{{ t('extraWorkdirs.readOnly') }}</span>
                </label>
              </div>
            </div>
          </section>
          <p v-if="error" class="workspace-setup-error" role="alert">{{ error }}</p>
        </div>
        <footer>
          <BaseButton :disabled="saving" @click="cancel">{{ t('common.cancel') }}</BaseButton>
          <BaseButton type="submit" :disabled="saving || pickingFolder">{{ t(saving ? 'common.loading' : request.mode === 'add' ? 'workspace.setup.open' : 'common.save') }}</BaseButton>
        </footer>
      </form>
    </dialog>
  </Teleport>
</template>

<style scoped>
.workspace-setup-dialog { width: 620px; max-width: calc(100vw - 48px); max-height: calc(100vh - 48px); margin: auto; padding: 0; border: 1px solid var(--border-color); border-radius: 10px; background: var(--panel-bg); color: var(--text-color); font-family: var(--font-ui); overflow: hidden; }
.workspace-setup-dialog::backdrop { background: color-mix(in srgb, var(--bg-color) 55%, transparent); }
form { display: flex; flex-direction: column; max-height: calc(100vh - 50px); }
header { flex-shrink: 0; padding: 14px 20px; border-bottom: 1px solid var(--border-color); font-size: 14px; font-weight: 600; }
.workspace-setup-body { min-height: 0; overflow: auto; padding: 16px 20px 20px; }
.workspace-setup-path { overflow-wrap: anywhere; font-family: var(--font-mono-identifier); font-size: 12px; }
.workspace-setup-type { margin: 6px 0 20px; font-size: 12px; color: var(--text-secondary); }
.workspace-setup-option { display: flex; align-items: center; gap: 10px; margin-top: 12px; font-size: 13px; cursor: pointer; }
.workspace-setup-error { margin: 12px 0 0; color: var(--status-danger-fg); font-size: 12px; }
footer { display: flex; flex-shrink: 0; justify-content: flex-end; gap: 8px; padding: 12px 20px; border-top: 1px solid var(--border-color); }
.workspace-setup-services { margin-top: 20px; padding-top: 16px; border-top: 1px solid var(--border-color); }
.workspace-setup-service { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-top: 12px; font-size: 13px; }
.workspace-setup-directories { margin-top: 20px; padding-top: 16px; border-top: 1px solid var(--border-color); }
.workspace-setup-section-header { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
h2 { margin: 0; font-size: 13px; font-weight: 600; }
.workspace-setup-empty { margin: 16px 0 0; font-size: 12px; color: var(--text-secondary); }
.workspace-setup-directory { padding: 12px 0; border-bottom: 1px solid var(--border-color); }
.workspace-setup-directory:last-child { padding-bottom: 0; border-bottom: none; }
.workspace-setup-directory-main { display: flex; align-items: center; gap: 8px; min-width: 0; color: var(--text-secondary); }
.workspace-setup-directory-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
.workspace-setup-directory-name { font-size: 12px; font-weight: 500; color: var(--text-color); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.workspace-setup-directory-path { font-size: 11px; font-family: var(--font-mono-identifier); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.workspace-setup-directory-missing { font-size: 11px; color: var(--status-danger-fg); }
.workspace-setup-remove { flex-shrink: 0; width: 28px; padding: 0; border-color: transparent; }
.workspace-setup-directory-fields { display: flex; align-items: center; gap: 12px; margin-top: 8px; }
.workspace-setup-directory-fields input { flex: 1; min-width: 0; padding: 6px 8px; border: 1px solid var(--border-color); border-radius: 6px; background: var(--input-bg, var(--bg-color)); color: var(--text-color); font-family: var(--font-ui); font-size: 12px; }
.workspace-setup-directory-fields input:focus-visible { outline: 1px solid var(--accent-color); }
.workspace-setup-directory-fields input:disabled { opacity: 0.5; }
.workspace-setup-readonly { display: inline-flex; align-items: center; gap: 5px; flex-shrink: 0; font-size: 12px; color: var(--text-secondary); cursor: pointer; }
</style>
