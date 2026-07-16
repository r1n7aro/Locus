<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import BaseButton from "./ui/BaseButton.vue";
import {
  extraWorkdirsGet,
  extraWorkdirsSet,
  type ExtraWorkdirStatus,
} from "../services/extraWorkdirs";
import {
  broadcastExtraWorkdirsUpdated,
  EXTRA_WORKDIRS_PAYLOAD_EVENT,
  EXTRA_WORKDIRS_WINDOW_LABEL,
  getExtraWorkdirsWindowPayload,
  type ExtraWorkdirsWindowPayload,
} from "../services/extraWorkdirsWindow";
import { getSubWindowClaimedQuery } from "../services/subWindow";

const appWindow = getCurrentWindow();
const workspacePath = ref(getExtraWorkdirsWindowPayload().workspacePath);
const entries = ref<ExtraWorkdirStatus[]>([]);
const loading = ref(false);
const saving = ref(false);
const loadError = ref("");
const formNotice = ref("");
let payloadEventUnlisten: UnlistenFn | null = null;
let payloadEventSeen = false;
// Generation guard: on rapid workspace re-assignments two loads can be in
// flight at once; only the latest one may commit entries/loading state,
// otherwise a late response from workspace A overwrites workspace B's form.
let loadEntriesRequestId = 0;

const workspaceName = computed(() => {
  const parts = workspacePath.value.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : workspacePath.value;
});

function shortName(path: string): string {
  const parts = path.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : path;
}

function normalizedKey(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
}

async function loadEntries() {
  if (!workspacePath.value) return;
  const requestId = ++loadEntriesRequestId;
  loading.value = true;
  loadError.value = "";
  formNotice.value = "";
  try {
    const next = await extraWorkdirsGet(workspacePath.value);
    if (requestId !== loadEntriesRequestId) return;
    entries.value = next;
  } catch (error) {
    if (requestId !== loadEntriesRequestId) return;
    const err = normalizeAppError(error);
    loadError.value = err.message;
  } finally {
    if (requestId === loadEntriesRequestId) {
      loading.value = false;
    }
  }
}

async function applyPayload(payload: ExtraWorkdirsWindowPayload) {
  const nextPath = payload.workspacePath?.trim() || "";
  if (!nextPath || nextPath === workspacePath.value) return;
  workspacePath.value = nextPath;
  await loadEntries();
}

async function addFolder() {
  if (saving.value) return;
  formNotice.value = "";
  let selected: string | string[] | null = null;
  try {
    selected = await open({
      directory: true,
      multiple: false,
      defaultPath: workspacePath.value || undefined,
    });
  } catch (error) {
    const err = normalizeAppError(error);
    formNotice.value = err.message;
    return;
  }
  if (!selected || typeof selected !== "string") return;

  const key = normalizedKey(selected);
  const workspaceKey = normalizedKey(workspacePath.value);
  if (key === workspaceKey || key.startsWith(`${workspaceKey}/`)) {
    formNotice.value = t("extraWorkdirs.insideWorkspace");
    return;
  }
  if (entries.value.some((entry) => normalizedKey(entry.path) === key)) {
    formNotice.value = t("extraWorkdirs.duplicate");
    return;
  }
  entries.value = [...entries.value, { path: selected, comment: "", exists: true }];
}

function removeEntry(index: number) {
  if (saving.value) return;
  formNotice.value = "";
  entries.value = entries.value.filter((_, i) => i !== index);
}

async function save() {
  if (saving.value || loading.value || !workspacePath.value) return;
  saving.value = true;
  formNotice.value = "";
  try {
    const saved = await extraWorkdirsSet(
      workspacePath.value,
      entries.value.map((entry) => ({ path: entry.path, comment: entry.comment })),
    );
    entries.value = saved;
    await broadcastExtraWorkdirsUpdated(workspacePath.value);
    await closeWindow();
  } catch (error) {
    const err = normalizeAppError(error);
    formNotice.value = t("extraWorkdirs.saveFailed", err.message);
  } finally {
    saving.value = false;
  }
}

async function closeWindow() {
  try {
    await appWindow.close();
    return;
  } catch {
    // fall through to destroy
  }
  try {
    await appWindow.destroy();
  } catch {
    // ignore teardown failures
  }
}

