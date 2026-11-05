<script setup lang="ts">
// Asked on the first image paste of a project that has no image folder for
// the document: saves the project's default folder, or a rule for the
// document's folder, then lets the paste continue.
import { computed, nextTick, onMounted, ref } from "vue";
import { t } from "../../../i18n";
import type { WorkspaceRef } from "../../../services/project";
import {
  DEFAULT_MARKDOWN_IMAGE_DIRECTORY,
  MARKDOWN_IMAGE_DIRECTORY_KEY,
  MARKDOWN_IMAGE_RULES_KEY,
  markdownImageTarget,
  workspaceRelativeFolder,
  type MarkdownImageTarget,
} from "../../../services/markdownImage";
import { getSettings, setSettings, settingsErrorMessage } from "../../../services/settings";
import { hasTauriWindowRuntime } from "../../../services/tauriRuntime";
import BaseButton from "../BaseButton.vue";
import BaseCheckbox from "../BaseCheckbox.vue";
import MarkdownFloatingPanel from "./MarkdownFloatingPanel.vue";
import type { FloatingAnchorRect } from "./markdownFloating";

const props = defineProps<{
  workspaceRef: WorkspaceRef;
  contentPath: string;
  anchor: FloatingAnchorRect;
}>();

const emit = defineEmits<{ done: [saved: boolean] }>();

const target = ref<MarkdownImageTarget | null>(null);
const folder = ref(DEFAULT_MARKDOWN_IMAGE_DIRECTORY);
const onlyDocumentFolder = ref(false);
const busy = ref(true);
const error = ref("");
const inputRef = ref<HTMLInputElement | null>(null);
const canBrowse = hasTauriWindowRuntime();
const documentFolder = computed(() => target.value?.documentFolder ?? "");
let finished = false;

function finish(saved: boolean): void {
  if (finished) return;
  finished = true;
  emit("done", saved);
}

onMounted(async () => {
  try {
    const current = await markdownImageTarget(props.workspaceRef, props.contentPath);
    if (finished) return;
    // Another window may have chosen a folder in the meantime.
    if (current.folder) { finish(true); return; }
    target.value = current;
    busy.value = false;
    await nextTick();
    inputRef.value?.focus();
    inputRef.value?.select();
  } catch (cause) {
    error.value = settingsErrorMessage(cause);
    busy.value = false;
  }
});

async function browse(): Promise<void> {
  const root = target.value?.root;
  if (!root || busy.value) return;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const current = folder.value.trim().replace(/\\/g, "/");
  const selected = await open({ directory: true, multiple: false, defaultPath: current ? `${root}/${current}` : root });
  if (typeof selected !== "string" || !selected.trim()) return;
  const relative = workspaceRelativeFolder(root, selected);
  if (relative === null) {
    error.value = t("editor.imageFolder.outside");
    return;
  }
  folder.value = relative;
  error.value = "";
}

async function save(): Promise<void> {
  const value = folder.value.trim();
  if (busy.value || !target.value) return;
  if (!value) {
    error.value = t("editor.imageFolder.required");
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    if (onlyDocumentFolder.value && documentFolder.value) {
      const current = await getSettings([MARKDOWN_IMAGE_RULES_KEY], props.workspaceRef);
      const rules = current[MARKDOWN_IMAGE_RULES_KEY];
      await setSettings([{
        key: MARKDOWN_IMAGE_RULES_KEY,
        value: { ...(rules && typeof rules === "object" ? rules : {}), [documentFolder.value]: value },
      }], props.workspaceRef);
    } else {
      await setSettings([{ key: MARKDOWN_IMAGE_DIRECTORY_KEY, value }], props.workspaceRef);
    }
    finish(true);
  } catch (cause) {
    error.value = settingsErrorMessage(cause);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <MarkdownFloatingPanel :anchor="anchor" :width="340" :aria-label="t('editor.imageFolder.title')" @close="finish(false)">
    <form class="image-folder-popover" :aria-busy="busy" @submit.prevent="save">
      <div class="image-folder-title">{{ t("editor.imageFolder.title") }}</div>
      <p class="image-folder-desc">{{ t("editor.imageFolder.desc") }}</p>
      <div class="image-folder-field">
        <input
          ref="inputRef"
          v-model="folder"
          class="image-folder-input"
          type="text"
          spellcheck="false"
          autocomplete="off"
          :placeholder="DEFAULT_MARKDOWN_IMAGE_DIRECTORY"
          :aria-label="t('editor.imageFolder.folder')"
          :disabled="busy && !target"
        />
        <BaseButton v-if="canBrowse" :disabled="busy || !target" @click="browse">{{ t("editor.imageFolder.browse") }}</BaseButton>
      </div>
      <label v-if="documentFolder" class="image-folder-scope">
        <BaseCheckbox v-model="onlyDocumentFolder" :disabled="busy" :aria-label="t('editor.imageFolder.onlyFolder', documentFolder)" />
        <span>{{ t("editor.imageFolder.onlyFolder", documentFolder) }}</span>
      </label>
      <p v-if="error" class="image-folder-error" role="alert">{{ error }}</p>
      <p class="image-folder-hint">{{ t("editor.imageFolder.hint") }}</p>
      <div class="image-folder-actions">
        <BaseButton @click="finish(false)">{{ t("common.cancel") }}</BaseButton>
        <BaseButton variant="primary" type="submit" :disabled="busy || !target">{{ t("editor.imageFolder.save") }}</BaseButton>
      </div>
    </form>
  </MarkdownFloatingPanel>
</template>

<style scoped>
.image-folder-popover {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0;
  padding: 10px 12px;
  font-family: var(--font-ui);
  font-size: 12px;
  line-height: 1.5;
}

.image-folder-title {
  color: var(--text-color);
  font-size: 13px;
  font-weight: 600;
}

.image-folder-desc,
.image-folder-hint,
.image-folder-error {
  margin: 0;
}

.image-folder-desc,
.image-folder-hint {
  color: var(--text-secondary);
}

.image-folder-error {
  color: var(--status-danger-fg);
  overflow-wrap: anywhere;
}

.image-folder-field {
  display: flex;
  align-items: center;
  gap: 6px;
}

.image-folder-input {
  flex: 1;
  min-width: 0;
  height: 28px;
  padding: 0 8px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg, var(--bg-color));
  color: var(--text-color);
  font-family: var(--font-mono-editor);
  font-size: 12px;
  outline: none;
  box-sizing: border-box;
}

.image-folder-input:focus {
  border-color: var(--accent-border, var(--accent-color));
}

.image-folder-scope {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  color: var(--text-color);
  cursor: pointer;
  overflow-wrap: anywhere;
}

.image-folder-actions {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
}
</style>
