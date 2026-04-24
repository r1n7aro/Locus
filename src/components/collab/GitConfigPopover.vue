<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import type { SegmentedOption } from "../ui/BaseSegmented.vue";
import type { GitConfigEntry, GitConfigScope, GitConfigScopeSnapshot, GitConfigSnapshot } from "../../types";
import { gitConfigSnapshot, gitSaveConfig } from "../../services/git";
import { normalizeAppError } from "../../services/errors";
import { t } from "../../i18n";
import { gitConfigEntriesSignature, isSafeGitConfigKey, normalizeGitConfigEntries } from "./gitConfigEditor";
import BaseButton from "../ui/BaseButton.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";

interface EditableConfigEntry extends GitConfigEntry {
  id: string;
}

const props = defineProps<{
  open: boolean;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "saved", scope: GitConfigScope): void;
}>();

const activeScope = ref<GitConfigScope>("repo");
const loading = ref(false);
const saving = ref(false);
const error = ref("");
const savedMessage = ref("");
const snapshot = ref<GitConfigSnapshot | null>(null);
const originalEntries = ref<Record<GitConfigScope, GitConfigEntry[]>>({
  repo: [],
  global: [],
});
const draftEntries = ref<Record<GitConfigScope, EditableConfigEntry[]>>({
  repo: [],
  global: [],
});
let nextEntryId = 0;

const scopeOptions = computed<SegmentedOption[]>(() => [
  { value: "repo", label: t("git.config.repo") },
  { value: "global", label: t("git.config.global") },
]);

const activeSnapshot = computed<GitConfigScopeSnapshot | null>(() => {
  if (!snapshot.value) return null;
  return activeScope.value === "repo" ? snapshot.value.repo : snapshot.value.global;
});

const activeEntries = computed(() => draftEntries.value[activeScope.value]);
const invalidEntryIds = computed(() => {
  const invalid = new Set<string>();
  for (const entry of activeEntries.value) {
    if (!isSafeGitConfigKey(entry.key.trim())) invalid.add(entry.id);
  }
  return invalid;
});

const activeDirty = computed(() =>
  gitConfigEntriesSignature(activeEntries.value) !== gitConfigEntriesSignature(originalEntries.value[activeScope.value]),
);

const saveDisabled = computed(() =>
  loading.value || saving.value || !activeDirty.value || invalidEntryIds.value.size > 0,
);

function cloneEntries(entries: GitConfigEntry[]): EditableConfigEntry[] {
  return entries.map((entry) => ({
    id: `git-config-entry-${nextEntryId++}`,
    key: entry.key,
    value: entry.value,
  }));
}

async function loadConfig() {
  loading.value = true;
  error.value = "";
  savedMessage.value = "";
  try {
    const result = await gitConfigSnapshot();
    snapshot.value = result;
    originalEntries.value = {
      repo: result.repo.entries,
      global: result.global.entries,
    };
    draftEntries.value = {
      repo: cloneEntries(result.repo.entries),
      global: cloneEntries(result.global.entries),
    };
  } catch (e) {
    error.value = normalizeAppError(e).message;
  } finally {
    loading.value = false;
  }
}

function addEntry() {
  draftEntries.value[activeScope.value].push({
    id: `git-config-entry-${nextEntryId++}`,
    key: "",
    value: "",
  });
  savedMessage.value = "";
}

function removeEntry(id: string) {
  draftEntries.value[activeScope.value] = activeEntries.value.filter((entry) => entry.id !== id);
  savedMessage.value = "";
}

async function saveActiveScope() {
  if (saveDisabled.value) return;
  saving.value = true;
  error.value = "";
  savedMessage.value = "";
  try {
    const scope = activeScope.value;
    const result = await gitSaveConfig(scope, normalizeGitConfigEntries(draftEntries.value[scope]));
    if (snapshot.value) {
      snapshot.value = {
        ...snapshot.value,
        [scope]: result,
      };
    }
    originalEntries.value[scope] = result.entries;
    draftEntries.value[scope] = cloneEntries(result.entries);
    savedMessage.value = t("git.config.saved", t(scope === "repo" ? "git.config.repo" : "git.config.global"));
    emit("saved", scope);
  } catch (e) {
    error.value = normalizeAppError(e).message;
  } finally {
    saving.value = false;
  }
}

function requestClose() {
  if (saving.value) return;
  emit("close");
}

function onWindowKeydown(event: KeyboardEvent) {
  if (event.key === "Escape" && props.open) {
    requestClose();
  }
}

watch(
  () => props.open,
  async (open) => {
    if (!open) {
      window.removeEventListener("keydown", onWindowKeydown);
      return;
    }
    activeScope.value = "repo";
    window.addEventListener("keydown", onWindowKeydown);
    await loadConfig();
  },
);

watch(activeScope, () => {
  error.value = "";
  savedMessage.value = "";
});