onMounted(() => {
  void loadEntries();
  void appWindow
    .listen<ExtraWorkdirsWindowPayload>(EXTRA_WORKDIRS_PAYLOAD_EVENT, (event) => {
      payloadEventSeen = true;
      void applyPayload(event.payload ?? { workspacePath: "" });
    })
    .then(async (dispose) => {
      payloadEventUnlisten = dispose;
      // A re-open of this window kind may have emitted its payload before
      // the listener above was live; pull the latest recorded query to
      // recover it. Skip when an event already arrived — it is at least as
      // new as the pulled snapshot.
      const query = await getSubWindowClaimedQuery(EXTRA_WORKDIRS_WINDOW_LABEL).catch(() => null);
      if (!query || payloadEventSeen) return;
      await applyPayload(getExtraWorkdirsWindowPayload(`?${query}`));
    })
    .catch(() => {
      // keep the window usable even if event hooks are unavailable
    });
});

onUnmounted(() => {
  payloadEventUnlisten?.();
});
</script>

<template>
  <div class="extra-workdirs-window-root">
    <div class="extra-workdirs-titlebar">
      <div class="extra-workdirs-titlebar-label">{{ t("extraWorkdirs.windowTitle") }}</div>
      <div class="extra-workdirs-titlebar-actions">
        <div class="extra-workdirs-titlebar-meta" :title="workspacePath">{{ workspaceName }}</div>
        <button
          type="button"
          class="extra-workdirs-window-close"
          :aria-label="t('common.close')"
          :title="t('common.close')"
          @click="void closeWindow()"
        >
          <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14" aria-hidden="true">
            <path d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06z"/>
          </svg>
        </button>
      </div>
    </div>

    <div class="extra-workdirs-body">
      <p class="extra-workdirs-description">{{ t("extraWorkdirs.description") }}</p>
      <div class="extra-workdirs-workspace" :title="workspacePath">
        <svg viewBox="0 0 16 16" fill="currentColor" width="12" height="12" aria-hidden="true">
          <path d="M1 3.5A1.5 1.5 0 0 1 2.5 2h3.879a1.5 1.5 0 0 1 1.06.44l1.122 1.12A1.5 1.5 0 0 0 9.62 4H13.5A1.5 1.5 0 0 1 15 5.5v7a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 1 12.5v-9z"/>
        </svg>
        <span class="extra-workdirs-workspace-path">{{ workspacePath }}</span>
      </div>

      <div v-if="loadError" class="extra-workdirs-error">{{ loadError }}</div>
      <div v-else-if="loading" class="extra-workdirs-empty">{{ t("common.loading") }}</div>
      <template v-else>
        <div v-if="entries.length === 0" class="extra-workdirs-empty">
          {{ t("extraWorkdirs.empty") }}
        </div>
        <div v-else class="extra-workdirs-list">
          <div v-for="(entry, index) in entries" :key="entry.path" class="extra-workdirs-row">
            <div class="extra-workdirs-row-main">
              <svg class="extra-workdirs-row-icon" viewBox="0 0 16 16" fill="currentColor" width="12" height="12" aria-hidden="true">
                <path d="M1 3.5A1.5 1.5 0 0 1 2.5 2h3.879a1.5 1.5 0 0 1 1.06.44l1.122 1.12A1.5 1.5 0 0 0 9.62 4H13.5A1.5 1.5 0 0 1 15 5.5v7a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 1 12.5v-9z"/>
              </svg>
              <div class="extra-workdirs-row-text" :title="entry.path">
                <span class="extra-workdirs-row-name">{{ shortName(entry.path) }}</span>
                <span class="extra-workdirs-row-path">{{ entry.path }}</span>
              </div>
              <span v-if="!entry.exists" class="extra-workdirs-missing-badge">
                {{ t("extraWorkdirs.missingBadge") }}
              </span>
              <button
                type="button"
                class="extra-workdirs-row-remove"
                :disabled="saving"
                :aria-label="t('extraWorkdirs.remove')"
                :title="t('extraWorkdirs.remove')"
                @click="removeEntry(index)"
              >
                <svg viewBox="0 0 16 16" fill="currentColor" width="12" height="12" aria-hidden="true">
                  <path d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06z"/>
                </svg>
              </button>
            </div>
            <input
              v-model="entry.comment"
              class="extra-workdirs-comment-input"
              type="text"
              :placeholder="t('extraWorkdirs.commentPlaceholder')"
              :disabled="saving"
              spellcheck="false"
            />
          </div>
        </div>

        <div class="extra-workdirs-add">
          <BaseButton :disabled="saving" @click="void addFolder()">
            <svg viewBox="0 0 16 16" fill="currentColor" width="12" height="12" aria-hidden="true">
              <path d="M8 2a.75.75 0 0 1 .75.75v4.5h4.5a.75.75 0 0 1 0 1.5h-4.5v4.5a.75.75 0 0 1-1.5 0v-4.5h-4.5a.75.75 0 0 1 0-1.5h4.5v-4.5A.75.75 0 0 1 8 2z"/>
            </svg>
            {{ t("extraWorkdirs.addFolder") }}
          </BaseButton>
        </div>
      </template>
    </div>

    <div class="extra-workdirs-footer">
      <div class="extra-workdirs-form-notice">{{ formNotice }}</div>
      <div class="extra-workdirs-footer-actions">
        <BaseButton :disabled="saving" @click="void closeWindow()">
          {{ t("common.cancel") }}
        </BaseButton>
        <BaseButton variant="primary" :disabled="saving || loading || !!loadError" @click="void save()">
          {{ t("common.save") }}
        </BaseButton>
      </div>
    </div>
  </div>
