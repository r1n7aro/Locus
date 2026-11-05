<script setup lang="ts">
// Project settings for images pasted into Markdown and knowledge documents:
// the default workspace-relative folder and per-folder rules
// (`project.markdownImages.*`, stored in the project's Locus/config.json).
import { computed, ref, watch } from "vue";
import { FolderOpen, Plus, Trash2 } from "lucide";
import { t } from "../../i18n";
import { useProjectSetting } from "../../composables/useProjectSetting";
import {
  MARKDOWN_IMAGE_DIRECTORY_KEY,
  MARKDOWN_IMAGE_RULES_KEY,
  workspaceRelativeFolder,
} from "../../services/markdownImage";
import { settingsErrorMessage } from "../../services/settings";
import { hasTauriWindowRuntime } from "../../services/tauriRuntime";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import LucideIcon from "../icons/LucideIcon.vue";
import BaseButton from "../ui/BaseButton.vue";

const props = withDefaults(defineProps<{ disabled?: boolean }>(), { disabled: false });

type Rules = Record<string, string>;

function readRules(raw: unknown): Rules {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
  return Object.fromEntries(Object.entries(raw).filter((entry): entry is [string, string] => typeof entry[1] === "string"));
}

const workspaceContext = useWorkspaceContextStore();
const root = computed(() => (workspaceContext.focusedWorkspaceRef ? workspaceContext.focusedRoot : ""));
const directory = useProjectSetting<string>(MARKDOWN_IMAGE_DIRECTORY_KEY, (raw) => (typeof raw === "string" ? raw : ""));
const rules = useProjectSetting<Rules>(MARKDOWN_IMAGE_RULES_KEY, readRules);
const canBrowse = hasTauriWindowRuntime();

const directoryInactive = computed(() => props.disabled || !directory.ready.value || directory.busy.value);
const rulesInactive = computed(() => props.disabled || !rules.ready.value || rules.busy.value);
// Write errors are kept here so that they clear once the input is valid
// again (also when it returns to the saved value); load errors come from
// the setting itself.
const directoryError = ref("");
const rulesError = ref("");
const directoryMessage = computed(() => directoryError.value || (directory.ready.value ? "" : directory.error.value ?? ""));
const rulesMessage = computed(() => rulesError.value || (rules.ready.value ? "" : rules.error.value ?? ""));

interface RuleRow {
  id: number;
  documents: string;
  images: string;
}

let nextRowId = 0;
const rows = ref<RuleRow[]>([]);
const complete = (row: RuleRow) => Boolean(row.documents.trim() && row.images.trim());

const folderKey = (value: string) => value.trim().replace(/\\/g, "/").split("/")
  .filter((part) => part && part !== ".").join("/").toLowerCase();

/**
 * Applies saved rules to the rows without reordering them (the backend
 * stores rules sorted): matching rows take the normalized values, rules
 * that are gone drop their rows, new rules are appended, and rows still
 * being filled in stay where they are.
 */
function syncRows(saved: Rules): void {
  const remaining = new Map(Object.entries(saved).map(([documents, images]) => [folderKey(documents), { documents, images }]));
  const next: RuleRow[] = [];
  for (const row of rows.value) {
    if (!complete(row)) { next.push(row); continue; }
    const match = remaining.get(folderKey(row.documents));
    if (!match) continue;
    remaining.delete(folderKey(row.documents));
    next.push({ ...row, ...match });
  }
  for (const rule of remaining.values()) next.push({ id: nextRowId++, ...rule });
  rows.value = next;
}

// While this component writes, the optimistic value and a rollback must not
// replace what the user typed; the saved result is applied afterwards.
let committing = false;
watch(() => rules.value.value, (saved) => { if (!committing) syncRows(saved); }, { immediate: true, flush: "sync" });

async function writeDirectory(next: string): Promise<void> {
  directoryError.value = "";
  if (next === directory.value.value) return;
  // Failures roll back the input and show the backend reason inline.
  await directory.set(next).catch((cause: unknown) => { directoryError.value = settingsErrorMessage(cause); });
}

function commitDirectory(event: Event): void {
  void writeDirectory((event.target as HTMLInputElement).value.trim());
}