onUnmounted(() => {
  window.removeEventListener("keydown", onWindowKeydown);
});
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="git-config-popover-layer"
      @mousedown.self="requestClose"
    >
      <section
        class="git-config-popover"
        role="dialog"
        aria-modal="true"
        :aria-label="t('git.config.editorTitle')"
        @mousedown.stop
      >
        <header class="git-config-popover-header">
          <div class="git-config-popover-heading">
            <div class="git-config-popover-title">{{ t("git.config.editorTitle") }}</div>
            <div class="git-config-popover-path">
              {{ activeSnapshot?.path || t("git.config.pathFallback") }}
            </div>
          </div>
          <button
            type="button"
            class="git-config-close-btn"
            :title="t('common.close')"
            :aria-label="t('common.close')"
            @click="requestClose"
          >
            &times;
          </button>
        </header>

        <div class="git-config-toolbar">
          <BaseSegmented v-model="activeScope" :options="scopeOptions" size="sm" />
          <BaseButton size="sm" :disabled="loading || saving" @click="loadConfig">
            {{ t("git.config.reload") }}
          </BaseButton>
          <BaseButton class="git-config-add-btn" size="sm" :disabled="loading || saving" @click="addEntry">
            {{ t("git.config.add") }}
          </BaseButton>
        </div>

        <div class="git-config-body">
          <div v-if="loading" class="git-config-state">{{ t("common.loading") }}</div>
          <div v-else-if="activeEntries.length === 0" class="git-config-state">{{ t("git.config.empty") }}</div>
          <table v-else class="git-config-table">
            <colgroup>
              <col class="git-config-key-col" />
              <col class="git-config-value-col" />
              <col class="git-config-action-col" />
            </colgroup>
            <thead>
              <tr>
                <th>{{ t("git.config.key") }}</th>
                <th>{{ t("git.config.value") }}</th>
                <th aria-hidden="true"></th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="entry in activeEntries" :key="entry.id">
                <td>
                  <input
                    v-model="entry.key"
                    class="git-config-cell-input git-config-key-input"
                    :class="{ invalid: invalidEntryIds.has(entry.id) }"
                    :title="invalidEntryIds.has(entry.id) ? t('git.config.invalidKey') : entry.key"
                    spellcheck="false"
                  />
                </td>
                <td>
                  <input
                    v-model="entry.value"
                    class="git-config-cell-input"
                    :title="entry.value"
                    spellcheck="false"
                  />
                </td>
                <td class="git-config-action-cell">
                  <button
                    type="button"
                    class="git-config-remove-btn"
                    :title="t('git.config.delete')"
                    :aria-label="t('git.config.delete')"
                    @click="removeEntry(entry.id)"
                  >
                    &times;
                  </button>
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <footer class="git-config-footer">
          <div class="git-config-feedback">
            <span v-if="error" class="git-config-error">{{ error }}</span>
            <span v-else-if="invalidEntryIds.size > 0" class="git-config-error">{{ t("git.config.invalidKey") }}</span>
            <span v-else-if="savedMessage" class="git-config-saved">{{ savedMessage }}</span>
          </div>
          <BaseButton size="sm" @click="requestClose">
            {{ t("common.cancel") }}
          </BaseButton>
          <BaseButton
            variant="primary"
            size="sm"
            :disabled="saveDisabled"
            @click="saveActiveScope"
          >
            {{ saving ? t("git.config.saving") : t("common.save") }}
          </BaseButton>
        </footer>
      </section>
    </div>
  </Teleport>
</template>

<style scoped>
.git-config-popover-layer {
  position: fixed;
  inset: 0;
  z-index: 9998;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background: rgba(0, 0, 0, 0.42);
  backdrop-filter: blur(3px);
  box-sizing: border-box;
}

.git-config-popover {
  position: relative;
  width: min(620px, calc(100vw - 48px));
  max-height: min(640px, calc(100vh - 48px));
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--panel-bg, var(--bg-color));
  box-shadow: 0 12px 36px rgba(0, 0, 0, 0.38);
  color: var(--text-color);
}

.git-config-popover-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 86%, var(--sidebar-bg) 14%);
}

.git-config-popover-heading {
  min-width: 0;
}

.git-config-popover-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.git-config-popover-path {
  margin-top: 2px;
  font-size: 11px;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono-identifier);
}

.git-config-close-btn {
  width: 24px;
  height: 24px;
  border: 1px solid transparent;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  line-height: 1;
}

.git-config-close-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
  border-color: var(--border-color);
}

.git-config-toolbar,
.git-config-footer {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
}

.git-config-add-btn {
  margin-left: auto;
}

.git-config-cell-input {
  min-width: 0;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg, var(--bg-color));
  color: var(--text-color);
  outline: none;
  box-sizing: border-box;
}

.git-config-cell-input:focus {
  border-color: var(--accent-color);
}

.git-config-body {
  flex: 1;
  min-height: 180px;
  overflow: auto;
}

.git-config-state {
  padding: 28px 12px;
  color: var(--text-secondary);
  font-size: 12px;
  text-align: center;
}

.git-config-table {
  width: 100%;
  table-layout: fixed;
  border-collapse: collapse;
  font-size: 12px;
}

.git-config-key-col {
  width: 42%;
}

.git-config-value-col {
  width: auto;
}

.git-config-action-col {
  width: 38px;
}

.git-config-table thead {
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--panel-bg, var(--bg-color));
}

.git-config-table th {
  height: 36px;
  padding: 0 10px;
  border-bottom: 1px solid var(--border-color);
  text-align: left;
  background: var(--panel-bg, var(--bg-color));
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
}

.git-config-table td {
  height: 40px;
  padding: 4px 10px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 70%, transparent);
}

.git-config-table tbody tr:hover {
  background: var(--hover-bg);
}

.git-config-cell-input {
  width: 100%;
  height: 28px;
  padding: 0 8px;
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}

.git-config-action-cell {
  text-align: center;
}

.git-config-key-input.invalid {
  border-color: var(--status-danger-border);
  color: var(--status-danger-fg);
}

.git-config-remove-btn {
  width: 26px;
  height: 26px;
  border: 1px solid transparent;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  line-height: 1;
}

.git-config-remove-btn:hover {
  border-color: var(--status-danger-border);
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.git-config-footer {
  justify-content: flex-end;
  border-top: 1px solid var(--border-color);
  border-bottom: none;
}

.git-config-feedback {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.git-config-error {
  color: var(--status-danger-fg);
}

.git-config-saved {
  color: var(--status-good-fg);
}
</style>