</template>

<style scoped>
.extra-workdirs-window-root {
  position: relative;
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--bg-color) 6%);
  border: 1px solid var(--border-strong);
  box-shadow:
    inset 0 1px 0 color-mix(in srgb, white 8%, transparent),
    inset 0 0 0 1px color-mix(in srgb, var(--border-strong) 82%, transparent);
  overflow: hidden;
}

.extra-workdirs-titlebar {
  -webkit-app-region: drag;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 38px;
  padding: 0 14px;
  background: var(--sidebar-bg);
  border-bottom: 1px solid var(--border-color);
  box-shadow: inset 0 1px 0 color-mix(in srgb, white 6%, transparent);
}

.extra-workdirs-titlebar-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.extra-workdirs-titlebar-actions {
  min-width: 0;
  -webkit-app-region: no-drag;
  display: flex;
  align-items: center;
  gap: 12px;
}

.extra-workdirs-titlebar-meta {
  font-size: 11px;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.extra-workdirs-window-close {
  -webkit-app-region: no-drag;
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}

.extra-workdirs-window-close:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.extra-workdirs-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.extra-workdirs-description {
  margin: 0;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.extra-workdirs-workspace {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--sidebar-bg);
  color: var(--text-secondary);
  font-size: 11px;
  font-family: var(--font-mono-identifier);
}

.extra-workdirs-workspace-path {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.extra-workdirs-error {
  font-size: 12px;
  color: var(--error-color, #e5534b);
}

.extra-workdirs-empty {
  padding: 20px 0;
  text-align: center;
  font-size: 12px;
  color: var(--text-secondary);
}

.extra-workdirs-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.extra-workdirs-row {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 80%, var(--bg-color) 20%);
}

.extra-workdirs-row-main {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.extra-workdirs-row-icon {
  flex-shrink: 0;
  color: var(--text-secondary);
}

.extra-workdirs-row-text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.extra-workdirs-row-name {
  font-size: 12px;
  font-weight: 500;
  color: var(--text-color);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.extra-workdirs-row-path {
  font-size: 10px;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.extra-workdirs-missing-badge {
  flex-shrink: 0;
  font-size: 10px;
  padding: 2px 6px;
  border-radius: 4px;
  color: var(--error-color, #e5534b);
  border: 1px solid color-mix(in srgb, var(--error-color, #e5534b) 45%, transparent);
  background: color-mix(in srgb, var(--error-color, #e5534b) 10%, transparent);
}

.extra-workdirs-row-remove {
  flex-shrink: 0;
  width: 24px;
  height: 24px;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}

.extra-workdirs-row-remove:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--error-color, #e5534b);
}

.extra-workdirs-row-remove:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.extra-workdirs-comment-input {
  width: 100%;
  box-sizing: border-box;
  padding: 6px 8px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--bg-color);
  color: var(--text-color);
  font-size: 12px;
  font-family: var(--font-ui);
  outline: none;
}

.extra-workdirs-comment-input:focus {
  border-color: var(--accent-color);
}

.extra-workdirs-comment-input:disabled {
  opacity: 0.6;
}

.extra-workdirs-add {
  display: flex;
}

.extra-workdirs-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 16px;
  border-top: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}

.extra-workdirs-form-notice {
  min-width: 0;
  font-size: 11px;
  color: var(--error-color, #e5534b);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.extra-workdirs-footer-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
</style>