/** Picks a folder inside the focused workspace; `null` when cancelled. */
async function pickFolder(current: string, report: (message: string) => void): Promise<string | null> {
  const base = root.value.replace(/\\/g, "/");
  if (!base) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const start = current.trim().replace(/\\/g, "/");
  const selected = await open({ directory: true, multiple: false, defaultPath: start ? `${base}/${start}` : base });
  if (typeof selected !== "string" || !selected.trim()) return null;
  const relative = workspaceRelativeFolder(base, selected);
  if (relative === null) {
    report(t("editor.imageFolder.outside"));
    return null;
  }
  return relative;
}

async function browseDirectory(): Promise<void> {
  if (directoryInactive.value) return;
  const picked = await pickFolder(directory.value.value, (message) => { directoryError.value = message; });
  if (picked !== null) await writeDirectory(picked);
}

async function commitRules(): Promise<void> {
  const incomplete = rows.value.some((row) => !complete(row) && (row.documents.trim() || row.images.trim()));
  rulesError.value = incomplete ? t("settings.project.markdownImages.incomplete") : "";
  const next = Object.fromEntries(rows.value.filter(complete).map((row) => [row.documents.trim(), row.images.trim()]));
  const saved = rules.value.value;
  const unchanged = Object.keys(next).length === Object.keys(saved).length
    && Object.entries(next).every(([documents, images]) => saved[documents] === images);
  if (unchanged) return;
  committing = true;
  try {
    await rules.set(next);
    syncRows(rules.value.value);
  } catch (cause) {
    // The rows keep the user's input next to the backend reason.
    rulesError.value = settingsErrorMessage(cause);
  } finally {
    committing = false;
  }
}

async function browseRule(row: RuleRow, field: "documents" | "images"): Promise<void> {
  if (rulesInactive.value) return;
  const picked = await pickFolder(row[field], (message) => { rulesError.value = message; });
  if (picked === null) return;
  row[field] = picked;
  await commitRules();
}

function addRule(): void {
  rows.value.push({ id: nextRowId++, documents: "", images: "" });
}

function removeRule(row: RuleRow): void {
  rows.value = rows.value.filter((item) => item.id !== row.id);
  void commitRules();
}

// Leaving a rule row (not moving between its own fields) saves the rules.
function onRuleFocusOut(event: FocusEvent): void {
  const row = event.currentTarget as HTMLElement;
  if (event.relatedTarget instanceof Node && row.contains(event.relatedTarget)) return;
  void commitRules();
}
</script>

<template>
  <div
    class="markdown-image-setting"
    :data-setting-key="MARKDOWN_IMAGE_DIRECTORY_KEY"
    :aria-busy="!directory.ready.value || directory.busy.value"
  >
    <div class="markdown-image-text">
      <div class="markdown-image-label">{{ t(`settings.item.${MARKDOWN_IMAGE_DIRECTORY_KEY}.label`) }}</div>
      <div class="markdown-image-desc">{{ t(`settings.item.${MARKDOWN_IMAGE_DIRECTORY_KEY}.desc`) }}</div>
      <div v-if="directoryMessage" class="markdown-image-error" role="alert">{{ directoryMessage }}</div>
    </div>
    <div class="markdown-image-field">
      <input
        class="markdown-image-input"
        type="text"
        spellcheck="false"
        autocomplete="off"
        :value="directory.value.value"
        :placeholder="t('settings.project.markdownImages.unset')"
        :disabled="directoryInactive"
        :aria-label="t(`settings.item.${MARKDOWN_IMAGE_DIRECTORY_KEY}.label`)"
        @change="commitDirectory"
        @keydown.enter="($event.target as HTMLInputElement).blur()"
      />
      <BaseButton v-if="canBrowse" :disabled="directoryInactive || !root" @click="browseDirectory">
        {{ t("editor.imageFolder.browse") }}
      </BaseButton>
    </div>
  </div>

  <div
    class="markdown-image-setting"
    :data-setting-key="MARKDOWN_IMAGE_RULES_KEY"
    :aria-busy="!rules.ready.value || rules.busy.value"
  >
    <div class="markdown-image-text">
      <div class="markdown-image-label">{{ t(`settings.item.${MARKDOWN_IMAGE_RULES_KEY}.label`) }}</div>
      <div class="markdown-image-desc">{{ t(`settings.item.${MARKDOWN_IMAGE_RULES_KEY}.desc`) }}</div>
      <div v-if="rulesMessage" class="markdown-image-error" role="alert">{{ rulesMessage }}</div>
    </div>
    <div v-if="rows.length" class="markdown-image-rules" role="list">
      <div class="markdown-image-rule markdown-image-rule-head" aria-hidden="true">
        <span>{{ t("settings.project.markdownImages.documentFolder") }}</span>
        <span />
        <span>{{ t("settings.project.markdownImages.imageFolder") }}</span>
      </div>
      <div
        v-for="row in rows"
        :key="row.id"
        class="markdown-image-rule"
        role="listitem"
        @focusout="onRuleFocusOut"
      >
        <div class="markdown-image-field">
          <input
            v-model="row.documents"
            class="markdown-image-input"
            type="text"
            spellcheck="false"
            autocomplete="off"
            placeholder="docs/design"
            :disabled="rulesInactive"
            :aria-label="t('settings.project.markdownImages.documentFolder')"
            @keydown.enter="($event.target as HTMLInputElement).blur()"
          />
          <button
            v-if="canBrowse"
            type="button"
            class="markdown-image-icon-button"
            :title="t('editor.imageFolder.browse')"
            :aria-label="t('editor.imageFolder.browse')"
            :disabled="rulesInactive || !root"
            @click="browseRule(row, 'documents')"
          >
            <LucideIcon :icon="FolderOpen" :size="13" />
          </button>
        </div>
        <span class="markdown-image-arrow" aria-hidden="true">→</span>
        <div class="markdown-image-field">
          <input
            v-model="row.images"
            class="markdown-image-input"
            type="text"
            spellcheck="false"
            autocomplete="off"
            placeholder="docs/design/images"
            :disabled="rulesInactive"
            :aria-label="t('settings.project.markdownImages.imageFolder')"
            @keydown.enter="($event.target as HTMLInputElement).blur()"
          />
          <button
            v-if="canBrowse"
            type="button"
            class="markdown-image-icon-button"
            :title="t('editor.imageFolder.browse')"
            :aria-label="t('editor.imageFolder.browse')"
            :disabled="rulesInactive || !root"
            @click="browseRule(row, 'images')"
          >
            <LucideIcon :icon="FolderOpen" :size="13" />
          </button>
        </div>
        <button
          type="button"
          class="markdown-image-icon-button"
          :title="t('settings.project.markdownImages.removeRule')"
          :aria-label="t('settings.project.markdownImages.removeRule')"
          :disabled="rulesInactive"
          @click="removeRule(row)"
        >
          <LucideIcon :icon="Trash2" :size="13" />
        </button>
      </div>
    </div>
    <div>
      <BaseButton :disabled="rulesInactive" @click="addRule">
        <LucideIcon :icon="Plus" :size="13" />
        {{ t("settings.project.markdownImages.addRule") }}
      </BaseButton>
    </div>
  </div>
</template>

<style scoped>
.markdown-image-setting {
  display: flex;
  flex-direction: column;
  gap: 8px;
  max-width: 760px;
  padding: 10px 0;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 70%, transparent);
}

.markdown-image-text {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.markdown-image-label {
  color: var(--text-color);
  font-size: 13px;
}

.markdown-image-desc {
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.45;
}

.markdown-image-error {
  color: var(--status-danger-fg);
  font-size: 12px;
  line-height: 1.45;
}

.markdown-image-field {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  max-width: 560px;
}

.markdown-image-input {
  flex: 1;
  min-width: 0;
  height: 30px;
  padding: 0 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 13px;
  font-family: inherit;
  outline: none;
  box-sizing: border-box;
  transition: border-color 0.15s ease, background 0.15s ease;
}

.markdown-image-input:focus {
  border-color: var(--accent-border);
  background: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

.markdown-image-input:disabled {
  opacity: 0.6;
}

.markdown-image-rules {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-width: 760px;
}

.markdown-image-rule {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr) 28px;
  align-items: center;
  gap: 6px;
}

.markdown-image-rule-head {
  color: var(--text-secondary);
  font-size: 12px;
}

.markdown-image-arrow {
  color: var(--text-secondary);
  font-size: 12px;
}

.markdown-image-icon-button {
  display: inline-flex;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.markdown-image-icon-button:hover:not(:disabled),
.markdown-image-icon-button:focus-visible {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
  outline: none;
}

.markdown-image-icon-button:disabled {
  cursor: default;
  opacity: 0.5;
}
</style>
